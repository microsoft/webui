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
    StreamStep,
)


def require_boundary(step: StreamStep) -> BoundaryDescriptor:
    assert not step.done
    assert step.boundary is not None
    return step.boundary


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
        key=None,
    )

    greeting_step = session.resume(
        greeting.instance_id,
        STREAMING_STATE,
        mode=BoundaryMode.UPDATABLE,
    )
    assert b"Hello, Ada!" in greeting_step.bytes
    assert require_boundary(greeting_step).name == "status"

    update = session.update(greeting.instance_id, {"name": "Grace"})
    assert isinstance(update, bytes)
    assert b"Grace" in update

    chunks = [start.bytes, greeting_step.bytes, update]
    step = greeting_step
    while not step.done:
        boundary = require_boundary(step)
        step = session.resume(boundary.instance_id, STREAMING_STATE)
        chunks.append(step.bytes)

    assert step.done
    assert step.boundary is None
    assert b"</html>" in step.bytes
    assert b'nonce="stream-nonce"' in b"".join(chunks)


def test_repeated_boundary_keys_preserve_python_types(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    step = session.start(STREAMING_STATE)
    keys: list[str | int | float] = []

    while not step.done:
        boundary = require_boundary(step)
        if boundary.name == "row":
            assert boundary.key is not None
            keys.append(boundary.key)
        step = session.resume(boundary.instance_id, STREAMING_STATE)

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

    step = session.resume(greeting.instance_id, STREAMING_STATE)
    with pytest.raises(StreamingError, match="final"):
        session.update(greeting.instance_id, {"name": "Grace"})

    while not step.done:
        boundary = require_boundary(step)
        step = session.resume(boundary.instance_id, STREAMING_STATE)


def test_resume_rejects_wrong_instance_without_advancing(
    streaming_renderer: Renderer,
) -> None:
    session = streaming_renderer.stream_response()
    greeting = require_boundary(session.start(STREAMING_STATE))

    with pytest.raises(StreamingError, match="pending"):
        session.resume(greeting.instance_id + 1, STREAMING_STATE)

    assert require_boundary(session.resume(greeting.instance_id, STREAMING_STATE)).name == "status"


def test_session_outlives_renderer_reference(streaming_renderer: Renderer) -> None:
    renderer = streaming_renderer
    session = renderer.stream_response()
    del renderer
    gc.collect()

    step = session.start(STREAMING_STATE)
    while not step.done:
        boundary = require_boundary(step)
        step = session.resume(boundary.instance_id, STREAMING_STATE)

    assert b"</html>" in step.bytes


def test_completed_session_rejects_further_work(streaming_renderer: Renderer) -> None:
    session = streaming_renderer.stream_response()
    step = session.start({**STREAMING_STATE, "show": False})
    assert step.done

    with pytest.raises(StreamingError, match="already started"):
        session.start(STREAMING_STATE)
