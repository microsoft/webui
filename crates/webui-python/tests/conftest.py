# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

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
    "items": [
        {"id": 10, "label": "ten"},
        {"id": 2.5, "label": "two-point-five"},
        {"id": "last", "label": "last"},
    ],
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
