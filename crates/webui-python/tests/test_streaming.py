# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import gc
from dataclasses import FrozenInstanceError

import pytest
from conftest import STREAMING_STATE
from microsoft_webui import (
    BoundaryDescriptor,
    BoundaryMode,
    Renderer,
    StateError,
    StreamingError,
    StreamingSession,
    StreamStep,
)


def require_boundary(step: StreamStep) -> BoundaryDescriptor:
    assert not step.done
    assert step.boundary is not None
    return step.boundary


def finish_stream(session: StreamingSession, step: StreamStep) -> StreamStep:
    while not step.done:
        boundary = require_boundary(step)
        committed = session.resume(boundary.instance_id, STREAMING_STATE)
        assert not committed.done
        assert committed.boundary is None
        step = session.advance()
    return step


def test_legacy_streaming_surface_is_removed(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()

    for name in (
        "boundary",
        "boundary_count",
        "finished",
        "write_shell",
        "write_boundary",
        "finish",
    ):
        assert not hasattr(session, name)


def test_discovers_resumes_updates_and_completes(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response(nonce="stream-nonce")

    start = session.start(STREAMING_STATE)
    greeting = require_boundary(start)
    assert isinstance(start, StreamStep)
    assert isinstance(start.bytes, bytes)
    assert greeting == BoundaryDescriptor(
        instance_id=0,
        declaration_id=0,
        owner="index.html",
        name="greeting",
        key=10,
    )

    greeting_commit = session.resume(
        greeting.instance_id,
        STREAMING_STATE,
        mode=BoundaryMode.UPDATABLE,
    )
    assert not greeting_commit.done
    assert greeting_commit.boundary is None
    assert b"Hello, Ada!" in greeting_commit.bytes
    assert greeting_commit.bytes.endswith(b"<webui-hydrate></webui-hydrate>")
    assert b"greeting-tail" not in greeting_commit.bytes
    assert b'id="status"' not in greeting_commit.bytes

    update = session.update(greeting.instance_id, {"name": "Grace"})
    assert isinstance(update, bytes)
    assert b"Grace" in update

    status_prefix = session.advance()
    assert b"greeting-tail" in status_prefix.bytes
    assert b'id="status"' not in status_prefix.bytes
    status = require_boundary(status_prefix)
    assert status.name == "status"

    status_commit = session.resume(status.instance_id, STREAMING_STATE)
    assert status_commit.boundary is None
    assert b'id="status">ready</p>' in status_commit.bytes
    assert b"status-tail" not in status_commit.bytes
    assert b'id="summary"' not in status_commit.bytes

    summary_prefix = session.advance()
    assert b"status-tail" in summary_prefix.bytes
    summary = require_boundary(summary_prefix)
    assert summary.name == "summary"

    summary_commit = session.resume(summary.instance_id, STREAMING_STATE)
    assert summary_commit.boundary is None
    assert b'id="summary">All ready</p>' in summary_commit.bytes
    assert b"Streaming complete." not in summary_commit.bytes

    step = session.advance()
    chunks = [
        start.bytes,
        greeting_commit.bytes,
        update,
        status_prefix.bytes,
        status_commit.bytes,
        summary_prefix.bytes,
        summary_commit.bytes,
        step.bytes,
    ]

    assert step.done
    assert step.boundary is None
    assert b"</html>" in step.bytes
    assert b"Streaming complete." in step.bytes
    assert b'nonce="stream-nonce"' in b"".join(chunks)


def test_static_boundary_keys_preserve_python_types(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    step = session.start(STREAMING_STATE)
    keys: list[str | int | float] = []

    while not step.done:
        boundary = require_boundary(step)
        assert boundary.key is not None
        keys.append(boundary.key)
        committed = session.resume(boundary.instance_id, STREAMING_STATE)
        assert committed.boundary is None
        step = session.advance()

    assert keys == [10, 2.5, "last"]
    assert type(keys[0]) is int
    assert type(keys[1]) is float
    assert type(keys[2]) is str


def test_boundary_free_start_completes(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    step = session.start({**STREAMING_STATE, "show": False})

    assert step.done
    assert step.boundary is None
    assert b"</html>" in step.bytes


def test_stream_values_are_immutable(streaming_renderer: Renderer) -> None:
    step = streaming_renderer.stream_response().start(STREAMING_STATE)
    boundary = require_boundary(step)

    with pytest.raises(FrozenInstanceError):
        step.done = True  # type: ignore[misc]
    with pytest.raises(FrozenInstanceError):
        boundary.name = "changed"  # type: ignore[misc]


def test_invalid_state_does_not_advance_session(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()

    with pytest.raises(StateError, match="parse state JSON"):
        session.start(b"{not-json")

    assert require_boundary(session.start(STREAMING_STATE)).name == "greeting"


def test_updates_require_committed_updatable_occurrence(
    streaming_renderer: Renderer,
) -> None:
    session = streaming_renderer.stream_response()
    greeting = require_boundary(session.start(STREAMING_STATE))

    with pytest.raises(StreamingError, match=r"not.*committed"):
        session.update(greeting.instance_id, {"name": "Grace"})

    committed = session.resume(greeting.instance_id, STREAMING_STATE)
    with pytest.raises(StreamingError, match="final"):
        session.update(greeting.instance_id, {"name": "Grace"})

    assert committed.boundary is None
    finish_stream(session, session.advance())


def test_advance_requires_a_committed_boundary(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    greeting = require_boundary(session.start(STREAMING_STATE))

    with pytest.raises(StreamingError, match="pending"):
        session.advance()

    committed = session.resume(greeting.instance_id, STREAMING_STATE)
    assert committed.boundary is None
    assert require_boundary(session.advance()).name == "status"


def test_resume_rejects_wrong_instance_without_advancing(
    streaming_renderer: Renderer,
) -> None:
    session = streaming_renderer.stream_response()
    greeting = require_boundary(session.start(STREAMING_STATE))

    with pytest.raises(StreamingError, match="pending"):
        session.resume(greeting.instance_id + 1, STREAMING_STATE)

    committed = session.resume(greeting.instance_id, STREAMING_STATE)
    assert committed.boundary is None
    assert require_boundary(session.advance()).name == "status"


def test_session_outlives_renderer_reference(streaming_renderer: Renderer) -> None:
    renderer = streaming_renderer
    session = renderer.stream_response()
    del renderer
    gc.collect()

    step = finish_stream(session, session.start(STREAMING_STATE))

    assert b"</html>" in step.bytes


def test_completed_session_rejects_further_work(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    step = session.start({**STREAMING_STATE, "show": False})
    assert step.done

    with pytest.raises(StreamingError, match="already started"):
        session.start(STREAMING_STATE)
