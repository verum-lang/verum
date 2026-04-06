// SIMD Cross-Platform Tests

use super::detection::{Architecture, FeatureDetector, PlatformInfo};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_detection() {
        let detector = FeatureDetector::new();
        let platform = detector.platform();

        println!("Architecture: {}", platform.architecture);
        println!("SIMD available: {}", detector.has_simd());

        match platform.architecture {
            Architecture::X86_64 | Architecture::Aarch64 => {
                assert!(detector.has_simd());
            }
            _ => {}
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_x86_64_features() {
        let detector = FeatureDetector::new();
        let features = detector.x86_64_features();

        println!("SSE2: {}", features.sse2);
        println!("SSE4.2: {}", features.sse4_2);
        println!("AVX: {}", features.avx);
        println!("AVX2: {}", features.avx2);
        println!("AVX-512F: {}", features.avx512f);

        // SSE2 is required for x86_64
        assert!(features.sse2);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_sse2_operations() {
        if is_x86_feature_detected!("sse2") {
            #[cfg(target_arch = "x86_64")]
            {
                use std::arch::x86_64::*;

                unsafe {
                    // Create two SSE vectors
                    let a = _mm_set_epi32(4, 3, 2, 1);
                    let b = _mm_set_epi32(8, 7, 6, 5);

                    // Add them
                    let c = _mm_add_epi32(a, b);

                    // Extract results
                    let mut result = [0i32; 4];
                    _mm_storeu_si128(result.as_mut_ptr() as *mut __m128i, c);

                    assert_eq!(result, [6, 9, 10, 12]);
                }
            }
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx2_operations() {
        if is_x86_feature_detected!("avx2") {
            #[cfg(target_arch = "x86_64")]
            {
                use std::arch::x86_64::*;

                unsafe {
                    // AVX2: 256-bit operations
                    let a = _mm256_set_epi32(8, 7, 6, 5, 4, 3, 2, 1);
                    let b = _mm256_set_epi32(16, 15, 14, 13, 12, 11, 10, 9);

                    let c = _mm256_add_epi32(a, b);

                    let mut result = [0i32; 8];
                    _mm256_storeu_si256(result.as_mut_ptr() as *mut __m256i, c);

                    assert_eq!(result, [10, 13, 16, 17, 18, 19, 20, 24]);
                }
            }
        } else {
            println!("AVX2 not available");
        }
    }

    #[test]
    #[cfg(target_arch = "aarch64")]
    fn test_neon_operations() {
        #[cfg(target_arch = "aarch64")]
        {
            use std::arch::aarch64::*;

            unsafe {
                // NEON is always available on aarch64
                let a = vdupq_n_s32(5);
                let b = vdupq_n_s32(3);
                let c = vaddq_s32(a, b);

                let mut result = [0i32; 4];
                vst1q_s32(result.as_mut_ptr(), c);

                assert_eq!(result, [8, 8, 8, 8]);
            }
        }
    }

    #[test]
    fn test_simd_performance() {
        const SIZE: usize = 1024;
        let a: Vec<f32> = (0..SIZE).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..SIZE).map(|i| (i * 2) as f32).collect();
        let mut c = vec![0.0f32; SIZE];

        // Scalar addition
        let start = std::time::Instant::now();
        for i in 0..SIZE {
            c[i] = a[i] + b[i];
        }
        let scalar_time = start.elapsed();

        // SIMD addition (if available)
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("sse") {
                use std::arch::x86_64::*;

                let mut c_simd = vec![0.0f32; SIZE];
                let start = std::time::Instant::now();

                unsafe {
                    for i in (0..SIZE).step_by(4) {
                        let va = _mm_loadu_ps(a.as_ptr().add(i));
                        let vb = _mm_loadu_ps(b.as_ptr().add(i));
                        let vc = _mm_add_ps(va, vb);
                        _mm_storeu_ps(c_simd.as_mut_ptr().add(i), vc);
                    }
                }

                let simd_time = start.elapsed();

                println!("Scalar: {:?}, SIMD: {:?}", scalar_time, simd_time);
                println!("Speedup: {:.2}x", scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64);

                // Verify results match
                for i in 0..SIZE {
                    assert!((c[i] - c_simd[i]).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_fma_operations() {
        if is_x86_feature_detected!("fma") {
            use std::arch::x86_64::*;

            unsafe {
                let a = _mm_set_ps(4.0, 3.0, 2.0, 1.0);
                let b = _mm_set_ps(2.0, 2.0, 2.0, 2.0);
                let c = _mm_set_ps(1.0, 1.0, 1.0, 1.0);

                // a * b + c
                let result = _mm_fmadd_ps(a, b, c);

                let mut output = [0.0f32; 4];
                _mm_storeu_ps(output.as_mut_ptr(), result);

                assert_eq!(output, [3.0, 5.0, 7.0, 9.0]);
            }
        }
    }

    #[test]
    fn test_auto_vectorization() {
        const SIZE: usize = 1024;
        let mut data: Vec<f32> = (0..SIZE).map(|i| i as f32).collect();

        // Compiler should auto-vectorize this
        for x in data.iter_mut() {
            *x = *x * 2.0 + 1.0;
        }

        for (i, &val) in data.iter().enumerate() {
            assert_eq!(val, (i as f32) * 2.0 + 1.0);
        }
    }
}
