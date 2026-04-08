// Function Call Overhead Baseline - Go Implementation
//
// This benchmark measures function call overhead in Go as a comparison
// for Verum's function call performance targets.
//
// Run: go build -o function_call function_call.go && ./function_call

package main

import (
	"fmt"
	"time"
)

const (
	warmupIterations    = 100000
	benchmarkIterations = 10000000
)

//go:noinline
func directAdd(a, b int64) int64 {
	return a + b
}

//go:noinline
func complexFunction(a, b, c int64) int64 {
	x := a * b
	y := b * c
	z := c * a
	return x + y + z
}

// Interface for virtual dispatch
type Computable interface {
	Compute(arg int64) int64
}

type Impl1 struct{ value int64 }
type Impl2 struct{ value int64 }
type Impl3 struct{ value int64 }

//go:noinline
func (i *Impl1) Compute(arg int64) int64 { return i.value + arg }

//go:noinline
func (i *Impl2) Compute(arg int64) int64 { return i.value * arg }

//go:noinline
func (i *Impl3) Compute(arg int64) int64 { return i.value - arg }

//go:noinline
func sink(x int64) {
	_ = x
}

func benchmarkInline() {
	var sum int64 = 0

	for i := 0; i < warmupIterations; i++ {
		sum = sum + 1
	}

	sum = 0
	start := time.Now()

	for i := 0; i < benchmarkIterations; i++ {
		sum = sum + 1
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("inline_baseline:      %.2f ns/op\n", nsPerOp)
}

func benchmarkDirectCall() {
	var sum int64 = 0

	for i := 0; i < warmupIterations; i++ {
		sum = directAdd(sum, 1)
	}

	sum = 0
	start := time.Now()

	for i := 0; i < benchmarkIterations; i++ {
		sum = directAdd(sum, 1)
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("direct_call:          %.2f ns/op\n", nsPerOp)
}

func benchmarkComplexCall() {
	var result int64 = 0

	for i := int64(0); i < warmupIterations; i++ {
		result = complexFunction(result, i, i+1)
	}

	result = 0
	start := time.Now()

	for i := int64(0); i < benchmarkIterations; i++ {
		result = complexFunction(result, i, i+1)
	}

	elapsed := time.Since(start)
	sink(result)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("complex_call:         %.2f ns/op\n", nsPerOp)
}

func benchmarkInterfaceCallMonomorphic() {
	obj := &Impl1{value: 0}
	var iface Computable = obj
	var sum int64 = 0

	for i := 0; i < warmupIterations; i++ {
		sum = iface.Compute(1)
		obj.value = sum
	}

	obj.value = 0
	sum = 0
	start := time.Now()

	for i := 0; i < benchmarkIterations; i++ {
		sum = iface.Compute(1)
		obj.value = sum
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("interface_mono:       %.2f ns/op\n", nsPerOp)
}

func benchmarkInterfaceCallPolymorphic() {
	objs := []Computable{
		&Impl1{value: 1},
		&Impl2{value: 2},
		&Impl3{value: 3},
	}
	var sum int64 = 0

	for i := 0; i < warmupIterations; i++ {
		sum += objs[i%3].Compute(1)
	}

	sum = 0
	start := time.Now()

	for i := 0; i < benchmarkIterations; i++ {
		sum += objs[i%3].Compute(1)
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("interface_poly:       %.2f ns/op\n", nsPerOp)
}

func benchmarkClosureNoCapture() {
	closure := func(a, b int64) int64 { return a + b }
	var sum int64 = 0

	for i := 0; i < warmupIterations; i++ {
		sum = closure(sum, 1)
	}

	sum = 0
	start := time.Now()

	for i := 0; i < benchmarkIterations; i++ {
		sum = closure(sum, 1)
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("closure_no_capture:   %.2f ns/op\n", nsPerOp)
}

func benchmarkClosureWithCapture() {
	multiplier := int64(2)
	offset := int64(10)
	closure := func(a int64) int64 { return a*multiplier + offset }
	var sum int64 = 0

	for i := int64(0); i < warmupIterations; i++ {
		sum = closure(i)
	}

	sum = 0
	start := time.Now()

	for i := int64(0); i < benchmarkIterations; i++ {
		sum = closure(i)
	}

	elapsed := time.Since(start)
	sink(sum)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(benchmarkIterations)
	fmt.Printf("closure_with_capture: %.2f ns/op\n", nsPerOp)
}

//go:noinline
func recursiveSum(n int64) int64 {
	if n <= 0 {
		return 0
	}
	return n + recursiveSum(n-1)
}

func benchmarkRecursiveCall() {
	depth := int64(10)
	iterations := benchmarkIterations / int(depth)

	for i := 0; i < warmupIterations/int(depth); i++ {
		sink(recursiveSum(depth))
	}

	start := time.Now()

	for i := 0; i < iterations; i++ {
		sink(recursiveSum(depth))
	}

	elapsed := time.Since(start)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(int64(iterations)*depth)
	fmt.Printf("recursive_depth_10:   %.2f ns/op\n", nsPerOp)
}

// Note: Go doesn't guarantee tail call optimization
//
//go:noinline
func tailSum(n, acc int64) int64 {
	if n <= 0 {
		return acc
	}
	return tailSum(n-1, acc+n)
}

func benchmarkTailRecursive() {
	depth := int64(100)
	iterations := benchmarkIterations / int(depth)

	for i := 0; i < warmupIterations/int(depth); i++ {
		sink(tailSum(depth, 0))
	}

	start := time.Now()

	for i := 0; i < iterations; i++ {
		sink(tailSum(depth, 0))
	}

	elapsed := time.Since(start)

	nsPerOp := float64(elapsed.Nanoseconds()) / float64(int64(iterations)*depth)
	fmt.Printf("tail_recursive_100:   %.2f ns/op\n", nsPerOp)
}

func main() {
	fmt.Println("=== Go Function Call Baseline ===")
	fmt.Println()

	benchmarkInline()
	benchmarkDirectCall()
	benchmarkComplexCall()
	benchmarkInterfaceCallMonomorphic()
	benchmarkInterfaceCallPolymorphic()
	benchmarkClosureNoCapture()
	benchmarkClosureWithCapture()
	benchmarkRecursiveCall()
	benchmarkTailRecursive()

	fmt.Println()
	fmt.Println("Verum targets (vs Go):")
	fmt.Println("  - Direct call:    Faster than Go (< 5ns vs Go's ~2-3ns)")
	fmt.Println("  - Interface call: Comparable to Go (< 15ns)")
	fmt.Println("  - Context call:   Acceptable overhead (< 30ns)")
	fmt.Println()
	fmt.Println("Note: Go has higher call overhead than C/Rust due to goroutine stack checks")
}
