# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import os
import subprocess
from pathlib import Path

import pytest
from microsoft_webui import Plugin, Renderer

FIXTURES = Path(__file__).parent / "fixtures"
PROTOCOL_PATH = FIXTURES / "protocol.bin"
STREAMING_PROTOCOL_PATH = FIXTURES / "streaming_protocol.bin"

STATE = {
    "title": "Python renderer",
    "name": "Ada",
    "status": "ready",
}

STREAMING_STATE = {
    **STATE,
    "show": True,
    "integerKey": 10,
    "floatKey": 2.5,
    "stringKey": "last",
    "summary": "All ready",
}


@pytest.fixture(scope="session")
def protocol_bytes() -> bytes:
    return PROTOCOL_PATH.read_bytes()


@pytest.fixture
def renderer(protocol_bytes: bytes) -> Renderer:
    return Renderer(protocol_bytes, plugin=Plugin.WEBUI)


@pytest.fixture
def streaming_renderer() -> Renderer:
    return Renderer(STREAMING_PROTOCOL_PATH.read_bytes(), plugin=Plugin.WEBUI)


@pytest.fixture(scope="session")
def lazy_renderer() -> Renderer:
    root = Path(__file__).resolve().parents[3]
    protocol = Path(os.environ.get(
        "WEBUI_TEST_LAZY_PROTOCOL",
        root / "target" / "python-lazy-test" / "protocol.bin",
    ))
    if not protocol.exists():
        subprocess.run(
            [
                "cargo", "run", "--quiet", "-p", "microsoft-webui-cli", "--",
                "build", str(FIXTURES / "lazy-app"), "--plugin", "webui",
                "--out", str(protocol),
            ],
            cwd=root,
            check=True,
        )
    return Renderer.from_file(protocol, plugin=Plugin.WEBUI)
