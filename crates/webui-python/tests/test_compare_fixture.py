# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import json
import sys
from pathlib import Path

import compare_fixture
import pytest


def test_legacy_promotion_validates_dependencies_and_preserves_template_content() -> None:
    legacy = {
        "build_id": "",
        "fragments": {
            "owner-widget": {
                "fragments": [{"fragment": {"Component": {"fragment_id": "child-widget"}}}]
            },
            "child-widget": {"fragments": []},
        },
        "components": {
            "owner-widget": {"template_json": '{"h":"<child-widget></child-widget>"}'},
            "child-widget": {"template_json": '{"h":"<p>Original</p>"}'},
        },
    }
    current = json.loads(json.dumps(legacy))
    current["build_id"] = "sha256:" + "a" * 64
    current["components"]["owner-widget"]["template_json"] = (
        '{"h":"<child-widget></child-widget>","v":1,"d":["child-widget"]}'
    )
    current["components"]["child-widget"]["template_json"] = '{"h":"<p>Original</p>","v":1}'
    compare_fixture.normalize_legacy_metadata(legacy, current)
    assert current == legacy

    for changed in [
        '{"h":"<child-widget></child-widget>","v":1,"d":[]}',
        '{"h":"Changed","v":1,"d":["child-widget"]}',
        '{"h":"<child-widget></child-widget>","v":3,"d":["child-widget"]}',
    ]:
        altered = json.loads(json.dumps(legacy))
        altered["components"]["owner-widget"]["template_json"] = changed
        compare_fixture.normalize_legacy_metadata(legacy, altered)
        assert altered != legacy


def test_malformed_new_identity_is_not_ignored() -> None:
    legacy = {"build_id": ""}
    generated = {"build_id": "not-a-content-identity"}
    compare_fixture.normalize_legacy_metadata(legacy, generated)
    assert legacy != generated


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
