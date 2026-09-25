# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import importlib.metadata
import importlib.resources
import pickle
import tomllib
from concurrent.futures import ProcessPoolExecutor
from multiprocessing import get_context
from pathlib import Path
from typing import NoReturn

import microsoft_webui
import pytest
import validate_release_targets
from microsoft_webui import (
    BoundaryDescriptor,
    BoundaryMode,
    Plugin,
    ProtocolError,
    Renderer,
    RenderError,
    StateError,
    StreamingError,
    StreamingSession,
    StreamStep,
    WebUIError,
)


def _raise_state_error() -> NoReturn:
    raise StateError("state failed in worker")


def test_public_surface_and_version() -> None:
    assert microsoft_webui.__version__ == importlib.metadata.version("microsoft-webui")
    assert set(microsoft_webui.__all__) == {
        "BoundaryDescriptor",
        "BoundaryMode",
        "Plugin",
        "ProtocolError",
        "RenderError",
        "Renderer",
        "StateError",
        "StateInput",
        "StreamStep",
        "StreamingError",
        "StreamingSession",
        "WebUIError",
        "__version__",
    }
    assert issubclass(ProtocolError, WebUIError)
    assert issubclass(StateError, WebUIError)
    assert issubclass(RenderError, WebUIError)
    assert issubclass(StreamingError, WebUIError)
    assert Renderer is not None
    assert BoundaryDescriptor is not None
    assert StreamStep is not None
    assert StreamingSession is not None


def test_enum_values_are_strings() -> None:
    assert Plugin.WEBUI == "webui"
    assert Plugin.FAST_V3 == "fast-v3"
    assert BoundaryMode.FINAL == "final"
    assert BoundaryMode.UPDATABLE == "updatable"


@pytest.mark.parametrize(
    "error_type",
    [WebUIError, ProtocolError, StateError, RenderError, StreamingError],
)
def test_native_exceptions_are_importable_and_picklable(
    error_type: type[WebUIError],
) -> None:
    module = importlib.import_module(error_type.__module__)
    restored = pickle.loads(pickle.dumps(error_type("round trip")))

    assert error_type.__module__ == "microsoft_webui._native"
    assert getattr(module, error_type.__name__) is error_type
    assert type(restored) is error_type
    assert restored.args == ("round trip",)


def test_native_exception_crosses_spawn_process() -> None:
    context = get_context("spawn")
    with ProcessPoolExecutor(max_workers=1, mp_context=context) as executor:
        result = executor.submit(_raise_state_error)
        with pytest.raises(StateError, match="state failed in worker"):
            result.result(timeout=30)


def test_wheel_contains_native_module_and_type_marker() -> None:
    native_path = Path(microsoft_webui._native.__file__)  # type: ignore[attr-defined]
    marker = importlib.resources.files("microsoft_webui").joinpath("py.typed")

    assert native_path.suffix in {".pyd", ".so"}
    assert marker.is_file()


def test_source_configuration_excludes_python_caches() -> None:
    project = Path(__file__).parents[1]
    ignore_rules = set((project / ".gitignore").read_text(encoding="utf-8").splitlines())
    pyproject = tomllib.loads((project / "pyproject.toml").read_text(encoding="utf-8"))
    package_excludes = set(pyproject["tool"]["maturin"]["exclude"])

    assert {"__pycache__/", "*.py[cod]", ".mypy_cache/", ".pytest_cache/", ".ruff_cache/"} <= (
        ignore_rules
    )
    assert {
        "**/__pycache__",
        "**/*.py[cod]",
        ".mypy_cache/**",
        ".pytest_cache/**",
        ".ruff_cache/**",
    } <= package_excludes


def test_distribution_has_no_runtime_dependencies() -> None:
    requirements = importlib.metadata.requires("microsoft-webui") or []
    assert all("; extra ==" in requirement for requirement in requirements)


def test_release_target_contract_is_restated_consistently() -> None:
    contract = validate_release_targets._contract_tags()

    assert len(contract) == 6
    assert validate_release_targets.main() == 0


def test_release_target_drift_is_rejected(monkeypatch: pytest.MonkeyPatch) -> None:
    read_source = validate_release_targets._read

    def drop_one_arm64_target(relative: Path) -> str:
        text = read_source(relative)
        if relative == validate_release_targets.MATRIX_TAG_SOURCE:
            return text.replace("platform_tag: win_arm64", "platform_tag: win_arm64_typo")
        return text

    monkeypatch.setattr(validate_release_targets, "_read", drop_one_arm64_target)

    assert validate_release_targets.main() == 1
