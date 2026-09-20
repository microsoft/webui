#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Opaque C handle for a loaded WebUI protocol.
 */
typedef void webui_protocol_t;

/**
 * Opaque C handle for a host-driven progressive response.
 */
typedef void webui_streaming_session_t;

/**
 * Opaque owned result from one streaming start, resume, or advance call.
 */
typedef void webui_streaming_step_t;

/**
 * C-safe boundary mode value accepted by [`webui_streaming_session_resume`].
 */
typedef uint32_t webui_boundary_mode_t;

/**
 * C-safe boundary key discriminator returned by
 * [`webui_streaming_step_boundary_key_type`].
 */
typedef uint32_t webui_boundary_key_type_t;

/**
 * Commit the boundary once and release its boundary-local roots.
 */
#define WEBUI_BOUNDARY_MODE_FINAL 0

/**
 * Retain live roots until terminal so updates may target the boundary.
 */
#define WEBUI_BOUNDARY_MODE_UPDATABLE 1

/**
 * The boundary declaration has no runtime key.
 */
#define WEBUI_BOUNDARY_KEY_NONE 0

/**
 * The boundary key is a UTF-8 string.
 */
#define WEBUI_BOUNDARY_KEY_STRING 1

/**
 * The boundary key is a finite JSON number.
 */
#define WEBUI_BOUNDARY_KEY_NUMBER 2

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Return the last error message, or `NULL` if no error has occurred.
 *
 * The returned pointer is valid until the next FFI call **on the same thread**.
 * Callers **must not** free the returned pointer.
 *
 * # Thread Safety
 *
 * Each thread has its own independent error state.
 */
const char *webui_last_error(void);

/**
 * Create a new WebUI handler instance.
 *
 * Returns an opaque pointer that must be passed to other `webui_handler_*`
 * functions and eventually freed with [`webui_handler_destroy`].
 */
void *webui_handler_create(void);

/**
 * Create a new WebUI handler instance with a named plugin.
 *
 * # Arguments
 *
 * * `plugin_id` - Null-terminated UTF-8 string identifying the plugin.
 *   Refer to the CLI/crate documentation for the current list of supported
 *   identifiers.
 *
 * # Returns
 *
 * An opaque pointer that must be freed with [`webui_handler_destroy`],
 * or `NULL` on error (call [`webui_last_error`] for details).
 *
 * # Safety
 *
 * `plugin_id` must be a valid null-terminated UTF-8 string, or `NULL`.
 */
void *webui_handler_create_with_plugin(const char *plugin_id);

/**
 * Destroy a WebUI handler instance.
 *
 * # Safety
 *
 * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`],
 * or `NULL` (in which case this function is a no-op).
 */
void webui_handler_destroy(void *handler_ptr);

/**
 * Decode and index a WebUI protocol for repeated rendering.
 *
 * The returned handle is thread-safe and must be released with
 * [`webui_protocol_destroy`].
 *
 * # Safety
 *
 * `protocol_data` must point to `protocol_len` readable bytes.
 */
webui_protocol_t *webui_protocol_create(const uint8_t *protocol_data, uintptr_t protocol_len);

/**
 * Destroy a loaded WebUI protocol handle.
 *
 * # Safety
 *
 * `protocol_ptr` must be a pointer returned by [`webui_protocol_create`], or
 * `NULL` for a no-op.
 */
void webui_protocol_destroy(webui_protocol_t *protocol_ptr);

/**
 * Set the CSP nonce for inline `<script>` tags on a handler instance.
 *
 * When set, all subsequent renders via [`webui_handler_render`] will include
 * `nonce="VALUE"` on inline script tags and emit a
 * `<meta name="webui-nonce" content="VALUE">` tag in the `<head>`.
 *
 * Pass `NULL` to clear a previously set nonce.
 *
 * # Thread Safety
 *
 * Concurrent render calls are supported after configuration. Callers must not
 * call `set_nonce` or destroy the handler concurrently with any operation on
 * the same `handler_ptr`.
 *
 * # Safety
 *
 * * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
 * * `nonce` must be a valid null-terminated UTF-8 string, or `NULL`.
 * * Caller must ensure exclusive access to `handler_ptr` (no concurrent calls).
 */
void webui_handler_set_nonce(void *handler_ptr, const char *nonce);

/**
 * Render using a protocol previously returned by [`webui_protocol_create`].
 *
 * # Safety
 *
 * * `handler_ptr` must be a valid handler pointer.
 * * `protocol_ptr` must be a valid loaded protocol pointer.
 * * String arguments must be valid null-terminated UTF-8.
 */
char *webui_handler_render(void *handler_ptr,
                           const webui_protocol_t *protocol_ptr,
                           const char *data_json,
                           const char *entry_id,
                           const char *request_path);

/**
 * Produce a complete partial response using a loaded protocol handle.
 *
 * # Safety
 *
 * * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
 * * All string pointers must be valid, non-null, null-terminated UTF-8.
 */
char *webui_protocol_render_partial(const webui_protocol_t *protocol_ptr,
                                    const char *state_json,
                                    const char *entry_id,
                                    const char *request_path,
                                    const char *inventory_hex);

