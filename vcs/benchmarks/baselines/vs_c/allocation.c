/*
 * Allocation Baseline - C Implementation
 *
 * This benchmark measures malloc/free performance in C as a baseline
 * for Verum's allocation performance.
 *
 * Target: Verum allocation should be within 2x of C malloc for small objects.
 *
 * Compile: cc -O3 -march=native -o allocation allocation.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>

#define SMALL_ITERATIONS 1000000
#define MEDIUM_ITERATIONS 100000
#define LARGE_ITERATIONS 1000

static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

typedef struct {
    int32_t a;
    int32_t b;
    double c;
    char d;
} SmallStruct;

void benchmark_small_alloc(void) {
    for (int i = 0; i < SMALL_ITERATIONS; i++) {
        SmallStruct* obj = malloc(sizeof(SmallStruct));
        obj->a = 1;
        obj->b = 2;
        obj->c = 3.14;
        obj->d = 1;
        free(obj);
    }
}

void benchmark_medium_alloc(void) {
    for (int i = 0; i < MEDIUM_ITERATIONS; i++) {
        void* data = malloc(1024);
        memset(data, 0, 1024);
        free(data);
    }
}

void benchmark_large_alloc(void) {
    for (int i = 0; i < LARGE_ITERATIONS; i++) {
        void* data = malloc(1024 * 1024);
        memset(data, 0, 1024 * 1024);
        free(data);
    }
}

void benchmark_batch_alloc(void) {
    void* ptrs[1000];

    for (int round = 0; round < 1000; round++) {
        /* Allocate batch */
        for (int i = 0; i < 1000; i++) {
            ptrs[i] = malloc(sizeof(SmallStruct));
        }
        /* Free batch */
        for (int i = 0; i < 1000; i++) {
            free(ptrs[i]);
        }
    }
}

void benchmark_realloc(void) {
    for (int i = 0; i < 10000; i++) {
        void* data = malloc(64);
        for (int size = 64; size <= 16384; size *= 2) {
            data = realloc(data, size);
        }
        free(data);
    }
}

int main(int argc, char** argv) {
    uint64_t start, end;
    double ns_per_op;

    /* Warmup */
    benchmark_small_alloc();

    /* Small allocation */
    start = get_time_ns();
    benchmark_small_alloc();
    end = get_time_ns();
    ns_per_op = (double)(end - start) / SMALL_ITERATIONS;
    printf("Small alloc (64B):         %.2f ns/op\n", ns_per_op);

    /* Medium allocation */
    start = get_time_ns();
    benchmark_medium_alloc();
    end = get_time_ns();
    ns_per_op = (double)(end - start) / MEDIUM_ITERATIONS;
    printf("Medium alloc (1KB):        %.2f ns/op\n", ns_per_op);

    /* Large allocation */
    start = get_time_ns();
    benchmark_large_alloc();
    end = get_time_ns();
    ns_per_op = (double)(end - start) / LARGE_ITERATIONS;
    printf("Large alloc (1MB):         %.2f ns/op\n", ns_per_op);

    /* Batch allocation */
    start = get_time_ns();
    benchmark_batch_alloc();
    end = get_time_ns();
    ns_per_op = (double)(end - start) / 1000000;
    printf("Batch alloc:               %.2f ns/op\n", ns_per_op);

    /* Realloc pattern */
    start = get_time_ns();
    benchmark_realloc();
    end = get_time_ns();
    ns_per_op = (double)(end - start) / 10000;
    printf("Realloc pattern:           %.2f ns/op\n", ns_per_op);

    return 0;
}
