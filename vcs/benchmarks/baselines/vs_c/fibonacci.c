/*
 * Fibonacci Baseline - C Implementation
 *
 * Classic recursive fibonacci for function call overhead comparison.
 *
 * Compile: cc -O3 -march=native -o fibonacci fibonacci.c
 */

#include <stdio.h>
#include <stdint.h>
#include <time.h>

#define ITERATIONS 100

static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

/* Recursive fibonacci */
int64_t fib_recursive(int n) {
    if (n <= 1) return n;
    return fib_recursive(n - 1) + fib_recursive(n - 2);
}

/* Iterative fibonacci */
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

/* Tail-recursive style (optimizable) */
int64_t fib_tail_helper(int n, int64_t a, int64_t b) {
    if (n == 0) return a;
    if (n == 1) return b;
    return fib_tail_helper(n - 1, b, a + b);
}

int64_t fib_tail(int n) {
    return fib_tail_helper(n, 0, 1);
}

int main(int argc, char** argv) {
    uint64_t start, end;
    double ns_per_call;
    volatile int64_t result;

    /* Warmup */
    for (int i = 0; i < 10; i++) {
        result = fib_recursive(30);
    }

    /* Recursive fib(30) */
    start = get_time_ns();
    for (int i = 0; i < ITERATIONS; i++) {
        result = fib_recursive(30);
    }
    end = get_time_ns();
    ns_per_call = (double)(end - start) / ITERATIONS;
    printf("fib_recursive(30):         %.2f us/call (result: %lld)\n",
           ns_per_call / 1000.0, (long long)result);

    /* Iterative fib(30) */
    start = get_time_ns();
    for (int i = 0; i < ITERATIONS * 10000; i++) {
        result = fib_iterative(30);
    }
    end = get_time_ns();
    ns_per_call = (double)(end - start) / (ITERATIONS * 10000);
    printf("fib_iterative(30):         %.2f ns/call (result: %lld)\n",
           ns_per_call, (long long)result);

    /* Tail-recursive fib(30) */
    start = get_time_ns();
    for (int i = 0; i < ITERATIONS * 10000; i++) {
        result = fib_tail(30);
    }
    end = get_time_ns();
    ns_per_call = (double)(end - start) / (ITERATIONS * 10000);
    printf("fib_tail(30):              %.2f ns/call (result: %lld)\n",
           ns_per_call, (long long)result);

    /* Recursive fib(40) - stress test */
    printf("\nStress test (fib_recursive(40)):\n");
    start = get_time_ns();
    result = fib_recursive(40);
    end = get_time_ns();
    printf("fib_recursive(40):         %.2f ms (result: %lld)\n",
           (double)(end - start) / 1000000.0, (long long)result);

    return 0;
}
