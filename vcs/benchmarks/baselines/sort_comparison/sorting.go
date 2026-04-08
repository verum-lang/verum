// Sorting Algorithm Benchmark - Go Implementation
//
// Baseline implementation for comparison with Verum, C, and Rust.
// Run with: go run sorting.go

package main

import (
	"fmt"
	"sort"
	"time"
)

const warmupIterations = 5

// Simple xorshift64 RNG
type Rng struct {
	state uint64
}

func newRng(seed uint64) *Rng {
	return &Rng{state: seed}
}

func (r *Rng) next() int64 {
	r.state ^= r.state << 13
	r.state ^= r.state >> 7
	r.state ^= r.state << 17
	return int64(r.state)
}

// Generate random array
func generateRandomArray(size int, seed uint64) []int64 {
	rng := newRng(seed)
	arr := make([]int64, size)
	for i := 0; i < size; i++ {
		arr[i] = rng.next()
	}
	return arr
}

// Generate sorted array
func generateSortedArray(size int) []int64 {
	arr := make([]int64, size)
	for i := 0; i < size; i++ {
		arr[i] = int64(i)
	}
	return arr
}

// Generate reverse-sorted array
func generateReverseArray(size int) []int64 {
	arr := make([]int64, size)
	for i := 0; i < size; i++ {
		arr[i] = int64(size - 1 - i)
	}
	return arr
}

// Int64Slice for sorting
type Int64Slice []int64

func (s Int64Slice) Len() int           { return len(s) }
func (s Int64Slice) Less(i, j int) bool { return s[i] < s[j] }
func (s Int64Slice) Swap(i, j int)      { s[i], s[j] = s[j], s[i] }

// Prevent compiler optimization
//
//go:noinline
func blackBox(x []int64) int64 {
	if len(x) > 0 {
		return x[0]
	}
	return 0
}

func benchmarkSort100() {
	const size = 100
	const iterations = 100000

	// Warmup
	for i := 0; i < warmupIterations; i++ {
		arr := generateRandomArray(size, 42)
		sort.Sort(Int64Slice(arr))
	}

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateRandomArray(size, uint64(i))
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortUs := float64(elapsed.Microseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_100: %.3f us/sort\n", perSortUs)
}

func benchmarkSort1000() {
	const size = 1000
	const iterations = 10000

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateRandomArray(size, uint64(i))
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortUs := float64(elapsed.Microseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_1000: %.3f us/sort\n", perSortUs)
}

func benchmarkSort10000() {
	const size = 10000
	const iterations = 1000

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateRandomArray(size, uint64(i))
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortUs := float64(elapsed.Microseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_10000: %.3f us/sort\n", perSortUs)
}

func benchmarkSort100000() {
	const size = 100000
	const iterations = 100

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateRandomArray(size, uint64(i))
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_100000: %.3f ms/sort\n", perSortMs)
}

func benchmarkSort1000000() {
	const size = 1000000
	const iterations = 10

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateRandomArray(size, uint64(i))
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_1000000: %.3f ms/sort\n", perSortMs)
}

func benchmarkSortSorted() {
	const size = 100000
	const iterations = 100

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateSortedArray(size)
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_sorted_100k: %.3f ms/sort\n", perSortMs)
}

func benchmarkSortReverse() {
	const size = 100000
	const iterations = 100

	start := time.Now()
	for i := 0; i < iterations; i++ {
		arr := generateReverseArray(size)
		sort.Sort(Int64Slice(arr))
		blackBox(arr)
	}
	elapsed := time.Since(start)

	perSortMs := float64(elapsed.Milliseconds()) / float64(iterations)
	fmt.Printf("[Go] sort_reverse_100k: %.3f ms/sort\n", perSortMs)
}

func main() {
	fmt.Println("=== Sorting Benchmark - Go ===")
	fmt.Println()

	benchmarkSort100()
	benchmarkSort1000()
	benchmarkSort10000()
	benchmarkSort100000()
	benchmarkSort1000000()
	benchmarkSortSorted()
	benchmarkSortReverse()

	fmt.Println()
	fmt.Println("All benchmarks completed.")
}
