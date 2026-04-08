// Allocation Baseline - Go Implementation
//
// Measures allocation performance in Go as a baseline for Verum.
//
// Run: go build -o allocation allocation.go && ./allocation

package main

import (
	"fmt"
	"time"
)

const (
	smallIterations  = 1000000
	mediumIterations = 100000
	largeIterations  = 1000
)

type SmallStruct struct {
	A int32
	B int32
	C float64
	D bool
}

//go:noinline
func sink(x interface{}) {
	_ = x
}

func benchmarkSmallAlloc() {
	for i := 0; i < smallIterations; i++ {
		obj := &SmallStruct{
			A: 1,
			B: 2,
			C: 3.14,
			D: true,
		}
		sink(obj)
	}
}

func benchmarkMediumAlloc() {
	for i := 0; i < mediumIterations; i++ {
		data := make([]byte, 1024)
		sink(data)
	}
}

func benchmarkLargeAlloc() {
	for i := 0; i < largeIterations; i++ {
		data := make([]byte, 1024*1024)
		sink(data)
	}
}

func benchmarkSliceGrowth() {
	for round := 0; round < 1000; round++ {
		var s []int32
		for i := 0; i < 10000; i++ {
			s = append(s, int32(i))
		}
		sink(s)
	}
}

func benchmarkSliceWithCapacity() {
	for round := 0; round < 1000; round++ {
		s := make([]int32, 0, 10000)
		for i := 0; i < 10000; i++ {
			s = append(s, int32(i))
		}
		sink(s)
	}
}

func benchmarkMapAlloc() {
	for round := 0; round < 1000; round++ {
		m := make(map[string]int)
		for i := 0; i < 1000; i++ {
			m[fmt.Sprintf("key_%d", i)] = i
		}
		sink(m)
	}
}

func benchmarkChannelAlloc() {
	for i := 0; i < smallIterations/10; i++ {
		ch := make(chan int, 10)
		sink(ch)
	}
}

func main() {
	// Warmup
	benchmarkSmallAlloc()

	// Small allocation
	start := time.Now()
	benchmarkSmallAlloc()
	elapsed := time.Since(start)
	nsPerOp := float64(elapsed.Nanoseconds()) / float64(smallIterations)
	fmt.Printf("Small alloc (struct):      %.2f ns/op\n", nsPerOp)

	// Medium allocation
	start = time.Now()
	benchmarkMediumAlloc()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(mediumIterations)
	fmt.Printf("Medium alloc (1KB slice):  %.2f ns/op\n", nsPerOp)

	// Large allocation
	start = time.Now()
	benchmarkLargeAlloc()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(largeIterations)
	fmt.Printf("Large alloc (1MB slice):   %.2f ns/op\n", nsPerOp)

	// Slice growth
	start = time.Now()
	benchmarkSliceGrowth()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(1000*10000)
	fmt.Printf("Slice growth (append):     %.2f ns/op\n", nsPerOp)

	// Slice with capacity
	start = time.Now()
	benchmarkSliceWithCapacity()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(1000*10000)
	fmt.Printf("Slice with capacity:       %.2f ns/op\n", nsPerOp)

	// Map allocation
	start = time.Now()
	benchmarkMapAlloc()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(1000*1000)
	fmt.Printf("Map insert:                %.2f ns/op\n", nsPerOp)

	// Channel allocation
	start = time.Now()
	benchmarkChannelAlloc()
	elapsed = time.Since(start)
	nsPerOp = float64(elapsed.Nanoseconds()) / float64(smallIterations/10)
	fmt.Printf("Channel alloc:             %.2f ns/op\n", nsPerOp)
}