/**
 * Render component templates using a loaded protocol handle.
 *
 * # Safety
 *
 * * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
 * * String arguments must be valid, non-null, null-terminated UTF-8.
 */
char *webui_protocol_render_component_templates(const webui_protocol_t *protocol_ptr,
                                                const char *component_tags_json,
                                                const char *inventory_hex);

/**
 * Free a string returned by a WebUI FFI function.
 *
 * # Safety
 *
 * `string_ptr` must be a pointer returned by a WebUI FFI function such as
 * [`webui_handler_render`], or `NULL`
 * (in which case this function is a no-op).
 */
void webui_free(char *string_ptr);

/**
 * Extract CSS token names from a loaded protocol handle.
 *
 * Returns a newline-delimited representation.
 *
 * # Safety
 *
 * * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
 * * The returned pointer must be freed with [`webui_free`].
 */
char *webui_protocol_tokens(const webui_protocol_t *protocol_ptr);

/**
 * Open a host-driven progressive response for a streaming entry.
 *
 * Unlike [`webui_handler_render`], which produces the whole document in one
 * call, the returned session advances through [`webui_streaming_session_start`],
 * [`webui_streaming_session_resume`], and [`webui_streaming_session_advance`]
 * so the host owns the socket, write order, and backpressure. Any nonce
 * previously set with
 * [`webui_handler_set_nonce`] is captured for the life of the session.
 *
 * Returns `NULL` on error; call [`webui_last_error`] for details. The handle
 * must be released with [`webui_streaming_session_destroy`].
 *
 * # Thread Safety
 *
 * A session is **not** thread-safe. Drive one session from one thread at a
 * time. Independent sessions may run concurrently on the same handler and
 * protocol.
 *
 * # Safety
 *
 * * `handler_ptr` must be a valid pointer from [`webui_handler_create`].
 * * `protocol_ptr` must be a valid pointer from [`webui_protocol_create`].
 * * `entry_id` and `request_path` must be non-null, null-terminated UTF-8.
 */
webui_streaming_session_t *webui_streaming_session_create(void *handler_ptr,
                                                          const webui_protocol_t *protocol_ptr,
                                                          const char *entry_id,
                                                          const char *request_path);

/**
 * Release a streaming session handle.
 *
 * Safe to call on an unfinished session; any buffered bytes are dropped.
 *
 * # Safety
 *
 * `session_ptr` must be a pointer returned by
 * [`webui_streaming_session_create`], or `NULL` for a no-op. It must not be
 * used again afterwards.
 */
void webui_streaming_session_destroy(webui_streaming_session_t *session_ptr);

/**
 * Render until the first runtime boundary occurrence or terminal completion.
 *
 * The returned owned step must be released with
 * [`webui_streaming_step_destroy`]. A `NULL` return indicates an error
 * available through [`webui_last_error`].
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle with no concurrent operation.
 * * `state_json` must be non-null, null-terminated UTF-8 and remain readable
 *   for this call.
 */
webui_streaming_step_t *webui_streaming_session_start(webui_streaming_session_t *session_ptr,
                                                      const char *state_json);

/**
 * Commit the pending occurrence through its checkpoint and stop.
 *
 * `mode` must be [`WEBUI_BOUNDARY_MODE_FINAL`] or
 * [`WEBUI_BOUNDARY_MODE_UPDATABLE`]. The returned owned step must be released
 * with [`webui_streaming_step_destroy`]. A `NULL` return indicates an error
 * available through [`webui_last_error`].
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle with no concurrent operation.
 * * `state_json` must be non-null, null-terminated UTF-8 and remain readable
 *   for this call.
 */
webui_streaming_step_t *webui_streaming_session_resume(webui_streaming_session_t *session_ptr,
                                                       uint32_t instance_id,
                                                       const char *state_json,
                                                       webui_boundary_mode_t mode);

/**
 * Advance through parent bytes to the next occurrence or terminal completion.
 *
 * This call is valid only after [`webui_streaming_session_resume`] commits the
 * pending occurrence. The returned owned step follows the same ownership rules
 * as start and resume and must be released with [`webui_streaming_step_destroy`].
 * A `NULL` return indicates an error available through [`webui_last_error`].
 *
 * # Safety
 *
 * `session_ptr` must be a live session handle with no concurrent operation.
 */
webui_streaming_step_t *webui_streaming_session_advance(webui_streaming_session_t *session_ptr);

/**
 * Emit a projected state patch for a committed updatable occurrence.
 *
 * Returns allocated bytes that must be freed with [`webui_free`]. On success,
 * `out_len` receives the authoritative byte length excluding the allocation's
 * trailing NUL. On failure, `NULL` is returned and `out_len` is untouched.
 * Call [`webui_last_error`] for details.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle with no concurrent operation.
 * * `patch_json` must be non-null, null-terminated UTF-8 and remain readable
 *   for this call.
 * * `out_len` must be non-null and writable.
 */
char *webui_streaming_session_update(webui_streaming_session_t *session_ptr,
                                     uint32_t instance_id,
                                     const char *patch_json,
                                     uintptr_t *out_len);

