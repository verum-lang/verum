/*
 * SIMD Throughput Baseline - C Implementation
 *
 * This benchmark measures SIMD performance in C using AVX2 intrinsics
 * as a baseline for Verum's SIMD performance targets.
 *
 * Compile: cc -O3 -march=native -mavx2 -mfma -o simd_throughput simd_throughput.c
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
#include <immintrin.h>

#define WARMUP_ITERATIONS 1000
#define BENCHMARK_ITERATIONS 10000
#define ARRAY_SIZE (1024 * 1024)

static inline uint64_t get_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

/* Prevent optimization */
static volatile float sink_float;
static volatile __m256 sink_vec;

void benchmark_scalar_add(void) {
    float* a = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* b = aligned_alloc(32, ARRAY_SIZE * sizeof(float));

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        a[i] = (float)i;
        b[i] = (float)(i * 2);
    }

    /* Warmup */
    for (int iter = 0; iter < WARMUP_ITERATIONS; iter++) {
        for (size_t i = 0; i < ARRAY_SIZE; i++) {
            a[i] = a[i] + b[i];
        }
    }

    uint64_t start = get_time_ns();

    for (int iter = 0; iter < BENCHMARK_ITERATIONS; iter++) {
        for (size_t i = 0; i < ARRAY_SIZE; i++) {
            a[i] = a[i] + b[i];
        }
    }

    uint64_t end = get_time_ns();
    sink_float = a[ARRAY_SIZE / 2];

    double ops = (double)BENCHMARK_ITERATIONS * ARRAY_SIZE;
    double gflops = ops / (end - start);
    printf("scalar_add_f32:       %.2f GFLOPS\n", gflops);

    free(a);
    free(b);
}

void benchmark_avx2_add(void) {
    float* a = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* b = aligned_alloc(32, ARRAY_SIZE * sizeof(float));

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        a[i] = (float)i;
        b[i] = (float)(i * 2);
    }

    size_t vec_count = ARRAY_SIZE / 8;

    /* Warmup */
    for (int iter = 0; iter < WARMUP_ITERATIONS; iter++) {
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        for (size_t i = 0; i < vec_count; i++) {
            va[i] = _mm256_add_ps(va[i], vb[i]);
        }
    }

    uint64_t start = get_time_ns();

    for (int iter = 0; iter < BENCHMARK_ITERATIONS; iter++) {
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        for (size_t i = 0; i < vec_count; i++) {
            va[i] = _mm256_add_ps(va[i], vb[i]);
        }
    }

    uint64_t end = get_time_ns();
    sink_float = a[ARRAY_SIZE / 2];

    double ops = (double)BENCHMARK_ITERATIONS * ARRAY_SIZE;
    double gflops = ops / (end - start);
    double speedup = gflops;
    printf("avx2_add_f32x8:       %.2f GFLOPS\n", gflops);

    free(a);
    free(b);
}

void benchmark_fma(void) {
    float* a = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* b = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* c = aligned_alloc(32, ARRAY_SIZE * sizeof(float));

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        a[i] = (float)(i % 1000) * 0.001f;
        b[i] = 2.0f;
        c[i] = 1.0f;
    }

    size_t vec_count = ARRAY_SIZE / 8;

    /* Warmup */
    for (int iter = 0; iter < WARMUP_ITERATIONS; iter++) {
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        __m256* vc = (__m256*)c;
        for (size_t i = 0; i < vec_count; i++) {
            va[i] = _mm256_fmadd_ps(va[i], vb[i], vc[i]);
        }
    }

    uint64_t start = get_time_ns();

    for (int iter = 0; iter < BENCHMARK_ITERATIONS; iter++) {
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        __m256* vc = (__m256*)c;
        for (size_t i = 0; i < vec_count; i++) {
            va[i] = _mm256_fmadd_ps(va[i], vb[i], vc[i]);
        }
    }

    uint64_t end = get_time_ns();
    sink_float = a[ARRAY_SIZE / 2];

    /* FMA counts as 2 operations (multiply + add) */
    double ops = (double)BENCHMARK_ITERATIONS * ARRAY_SIZE * 2;
    double gflops = ops / (end - start);
    printf("avx2_fma_f32x8:       %.2f GFLOPS\n", gflops);

    free(a);
    free(b);
    free(c);
}

