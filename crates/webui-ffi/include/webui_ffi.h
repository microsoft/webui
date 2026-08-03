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
 * Enable or disable the reserved `$webui` state inject channel.
 *
 * When enabled, a top-level `$webui` object in the render state may carry
 * `headEnd`, `bodyStart`, and `bodyEnd` strings. Each is written **raw**
 * (not escaped) at the matching structural boundary, after WebUI's own
 * emissions. The `$webui` key itself is stripped from the hydration payload.
 *
 * This is **disabled by default** because it turns the state channel into a
 * raw-HTML sink. Only enable it when the render state is fully host-owned;
 * never enable it for state derived from untrusted request input.
 *
 * # Thread Safety
 *
 * Callers must not call this concurrently with any other operation on the
 * same `handler_ptr`.
 *
 * # Safety
 *
 * * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
 * * Caller must ensure exclusive access to `handler_ptr` (no concurrent calls).
 */
void webui_handler_set_state_inject(void *handler_ptr, bool enabled);

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
 * call, the returned session hands back one chunk per call so the host owns
 * the socket, the write order, and backpressure. Any nonce previously set
 * with [`webui_handler_set_nonce`] is captured for the life of the session.
 *
 * Returns `NULL` on error; call [`webui_last_error`] for details. The handle
 * must be released with [`webui_streaming_session_destroy`] even after
 * [`webui_streaming_session_finish`] succeeds.
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
 * Resolve an authored boundary name to a stable integer handle.
 *
 * Resolve once outside the write loop and reuse the handle; the write calls
 * never hash a name.
 *
 * Returns `true` on success and writes the handle to `out_boundary`. On
 * failure returns `false` and leaves `out_boundary` untouched; call
 * [`webui_last_error`] for the valid names and a suggestion.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle.
 * * `name` must be non-null, null-terminated UTF-8.
 * * `out_boundary` must be non-null and writable.
 */
bool webui_streaming_session_boundary(const webui_streaming_session_t *session_ptr,
                                      const char *name,
                                      uint32_t *out_boundary);

/**
 * Return the number of compile-time boundaries declared by this entry.
 *
 * Returns `0` for a `NULL` handle.
 *
 * # Safety
 *
 * `session_ptr` must be a live session handle, or `NULL`.
 */
uint32_t webui_streaming_session_boundary_count(const webui_streaming_session_t *session_ptr);

/**
 * Report whether the terminal record has been written.
 *
 * Returns `true` for a `NULL` handle, because a session that does not exist
 * can never accept another call.
 *
 * # Safety
 *
 * `session_ptr` must be a live session handle, or `NULL`.
 */
bool webui_streaming_session_is_finished(const webui_streaming_session_t *session_ptr);

/**
 * Render everything before the first boundary.
 *
 * Returns a NUL-terminated UTF-8 chunk that must be freed with
 * [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
 * the byte length excluding the terminator, so hosts writing to a socket do
 * not need `strlen`.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle.
 * * `state_json` must be non-null, null-terminated UTF-8.
 * * `out_len` must be writable, or `NULL`.
 */
char *webui_streaming_session_write_shell(webui_streaming_session_t *session_ptr,
                                          const char *state_json,
                                          uintptr_t *out_len);

/**
 * Render and commit the next boundary in declaration order.
 *
 * Pass `updatable = true` only for boundaries you intend to patch later with
 * [`webui_streaming_session_update`]; an updatable boundary retains its roots
 * and projection until the terminal record.
 *
 * Returns a NUL-terminated UTF-8 chunk that must be freed with
 * [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
 * the byte length excluding the terminator.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle.
 * * `state_json` must be non-null, null-terminated UTF-8.
 * * `out_len` must be writable, or `NULL`.
 */
char *webui_streaming_session_write_boundary(webui_streaming_session_t *session_ptr,
                                             uint32_t boundary,
                                             const char *state_json,
                                             bool updatable,
                                             uintptr_t *out_len);

/**
 * Push a projected state patch to a committed updatable boundary.
 *
 * Returns a NUL-terminated UTF-8 chunk that must be freed with
 * [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
 * the byte length excluding the terminator.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle.
 * * `state_json` must be non-null, null-terminated UTF-8.
 * * `out_len` must be writable, or `NULL`.
 */
char *webui_streaming_session_update(webui_streaming_session_t *session_ptr,
                                     uint32_t boundary,
                                     const char *state_json,
                                     uintptr_t *out_len);

/**
 * Render the document tail and emit the terminal record.
 *
 * Every later call fails. The handle must still be released with
 * [`webui_streaming_session_destroy`].
 *
 * Returns a NUL-terminated UTF-8 chunk that must be freed with
 * [`webui_free`], or `NULL` on error. When `out_len` is non-null it receives
 * the byte length excluding the terminator.
 *
 * # Safety
 *
 * * `session_ptr` must be a live session handle.
 * * `state_json` must be non-null, null-terminated UTF-8.
 * * `out_len` must be writable, or `NULL`.
 */
char *webui_streaming_session_finish(webui_streaming_session_t *session_ptr,
                                     const char *state_json,
                                     uintptr_t *out_len);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus
