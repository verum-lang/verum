// CBGR Check Baseline - Go Implementation
//
// This benchmark measures array access in Go with its built-in
// bounds checking as a comparison point for Verum's CBGR system.
//
// Run: go build -o cbgr_check cbgr_check.go && ./cbgr_check

package main

import (
	"fmt"
	"time"
)

const (
	iterations = 10000000
	arraySize  = 1000
)

func benchmarkArrayAccess(data []int64) int64 {
	var sum int64
	for i := 0; i < iterations; i++ {
		for j := 0; j < len(data); j++ {
			sum += data[j]
		}
	}
	return sum
}

func benchmarkRangeAccess(data []int64) int64 {
	var sum int64
	for i := 0; i < iterations; i++ {
		for _, v := range data {
			sum += v
		}
	}
	return sum
}

func benchmarkPointerAccess(data []int64) int64 {
	var sum int64
	for i := 0; i < iterations; i++ {
		p := &data[0]
		for j := 0; j < len(data); j++ {
			sum += *p
			// Note: Go doesn't allow pointer arithmetic in safe code
			// This is less efficient than C pointer arithmetic
		}
	}
	return sum
}

func benchmarkSliceWindow(data []int64) int64 {
	var sum int64
	for i := 0; i < iterations; i++ {
		slice := data[0:arraySize]
		for j := 0; j < len(slice); j++ {
			sum += slice[j]
		}
	}
	return sum
}

//go:noinline
func sink(x int64) {
	_ = x
}

func main() {
	data := make([]int64, arraySize)
	for i := range data {
		data[i] = int64(i)
	}

	// Warmup
	sink(benchmarkArrayAccess(data))

	// Index-based access
	start := time.Now()
	result := benchmarkArrayAccess(data)
	elapsed := time.Since(start)
	nsPerOp := float64(elapsed.Nanoseconds()) / float64(iterations*arraySize)
	fmt.Printf("Array access (index):      %.2f ns/op (result: %d)\n", nsPerOp, result)

	// Range-based access
	start = time.Now()
	result = benchmarkRangeAccess(data)
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(iterations*arraySize)
	fmt.Printf("Array access (range):      %.2f ns/op (result: %d)\n", nsPerOp, result)

	// Slice window
	start = time.Now()
	result = benchmarkSliceWindow(data)
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(iterations*arraySize)
	fmt.Printf("Slice window access:       %.2f ns/op (result: %d)\n", nsPerOp, result)
}
