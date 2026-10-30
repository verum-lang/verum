use crate::{Config, Studio};
use anyhow::Result;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use verum_common::Text;

pub async fn serve(config: Config) -> Result<()> {
    serve_with_library(config, None).await
}

pub async fn serve_with_library(
    config: Config,
    library: Option<Arc<verum_lsp::library_metadata::LibraryMetadata>>,
) -> Result<()> {
    let port = config.port;
    let studio = Arc::new(Studio::open_with_library(config, library)?);
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let host: Text = listener.local_addr()?.to_string().into();
    let token: Text = uuid::Uuid::new_v4().to_string().into();
    println!("Verum Studio: http://{host}/#{token}");
    println!(
        "Workspace: {}\nLocal trusted programs run with your account permissions. Ctrl+C stops Studio.",
        studio.config.workspace.display()
    );
    let connections = Arc::new(tokio::sync::Semaphore::new(32));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            incoming = listener.accept() => {
                let (stream,_) = incoming?;
                let Ok(permit) = connections.clone().try_acquire_owned() else {continue};
                let studio = studio.clone(); let token = token.clone(); let host = host.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let service = service_fn(move |req|handle(req,studio.clone(),token.clone(),host.clone()));
                    let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;
                });
            }
        }
    }
    Ok(())
}
pub fn authorized(
    host: &str,
    origin: Option<&str>,
    authority: Option<&str>,
    auth: Option<&str>,
    token: &str,
) -> bool {
    authority == Some(host)
        && origin.is_none_or(|o| o == format!("http://{host}"))
        && auth.is_some_and(|a| a.strip_prefix("Bearer ") == Some(token))
}
async fn handle(
    req: Request<Incoming>,
    studio: Arc<Studio>,
    token: Text,
    host: Text,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path();
    if path == "/api" {
        if req.method() != hyper::Method::POST
            || !authorized(
                &host,
                req.headers().get("origin").and_then(|v| v.to_str().ok()),
                req.headers().get("host").and_then(|v| v.to_str().ok()),
                req.headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok()),
                &token,
            )
        {
            return Ok(response(
                StatusCode::FORBIDDEN,
                "application/json",
                b"{\"error\":\"Unauthorized local session\"}".to_vec(),
            ));
        }
        let result: Result<Value> = async {
            let bytes = tokio::time::timeout(
                Duration::from_secs(10),
                Limited::new(req.into_body(), 1024 * 1024).collect(),
            )
            .await?
            .map_err(|e| anyhow::anyhow!("Request body: {e}"))?
            .to_bytes();
            let request: Value = serde_json::from_slice(&bytes)?;
            studio
                .call(
                    request["method"].as_str().unwrap_or(""),
                    request["params"].clone(),
                )
                .await
        }
        .await;
        let body = match result {
            Ok(value) => json!({"result":value}),
            Err(error) => json!({"error":error.to_string()}),
        };
        return Ok(response(
            StatusCode::OK,
            "application/json",
            body.to_string().into_bytes(),
        ));
    }
    let (status, mime, body): (_, _, &[u8]) = match (req.method(), path) {
        (&hyper::Method::GET, "/") => (
            StatusCode::OK,
            "text/html; charset=utf-8",
            include_bytes!("../ui/dist/index.html"),
        ),
        (&hyper::Method::GET, "/app.js") => (
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            include_bytes!("../ui/dist/app.js"),
        ),
        (&hyper::Method::GET, "/app.css") => (
            StatusCode::OK,
            "text/css; charset=utf-8",
            include_bytes!("../ui/dist/app.css"),
        ),
        (&hyper::Method::GET, "/licenses.txt") => (
            StatusCode::OK,
            "text/plain; charset=utf-8",
            include_bytes!("../ui/dist/licenses.txt"),
        ),
        (&hyper::Method::GET, "/fonts/InterVariable.woff2") => (
            StatusCode::OK,
            "font/woff2",
            include_bytes!("../ui/fonts/InterVariable.woff2"),
        ),
        (&hyper::Method::GET, "/fonts/JetBrainsMono-Regular.woff2") => (
            StatusCode::OK,
            "font/woff2",
            include_bytes!("../ui/fonts/JetBrainsMono-Regular.woff2"),
        ),
        _ => (StatusCode::NOT_FOUND, "text/plain", b"Not found"),
    };
    Ok(response(status, mime, body.to_vec()))
}
fn response(status: StatusCode, mime: &str, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder().status(status).header("content-type",mime).header("cache-control","no-store")
        .header("x-content-type-options","nosniff").header("referrer-policy","no-referrer")
        .header("content-security-policy","default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
        .body(Full::new(body.into())).expect("static response headers")
}
