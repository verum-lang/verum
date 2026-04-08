// Fibonacci Benchmark - Go Implementation
//
// Baseline implementation for comparison with Verum, C, and Rust.
// Run with: go run fibonacci.go

package main

import (
	"fmt"
	"time"
)

const warmupIterations = 10

// Recursive Fibonacci
func fibRecursive(n int) int64 {
	if n <= 1 {
		return int64(n)
	}
	return fibRecursive(n-1) + fibRecursive(n-2)
}

// Iterative Fibonacci
func fibIterative(n int) int64 {
	if n <= 1 {
		return int64(n)
	}

	var a, b int64 = 0, 1
	for i := 2; i <= n; i++ {
		a, b = b, a+b
	}
	return b
}

// Memoized Fibonacci
func fibMemoized(n int, cache map[int]int64) int64 {
	if n <= 1 {
		return int64(n)
	}

	if result, ok := cache[n]; ok {
		return result
	}

	result := fibMemoized(n-1, cache) + fibMemoized(n-2, cache)
	cache[n] = result
	return result
}

// Matrix multiplication for 2x2 matrices
func matrixMult(a, b [2][2]int64) [2][2]int64 {
	return [2][2]int64{
		{
			a[0][0]*b[0][0] + a[0][1]*b[1][0],
			a[0][0]*b[0][1] + a[0][1]*b[1][1],
		},
		{
			a[1][0]*b[0][0] + a[1][1]*b[1][0],
			a[1][0]*b[0][1] + a[1][1]*b[1][1],
		},
	}
}

// Matrix exponentiation Fibonacci O(log n)
func fibMatrix(n int) int64 {
	if n <= 1 {
		return int64(n)
	}

	result := [2][2]int64{{1, 0}, {0, 1}} // Identity
	base := [2][2]int64{{1, 1}, {1, 0}}

	for n > 0 {
		if n%2 == 1 {
			result = matrixMult(result, base)
		}
		base = matrixMult(base, base)
		n /= 2
	}

	return result[0][1]
}

// Prevent compiler optimization
//go:noinline
func blackBox(x int64) int64 {
	return x
}

func benchmarkRecursive30() {
	const n = 30
	const expected int64 = 832040
	const iterations = 100

	// Warmup
	for i := 0; i < warmupIterations; i++ {
		result := fibRecursive(n)
		if result != expected {
			panic(fmt.Sprintf("fib(%d) = %d, expected %d", n, result, expected))
		}
	}

	start := time.Now()
	for i := 0; i < iterations; i++ {
		blackBox(fibRecursive(n))
	}
	elapsed := time.Since(start)

	perCallMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_recursive_30: %.3f ms/call\n", perCallMs)
}

func benchmarkRecursive40() {
	const n = 40
	const expected int64 = 102334155
	const iterations = 3

	// Warmup
	result := fibRecursive(n)
	if result != expected {
		panic(fmt.Sprintf("fib(%d) = %d, expected %d", n, result, expected))
	}

	start := time.Now()
	for i := 0; i < iterations; i++ {
		blackBox(fibRecursive(n))
	}
	elapsed := time.Since(start)

	perCallMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_recursive_40: %.3f ms/call\n", perCallMs)
}

func benchmarkIterative45() {
	const n = 45
	const iterations = 10_000_000

	// Warmup
	for i := 0; i < warmupIterations; i++ {
		blackBox(fibIterative(n))
	}

	start := time.Now()
	var sum int64 = 0
	for i := 0; i < iterations; i++ {
		sum += fibIterative(n)
	}
	blackBox(sum)
	elapsed := time.Since(start)

	perCallNs := float64(elapsed.Nanoseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_iterative_45: %.3f ns/call\n", perCallNs)
}

func benchmarkIterative90() {
	const n = 90
	const iterations = 1_000_000

	start := time.Now()
	var sum int64 = 0
	for i := 0; i < iterations; i++ {
		sum += fibIterative(n)
	}
	blackBox(sum)
	elapsed := time.Since(start)

	perCallNs := float64(elapsed.Nanoseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_iterative_90: %.3f ns/call\n", perCallNs)
}

func benchmarkMemoized40() {
	const n = 40
	const iterations = 10_000

	// Warmup
	for i := 0; i < warmupIterations; i++ {
		cache := make(map[int]int64)
		blackBox(fibMemoized(n, cache))
	}

	start := time.Now()
	for i := 0; i < iterations; i++ {
		cache := make(map[int]int64)
		blackBox(fibMemoized(n, cache))
	}
	elapsed := time.Since(start)

	perCallUs := float64(elapsed.Microseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_memoized_40: %.3f us/call\n", perCallUs)
}

func benchmarkMatrix1000() {
	const n = 1000
	const iterations = 1_000_000

	// Warmup
	for i := 0; i < warmupIterations; i++ {
		blackBox(fibMatrix(n))
	}

	start := time.Now()
	var sum int64 = 0
	for i := 0; i < iterations; i++ {
		sum += fibMatrix(n)
	}
	blackBox(sum)
	elapsed := time.Since(start)

	perCallNs := float64(elapsed.Nanoseconds()) / float64(iterations)
	fmt.Printf("[Go] fib_matrix_1000: %.3f ns/call\n", perCallNs)
}

func main() {
	fmt.Println("=== Fibonacci Benchmark - Go ===")
	fmt.Println()

	benchmarkRecursive30()
	benchmarkRecursive40()
	benchmarkIterative45()
	benchmarkIterative90()
	benchmarkMemoized40()
	benchmarkMatrix1000()

	fmt.Println()
	fmt.Println("All benchmarks completed.")
}
