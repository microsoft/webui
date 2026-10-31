# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import json
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any

import compare_fixture
import pytest


def current_protocol() -> dict[str, Any]:
    return {
        "build_id": "sha256:" + "a" * 64,
        "components": {
            "owner-widget": {
                "template_json": '{"v":3,"h":"<child-widget></child-widget>","d":["child-widget"]}'
            },
        },
    }


def test_current_comparison_preserves_all_metadata() -> None:
    expected = current_protocol()
    actual = deepcopy(expected)
    compare_fixture.compare_protocols(expected, actual)
    assert expected == actual == current_protocol()


@pytest.mark.parametrize("version", [None, 1, 2, 4])
@pytest.mark.parametrize("group", ["components", "client_fragments"])
def test_old_or_missing_metadata_versions_are_not_normalized(
    version: int | None, group: str,
) -> None:
    expected = current_protocol()
    template: dict[str, object] = {"h": "<child-widget></child-widget>", "d": ["child-widget"]}
    if version is not None:
        template["v"] = version
    expected[group] = {"owner-widget": {"template_json": json.dumps(template)}}
    actual = deepcopy(expected)
    with pytest.raises(ValueError, match="must use v3"):
        compare_fixture.compare_protocols(expected, actual)
    assert actual == expected


@pytest.mark.parametrize("template", [
    '{"h":"<child-widget></child-widget>","v":3,"d":[]}',
    '{"h":"Changed","v":3,"d":["child-widget"]}',
    '{"h":"<child-widget></child-widget>","v":3,"d":["child-widget"],"rf":[["button",1]]}',
])
def test_current_content_dependencies_and_refs_cannot_be_discarded(template: str) -> None:
    expected = current_protocol()
    actual = deepcopy(expected)
    actual["components"]["owner-widget"]["template_json"] = template
    with pytest.raises(ValueError, match="fixtures differ"):
        compare_fixture.compare_protocols(expected, actual)
    assert actual["components"]["owner-widget"]["template_json"] == template


@pytest.mark.parametrize("identity", ["", "not-a-content-identity", "sha256:" + "g" * 64])
def test_invalid_identity_is_not_ignored(identity: str) -> None:
    actual = current_protocol()
    actual["build_id"] = identity
    with pytest.raises(ValueError, match="current build identity"):
        compare_fixture.compare_protocols(current_protocol(), actual)


def test_different_current_identity_is_not_normalized() -> None:
    actual = current_protocol()
    actual["build_id"] = "sha256:" + "b" * 64
    with pytest.raises(ValueError, match="fixtures differ"):
        compare_fixture.compare_protocols(current_protocol(), actual)


def test_reports_requested_mismatched_fixture(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    committed = tmp_path / "committed.json"
    generated = tmp_path / "generated.json"
    fixture = Path("target/python-fixtures/streaming-app/protocol.bin")
    current = current_protocol()
    committed.write_text(json.dumps(current), encoding="utf-8")
    current["tokens"] = ["unexpected-token"]
    generated.write_text(json.dumps(current), encoding="utf-8")
    monkeypatch.setattr(
        sys,
        "argv",
        ["compare_fixture.py", str(committed), str(generated), str(fixture)],
    )

    assert compare_fixture.main() == 1
    message = capsys.readouterr().err
    assert str(fixture) in message
    assert "fixtures differ" in message


def test_reports_default_generated_fixture_path(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    committed = tmp_path / "committed.json"
    generated = tmp_path / "generated.json"
    current = current_protocol()
    committed.write_text(json.dumps(current), encoding="utf-8")
    current["tokens"] = ["unexpected-token"]
    generated.write_text(json.dumps(current), encoding="utf-8")
    monkeypatch.setattr(
        sys,
        "argv",
        ["compare_fixture.py", str(committed), str(generated)],
    )

    assert compare_fixture.main() == 1
    assert str(compare_fixture.DEFAULT_FIXTURE) in capsys.readouterr().err


def test_comparison_accepts_map_order_but_does_not_rewrite_inputs(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    expected = tmp_path / "expected.json"
    generated = tmp_path / "generated.json"
    expected.write_text(json.dumps(current_protocol(), sort_keys=True), encoding="utf-8")
    actual = json.dumps(current_protocol(), indent=2)
    generated.write_text(actual, encoding="utf-8")
    monkeypatch.setattr(sys, "argv", ["compare_fixture.py", str(expected), str(generated)])
    assert compare_fixture.main() == 0
    assert "current protocol fixtures match" in capsys.readouterr().out
    assert generated.read_text(encoding="utf-8") == actual
