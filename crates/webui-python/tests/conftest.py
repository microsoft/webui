# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import os
from pathlib import Path

import pytest
from build_fixtures import NAMES, build_fixtures
from microsoft_webui import Plugin, Renderer

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
def fixture_directory(tmp_path_factory: pytest.TempPathFactory) -> Path:
    supplied = os.environ.get("WEBUI_TEST_FIXTURES")
    directory = Path(supplied) if supplied else build_fixtures(
        tmp_path_factory.mktemp("python-fixtures")
    )
    for name in NAMES:
        if not (directory / name / "protocol.bin").is_file():
            raise pytest.UsageError(
                f"Missing {name}/protocol.bin in {directory}. "
                "Run crates/webui-python/tests/build_fixtures.py "
                "or download the CI fixture artifact."
            )
    return directory


@pytest.fixture(scope="session")
def protocol_path(fixture_directory: Path) -> Path:
    return fixture_directory / "app" / "protocol.bin"


@pytest.fixture(scope="session")
def protocol_bytes(protocol_path: Path) -> bytes:
    return protocol_path.read_bytes()


@pytest.fixture
def renderer(protocol_bytes: bytes) -> Renderer:
    return Renderer(protocol_bytes, plugin=Plugin.WEBUI)


@pytest.fixture
def streaming_renderer(fixture_directory: Path) -> Renderer:
    return Renderer.from_file(
        fixture_directory / "streaming-app" / "protocol.bin", plugin=Plugin.WEBUI
    )


@pytest.fixture(scope="session")
def lazy_renderer(fixture_directory: Path) -> Renderer:
    return Renderer.from_file(fixture_directory / "lazy-app" / "protocol.bin", plugin=Plugin.WEBUI)
