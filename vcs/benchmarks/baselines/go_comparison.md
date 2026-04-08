# Verum vs Go Performance Comparison

This document outlines how to compare Verum performance against Go.

## Philosophy

Go is a comparison target for:
- Concurrent workloads (goroutines vs async)
- GC vs CBGR comparison
- Server application performance

Verum aims to significantly outperform Go in most scenarios due to:
- No garbage collection pauses
- LLVM vs Go compiler optimizations
- Zero-cost abstractions

## Expected Performance Ratio

| Category | Expected Verum/Go |
|----------|-------------------|
| Pure computation | 2-5x faster |
| Memory allocation | 1.5-3x faster |
| Concurrency | 1-2x faster |
| String operations | 2-3x faster |
| JSON parsing | 2-4x faster |

## Comparison Categories

### 1. Garbage Collection vs CBGR

Go uses a concurrent garbage collector with:
- STW pauses (usually < 1ms)
- Write barriers (~5ns overhead)
- GC CPU overhead (1-5%)

Verum uses CBGR:
- No pauses
- Reference check (~15ns per access)
- No background GC overhead

**Test:**
```go
// Go - will trigger GC
func allocateMany() {
    for i := 0; i < 1000000; i++ {
        _ = make([]int, 1000)
    }
}
```

```verum
// Verum - deterministic deallocation
fn allocate_many() {
    for _ in 0..1_000_000 {
        let _ = list![0; 1000];
    }
}
```

### 2. Concurrency Models

**Goroutines vs Async Tasks:**

```go
// Go
func main() {
    var wg sync.WaitGroup
    for i := 0; i < 10000; i++ {
        wg.Add(1)
        go func(n int) {
            defer wg.Done()
            // work
        }(i)
    }
    wg.Wait()
}
```

```verum
// Verum
fn main() async {
    let handles: List<_> = (0..10_000).map(|i| {
        spawn async move {
            // work
        }
    }).collect();

    for h in handles {
        h.await;
    }
}
```

**Expected:**
- Goroutine spawn: ~2-3us
- Verum async spawn: ~500ns
- Verum should be 4-6x faster for spawn

### 3. Channel Operations

```go
// Go
ch := make(chan int, 100)
go func() {
    for i := 0; i < 1000000; i++ {
        ch <- i
    }
    close(ch)
}()
for v := range ch {
    _ = v
}
```

```verum
// Verum
let (tx, rx) = channel::<Int>.bounded(100);

spawn async {
    for i in 0..1_000_000 {
        tx.send(i).await;
    }
};

while let Some(v) = rx.recv().await {
    let _ = v;
}
```

**Expected:** Similar performance, Verum slightly faster due to no GC

### 4. String Operations

```go
// Go
s := strings.Repeat("hello", 1000)
_ = strings.Contains(s, "world")
```

```verum
// Verum
let s = "hello".repeat(1000);
let _ = s.contains("world");
```

**Expected:** Verum 2-3x faster (LLVM optimizations, no GC)

### 5. JSON Parsing

```go
// Go
type User struct {
    Name string `json:"name"`
    Age  int    `json:"age"`
}
var user User
json.Unmarshal(data, &user)
```

```verum
// Verum
#[derive(Deserialize)]
struct User {
    name: Text,
    age: Int,
}
let user: User = Json.parse(data)?;
```

**Expected:** Verum 2-4x faster (compile-time codegen, no reflection)

## Benchmark Methodology

### Environment Setup

```bash
# Go
export GOGC=100  # Default GC target
go build -o benchmark

# Verum
verum build --release
```

### Measuring GC Impact

```go
// Go - measure GC pauses
import "runtime/debug"

func benchmark() {
    debug.SetGCPercent(100)

    var stats debug.GCStats
    // ... run benchmark ...
    debug.ReadGCStats(&stats)

    fmt.Printf("GC Pause Total: %v\n", stats.PauseTotal)
    fmt.Printf("Num GC: %d\n", stats.NumGC)
}
```

```verum
// Verum - no GC to measure
// Just measure total time
fn benchmark() {
    let start = Instant.now();
    // ... run benchmark ...
    let elapsed = start.elapsed();
    print(text!("Total time: {:?}", elapsed));
}
```

### Memory Profiling

```bash
# Go
go tool pprof -alloc_space benchmark mem.prof

# Verum
verum profile --memory benchmark
```

## Specific Benchmarks

### HTTP Server

```go
// Go
http.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
    w.Write([]byte("Hello, World!"))
})
http.ListenAndServe(":8080", nil)
```

```verum
// Verum
let server = HttpServer.bind(":8080").await;
server.route("/", |_| async {
    Response.text("Hello, World!")
});
server.run().await;
```

**Expected:** Verum 1.5-2x faster (no GC pauses under load)

### Database Access

```go
// Go
rows, _ := db.Query("SELECT * FROM users WHERE id = ?", id)
for rows.Next() {
    var user User
    rows.Scan(&user.ID, &user.Name)
}
```

```verum
// Verum
let rows = db.query("SELECT * FROM users WHERE id = ?", [id]).await?;
for row in rows {
    let user = User { id: row.get("id"), name: row.get("name") };
}
```

**Expected:** Similar performance (I/O bound)

## Reporting Results

```
Benchmark: [name]
Go:     X.XX ms/op (GC pauses: Y.YY ms)
Verum:  X.XX ms/op
Ratio:  X.Xx (Go/Verum)
Memory: Go XXMB / Verum XXMB
```

## CI Integration

```yaml
benchmark-vs-go:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v3
    - uses: actions/setup-go@v4
    - name: Install Verum
      run: cargo install verum-cli
    - name: Run Go benchmark
      run: go test -bench=. -benchmem ./...
    - name: Run Verum benchmark
      run: verum bench
    - name: Compare results
      run: verum compare --baseline go-baseline
```

## Common Pitfalls

1. **Warm-up**: Go's JIT-like optimizations need warm-up
2. **GC tuning**: GOGC affects Go performance significantly
3. **Escape analysis**: Both languages perform it differently
4. **Memory layout**: Go uses different struct padding

## When Go Might Win

1. **Very short-lived programs** (Go startup < 1ms)
2. **CGO-heavy workloads** (Go's C FFI is optimized)
3. **Extremely GC-tuned applications** (manual GC tuning)
