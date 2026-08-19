# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import gc

import pytest
from conftest import STATE
from microsoft_webui import BoundaryMode, Plugin, Renderer, StateError, StreamingError


def test_host_driven_streaming_lifecycle(renderer: Renderer) -> None:
    session = renderer.stream_response(nonce="stream-nonce")
    greeting = session.boundary("greeting")
    status = session.boundary("status")

    assert session.boundary_count == 2
    assert greeting == 0
    assert status == 1
    assert not session.finished

    chunks = [
        session.write_shell(STATE),
        session.write_boundary(greeting, STATE, mode=BoundaryMode.UPDATABLE),
        session.update(greeting, {**STATE, "name": "Grace"}),
        session.write_boundary(status, STATE),
        session.finish(STATE),
    ]

    assert all(isinstance(chunk, bytes) for chunk in chunks)
    assert all(chunks)
    assert b"Hello, Ada!" in chunks[1]
    assert b"Grace" in chunks[2]
    assert b"ready" in chunks[3]
    assert b"</html>" in chunks[4]
    assert b'nonce="stream-nonce"' in b"".join(chunks)
    assert session.finished


def test_unknown_boundary_is_actionable(renderer: Renderer) -> None:
    session = renderer.stream_response()
    with pytest.raises(StreamingError, match="missing"):
        session.boundary("missing")


def test_ordering_error_is_recoverable(renderer: Renderer) -> None:
    session = renderer.stream_response()
    greeting = session.boundary("greeting")
    status = session.boundary("status")

    with pytest.raises(StreamingError, match="shell"):
        session.write_boundary(greeting, STATE)

    assert session.write_shell(STATE)
    with pytest.raises(StreamingError, match="declaration order"):
        session.write_boundary(status, STATE)

    assert session.write_boundary(greeting, STATE)
    assert session.write_boundary(status, STATE)
    assert session.finish(STATE)


def test_finish_error_preserves_open_session(renderer: Renderer) -> None:
    session = renderer.stream_response()
    greeting = session.boundary("greeting")
    status = session.boundary("status")

    session.write_shell(STATE)
    with pytest.raises(StreamingError, match="every boundary must be committed"):
        session.finish(STATE)

    session.write_boundary(greeting, STATE)
    session.write_boundary(status, STATE)
    assert session.finish(STATE)


def test_updates_require_committed_updatable_boundary(renderer: Renderer) -> None:
    session = renderer.stream_response()
    greeting = session.boundary("greeting")
    status = session.boundary("status")
    session.write_shell(STATE)

    with pytest.raises(StreamingError, match=r"not.*committed"):
        session.update(greeting, STATE)

    session.write_boundary(greeting, STATE, mode=BoundaryMode.FINAL)
    with pytest.raises(StreamingError, match="final"):
        session.update(greeting, STATE)

    session.write_boundary(status, STATE)
    session.finish(STATE)


def test_invalid_state_does_not_advance_session(renderer: Renderer) -> None:
    session = renderer.stream_response()
    greeting = session.boundary("greeting")

    with pytest.raises(StateError, match="parse state JSON"):
        session.write_shell(b"{not-json")

    assert session.write_shell(STATE)
    assert session.write_boundary(greeting, STATE)


def test_session_outlives_renderer_reference(protocol_bytes: bytes) -> None:
    renderer = Renderer(protocol_bytes, plugin=Plugin.WEBUI)
    session = renderer.stream_response()
    del renderer
    gc.collect()

    greeting = session.boundary("greeting")
    status = session.boundary("status")
    assert session.write_shell(STATE)
    assert session.write_boundary(greeting, STATE)
    assert session.write_boundary(status, STATE)
    assert session.finish(STATE)


def test_finished_session_rejects_further_work(renderer: Renderer) -> None:
    session = renderer.stream_response()
    session.write_shell(STATE)
    session.write_boundary(session.boundary("greeting"), STATE)
    session.write_boundary(session.boundary("status"), STATE)
    session.finish(STATE)

    with pytest.raises(StreamingError, match="already finished"):
        session.finish(STATE)


def test_finish_requires_final_state(renderer: Renderer) -> None:
    session = renderer.stream_response()
    session.write_shell(STATE)
    session.write_boundary(session.boundary("greeting"), STATE)
    session.write_boundary(session.boundary("status"), STATE)

    with pytest.raises(TypeError, match=r"required positional argument.*state"):
        session.finish()  # type: ignore[call-arg]

    assert not session.finished
    assert session.finish(STATE)
