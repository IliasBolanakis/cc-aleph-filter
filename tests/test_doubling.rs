//! Integration tests verifying directory doubling and False Positive Rate bounds.

use cc_aleph::CcAlephFilter;

#[test]
fn test_ten_successive_doublings() {
    // Base fingerprint width = 20 bits allows 10 doublings down to 10 bits.
    let base_fp = 20;
    let filter = CcAlephFilter::with_fingerprint_bits(base_fp);

    let initial_blocks = filter.num_blocks();
    assert_eq!(initial_blocks, 1);

    // Insert keys until the filter performs at least 10 doublings (1024 blocks)
    let target_blocks = 1024;
    let mut inserted_keys = Vec::new();
    let mut counter = 0usize;

    while filter.num_blocks() < target_blocks {
        let key = format!("progressive_key_{counter}");
        let key_bytes = key.into_bytes();
        filter.insert(&key_bytes).unwrap();
        inserted_keys.push(key_bytes);
        counter += 1;
    }

    assert!(
        filter.num_blocks() >= target_blocks,
        "Filter failed to scale through 10 successive doublings"
    );
    assert!(
        filter.global_depth() >= 10,
        "Directory global depth must be at least 10"
    );

    // Verify 100% exact-match recall: zero false negatives
    for key in &inserted_keys {
        assert!(
            filter.contains(key),
            "False negative detected for inserted key"
        );
    }

    // Verify empirical False Positive Rate (FPR) bounds
    let num_negative_probes = 10_000;
    let mut false_positives = 0;

    for i in 0..num_negative_probes {
        let probe_key = format!("non_existent_negative_probe_{i}");
        if filter.contains(probe_key.as_bytes()) {
            false_positives += 1;
        }
    }

    let empirical_fpr = false_positives as f64 / num_negative_probes as f64;

    // Minimum fingerprint length remaining after at least 10 doublings is ~10 bits
    // Theoretical upper bound: FPR <= 2^(-F_min) = 2^(-10) ~= 0.000976 (~0.1%)
    let theoretical_max_fpr = 1.0 / (1u64 << (base_fp - filter.global_depth())) as f64;

    // Allow a 5x margin of safety for empirical variance in small probe sets
    let permissible_fpr = (theoretical_max_fpr * 5.0).max(0.01);

    assert!(
        empirical_fpr <= permissible_fpr,
        "Empirical FPR {empirical_fpr} exceeds permissible bound {permissible_fpr}"
    );
}
