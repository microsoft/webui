#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

extern "C" {

/// Create a new WebUI handler instance.
///
/// Returns a pointer to the handler context or null on failure.
void *webui_handler_create();

/// Destroy a WebUI handler instance.
///
/// # Safety
///
/// The handler_ptr must be a valid pointer returned by webui_handler_create.
void webui_handler_destroy(void *handler_ptr);

/// Render a WebUI protocol with data.
///
/// # Arguments
///
/// * `handler_ptr` - Pointer to a WebUI handler instance
/// * `protocol_json` - JSON string of the WebUI protocol
/// * `data_json` - JSON string of the data to render with
///
/// # Returns
///
/// A pointer to a null-terminated string containing the rendered HTML.
/// The caller is responsible for freeing this memory with webui_free_string.
///
/// # Safety
///
/// The handler_ptr must be a valid pointer returned by webui_handler_create.
/// protocol_json and data_json must be valid null-terminated UTF-8 strings.
char *webui_handler_render(void *handler_ptr, const char *protocol_json, const char *data_json);

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// The string_ptr must be a pointer returned by a WebUI FFI function,
/// or null (in which case this function does nothing).
void webui_free_string(char *string_ptr);

}  // extern "C"
