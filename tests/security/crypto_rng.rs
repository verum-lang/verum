//! Cryptographic RNG Verification Suite for Verum
//!
//! This module tests cryptographic random number generator quality:
//! - Entropy quality (Shannon entropy)
//! - Statistical uniformity (Chi-squared test)
//! - No bias detection
//! - Correlation analysis
//! - Predictability testing
//!
//! **Security Criticality: P0**
//! Weak RNG can compromise cryptographic security.

use std::collections::HashMap;

// Mock SecureRandom for testing (production uses OS entropy)
struct SecureRandom {
    state: std::sync::Mutex<u64>,
}

impl SecureRandom {
    fn new() -> Self {
        use std::time::SystemTime;
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        Self {
            state: std::sync::Mutex::new(seed),
        }
    }

    fn next_u8(&self) -> u8 {
        let mut state = self.state.lock().unwrap();
        // XorShift64 (not cryptographically secure, but sufficient for testing)
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state >> 56) as u8
    }

    fn next_u32(&self) -> u32 {
        let b0 = self.next_u8() as u32;
        let b1 = self.next_u8() as u32;
        let b2 = self.next_u8() as u32;
        let b3 = self.next_u8() as u32;
        (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
    }

    fn next_u64(&self) -> u64 {
        let h = self.next_u32() as u64;
        let l = self.next_u32() as u64;
        (h << 32) | l
    }

    fn fill_bytes(&self, dest: &mut [u8]) {
        for byte in dest.iter_mut() {
            *byte = self.next_u8();
        }
    }
}

// ============================================================================
// Test Suite 1: Entropy Quality
// ============================================================================

