// =============================================================================
// verum-techempower-loadgen
// =============================================================================
//
// A closed-loop HTTP/1.1 load generator that hits a single URL with N
// concurrent workers for a fixed duration and reports throughput +
// latency percentiles. Designed for the TechEmpower R23 acceptance
// gates in `internal/specs/net-framework.md` §1.1.
//
// Pure Rust + tokio + hyper — no external load-gen binary required, so
// it runs identically in CI, on a developer laptop, or inside the
// VCS test runner. Reports come back as JSON, suitable for `jq`-based
// gate enforcement in shell scripts.
//
// Closed-loop mode (the only mode here) keeps `concurrency` requests
// in flight at all times. To match a wrk2-style open-loop / rate-
// limited workload, add a `--rate` flag — left as v0.2 follow-up.
//
// Why hyper over reqwest: reqwest pulls in TLS, cookies, and
// follow-redirects machinery we don't need. Direct hyper client over
// a raw TCP TcpStream is the closest equivalent to the C-level
// `wrk` baseline.
// =============================================================================

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use hyper_util::rt::TokioExecutor;
use serde::Serialize;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(name = "loadgen", version, about = "Closed-loop HTTP/1.1 load-gen for Verum TechEmpower")]
struct Args {
    /// Target URL (e.g. http://127.0.0.1:8080/plaintext)
    #[arg(long, default_value = "http://127.0.0.1:8080/plaintext")]
    url: String,

    /// Concurrent in-flight requests
    #[arg(long, default_value_t = 64)]
    concurrency: usize,

    /// Test duration in seconds (excludes warmup)
    #[arg(long, default_value_t = 10)]
    duration_secs: u64,

    /// Warmup duration in seconds before measurement
    #[arg(long, default_value_t = 2)]
    warmup_secs: u64,

    /// Output format: human | json
    #[arg(long, default_value = "human")]
    output: String,
}

#[derive(Serialize)]
struct Report {
    url: String,
    concurrency: usize,
    duration_secs: u64,
    warmup_secs: u64,
    requests: u64,
    errors: u64,
    rps: f64,
    p50_us: u64,
    p90_us: u64,
    p99_us: u64,
    p999_us: u64,
    p9999_us: u64,
    p100_us: u64,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    let url: hyper::Uri = args.url.parse().context("invalid URL")?;
    let host = url.host().context("URL missing host")?.to_string();
    let port = url.port_u16().unwrap_or(80);
    let authority = format!("{}:{}", host, port);

    if url.scheme_str() != Some("http") {
        anyhow::bail!("only http:// supported in v0.1; got scheme {:?}", url.scheme_str());
    }

    let connector = HttpConnector::new();
    let client: Client<HttpConnector, Empty<Bytes>> =
        Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(args.concurrency * 2)
            .http1_max_buf_size(8 * 1024)
            .build(connector);
    let client = Arc::new(client);

    let stop = Arc::new(AtomicBool::new(false));
    let measuring = Arc::new(AtomicBool::new(false));
    let total_requests = Arc::new(AtomicU64::new(0));
    let total_errors = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::with_capacity(1_000_000)));

    eprintln!(
        "loadgen → {} concurrency={} warmup={}s measure={}s",
        args.url, args.concurrency, args.warmup_secs, args.duration_secs
    );

    let workers: Vec<_> = (0..args.concurrency)
        .map(|_| {
            let client = client.clone();
            let stop = stop.clone();
            let measuring = measuring.clone();
            let total_requests = total_requests.clone();
            let total_errors = total_errors.clone();
            let latencies = latencies.clone();
            let url = args.url.clone();
            let authority = authority.clone();

            tokio::spawn(async move {
                while !stop.load(Ordering::Relaxed) {
                    let req = match Request::builder()
                        .method("GET")
                        .uri(&url)
                        .header(hyper::header::HOST, &authority)
                        .body(Empty::<Bytes>::new())
                    {
                        Ok(r) => r,
                        Err(_) => {
                            total_errors.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                    };

                    let measure = measuring.load(Ordering::Relaxed);
                    let start = if measure { Some(Instant::now()) } else { None };

                    match client.request(req).await {
                        Ok(resp) => {
                            // Drain the body — for plaintext it's 13B,
                            // but we still must consume it to free the
                            // connection back into the pool.
                            let (parts, body) = resp.into_parts();
                            let _ = body.collect().await;
                            if !parts.status.is_success() {
                                total_errors.fetch_add(1, Ordering::Relaxed);
                            } else if let Some(start) = start {
                                let elapsed = start.elapsed().as_micros() as u64;
                                latencies.lock().await.push(elapsed);
                                total_requests.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(_) => {
                            total_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        })
        .collect();

    // Warmup
    if args.warmup_secs > 0 {
        tokio::time::sleep(Duration::from_secs(args.warmup_secs)).await;
    }

    // Measurement window
    measuring.store(true, Ordering::Relaxed);
    let measure_start = Instant::now();
    tokio::time::sleep(Duration::from_secs(args.duration_secs)).await;
    measuring.store(false, Ordering::Relaxed);
    let measure_elapsed = measure_start.elapsed();

    // Stop workers
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        let _ = w.await;
    }

    // Compile report
    let mut lat = latencies.lock().await;
    lat.sort_unstable();
    let count = lat.len();
    let pct = |p: f64| -> u64 {
        if count == 0 {
            0
        } else {
            let idx = ((count as f64) * p) as usize;
            lat[idx.min(count - 1)]
        }
    };

    let requests = total_requests.load(Ordering::Relaxed);
    let errors = total_errors.load(Ordering::Relaxed);
    let rps = (requests as f64) / measure_elapsed.as_secs_f64();

    let report = Report {
        url: args.url.clone(),
        concurrency: args.concurrency,
        duration_secs: args.duration_secs,
        warmup_secs: args.warmup_secs,
        requests,
        errors,
        rps,
        p50_us: pct(0.50),
        p90_us: pct(0.90),
        p99_us: pct(0.99),
        p999_us: pct(0.999),
        p9999_us: pct(0.9999),
        p100_us: pct(1.0),
    };

    if args.output == "json" {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("================ loadgen report ================");
        println!("url:            {}", report.url);
        println!("concurrency:    {}", report.concurrency);
        println!("duration:       {} s (warmup {} s)", report.duration_secs, report.warmup_secs);
        println!("requests:       {}", report.requests);
        println!("errors:         {}", report.errors);
        println!("rps:            {:.0}", report.rps);
        println!("p50:            {} us", report.p50_us);
        println!("p90:            {} us", report.p90_us);
        println!("p99:            {} us", report.p99_us);
        println!("p99.9:          {} us", report.p999_us);
        println!("p99.99:         {} us", report.p9999_us);
        println!("p100 (max):     {} us", report.p100_us);
        println!("================================================");
    }

    if errors > 0 {
        std::process::exit(2);
    }
    Ok(())
}
