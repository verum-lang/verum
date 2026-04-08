// GC Pause Time Baseline - Go Implementation
//
// This benchmark measures GC pause times in Go as a comparison
// for Verum's GC performance targets.
//
// Run: go build -o gc_pause gc_pause.go && ./gc_pause

package main

import (
	"fmt"
	"runtime"
	"sort"
	"sync"
	"time"
)

const (
	benchmarkDurationSecs = 5
	allocationRatePerSec  = 100000
)

type pauseStats struct {
	pauses    []time.Duration
	allocCount int
}

// Measure GC pauses under steady allocation
func benchmarkSteadyState() pauseStats {
	var pauses []time.Duration
	allocInterval := time.Second / time.Duration(allocationRatePerSec)

	// Warmup
	for i := 0; i < 10000; i++ {
		obj := make([]byte, 64)
		_ = obj
	}

	runtime.GC()

	start := time.Now()
	lastCheck := start
	allocCount := 0

	for time.Since(start) < time.Duration(benchmarkDurationSecs)*time.Second {
		// Allocate objects
		for i := 0; i < 100; i++ {
			obj := make([]byte, 64)
			_ = obj
			allocCount++
		}

		now := time.Now()
		elapsed := now.Sub(lastCheck)

		expected := allocInterval * 100
		if elapsed > expected*2 {
			pauseTime := elapsed - expected
			pauses = append(pauses, pauseTime)
		}

		lastCheck = now
	}

	return pauseStats{pauses: pauses, allocCount: allocCount}
}

// Measure GC under allocation burst
func benchmarkAllocationBurst() pauseStats {
	var pauses []time.Duration

	runtime.GC()

	allocCount := 0

	for burst := 0; burst < 100; burst++ {
		burstStart := time.Now()

		// Allocate burst
		for i := 0; i < 10000; i++ {
			obj := make([]byte, 256)
			_ = obj
			allocCount++
		}

		burstElapsed := time.Since(burstStart)
		expected := time.Millisecond

		if burstElapsed > expected*2 {
			pauses = append(pauses, burstElapsed-expected)
		}

		time.Sleep(10 * time.Millisecond)
	}

	return pauseStats{pauses: pauses, allocCount: allocCount}
}

// Measure GC with long-lived objects
func benchmarkGenerational() pauseStats {
	var pauses []time.Duration

	// Create long-lived objects
	longLived := make([][]byte, 10000)
	for i := range longLived {
		longLived[i] = make([]byte, 1024)
	}

	runtime.GC()

	start := time.Now()
	allocCount := 0

	for time.Since(start) < 3*time.Second {
		checkStart := time.Now()

		for i := 0; i < 1000; i++ {
			obj := make([]byte, 64)
			_ = obj
			allocCount++
		}

		elapsed := time.Since(checkStart)
		expected := 100 * time.Microsecond

		if elapsed > expected*3 {
			pauses = append(pauses, elapsed-expected)
		}
	}

	// Keep long_lived alive
	_ = longLived[0]

	return pauseStats{pauses: pauses, allocCount: allocCount}
}

// Measure GC with concurrent workload
func benchmarkConcurrent() pauseStats {
	var pauses []time.Duration
	var mu sync.Mutex
	var wg sync.WaitGroup

	runtime.GC()

	start := time.Now()
	totalAllocs := 0

	// Start worker goroutines
	for w := 0; w < 4; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			localAllocs := 0

			for time.Since(start) < 3*time.Second {
				checkStart := time.Now()

				for i := 0; i < 250; i++ {
					obj := make([]byte, 128)
					_ = obj
					localAllocs++
				}

				elapsed := time.Since(checkStart)
				expected := 50 * time.Microsecond

				if elapsed > expected*3 {
					mu.Lock()
					pauses = append(pauses, elapsed-expected)
					mu.Unlock()
				}
			}

			mu.Lock()
			totalAllocs += localAllocs
			mu.Unlock()
		}()
	}

	wg.Wait()

	return pauseStats{pauses: pauses, allocCount: totalAllocs}
}

// Measure explicit GC latency
func benchmarkExplicitGC() pauseStats {
	var pauses []time.Duration

	allocCount := 0

	for i := 0; i < 100; i++ {
		// Create some garbage
		for j := 0; j < 10000; j++ {
			obj := make([]byte, 64)
			_ = obj
			allocCount++
		}

		// Measure explicit collection time
		gcStart := time.Now()
		runtime.GC()
		gcTime := time.Since(gcStart)
		pauses = append(pauses, gcTime)
	}

	return pauseStats{pauses: pauses, allocCount: allocCount}
}

func percentile(durations []time.Duration, p float64) time.Duration {
	if len(durations) == 0 {
		return 0
	}
	sort.Slice(durations, func(i, j int) bool {
		return durations[i] < durations[j]
	})
	idx := int(float64(len(durations)-1) * p / 100.0)
	return durations[idx]
}

func maxDuration(durations []time.Duration) time.Duration {
	if len(durations) == 0 {
		return 0
	}
	max := durations[0]
	for _, d := range durations[1:] {
		if d > max {
			max = d
		}
	}
	return max
}

func printResult(name string, stats pauseStats) {
	p50 := percentile(stats.pauses, 50)
	p99 := percentile(stats.pauses, 99)
	maxP := maxDuration(stats.pauses)

	p50Ok := p50 < 100*time.Microsecond
	p99Ok := p99 < time.Millisecond
	status := "PASS"
	if !p50Ok || !p99Ok {
		status = "WARN"
	}

	fmt.Printf("%-20s %10d %12s %12s %12s [%s]\n",
		name,
		len(stats.pauses),
		p50.String(),
		p99.String(),
		maxP.String(),
		status)
}

func main() {
	fmt.Println("=== Go GC Pause Time Baseline ===")
	fmt.Println()
	fmt.Printf("%-20s %10s %12s %12s %12s\n", "Benchmark", "Pauses", "P50", "P99", "Max")
	fmt.Println("--------------------------------------------------------------------------------")

	printResult("steady_state", benchmarkSteadyState())
	printResult("allocation_burst", benchmarkAllocationBurst())
	printResult("generational", benchmarkGenerational())
	printResult("concurrent", benchmarkConcurrent())
	printResult("explicit_gc", benchmarkExplicitGC())

	fmt.Println()
	fmt.Println("Verum GC targets (vs Go):")
	fmt.Println("  - P50 pause:  < 100us (comparable to Go)")
	fmt.Println("  - P99 pause:  < 1ms (comparable to Go)")
	fmt.Println("  - Throughput: > 95% (better than Go due to CBGR)")
	fmt.Println()
	fmt.Println("Note: Go's GC is concurrent and low-latency.")
	fmt.Println("Verum uses CBGR for deterministic cleanup of most objects,")
	fmt.Println("with optional concurrent GC for reference cycles.")

	// Print Go GC stats
	var stats runtime.MemStats
	runtime.ReadMemStats(&stats)
	fmt.Printf("\nGo GC Stats:\n")
	fmt.Printf("  NumGC:        %d\n", stats.NumGC)
	fmt.Printf("  PauseTotalNs: %s\n", time.Duration(stats.PauseTotalNs))
	if stats.NumGC > 0 {
		avgPause := time.Duration(stats.PauseTotalNs / uint64(stats.NumGC))
		fmt.Printf("  AvgPause:     %s\n", avgPause)
	}
}