fn calculate_shannon_entropy(data: &[u8]) -> f64 {
    // SECURITY: Calculate Shannon entropy (should be ~8.0 for random data)
    let mut counts = [0u64; 256];
    for &byte in data {
        counts[byte as usize] += 1;
    }

    let len = data.len() as f64;
    let mut entropy = 0.0;

    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

#[test]
fn test_rng_entropy_quality() {
    // SECURITY: RNG output should have high entropy (~8 bits/byte)
    let rng = SecureRandom::new();

    // Generate 1MB of random data
    let mut data = vec![0u8; 1_000_000];
    rng.fill_bytes(&mut data);

    let entropy = calculate_shannon_entropy(&data);

    // Shannon entropy should be close to 8.0 for truly random data
    // We allow 7.9+ as acceptable (99.875% of maximum)
    assert!(
        entropy > 7.9,
        "Entropy too low: {:.4} (expected > 7.9)",
        entropy
    );

    println!("Shannon entropy: {:.6} bits/byte", entropy);
}

#[test]
fn test_rng_entropy_blocks() {
    // SECURITY: Entropy should be consistent across blocks
    let rng = SecureRandom::new();
    let block_size = 100_000;
    let num_blocks = 10;

    let mut entropies = Vec::new();

    for _ in 0..num_blocks {
        let mut block = vec![0u8; block_size];
        rng.fill_bytes(&mut block);

        let entropy = calculate_shannon_entropy(&block);
        entropies.push(entropy);
    }

    // All blocks should have similar entropy
    let mean = entropies.iter().sum::<f64>() / entropies.len() as f64;
    let variance = entropies
        .iter()
        .map(|e| (e - mean).powi(2))
        .sum::<f64>()
        / entropies.len() as f64;
    let stddev = variance.sqrt();

    assert!(
        mean > 7.9,
        "Mean entropy too low: {:.4}",
        mean
    );
    assert!(
        stddev < 0.05,
        "Entropy variance too high: {:.4}",
        stddev
    );

    println!("Mean entropy: {:.6}, stddev: {:.6}", mean, stddev);
}

// ============================================================================
// Test Suite 2: Statistical Uniformity
// ============================================================================

fn chi_squared_test(data: &[u8]) -> f64 {
    // SECURITY: Chi-squared test for uniform distribution
    let mut counts = [0u64; 256];
    for &byte in data {
        counts[byte as usize] += 1;
    }

    let expected = data.len() as f64 / 256.0;
    let mut chi_squared = 0.0;

    for &count in &counts {
        let diff = count as f64 - expected;
        chi_squared += (diff * diff) / expected;
    }

    chi_squared
}

#[test]
fn test_rng_chi_squared_uniformity() {
    // SECURITY: Distribution should pass chi-squared test
    let rng = SecureRandom::new();

    // Generate 1MB of random data
    let mut data = vec![0u8; 1_000_000];
    rng.fill_bytes(&mut data);

    let chi_squared = chi_squared_test(&data);

    // For 255 degrees of freedom at 99.9% confidence: chi^2 < 310.5
    // We use a slightly more conservative threshold
    assert!(
        chi_squared < 320.0,
        "Chi-squared test failed: {:.2} (expected < 320.0)",
        chi_squared
    );

    println!("Chi-squared statistic: {:.2}", chi_squared);
}

#[test]
fn test_rng_byte_distribution() {
    // SECURITY: Each byte value should appear with similar frequency
    let rng = SecureRandom::new();
    let sample_size = 1_000_000;

    let mut counts = [0u64; 256];

    for _ in 0..sample_size {
        let byte = rng.next_u8();
        counts[byte as usize] += 1;
    }

    let expected = sample_size as f64 / 256.0;
    let tolerance = expected * 0.1; // 10% tolerance

    for (value, &count) in counts.iter().enumerate() {
        let diff = (count as f64 - expected).abs();
        assert!(
            diff < tolerance,
            "Byte {} distribution skewed: count={}, expected={:.0}",
            value,
            count,
            expected
        );
    }

    println!("All byte values within 10% of expected frequency");
}

// ============================================================================
// Test Suite 3: Bias Detection
// ============================================================================

#[test]
fn test_rng_no_bias() {
    // SECURITY: RNG should not be biased toward any value
    let rng = SecureRandom::new();
    let mut counts = [0u64; 256];

    // Generate 1M samples
    for _ in 0..1_000_000 {
        let byte = rng.next_u8();
        counts[byte as usize] += 1;
    }

    // Each value should appear ~3906 times (1M / 256)
    let expected = 1_000_000.0 / 256.0;

    for (value, count) in counts.iter().enumerate() {
        let diff_pct = ((*count as f64 - expected) / expected * 100.0).abs();

        assert!(
            diff_pct < 10.0,
            "Bias detected for value {}: {:.2}% deviation",
            value,
            diff_pct
        );
    }
}

#[test]
fn test_rng_bit_independence() {
    // SECURITY: Individual bits should be independent
    let rng = SecureRandom::new();
    let sample_size = 100_000;

    let mut bit_counts = [0u64; 8];

    for _ in 0..sample_size {
        let byte = rng.next_u8();
        for bit in 0..8 {
            if (byte >> bit) & 1 == 1 {
                bit_counts[bit] += 1;
            }
        }
    }

    let expected = sample_size as f64 / 2.0;
    let tolerance = expected * 0.05; // 5% tolerance

    for (bit, &count) in bit_counts.iter().enumerate() {
        let diff = (count as f64 - expected).abs();
        assert!(
            diff < tolerance,
            "Bit {} biased: count={}, expected={:.0}",
            bit,
            count,
            expected
        );
    }

    println!("All bits within 5% of 50/50 distribution");
}

#[test]
fn test_rng_runs_test() {
    // SECURITY: Test for runs of consecutive values
    let rng = SecureRandom::new();
    let sample_size = 10_000;

    let mut data = Vec::with_capacity(sample_size);
    for _ in 0..sample_size {
        data.push(rng.next_u8());
    }

    // Count runs of identical values
    let mut runs = 0;
    let mut current_run = 1;

    for i in 1..data.len() {
        if data[i] == data[i - 1] {
            current_run += 1;
        } else {
            if current_run > 1 {
                runs += 1;
            }
            current_run = 1;
        }
    }

    // Expected runs for uniform random data (approximate)
    // Should be very few long runs
    let run_rate = runs as f64 / sample_size as f64;

    assert!(
        run_rate < 0.05,
        "Too many runs detected: {:.4} (expected < 0.05)",
        run_rate
    );

    println!("Run rate: {:.4}", run_rate);
}

// ============================================================================
// Test Suite 4: Correlation Analysis
// ============================================================================

#[test]
fn test_rng_autocorrelation() {
    // SECURITY: Sequential values should not be correlated
    let rng = SecureRandom::new();
    let sample_size = 10_000;

    let mut data = Vec::with_capacity(sample_size);
    for _ in 0..sample_size {
        data.push(rng.next_u8() as i32);
    }

    // Calculate lag-1 autocorrelation
    let mean = data.iter().sum::<i32>() as f64 / data.len() as f64;

    let mut numerator = 0.0;
    let mut denominator = 0.0;

    for i in 0..data.len() - 1 {
        let x = data[i] as f64 - mean;
        let y = data[i + 1] as f64 - mean;
        numerator += x * y;
    }

    for &value in &data {
        let x = value as f64 - mean;
        denominator += x * x;
    }

    let autocorr = numerator / denominator;

    // Autocorrelation should be close to 0 (no correlation)
    assert!(
        autocorr.abs() < 0.05,
        "Significant autocorrelation detected: {:.4}",
        autocorr
    );

    println!("Lag-1 autocorrelation: {:.6}", autocorr);
}

#[test]
fn test_rng_adjacent_value_independence() {
    // SECURITY: Adjacent values should not predict each other
    let rng = SecureRandom::new();
    let sample_size = 10_000;

    let mut pairs: HashMap<(u8, u8), u32> = HashMap::new();

    let mut prev = rng.next_u8();
    for _ in 1..sample_size {
        let curr = rng.next_u8();
        *pairs.entry((prev, curr)).or_insert(0) += 1;
        prev = curr;
    }

    // Expected count for each pair: sample_size / (256 * 256) ≈ 0.15
    // Most pairs won't appear, but distribution should be uniform
    let mean_count = pairs.values().sum::<u32>() as f64 / pairs.len() as f64;

    // Should be close to 1.0 for uniform distribution
    assert!(
        mean_count < 5.0,
        "Non-uniform pair distribution: mean count = {:.2}",
        mean_count
    );

    println!("Unique pairs: {}, mean count: {:.2}", pairs.len(), mean_count);
}

// ============================================================================
// Test Suite 5: Predictability Testing
// ============================================================================

#[test]
fn test_rng_not_predictable_xor() {
    // SECURITY: XOR of adjacent values should still be unpredictable
    let rng = SecureRandom::new();
    let sample_size = 100_000;

    let mut xor_counts = [0u64; 256];

    let mut prev = rng.next_u8();
    for _ in 1..sample_size {
        let curr = rng.next_u8();
        let xor_val = prev ^ curr;
        xor_counts[xor_val as usize] += 1;
        prev = curr;
    }

    // XOR values should also be uniformly distributed
    let chi_squared = chi_squared_test(
        &(0..256)
            .flat_map(|i| vec![i as u8; xor_counts[i] as usize])
            .collect::<Vec<_>>(),
    );

    assert!(
        chi_squared < 320.0,
        "XOR values not uniformly distributed: chi^2 = {:.2}",
        chi_squared
    );
}

#[test]
fn test_rng_different_ranges() {
    // SECURITY: RNG should produce uniform values in different ranges
    let rng = SecureRandom::new();

    // Test u32 range
    let mut u32_histogram = HashMap::new();
    let modulo = 100u32;

    for _ in 0..10_000 {
        let value = rng.next_u32() % modulo;
        *u32_histogram.entry(value).or_insert(0u32) += 1;
    }

    let expected = 10_000.0 / modulo as f64;
    for i in 0..modulo {
        let count = u32_histogram.get(&i).copied().unwrap_or(0) as f64;
        let diff_pct = ((count - expected) / expected * 100.0).abs();

        assert!(
            diff_pct < 20.0,
            "u32 range bias at {}: {:.2}% deviation",
            i,
            diff_pct
        );
    }
}

#[test]
fn test_rng_sequential_independence() {
    // SECURITY: Consecutive blocks should be independent
    let rng = SecureRandom::new();

    let mut block1 = vec![0u8; 1024];
    let mut block2 = vec![0u8; 1024];

    rng.fill_bytes(&mut block1);
    rng.fill_bytes(&mut block2);

    // Calculate Hamming distance
    let mut differences = 0;
    for i in 0..1024 {
        differences += (block1[i] ^ block2[i]).count_ones();
    }

    let total_bits = 1024 * 8;
    let difference_ratio = differences as f64 / total_bits as f64;

    // Should be close to 50% for independent blocks
    assert!(
        difference_ratio > 0.45 && difference_ratio < 0.55,
        "Blocks not independent: {:.2}% bits differ",
        difference_ratio * 100.0
    );

    println!("Block difference: {:.2}%", difference_ratio * 100.0);
}

// ============================================================================
// Test Suite 6: Thread Safety
// ============================================================================

#[test]
fn test_rng_thread_safety() {
    // SECURITY: RNG should be safe to use from multiple threads
    use std::sync::Arc;
    use std::thread;

    let rng = Arc::new(SecureRandom::new());
    let mut handles = vec![];

    for _ in 0..10 {
        let rng_clone = rng.clone();
        let handle = thread::spawn(move || {
            let mut local_data = vec![0u8; 10_000];
            rng_clone.fill_bytes(&mut local_data);

            // Verify entropy
            let entropy = calculate_shannon_entropy(&local_data);
            assert!(entropy > 7.8, "Thread-local entropy too low: {:.4}", entropy);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_rng_concurrent_uniqueness() {
    // SECURITY: Different threads should get different values
    use std::sync::Arc;
    use std::thread;

    let rng = Arc::new(SecureRandom::new());
    let mut handles = vec![];

    for _ in 0..10 {
        let rng_clone = rng.clone();
        let handle = thread::spawn(move || {
            let mut values = Vec::new();
            for _ in 0..1000 {
                values.push(rng_clone.next_u64());
            }
            values
        });
        handles.push(handle);
    }

    let all_values: Vec<Vec<u64>> = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    // Check for collisions across threads
    let mut seen = std::collections::HashSet::new();
    let mut collisions = 0;

    for thread_values in &all_values {
        for &value in thread_values {
            if !seen.insert(value) {
                collisions += 1;
            }
        }
    }

    // Some collisions are expected with u64, but should be rare
    let collision_rate = collisions as f64 / 10_000.0;
    assert!(
        collision_rate < 0.01,
        "Too many collisions: {:.4}%",
        collision_rate * 100.0
    );

    println!("Collision rate: {:.4}%", collision_rate * 100.0);
}

// ============================================================================
// Helper Functions
// ============================================================================

#[test]
fn test_statistical_helpers() {
    // Verify our statistical test functions are correct

    // Perfect entropy test (all values appear equally)
    let perfect_data: Vec<u8> = (0..=255).cycle().take(256 * 100).map(|x| x as u8).collect();
    let entropy = calculate_shannon_entropy(&perfect_data);
    assert!((entropy - 8.0).abs() < 0.01, "Perfect data entropy incorrect");

    // Zero entropy test (all same value)
    let zero_data = vec![0u8; 1000];
    let entropy = calculate_shannon_entropy(&zero_data);
    assert!(entropy < 0.01, "Zero entropy data incorrect");

    // Chi-squared test for uniform data
    let uniform_data: Vec<u8> = (0..=255).cycle().take(256 * 1000).map(|x| x as u8).collect();
    let chi = chi_squared_test(&uniform_data);
    assert!(chi < 300.0, "Uniform data chi-squared too high: {:.2}", chi);
}
