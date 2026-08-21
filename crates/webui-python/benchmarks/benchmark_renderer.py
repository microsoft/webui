# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import json
import os
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest
from ctypes_baseline import CtypesRenderer
from microsoft_webui import BoundaryMode, Plugin, Renderer

if TYPE_CHECKING:
    from collections.abc import Callable, Generator

FIXTURES = Path(__file__).parents[1] / "tests" / "fixtures"
PROTOCOL_PATH = FIXTURES / "protocol.bin"
PROTOCOL_BYTES = PROTOCOL_PATH.read_bytes()
STATE = {"title": "Benchmark", "name": "Ada", "status": "ready"}
STATE_BYTES = json.dumps(STATE, separators=(",", ":")).encode()


@pytest.fixture(scope="module")
def renderer() -> Renderer:
    return Renderer(PROTOCOL_BYTES, plugin=Plugin.WEBUI)


@pytest.fixture(scope="module")
def ffi_renderer() -> Generator[CtypesRenderer, None, None]:
    path = os.environ.get("WEBUI_FFI_LIBRARY")
    if not path:
        pytest.skip("set WEBUI_FFI_LIBRARY to benchmark the ctypes baseline")
    baseline = CtypesRenderer(Path(path), PROTOCOL_BYTES)
    yield baseline
    baseline.close()


def test_construct_from_bytes(benchmark: Any) -> None:
    benchmark.group = "construct"
    benchmark(Renderer, PROTOCOL_BYTES, plugin=Plugin.WEBUI)


def test_construct_from_file(benchmark: Any) -> None:
    benchmark.group = "construct"
    benchmark(Renderer.from_file, PROTOCOL_PATH, plugin=Plugin.WEBUI)


def test_render_mapping(benchmark: Any, renderer: Renderer) -> None:
    benchmark.group = "render"
    benchmark(renderer.render, STATE)


def test_render_preserialized(benchmark: Any, renderer: Renderer) -> None:
    benchmark.group = "render"
    benchmark(renderer.render, STATE_BYTES)


def test_render_ctypes(
    benchmark: Any,
    ffi_renderer: CtypesRenderer,
    renderer: Renderer,
) -> None:
    benchmark.group = "render"
    result = benchmark(ffi_renderer.render, STATE_BYTES)
    assert result == renderer.render(STATE_BYTES)


def test_render_partial(benchmark: Any, renderer: Renderer) -> None:
    benchmark.group = "router"
    benchmark(renderer.render_partial, STATE_BYTES)


def test_render_component_templates(benchmark: Any, renderer: Renderer) -> None:
    benchmark.group = "router"
    benchmark(renderer.render_component_templates, ["greeting-card"])


def _stream_once(renderer: Renderer) -> bytes:
    session = renderer.stream_response()
    greeting = session.boundary("greeting")
    status = session.boundary("status")
    return b"".join(
        (
            session.write_shell(STATE_BYTES),
            session.write_boundary(
                greeting,
                STATE_BYTES,
                mode=BoundaryMode.UPDATABLE,
            ),
            session.update(greeting, STATE_BYTES),
            session.write_boundary(status, STATE_BYTES),
            session.finish(STATE_BYTES),
        )
    )


def test_streaming_session(benchmark: Any, renderer: Renderer) -> None:
    benchmark.group = "streaming"
    benchmark(_stream_once, renderer)


@pytest.mark.parametrize("workers", [1, 2, 4])
def test_threaded_throughput(benchmark: Any, renderer: Renderer, workers: int) -> None:
    benchmark.group = "thread-throughput"

    with ThreadPoolExecutor(max_workers=workers) as executor:

        def run_batch() -> list[bytes]:
            render: Callable[[bytes], bytes] = renderer.render
            return list(executor.map(render, [STATE_BYTES] * 100))

        benchmark(run_batch)