void benchmark_dot_product(void) {
    float* a = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* b = aligned_alloc(32, ARRAY_SIZE * sizeof(float));

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        a[i] = (float)(i % 100) * 0.01f;
        b[i] = (float)(i % 100) * 0.02f;
    }

    size_t vec_count = ARRAY_SIZE / 8;
    float result = 0.0f;

    /* Warmup */
    for (int iter = 0; iter < WARMUP_ITERATIONS; iter++) {
        __m256 sum = _mm256_setzero_ps();
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        for (size_t i = 0; i < vec_count; i++) {
            sum = _mm256_fmadd_ps(va[i], vb[i], sum);
        }
        /* Horizontal sum */
        __m128 hi = _mm256_extractf128_ps(sum, 1);
        __m128 lo = _mm256_castps256_ps128(sum);
        __m128 sum128 = _mm_add_ps(lo, hi);
        sum128 = _mm_hadd_ps(sum128, sum128);
        sum128 = _mm_hadd_ps(sum128, sum128);
        result = _mm_cvtss_f32(sum128);
    }

    uint64_t start = get_time_ns();

    for (int iter = 0; iter < BENCHMARK_ITERATIONS; iter++) {
        __m256 sum = _mm256_setzero_ps();
        __m256* va = (__m256*)a;
        __m256* vb = (__m256*)b;
        for (size_t i = 0; i < vec_count; i++) {
            sum = _mm256_fmadd_ps(va[i], vb[i], sum);
        }
        __m128 hi = _mm256_extractf128_ps(sum, 1);
        __m128 lo = _mm256_castps256_ps128(sum);
        __m128 sum128 = _mm_add_ps(lo, hi);
        sum128 = _mm_hadd_ps(sum128, sum128);
        sum128 = _mm_hadd_ps(sum128, sum128);
        result = _mm_cvtss_f32(sum128);
    }

    uint64_t end = get_time_ns();
    sink_float = result;

    double ops = (double)BENCHMARK_ITERATIONS * ARRAY_SIZE * 2;  /* mul + add */
    double gflops = ops / (end - start);
    printf("avx2_dot_product:     %.2f GFLOPS\n", gflops);

    free(a);
    free(b);
}

void benchmark_memory_bandwidth(void) {
    float* src = aligned_alloc(32, ARRAY_SIZE * sizeof(float));
    float* dst = aligned_alloc(32, ARRAY_SIZE * sizeof(float));

    for (size_t i = 0; i < ARRAY_SIZE; i++) {
        src[i] = (float)i;
    }

    size_t vec_count = ARRAY_SIZE / 8;

    /* Warmup */
    for (int iter = 0; iter < WARMUP_ITERATIONS; iter++) {
        __m256* vsrc = (__m256*)src;
        __m256* vdst = (__m256*)dst;
        for (size_t i = 0; i < vec_count; i++) {
            vdst[i] = vsrc[i];
        }
    }

    uint64_t start = get_time_ns();

    for (int iter = 0; iter < BENCHMARK_ITERATIONS; iter++) {
        __m256* vsrc = (__m256*)src;
        __m256* vdst = (__m256*)dst;
        for (size_t i = 0; i < vec_count; i++) {
            vdst[i] = vsrc[i];
        }
    }

    uint64_t end = get_time_ns();
    sink_float = dst[ARRAY_SIZE / 2];

    double bytes = (double)BENCHMARK_ITERATIONS * ARRAY_SIZE * sizeof(float) * 2;  /* read + write */
    double gb_per_sec = bytes / (end - start);
    printf("memory_bandwidth:     %.2f GB/s\n", gb_per_sec);

    free(src);
    free(dst);
}

int main(void) {
    printf("=== C SIMD Throughput Baseline (AVX2) ===\n\n");

    benchmark_scalar_add();
    benchmark_avx2_add();
    benchmark_fma();
    benchmark_dot_product();
    benchmark_memory_bandwidth();

    printf("\nThese values serve as baseline for Verum SIMD targets:\n");
    printf("  - Verum should achieve > 80%% of C SIMD performance\n");
    printf("  - Auto-vectorization should reach > 70%% of explicit SIMD\n");

    return 0;
}
