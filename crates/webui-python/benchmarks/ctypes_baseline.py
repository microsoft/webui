# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import ctypes
from ctypes import POINTER, c_char_p, c_size_t, c_ubyte, c_void_p
from pathlib import Path


class CtypesRenderer:
    """Minimal loaded-protocol FFI baseline for binding overhead benchmarks."""

    def __init__(self, library_path: Path, protocol_bytes: bytes) -> None:
        self._library = ctypes.CDLL(str(library_path))
        self._configure()
        self._handler = self._library.webui_handler_create_with_plugin(b"webui")
        buffer = (c_ubyte * len(protocol_bytes)).from_buffer_copy(protocol_bytes)
        self._protocol = self._library.webui_protocol_create(buffer, len(protocol_bytes))
        if not self._handler or not self._protocol:
            self.close()
            raise RuntimeError(self._last_error() or "failed to initialize FFI baseline")

    def _configure(self) -> None:
        library = self._library
        library.webui_handler_create_with_plugin.argtypes = [c_char_p]
        library.webui_handler_create_with_plugin.restype = c_void_p
        library.webui_handler_destroy.argtypes = [c_void_p]
        library.webui_protocol_create.argtypes = [POINTER(c_ubyte), c_size_t]
        library.webui_protocol_create.restype = c_void_p
        library.webui_protocol_destroy.argtypes = [c_void_p]
        library.webui_handler_render.argtypes = [
            c_void_p,
            c_void_p,
            c_char_p,
            c_char_p,
            c_char_p,
        ]
        library.webui_handler_render.restype = c_void_p
        library.webui_free.argtypes = [c_void_p]
        library.webui_last_error.argtypes = []
        library.webui_last_error.restype = c_char_p

    def _last_error(self) -> str | None:
        error = self._library.webui_last_error()
        return error.decode() if error else None

    def render(self, state_json: bytes) -> bytes:
        pointer = self._library.webui_handler_render(
            self._handler,
            self._protocol,
            state_json,
            b"index.html",
            b"/",
        )
        if not pointer:
            raise RuntimeError(self._last_error() or "FFI render failed")
        try:
            return ctypes.string_at(pointer)
        finally:
            self._library.webui_free(pointer)

    def close(self) -> None:
        protocol = getattr(self, "_protocol", None)
        handler = getattr(self, "_handler", None)
        if protocol:
            self._library.webui_protocol_destroy(protocol)
            self._protocol = None
        if handler:
            self._library.webui_handler_destroy(handler)
            self._handler = None
