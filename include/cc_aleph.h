#ifndef CC_ALEPH_H
#define CC_ALEPH_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Opaque pointer to a heap-allocated CC-Aleph filter instance.
 */
typedef struct CcAlephFilter CcAlephFilter;
typedef CcAlephFilter* CcAlephHandle;

/**
 * Allocates and initializes a new CC-Aleph filter instance.
 *
 * @param base_fp_bits Base fingerprint bitwidth (typically 16-24).
 * @return Handle pointer to the filter instance.
 */
CcAlephHandle cc_aleph_create(size_t base_fp_bits);

/**
 * Inserts a key into the filter.
 *
 * @param handle Filter handle returned by cc_aleph_create.
 * @param key_ptr Pointer to the byte array of the key.
 * @param key_len Length of the key in bytes.
 * @return 0 on success, -1 on failure or null pointer.
 */
int32_t cc_aleph_insert(CcAlephHandle handle, const uint8_t* key_ptr, size_t key_len);

/**
 * Queries key membership in the filter.
 *
 * @param handle Filter handle returned by cc_aleph_create.
 * @param key_ptr Pointer to the byte array of the key.
 * @param key_len Length of the key in bytes.
 * @return 1 if probably present, 0 if definitely absent.
 */
int32_t cc_aleph_contains(CcAlephHandle handle, const uint8_t* key_ptr, size_t key_len);

/**
 * Deallocates and destroys the CC-Aleph filter instance.
 *
 * @param handle Filter handle to destroy.
 */
void cc_aleph_destroy(CcAlephHandle handle);

#ifdef __cplusplus
}
#endif

#endif /* CC_ALEPH_H */