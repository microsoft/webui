# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from os import PathLike
from typing import Self, TypeAlias, final

_JsonInput: TypeAlias = str | bytes | bytearray
_RenderOptions: TypeAlias = tuple[
    str,
    str,
    str | None,
    str | None,
    str | None,
]

class WebUIError(Exception): ...
class ProtocolError(WebUIError): ...
class StateError(WebUIError): ...
class RenderError(WebUIError): ...
class StreamingError(WebUIError): ...

@final
class _Renderer:
    def __new__(
        cls,
        protocol: bytes | bytearray,
        *,
        plugin: str | None = None,
    ) -> Self: ...
    @classmethod
    def from_file(
        cls,
        path: str | PathLike[str],
        *,
        plugin: str | None = None,
    ) -> _Renderer: ...
    def render(self, state_json: _JsonInput, options: _RenderOptions) -> bytes: ...
    def render_partial(
        self,
        state_json: _JsonInput,
        options: tuple[str, str, str],
    ) -> bytes: ...
    def render_component_templates(
        self,
        options: tuple[list[str], str],
    ) -> bytes: ...
    def tokens(self) -> list[str]: ...
    def stream_response(self, options: _RenderOptions) -> _StreamingSession: ...

@final
class _StreamingSession:
    def boundary(self, name: str) -> int: ...
    @property
    def boundary_count(self) -> int: ...
    @property
    def finished(self) -> bool: ...
    def write_shell(self, state_json: _JsonInput) -> bytes: ...
    def write_boundary(
        self,
        state_json: _JsonInput,
        boundary: int,
        updatable: bool,
    ) -> bytes: ...
    def update(self, state_json: _JsonInput, boundary: int) -> bytes: ...
    def finish(self, state_json: _JsonInput) -> bytes: ...

__version__: str
__all__ = [
    "ProtocolError",
    "RenderError",
    "StateError",
    "StreamingError",
    "WebUIError",
    "_Renderer",
    "_StreamingSession",
    "__version__",
]
