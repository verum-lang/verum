/*
 * Function Call Overhead Baseline - C Implementation
 *
 * This benchmark measures function call overhead in C as a baseline
 * for Verum's function call performance targets.
 *
 * Compile: cc -O3 -march=native -o function_call function_call.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>

#define WARMUP_ITERATIONS 100000
#define BENCHMARK_ITERATIONS 10000000

static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

/* Prevent inlining for accurate measurements */
__attribute__((noinline))
int64_t direct_add(int64_t a, int64_t b) {
    return a + b;
}

__attribute__((noinline))
int64_t complex_function(int64_t a, int64_t b, int64_t c) {
    int64_t x = a * b;
    int64_t y = b * c;
    int64_t z = c * a;
    return x + y + z;
}

/* Virtual dispatch via function pointer */
typedef int64_t (*operation_fn)(void* self, int64_t arg);

typedef struct {
    operation_fn op;
    int64_t value;
} VirtualObj;

__attribute__((noinline))
int64_t virtual_add(void* self, int64_t arg) {
    VirtualObj* obj = (VirtualObj*)self;
    return obj->value + arg;
}

/* Volatile sink to prevent optimization */
static volatile int64_t sink;

void benchmark_inline(void) {
    int64_t sum = 0;

    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        sum = sum + 1;
    }

    sum = 0;
    uint64_t start = get_time_ns();

    for (int i = 0; i < BENCHMARK_ITERATIONS; i++) {
        sum = sum + 1;
    }

    uint64_t end = get_time_ns();
    sink = sum;

    double ns_per_op = (double)(end - start) / BENCHMARK_ITERATIONS;
    printf("inline_baseline:      %.2f ns/op\n", ns_per_op);
}

void benchmark_direct_call(void) {
    int64_t sum = 0;

    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        sum = direct_add(sum, 1);
    }

    sum = 0;
    uint64_t start = get_time_ns();

    for (int i = 0; i < BENCHMARK_ITERATIONS; i++) {
        sum = direct_add(sum, 1);
    }

    uint64_t end = get_time_ns();
    sink = sum;

    double ns_per_op = (double)(end - start) / BENCHMARK_ITERATIONS;
    printf("direct_call:          %.2f ns/op\n", ns_per_op);
}

void benchmark_complex_call(void) {
    int64_t result = 0;

    for (int64_t i = 0; i < WARMUP_ITERATIONS; i++) {
        result = complex_function(result, i, i + 1);
    }

    result = 0;
    uint64_t start = get_time_ns();

    for (int64_t i = 0; i < BENCHMARK_ITERATIONS; i++) {
        result = complex_function(result, i, i + 1);
    }

    uint64_t end = get_time_ns();
    sink = result;

    double ns_per_op = (double)(end - start) / BENCHMARK_ITERATIONS;
    printf("complex_call:         %.2f ns/op\n", ns_per_op);
}

void benchmark_virtual_call(void) {
    VirtualObj obj = { .op = virtual_add, .value = 0 };
    int64_t sum = 0;

    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        sum = obj.op(&obj, 1);
        obj.value = sum;
    }

    obj.value = 0;
    sum = 0;
    uint64_t start = get_time_ns();

    for (int i = 0; i < BENCHMARK_ITERATIONS; i++) {
        sum = obj.op(&obj, 1);
        obj.value = sum;
    }

    uint64_t end = get_time_ns();
    sink = sum;

    double ns_per_op = (double)(end - start) / BENCHMARK_ITERATIONS;
    printf("virtual_call:         %.2f ns/op\n", ns_per_op);
}

/* Recursive function */
__attribute__((noinline))
int64_t recursive_sum(int64_t n) {
    if (n <= 0) return 0;
    return n + recursive_sum(n - 1);
}

void benchmark_recursive_call(void) {
    int depth = 10;
    int iterations = BENCHMARK_ITERATIONS / depth;
    int64_t result = 0;

    for (int i = 0; i < WARMUP_ITERATIONS / depth; i++) {
        result = recursive_sum(depth);
    }

    uint64_t start = get_time_ns();

    for (int i = 0; i < iterations; i++) {
        result = recursive_sum(depth);
    }

    uint64_t end = get_time_ns();
    sink = result;

    double ns_per_op = (double)(end - start) / (iterations * depth);
    printf("recursive_depth_10:   %.2f ns/op\n", ns_per_op);
}

/* Tail-recursive (should optimize to loop) */
__attribute__((noinline))
int64_t tail_sum(int64_t n, int64_t acc) {
    if (n <= 0) return acc;
    return tail_sum(n - 1, acc + n);
}

void benchmark_tail_recursive(void) {
    int depth = 100;
    int iterations = BENCHMARK_ITERATIONS / depth;
    int64_t result = 0;

    for (int i = 0; i < WARMUP_ITERATIONS / depth; i++) {
        result = tail_sum(depth, 0);
    }

    uint64_t start = get_time_ns();

    for (int i = 0; i < iterations; i++) {
        result = tail_sum(depth, 0);
    }

    uint64_t end = get_time_ns();
    sink = result;

    double ns_per_op = (double)(end - start) / (iterations * depth);
    printf("tail_recursive_100:   %.2f ns/op\n", ns_per_op);
}

int main(void) {
    printf("=== C Function Call Baseline ===\n\n");

    benchmark_inline();
    benchmark_direct_call();
    benchmark_complex_call();
    benchmark_virtual_call();
    benchmark_recursive_call();
    benchmark_tail_recursive();

    printf("\nThese values serve as baseline for Verum targets:\n");
    printf("  - Direct call:  < 5ns (should match C)\n");
    printf("  - Virtual call: < 15ns (vtable overhead)\n");
    printf("  - Context call: < 30ns (DI lookup + dispatch)\n");

    return 0;
}