/**
 * Release an owned streaming step.
 *
 * Destroying a step invalidates its byte pointer and all descriptor string
 * pointers previously returned by step accessors.
 *
 * # Safety
 *
 * `step_ptr` must be a pointer returned by
 * [`webui_streaming_session_start`], [`webui_streaming_session_resume`], or
 * [`webui_streaming_session_advance`], or `NULL` for a no-op. A non-null
 * pointer must not be used after this call.
 */
void webui_streaming_step_destroy(webui_streaming_step_t *step_ptr);

/**
 * Borrow the bytes produced by this step and write their length to `out_len`.
 *
 * The returned pointer is borrowed from `step_ptr`, is not NUL-terminated,
 * and remains valid only until [`webui_streaming_step_destroy`]. It may be
 * read for exactly `out_len` bytes. Returns `NULL` on error.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_len` must be non-null and writable.
 */
const uint8_t *webui_streaming_step_bytes(const webui_streaming_step_t *step_ptr,
                                          uintptr_t *out_len);

/**
 * Observe whether this step emitted the terminal record.
 *
 * A valid non-terminal step returns `false` with no last error. A null handle
 * returns `false` and sets [`webui_last_error`].
 *
 * # Safety
 *
 * `step_ptr` must be a live step handle with no concurrent destroy.
 */
bool webui_streaming_step_done(const webui_streaming_step_t *step_ptr);

/**
 * Observe whether this step carries a pending boundary descriptor.
 *
 * A valid boundary-free step returns `false` with no last error. A null handle
 * returns `false` and sets [`webui_last_error`].
 *
 * # Safety
 *
 * `step_ptr` must be a live step handle with no concurrent destroy.
 */
bool webui_streaming_step_has_boundary(const webui_streaming_step_t *step_ptr);

/**
 * Read the pending boundary's response-local instance ID.
 *
 * Returns `false` and leaves `out_instance_id` untouched when the step has no
 * boundary or an argument is invalid.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_instance_id` must be non-null and writable.
 */
bool webui_streaming_step_boundary_instance_id(const webui_streaming_step_t *step_ptr,
                                               uint32_t *out_instance_id);

/**
 * Read the pending boundary's stable compiler declaration ID.
 *
 * Returns `false` and leaves `out_declaration_id` untouched when the step has
 * no boundary or an argument is invalid.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_declaration_id` must be non-null and writable.
 */
bool webui_streaming_step_boundary_declaration_id(const webui_streaming_step_t *step_ptr,
                                                  uint32_t *out_declaration_id);

/**
 * Borrow the pending boundary owner's UTF-8 bytes.
 *
 * The returned string pointer is not NUL-terminated. It is borrowed from the
 * step and remains valid only until [`webui_streaming_step_destroy`]. Read it
 * for exactly the byte length written to `out_len`. Returns `NULL` on error.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_len` must be non-null and writable.
 */
const char *webui_streaming_step_boundary_owner(const webui_streaming_step_t *step_ptr,
                                                uintptr_t *out_len);

/**
 * Borrow the pending boundary name's UTF-8 bytes.
 *
 * The returned string pointer is not NUL-terminated. It is borrowed from the
 * step and remains valid only until [`webui_streaming_step_destroy`]. Read it
 * for exactly the byte length written to `out_len`. Returns `NULL` on error.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_len` must be non-null and writable.
 */
const char *webui_streaming_step_boundary_name(const webui_streaming_step_t *step_ptr,
                                               uintptr_t *out_len);

/**
 * Read the pending boundary key discriminator.
 *
 * On success, writes exactly one of [`WEBUI_BOUNDARY_KEY_NONE`],
 * [`WEBUI_BOUNDARY_KEY_STRING`], or [`WEBUI_BOUNDARY_KEY_NUMBER`]. Returns
 * `false` and leaves `out_key_type` untouched on error.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_key_type` must be non-null and writable.
 */
bool webui_streaming_step_boundary_key_type(const webui_streaming_step_t *step_ptr,
                                            webui_boundary_key_type_t *out_key_type);

/**
 * Borrow the pending boundary's string key as UTF-8 bytes.
 *
 * The returned string pointer is not NUL-terminated. It is borrowed from the
 * step and remains valid only until [`webui_streaming_step_destroy`]. Read it
 * for exactly the byte length written to `out_len`. A non-string key returns
 * `NULL`, leaves `out_len` untouched, and sets [`webui_last_error`].
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_len` must be non-null and writable.
 */
const char *webui_streaming_step_boundary_key_string(const webui_streaming_step_t *step_ptr,
                                                     uintptr_t *out_len);

/**
 * Read the pending boundary's numeric key.
 *
 * Returns `false`, leaves `out_value` untouched, and sets
 * [`webui_last_error`] when the key is absent or is a string.
 *
 * # Safety
 *
 * * `step_ptr` must be a live step handle with no concurrent destroy.
 * * `out_value` must be non-null and writable.
 */
bool webui_streaming_step_boundary_key_number(const webui_streaming_step_t *step_ptr,
                                              double *out_value);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus
