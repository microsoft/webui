"""
Integration tests for the webui-ffi shared library.

Uses only the Python stdlib (ctypes) -- no pip dependencies required.

Usage (macOS):
    DYLD_LIBRARY_PATH=target/debug python crates/webui-ffi/tests/python/test_webui_ffi.py

Usage (Linux):
    LD_LIBRARY_PATH=target/debug python crates/webui-ffi/tests/python/test_webui_ffi.py
"""

import ctypes
import os
import platform
import sys
import unittest
from ctypes import c_char_p, c_void_p, c_size_t, POINTER, c_ubyte
from typing import Optional


def _load_library():
    """Load the webui_ffi shared library from the build output directory."""
    # Determine library name based on platform
    system = platform.system()
    if system == "Darwin":
        lib_name = "libwebui_ffi.dylib"
    elif system == "Windows":
        lib_name = "webui_ffi.dll"
    else:
        lib_name = "libwebui_ffi.so"

    # Try a few likely paths relative to the repo root
    script_dir = os.path.dirname(os.path.abspath(__file__))
    repo_root = os.path.abspath(os.path.join(script_dir, "..", "..", "..", ".."))

    search_paths = [
        os.path.join(repo_root, "target", "debug", lib_name),
        os.path.join(repo_root, "target", "release", lib_name),
    ]

    for path in search_paths:
        if os.path.exists(path):
            return ctypes.cdll.LoadLibrary(path)

    # Fall back to default search (LD_LIBRARY_PATH / DYLD_LIBRARY_PATH)
    return ctypes.cdll.LoadLibrary(lib_name)


# ---------------------------------------------------------------------------
# Load library and declare FFI signatures
# ---------------------------------------------------------------------------

lib = _load_library()

# void *webui_handler_create()
lib.webui_handler_create.argtypes = []
lib.webui_handler_create.restype = c_void_p

# void webui_handler_destroy(void *handler_ptr)
lib.webui_handler_destroy.argtypes = [c_void_p]
lib.webui_handler_destroy.restype = None

# char *webui_handler_render(void *handler_ptr, const uint8_t *protocol_data,
#                             uintptr_t protocol_len, const char *data_json)
lib.webui_handler_render.argtypes = [c_void_p, POINTER(c_ubyte), c_size_t, c_char_p]
lib.webui_handler_render.restype = c_void_p  # c_void_p to manage manually

# char *webui_render(const char *html, const char *data_json)
lib.webui_render.argtypes = [c_char_p, c_char_p]
lib.webui_render.restype = c_void_p  # c_void_p to manage manually

# void webui_free(char *string_ptr)
lib.webui_free.argtypes = [c_void_p]
lib.webui_free.restype = None

# const char *webui_last_error()
lib.webui_last_error.argtypes = []
lib.webui_last_error.restype = c_char_p


def parse_and_render(html: str, data_json: str) -> str:
    """Call webui_render with proper memory management."""
    ptr = lib.webui_render(html.encode("utf-8"), data_json.encode("utf-8"))
    if ptr is None or ptr == 0:
        err = lib.webui_last_error()
        err_msg = err.decode("utf-8") if err else "<no error>"
        raise RuntimeError(f"webui_render failed: {err_msg}")
    # Copy string before freeing
    result = ctypes.cast(ptr, c_char_p).value.decode("utf-8")
    lib.webui_free(ptr)
    return result


def get_last_error() -> Optional[str]:
    """Return the last error message or None."""
    err = lib.webui_last_error()
    if err is None:
        return None
    return err.decode("utf-8")


# ---------------------------------------------------------------------------
# Test cases
# ---------------------------------------------------------------------------


class TestParseAndRender(unittest.TestCase):
    """Tests for the webui_render high-level API."""

    def test_simple_passthrough(self):
        result = parse_and_render("<p>Hello</p>", "{}")
        self.assertEqual(result, "<p>Hello</p>")

    def test_signal_substitution(self):
        result = parse_and_render("Hello, {{name}}!", '{"name":"WebUI"}')
        self.assertEqual(result, "Hello, WebUI!")

    def test_for_loop(self):
        html = '<ul><for each="item in items"><li>{{item}}</li></for></ul>'
        result = parse_and_render(html, '{"items":["a","b","c"]}')
        self.assertEqual(result, "<ul><li>a</li><li>b</li><li>c</li></ul>")

    def test_if_condition_true(self):
        html = '<if condition="show"><p>Visible</p></if>'
        result = parse_and_render(html, '{"show":true}')
        self.assertEqual(result, "<p>Visible</p>")

    def test_if_condition_false(self):
        html = '<if condition="show"><p>Hidden</p></if>'
        result = parse_and_render(html, '{"show":false}')
        self.assertEqual(result, "")

    def test_html_escaping(self):
        result = parse_and_render(
            "<div>{{content}}</div>",
            '{"content":"<script>alert(\'xss\')</script>"}',
        )
        self.assertNotIn("<script>", result)
        self.assertIn("&lt;script&gt;", result)

    def test_raw_signal_unescaped(self):
        result = parse_and_render(
            "<div>{{{content}}}</div>", '{"content":"<b>bold</b>"}'
        )
        self.assertEqual(result, "<div><b>bold</b></div>")

    def test_empty_data(self):
        result = parse_and_render("<p>static</p>", "{}")
        self.assertEqual(result, "<p>static</p>")


class TestErrorHandling(unittest.TestCase):
    """Tests for error reporting via webui_last_error."""

    def test_null_html(self):
        ptr = lib.webui_render(None, b"{}")
        self.assertTrue(ptr is None or ptr == 0)
        err = get_last_error()
        self.assertIsNotNone(err)
        self.assertIn("null", err)

    def test_null_json(self):
        ptr = lib.webui_render(b"<p>hi</p>", None)
        self.assertTrue(ptr is None or ptr == 0)
        self.assertIsNotNone(get_last_error())

    def test_invalid_json(self):
        ptr = lib.webui_render(b"<p>hi</p>", b"NOT JSON")
        self.assertTrue(ptr is None or ptr == 0)
        err = get_last_error()
        self.assertIsNotNone(err)
        self.assertIn("JSON", err)

    def test_successful_call_clears_error(self):
        # Trigger an error first
        lib.webui_render(b"<p>hi</p>", b"NOT JSON")
        self.assertIsNotNone(get_last_error())

        # Successful call should clear it
        parse_and_render("<p>ok</p>", "{}")
        self.assertIsNone(get_last_error())


class TestHandlerLifecycle(unittest.TestCase):
    """Tests for handler create/destroy."""

    def test_create_and_destroy(self):
        handler = lib.webui_handler_create()
        self.assertIsNotNone(handler)
        self.assertNotEqual(handler, 0)
        lib.webui_handler_destroy(handler)

    def test_destroy_null(self):
        lib.webui_handler_destroy(None)  # should not crash


class TestFixtureFile(unittest.TestCase):
    """Test using shared fixture files."""

    def test_fixture_renders_correctly(self):
        fixtures_dir = os.path.join(
            os.path.dirname(os.path.abspath(__file__)), "..", "fixtures"
        )
        with open(os.path.join(fixtures_dir, "simple.html")) as f:
            html = f.read()
        with open(os.path.join(fixtures_dir, "state.json")) as f:
            data_json = f.read()
        with open(os.path.join(fixtures_dir, "expected_output.html")) as f:
            expected = f.read()

        result = parse_and_render(html, data_json)
        self.assertEqual(result, expected)


class TestFreeString(unittest.TestCase):
    """Tests for webui_free."""

    def test_free_null(self):
        lib.webui_free(None)  # should not crash


if __name__ == "__main__":
    unittest.main()
