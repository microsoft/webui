#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

extern "C" {

/// Return the last error message, or `NULL` if no error has occurred.
///
/// The returned pointer is valid until the next FFI call **on the same thread**.
/// Callers **must not** free the returned pointer.
///
/// # Thread Safety
///
/// Each thread has its own independent error state.
const char *webui_last_error();

/// Create a new WebUI handler instance.
///
/// Returns an opaque pointer that must be passed to other `webui_handler_*`
/// functions and eventually freed with [`webui_handler_destroy`].
void *webui_handler_create();

/// Create a new WebUI handler instance with a named plugin.
///
/// # Arguments
///
/// * `plugin_id` - Null-terminated UTF-8 string identifying the plugin.
///   Refer to the CLI/crate documentation for the current list of supported
///   identifiers.
///
/// # Returns
///
/// An opaque pointer that must be freed with [`webui_handler_destroy`],
/// or `NULL` on error (call [`webui_last_error`] for details).
///
/// # Safety
///
/// `plugin_id` must be a valid null-terminated UTF-8 string, or `NULL`.
void *webui_handler_create_with_plugin(const char *plugin_id);

/// Destroy a WebUI handler instance.
///
/// # Safety
///
/// `handler_ptr` must be a valid pointer returned by [`webui_handler_create`],
/// or `NULL` (in which case this function is a no-op).
void webui_handler_destroy(void *handler_ptr);

/// Decode and index a WebUI protocol for repeated rendering.
///
/// The returned handle is thread-safe and must be released with
/// [`webui_protocol_destroy`].
///
/// # Safety
///
/// `protocol_data` must point to `protocol_len` readable bytes.
void *webui_protocol_create(const uint8_t *protocol_data, uintptr_t protocol_len);

/// Destroy a prepared WebUI protocol handle.
///
/// # Safety
///
/// `protocol_ptr` must be a pointer returned by [`webui_protocol_create`], or
/// `NULL` for a no-op.
void webui_protocol_destroy(void *protocol_ptr);

/// Set the CSP nonce for inline `<script>` tags on a handler instance.
///
/// When set, all subsequent renders via [`webui_handler_render`] will include
/// `nonce="VALUE"` on inline script tags and emit a
/// `<meta name="webui-nonce" content="VALUE">` tag in the `<head>`.
///
/// Pass `NULL` to clear a previously set nonce.
///
/// # Thread Safety
///
/// Concurrent render calls are supported after configuration. Callers must not
/// call `set_nonce` or destroy the handler concurrently with any operation on
/// the same `handler_ptr`.
///
/// # Safety
///
/// * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
/// * `nonce` must be a valid null-terminated UTF-8 string, or `NULL`.
/// * Caller must ensure exclusive access to `handler_ptr` (no concurrent calls).
void webui_handler_set_nonce(void *handler_ptr, const char *nonce);

/// Render using a protocol previously returned by [`webui_protocol_create`].
///
/// # Safety
///
/// * `handler_ptr` must be a valid handler pointer.
/// * `protocol_ptr` must be a valid prepared protocol pointer.
/// * String arguments must be valid null-terminated UTF-8.
char *webui_handler_render(void *handler_ptr,
                           const void *protocol_ptr,
                           const char *data_json,
                           const char *entry_id,
                           const char *request_path);

/// Parse an HTML template and render it with JSON data in a single call.
///
/// This convenience entry point is intended for one-shot templates and
/// prototypes. Repeated production renders should compile a protocol ahead of
/// time and use [`webui_protocol_create`] with [`webui_handler_render`].
///
/// Requires the `parser` feature (enabled by default). When built without
/// the `parser` feature, this function always returns `NULL` and sets an
/// error via [`webui_last_error`].
///
/// # Arguments
///
/// * `html`      - Null-terminated UTF-8 string containing the HTML template.
/// * `data_json` - Null-terminated UTF-8 JSON string with the render state.
///
/// # Returns
///
/// A pointer to a null-terminated UTF-8 string with the rendered HTML, or
/// `NULL` on error.  The caller **must** free the returned string with
/// [`webui_free`].  On error, call [`webui_last_error`] for details.
///
/// # Safety
///
/// Both `html` and `data_json` must be valid null-terminated UTF-8 strings.
char *webui_render(const char *html, const char *data_json);

/// Produce a complete partial response using a prepared protocol handle.
///
/// This is equivalent to [`webui_render_partial`] but avoids protobuf decoding
/// and reuses parsed component metadata across calls. State is projected to the
/// reachable authored client components before serialization.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * All string pointers must be valid, non-null, null-terminated UTF-8.
char *webui_render_partial(const void *protocol_ptr,
                           const char *state_json,
                           const char *entry_id,
                           const char *request_path,
                           const char *inventory_hex);

/// Render component templates using a prepared protocol handle.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * String arguments must be valid, non-null, null-terminated UTF-8.
char *webui_render_component_templates(const void *protocol_ptr,
                                       const char *component_tags_json,
                                       const char *inventory_hex);

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// `string_ptr` must be a pointer returned by a WebUI FFI function (e.g.
/// [`webui_handler_render`] or [`webui_render`]), or `NULL`
/// (in which case this function is a no-op).
void webui_free(char *string_ptr);

/// Extract CSS token names from a prepared protocol handle.
///
/// Returns a newline-delimited representation.
///
/// # Safety
///
/// * `protocol_ptr` must be a valid pointer returned by [`webui_protocol_create`].
/// * The returned pointer must be freed with [`webui_free`].
char *webui_protocol_tokens(const void *protocol_ptr);

}  // extern "C"
