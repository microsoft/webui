#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

extern "C" {

/// Create a new WebUI handler instance.
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
/// * `protocol_data` - Pointer to protobuf binary data of the WebUI protocol
/// * `protocol_len` - Length of the protobuf binary data in bytes
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
/// protocol_data must be a valid pointer to protobuf binary data of protocol_len bytes.
/// data_json must be a valid null-terminated UTF-8 string.
char *webui_handler_render(void *handler_ptr,
                           const uint8_t *protocol_data,
                           uintptr_t protocol_len,
                           const char *data_json);

/// Free a string returned by a WebUI FFI function.
///
/// # Safety
///
/// The string_ptr must be a pointer returned by a WebUI FFI function,
/// or null (in which case this function does nothing).
void webui_free_string(char *string_ptr);

}  // extern "C"
