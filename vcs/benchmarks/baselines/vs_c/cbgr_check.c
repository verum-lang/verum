/*
 * CBGR Check Baseline - C Implementation
 *
 * This benchmark measures the baseline cost of array access in C,
 * which Verum's CBGR (Checked Borrow with Generation and Region) aims
 * to approach while providing memory safety guarantees.
 *
 * Target: Verum CBGR check should be < 15ns overhead vs this baseline.
 *
 * Compile: cc -O3 -march=native -o cbgr_check cbgr_check.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>

#define ITERATIONS 10000000
#define ARRAY_SIZE 1000

static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

/* Prevent compiler from optimizing away the result */
static volatile int64_t sink;

void benchmark_array_access(int64_t* data, int size) {
    int64_t sum = 0;
    for (int i = 0; i < ITERATIONS; i++) {
        for (int j = 0; j < size; j++) {
            sum += data[j];
        }
    }
    sink = sum;
}

void benchmark_pointer_chase(int64_t** ptrs, int size) {
    int64_t sum = 0;
    for (int i = 0; i < ITERATIONS; i++) {
        for (int j = 0; j < size; j++) {
            sum += *ptrs[j];
        }
    }
    sink = sum;
}

void benchmark_bounds_check(int64_t* data, int size) {
    int64_t sum = 0;
    for (int i = 0; i < ITERATIONS; i++) {
        for (int j = 0; j < size; j++) {
            /* Explicit bounds check (what CBGR does implicitly) */
            if (j >= 0 && j < size) {
                sum += data[j];
            }
        }
    }
    sink = sum;
}

int main(int argc, char** argv) {
    int64_t* data = malloc(ARRAY_SIZE * sizeof(int64_t));
    int64_t** ptrs = malloc(ARRAY_SIZE * sizeof(int64_t*));

    /* Initialize data */
    for (int i = 0; i < ARRAY_SIZE; i++) {
        data[i] = i;
        ptrs[i] = &data[i];
    }

    uint64_t start, end;
    double ns_per_op;

    /* Warmup */
    benchmark_array_access(data, ARRAY_SIZE);

    /* Benchmark: Raw array access (no safety) */
    start = get_time_ns();
    benchmark_array_access(data, ARRAY_SIZE);
    end = get_time_ns();
    ns_per_op = (double)(end - start) / (ITERATIONS * ARRAY_SIZE);
    printf("Array access (unsafe):     %.2f ns/op\n", ns_per_op);

    /* Benchmark: Pointer chase */
    start = get_time_ns();
    benchmark_pointer_chase(ptrs, ARRAY_SIZE);
    end = get_time_ns();
    ns_per_op = (double)(end - start) / (ITERATIONS * ARRAY_SIZE);
    printf("Pointer chase:             %.2f ns/op\n", ns_per_op);

    /* Benchmark: With bounds check */
    start = get_time_ns();
    benchmark_bounds_check(data, ARRAY_SIZE);
    end = get_time_ns();
    ns_per_op = (double)(end - start) / (ITERATIONS * ARRAY_SIZE);
    printf("With bounds check:         %.2f ns/op\n", ns_per_op);

    free(data);
    free(ptrs);

    return 0;
}
