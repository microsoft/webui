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

/// Render a WebUI protocol (protobuf binary) with JSON data.
///
/// # Arguments
///
/// * `handler_ptr`   - Pointer returned by [`webui_handler_create`].
/// * `protocol_data` - Pointer to protobuf binary data.
/// * `protocol_len`  - Length of the protobuf data in bytes.
/// * `data_json`     - Null-terminated JSON string with the render state.
/// * `entry_id`      - Null-terminated UTF-8 string for the entry fragment.
/// * `request_path`  - Null-terminated UTF-8 string for the URL path to match
///   routes against (e.g., `"/contacts/42"`).
///
/// # Returns
///
/// A pointer to a null-terminated UTF-8 string with the rendered HTML, or
/// `NULL` on error.  The caller **must** free the returned string with
/// [`webui_free`].  On error, call [`webui_last_error`] for details.
///
/// # Safety
///
/// * `handler_ptr` must be a valid pointer returned by [`webui_handler_create`].
/// * `protocol_data` must point to `protocol_len` bytes of valid memory.
/// * `data_json`, `entry_id`, and `request_path` must be valid null-terminated UTF-8 strings.
char *webui_handler_render(void *handler_ptr,
                           const uint8_t *protocol_data,
                           uintptr_t protocol_len,
                           const char *data_json,
                           const char *entry_id,
                           const char *request_path);

/// Parse an HTML template and render it with JSON data in a single call.
///
/// This is the **recommended entry point** for Go, C#, and Python consumers.
/// It eliminates the need for callers to deal with protobuf serialisation.
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

/// Produce a complete JSON partial response for client-side navigation.
///
/// Combines route templates, inventory, and matched route chain into a single
/// JSON string: `{"templates":[...],"inventory":"...","chain":[...]}`.
///
/// # Arguments
///
/// * `protocol_data` - Pointer to protobuf binary data.
/// * `protocol_len`  - Length of the protobuf data in bytes.
/// * `entry_id`      - Null-terminated UTF-8 string for the persistent entry fragment.
/// * `request_path`  - Null-terminated UTF-8 route path used to select the active route chain.
/// * `inventory_hex` - Null-terminated hex string of the client's inventory bitmask
///   (pass empty string `""` if no inventory).
///
/// # Returns
///
/// A heap-allocated JSON string, or `NULL` on error. Caller frees with [`webui_free`].
///
/// # Safety
///
/// * `protocol_data` must point to `protocol_len` bytes of valid memory.
/// * `state_json`, `entry_id`, `request_path`, and `inventory_hex` must be valid
///   null-terminated UTF-8 strings.
char *webui_render_partial(const uint8_t *protocol_data,
                           uintptr_t protocol_len,
                           const char *state_json,
                           const char *entry_id,
                           const char *request_path,
                           const char *inventory_hex);

/// Render templates and CSS for specific components by tag name.
///
/// Returns a JSON string with `{ templates, templateStyles, cssHrefs, inventory }`.
/// The caller must free the returned string with [`webui_free`].
///
/// # Safety
///
/// All pointer arguments must be valid, non-null, null-terminated UTF-8 strings.
/// `protocol_data` must point to `protocol_len` readable bytes.
/// `component_tags_json` must be a JSON array of strings, e.g. `["settings-dialog"]`.
char *webui_render_component_templates(const uint8_t *protocol_data,
                                       uintptr_t protocol_len,
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

/// Extract the CSS token name list from a serialized WebUI protocol.
///
/// Returns a heap-allocated newline-delimited string of token names,
/// e.g. `"colorBrandBackground\nfontSizeBase300"`.
///
/// Returns an empty string `""` when the protocol has no tokens.
/// Returns `NULL` only on error (call [`webui_last_error`] for details).
///
/// The caller must free the returned string with [`webui_free`].
///
/// # Safety
///
/// * `protocol_data` must point to `protocol_len` valid bytes.
/// * The returned pointer must be freed with [`webui_free`].
char *webui_protocol_tokens(const uint8_t *protocol_data, uintptr_t protocol_len);

}  // extern "C"
