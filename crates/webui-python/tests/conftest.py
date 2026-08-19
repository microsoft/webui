# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from pathlib import Path

import pytest
from microsoft_webui import Plugin, Renderer

FIXTURES = Path(__file__).parent / "fixtures"
PROTOCOL_PATH = FIXTURES / "protocol.bin"

STATE = {
    "title": "Python renderer",
    "name": "Ada",
    "status": "ready",
}


@pytest.fixture(scope="session")
def protocol_bytes() -> bytes:
    return PROTOCOL_PATH.read_bytes()


@pytest.fixture
def renderer(protocol_bytes: bytes) -> Renderer:
    return Renderer(protocol_bytes, plugin=Plugin.WEBUI)
