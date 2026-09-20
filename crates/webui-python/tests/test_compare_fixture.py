# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import sys
from pathlib import Path

import compare_fixture
import pytest


def test_reports_requested_stale_fixture(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    committed = tmp_path / "committed.json"
    generated = tmp_path / "generated.json"
    fixture = Path("fixtures/streaming_protocol.bin")
    committed.write_text('{"value": 1}', encoding="utf-8")
    generated.write_text('{"value": 2}', encoding="utf-8")
    monkeypatch.setattr(
        sys,
        "argv",
        ["compare_fixture.py", str(committed), str(generated), str(fixture)],
    )

    assert compare_fixture.main() == 1
    assert str(fixture) in capsys.readouterr().err


def test_default_fixture_path_remains_compatible(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    committed = tmp_path / "committed.json"
    generated = tmp_path / "generated.json"
    committed.write_text('{"value": 1}', encoding="utf-8")
    generated.write_text('{"value": 2}', encoding="utf-8")
    monkeypatch.setattr(
        sys,
        "argv",
        ["compare_fixture.py", str(committed), str(generated)],
    )

    assert compare_fixture.main() == 1
    assert str(compare_fixture.DEFAULT_FIXTURE) in capsys.readouterr().err
