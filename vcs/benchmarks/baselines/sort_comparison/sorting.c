/*
 * Sorting Algorithm Benchmark - C Implementation
 *
 * Baseline implementation for comparison with Verum, Rust, and Go.
 * Compile with: gcc -O3 -march=native -o sorting sorting.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>

#define WARMUP_ITERATIONS 5

/* Prevent compiler from optimizing away the result */
volatile int64_t sink;

/* Simple xorshift64 RNG */
typedef struct {
    uint64_t state;
} Rng;

static inline uint64_t rng_next(Rng *rng) {
    rng->state ^= rng->state << 13;
    rng->state ^= rng->state >> 7;
    rng->state ^= rng->state << 17;
    return rng->state;
}

/* Get current time in nanoseconds */
static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

/* Generate random array */
int64_t* generate_random_array(size_t size, uint64_t seed) {
    int64_t *arr = malloc(size * sizeof(int64_t));
    Rng rng = {seed};

    for (size_t i = 0; i < size; i++) {
        arr[i] = (int64_t)rng_next(&rng);
    }
    return arr;
}

/* Generate sorted array */
int64_t* generate_sorted_array(size_t size) {
    int64_t *arr = malloc(size * sizeof(int64_t));
    for (size_t i = 0; i < size; i++) {
        arr[i] = (int64_t)i;
    }
    return arr;
}

/* Generate reverse-sorted array */
int64_t* generate_reverse_array(size_t size) {
    int64_t *arr = malloc(size * sizeof(int64_t));
    for (size_t i = 0; i < size; i++) {
        arr[i] = (int64_t)(size - 1 - i);
    }
    return arr;
}

/* Comparison function for qsort */
int compare_int64(const void *a, const void *b) {
    int64_t va = *(const int64_t*)a;
    int64_t vb = *(const int64_t*)b;
    return (va > vb) - (va < vb);
}

/* Quicksort implementation */
void swap(int64_t *a, int64_t *b) {
    int64_t temp = *a;
    *a = *b;
    *b = temp;
}

size_t partition(int64_t *arr, size_t low, size_t high) {
    size_t pivot_idx = low + (high - low) / 2;
    swap(&arr[pivot_idx], &arr[high]);
    int64_t pivot = arr[high];

    size_t i = low;
    for (size_t j = low; j < high; j++) {
        if (arr[j] < pivot) {
            swap(&arr[i], &arr[j]);
            i++;
        }
    }
    swap(&arr[i], &arr[high]);
    return i;
}

void quicksort(int64_t *arr, size_t low, size_t high) {
    if (low < high) {
        size_t pi = partition(arr, low, high);
        if (pi > 0) quicksort(arr, low, pi - 1);
        quicksort(arr, pi + 1, high);
    }
}

/* Heapsort implementation */
void heapify(int64_t *arr, size_t n, size_t i) {
    size_t largest = i;
    size_t left = 2 * i + 1;
    size_t right = 2 * i + 2;

    if (left < n && arr[left] > arr[largest])
        largest = left;

    if (right < n && arr[right] > arr[largest])
        largest = right;

    if (largest != i) {
        swap(&arr[i], &arr[largest]);
        heapify(arr, n, largest);
    }
}

void heapsort(int64_t *arr, size_t n) {
    for (size_t i = n / 2; i > 0; i--) {
        heapify(arr, n, i - 1);
    }
    for (size_t i = n - 1; i > 0; i--) {
        swap(&arr[0], &arr[i]);
        heapify(arr, i, 0);
    }
}

void benchmark_sort_100(void) {
    const size_t size = 100;
    const int iterations = 100000;

    /* Warmup */
    for (int i = 0; i < WARMUP_ITERATIONS; i++) {
        int64_t *arr = generate_random_array(size, 42);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        free(arr);
    }

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_us = (double)elapsed / 1000.0 / iterations;
    printf("[C] sort_100: %.3f us/sort\n", per_sort_us);
}

void benchmark_sort_1000(void) {
    const size_t size = 1000;
    const int iterations = 10000;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_us = (double)elapsed / 1000.0 / iterations;
    printf("[C] sort_1000: %.3f us/sort\n", per_sort_us);
}

void benchmark_sort_10000(void) {
    const size_t size = 10000;
    const int iterations = 1000;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_us = (double)elapsed / 1000.0 / iterations;
    printf("[C] sort_10000: %.3f us/sort\n", per_sort_us);
}

void benchmark_sort_100000(void) {
    const size_t size = 100000;
    const int iterations = 100;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] sort_100000: %.3f ms/sort\n", per_sort_ms);
}

void benchmark_sort_1000000(void) {
    const size_t size = 1000000;
    const int iterations = 10;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] sort_1000000: %.3f ms/sort\n", per_sort_ms);
}

void benchmark_sort_sorted(void) {
    const size_t size = 100000;
    const int iterations = 100;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_sorted_array(size);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] sort_sorted_100k: %.3f ms/sort\n", per_sort_ms);
}

void benchmark_sort_reverse(void) {
    const size_t size = 100000;
    const int iterations = 100;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_reverse_array(size);
        qsort(arr, size, sizeof(int64_t), compare_int64);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] sort_reverse_100k: %.3f ms/sort\n", per_sort_ms);
}

void benchmark_quicksort(void) {
    const size_t size = 100000;
    const int iterations = 50;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        quicksort(arr, 0, size - 1);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] quicksort_100k: %.3f ms/sort\n", per_sort_ms);
}

void benchmark_heapsort(void) {
    const size_t size = 100000;
    const int iterations = 50;

    uint64_t start = get_time_ns();
    for (int i = 0; i < iterations; i++) {
        int64_t *arr = generate_random_array(size, i);
        heapsort(arr, size);
        sink = arr[0];
        free(arr);
    }
    uint64_t elapsed = get_time_ns() - start;

    double per_sort_ms = (double)elapsed / 1000000.0 / iterations;
    printf("[C] heapsort_100k: %.3f ms/sort\n", per_sort_ms);
}

int main(void) {
    printf("=== Sorting Benchmark - C ===\n\n");

    benchmark_sort_100();
    benchmark_sort_1000();
    benchmark_sort_10000();
    benchmark_sort_100000();
    benchmark_sort_1000000();
    benchmark_sort_sorted();
    benchmark_sort_reverse();
    benchmark_quicksort();
    benchmark_heapsort();

    printf("\nAll benchmarks completed.\n");
    return 0;
}
