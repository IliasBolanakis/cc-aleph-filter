use cc_aleph::c_api::{cc_aleph_contains, cc_aleph_create, cc_aleph_destroy, cc_aleph_insert};
use std::ptr;

#[test]
fn test_c_api_lifecycle_and_lookups() {
    unsafe {
        // Test null safety
        assert_eq!(cc_aleph_insert(ptr::null_mut(), ptr::null(), 0), -1);
        assert_eq!(cc_aleph_contains(ptr::null_mut(), ptr::null(), 0), 0);
        cc_aleph_destroy(ptr::null_mut()); // No-op, must not segfault

        // Initialize instance
        let handle = cc_aleph_create(20);
        assert!(!handle.is_null());

        let key1 = b"user_account_90123";
        let key2 = b"user_account_90124";
        let key_absent = b"non_existent_key_99999";

        // Membership before insertion
        assert_eq!(cc_aleph_contains(handle, key1.as_ptr(), key1.len()), 0);

        // Ingestion
        assert_eq!(cc_aleph_insert(handle, key1.as_ptr(), key1.len()), 0);
        assert_eq!(cc_aleph_insert(handle, key2.as_ptr(), key2.len()), 0);

        // Verification
        assert_eq!(cc_aleph_contains(handle, key1.as_ptr(), key1.len()), 1);
        assert_eq!(cc_aleph_contains(handle, key2.as_ptr(), key2.len()), 1);
        assert_eq!(
            cc_aleph_contains(handle, key_absent.as_ptr(), key_absent.len()),
            0
        );

        // Deallocate
        cc_aleph_destroy(handle);
    }
}
