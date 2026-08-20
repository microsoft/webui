# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Typed public facade over the private PyO3 extension."""

from __future__ import annotations

import json
import os
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from enum import StrEnum
from typing import Any, TypeAlias

from . import _native

StateInput: TypeAlias = Mapping[str, Any] | str | bytes | bytearray | memoryview
PathLike: TypeAlias = str | os.PathLike[str]


class Plugin(StrEnum):
    """Hydration marker plugin bound to a renderer."""

    FAST = "fast"
    FAST_V2 = "fast-v2"
    FAST_V3 = "fast-v3"
    WEBUI = "webui"


class BoundaryMode(StrEnum):
    """Whether a committed streaming boundary can receive updates."""

    FINAL = "final"
    UPDATABLE = "updatable"


@dataclass(frozen=True, slots=True)
class BoundaryDescriptor:
    """One runtime boundary occurrence discovered during streaming."""

    instance_id: int
    declaration_id: int
    owner: str
    name: str
    key: str | int | float | None


@dataclass(frozen=True, slots=True)
class StreamStep:
    """Immutable bytes and continuation state produced by one streaming call."""

    bytes: bytes
    done: bool
    boundary: BoundaryDescriptor | None = None


def _plugin_value(plugin: Plugin | str | None) -> str | None:
    if plugin is None:
        return None
    return Plugin(plugin).value


def _state_json(state: StateInput) -> str | bytes | bytearray:
    if isinstance(state, (str, bytes, bytearray)):
        return state
    if isinstance(state, memoryview):
        return state.tobytes()
    if isinstance(state, Mapping):
        serializable = state if type(state) is dict else dict(state)
        return json.dumps(
            serializable,
            allow_nan=False,
            ensure_ascii=False,
            separators=(",", ":"),
        )
    raise TypeError(
        "state must be a mapping or pre-serialized str, bytes, bytearray, or memoryview"
    )


class Renderer:
    """A decoded WebUI protocol and its reusable renderer indices.

    Renderer instances are immutable and safe to share between threads.
    """

    __slots__ = ("_inner",)

    def __init__(
        self,
        protocol: bytes | bytearray | memoryview,
        *,
        plugin: Plugin | str | None = None,
    ) -> None:
        if isinstance(protocol, memoryview):
            protocol = protocol.tobytes()
        self._inner = _native._Renderer(protocol, plugin=_plugin_value(plugin))

    @classmethod
    def from_file(
        cls,
        path: PathLike,
        *,
        plugin: Plugin | str | None = None,
    ) -> Renderer:
        """Read, decode, and index a compiled ``protocol.bin``."""
        renderer = cls.__new__(cls)
        renderer._inner = _native._Renderer.from_file(
            os.fspath(path),
            plugin=_plugin_value(plugin),
        )
        return renderer

    def render(
        self,
        state: StateInput,
        *,
        entry: str = "index.html",
        request_path: str = "/",
        nonce: str | None = None,
        head_inject: str | None = None,
        body_inject: str | None = None,
    ) -> bytes:
        """Render a complete UTF-8 HTML response as bytes.

        ``head_inject`` and ``body_inject`` are emitted verbatim. Never pass
        untrusted or request-derived HTML through these options.
        """
        return self._inner.render(
            _state_json(state),
            (entry, request_path, nonce, head_inject, body_inject),
        )

    def render_text(
        self,
        state: StateInput,
        *,
        entry: str = "index.html",
        request_path: str = "/",
        nonce: str | None = None,
        head_inject: str | None = None,
        body_inject: str | None = None,
    ) -> str:
        """Render and decode a complete UTF-8 HTML response."""
        return self.render(
            state,
            entry=entry,
            request_path=request_path,
            nonce=nonce,
            head_inject=head_inject,
            body_inject=body_inject,
        ).decode("utf-8")

    def render_partial(
        self,
        state: StateInput,
        *,
        entry: str = "index.html",
        request_path: str = "/",
        inventory: str = "",
    ) -> bytes:
        """Render the JSON payload for client-side partial navigation."""
        return self._inner.render_partial(
            _state_json(state),
            (entry, request_path, inventory),
        )

    def render_component_templates(
        self,
        component_tags: str | Iterable[str],
        *,
        inventory: str = "",
    ) -> bytes:
        """Render component template and style JSON for one tag or an iterable."""
        tags = [component_tags] if isinstance(component_tags, str) else list(component_tags)
        return self._inner.render_component_templates(
            (tags, inventory),
        )

    @property
    def tokens(self) -> tuple[str, ...]:
        """CSS token names in build order."""
        return tuple(self._inner.tokens())

    def stream_response(
        self,
        *,
        entry: str = "index.html",
        request_path: str = "/",
        nonce: str | None = None,
        head_inject: str | None = None,
        body_inject: str | None = None,
    ) -> StreamingSession:
        """Open a host-driven progressive response.

        The caller owns transport writes, flushing, cancellation, and
        backpressure. The returned session is single-driver.
        """
        inner = self._inner.stream_response(
            (entry, request_path, nonce, head_inject, body_inject),
        )
        return StreamingSession(inner)


class StreamingSession:
    """A mutable, single-driver progressive HTML response.

    Each call returns immutable output while the session retains its continuation.
    """

    __slots__ = ("_inner",)

    def __init__(self, inner: _native._StreamingSession) -> None:
        self._inner = inner

    def start(self, state: StateInput) -> StreamStep:
        """Render until the first runtime boundary occurrence or completion."""
        return _stream_step(self._inner.start(_state_json(state)))

    def resume(
        self,
        instance_id: int,
        state: StateInput,
        *,
        mode: BoundaryMode | str = BoundaryMode.FINAL,
    ) -> StreamStep:
        """Commit the pending occurrence and advance to the next step."""
        parsed_mode = BoundaryMode(mode)
        return _stream_step(
            self._inner.resume(
                _state_json(state),
                instance_id,
                parsed_mode is BoundaryMode.UPDATABLE,
            )
        )

    def update(self, instance_id: int, patch: StateInput) -> bytes:
        """Push a projected state patch to an updatable occurrence."""
        return self._inner.update(_state_json(patch), instance_id)


def _stream_step(value: _native._StreamStep) -> StreamStep:
    boundary = value["boundary"]
    descriptor = (
        None
        if boundary is None
        else BoundaryDescriptor(
            instance_id=boundary["instance_id"],
            declaration_id=boundary["declaration_id"],
            owner=boundary["owner"],
            name=boundary["name"],
            key=boundary["key"],
        )
    )
    return StreamStep(
        bytes=value["bytes"],
        done=value["done"],
        boundary=descriptor,
    )
