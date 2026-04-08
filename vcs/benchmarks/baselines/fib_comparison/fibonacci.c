/*
 * Fibonacci Benchmark - C Implementation
 *
 * Baseline implementation for comparison with Verum, Rust, and Go.
 * Compile with: gcc -O3 -march=native -o fibonacci fibonacci.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>

#define WARMUP_ITERATIONS 10

/* Prevent compiler from optimizing away the result */
volatile int64_t sink;

/* Get current time in nanoseconds */
static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

/* Recursive Fibonacci */
int64_t fib_recursive(int n) {
    if (n <= 1) return n;
    return fib_recursive(n - 1) + fib_recursive(n - 2);
}

/* Iterative Fibonacci */
int64_t fib_iterative(int n) {
    if (n <= 1) return n;

    int64_t a = 0, b = 1;
    for (int i = 2; i <= n; i++) {
        int64_t temp = a + b;
        a = b;
        b = temp;
    }
    return b;
}

/* Matrix multiplication for 2x2 matrices */
static void matrix_mult(int64_t result[2][2], int64_t a[2][2], int64_t b[2][2]) {
    int64_t temp[2][2];
    temp[0][0] = a[0][0] * b[0][0] + a[0][1] * b[1][0];
    temp[0][1] = a[0][0] * b[0][1] + a[0][1] * b[1][1];
    temp[1][0] = a[1][0] * b[0][0] + a[1][1] * b[1][0];
    temp[1][1] = a[1][0] * b[0][1] + a[1][1] * b[1][1];

    result[0][0] = temp[0][0];
    result[0][1] = temp[0][1];
    result[1][0] = temp[1][0];
    result[1][1] = temp[1][1];
}

/* Matrix exponentiation Fibonacci O(log n) */
int64_t fib_matrix(int n) {
    if (n <= 1) return n;

    int64_t result[2][2] = {{1, 0}, {0, 1}};  /* Identity */
    int64_t base[2][2] = {{1, 1}, {1, 0}};
    int64_t temp[2][2];

    while (n > 0) {
        if (n % 2 == 1) {
            matrix_mult(temp, result, base);
            result[0][0] = temp[0][0]; result[0][1] = temp[0][1];
            result[1][0] = temp[1][0]; result[1][1] = temp[1][1];
        }
        matrix_mult(temp, base, base);
        base[0][0] = temp[0][0]; base[0][1] = temp[0][1];
        base[1][0] = temp[1][0]; base[1][1] = temp[1][1];
        n /= 2;
    }

    return result[0][1];
}

void benchmark_recursive_30(void) {
    const int n = 30;
    const int expected = 832040;
    const int iterations = 100;

    /* Warmup */
    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        int64_t result = fib_recursive(n);
        if (result != expected) {
            printf("ERROR: fib(%d) = %lld, expected %d\n", n, (long long)result, expected);
            exit(1);
        }
    }

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        sink = fib_recursive(n);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_call_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] fib_recursive_30: %.3f ms/call\n", per_call_ms);
}

void benchmark_recursive_40(void) {
    const int n = 40;
    const int64_t expected = 102334155;
    const int iterations = 3;

    /* Warmup */
    int64_t result = fib_recursive(n);
    if (result != expected) {
        printf("ERROR: fib(%d) = %lld, expected %lld\n", n, (long long)result, (long long)expected);
        exit(1);
    }

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        sink = fib_recursive(n);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_call_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] fib_recursive_40: %.3f ms/call\n", per_call_ms);
}

void benchmark_iterative_45(void) {
    const int n = 45;
    const int iterations = 10000000;

    /* Warmup */
    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        sink = fib_iterative(n);
    }

    uint64_t start = get_time_ns();
    int64_t sum = 0;
    for (int i = 0; i < iterations; i++) {
        sum += fib_iterative(n);
    }
    sink = sum;
    uint64_t elapsed = get_time_ns() - start;

    double per_call_ns = (double)elapsed / iterations;
    printf("[C] fib_iterative_45: %.3f ns/call\n", per_call_ns);
}

void benchmark_iterative_90(void) {
    const int n = 90;
    const int iterations = 1000000;

    uint64_t start = get_time_ns();
    int64_t sum = 0;
    for (int i = 0; i < iterations; i++) {
        sum += fib_iterative(n);
    }
    sink = sum;
    uint64_t elapsed = get_time_ns() - start;

    double per_call_ns = (double)elapsed / iterations;
    printf("[C] fib_iterative_90: %.3f ns/call\n", per_call_ns);
}

void benchmark_matrix_1000(void) {
    const int n = 1000;
    const int iterations = 1000000;

    /* Warmup */
    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        sink = fib_matrix(n);
    }

    uint64_t start = get_time_ns();
    int64_t sum = 0;
    for (int i = 0; i < iterations; i++) {
        sum += fib_matrix(n);
    }
    sink = sum;
    uint64_t elapsed = get_time_ns() - start;

    double per_call_ns = (double)elapsed / iterations;
    printf("[C] fib_matrix_1000: %.3f ns/call\n", per_call_ns);
}

int main(void) {
    printf("=== Fibonacci Benchmark - C ===\n\n");

    benchmark_recursive_30();
    benchmark_recursive_40();
    benchmark_iterative_45();
    benchmark_iterative_90();
    benchmark_matrix_1000();

    printf("\nAll benchmarks completed.\n");
    return 0;
}
