# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import json
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from types import MappingProxyType
from typing import Any

import pytest
from conftest import PROTOCOL_PATH, STATE
from microsoft_webui import (
    Plugin,
    ProtocolError,
    Renderer,
    RenderError,
    StateError,
)


def test_constructs_from_supported_protocol_inputs(protocol_bytes: bytes) -> None:
    renderers = [
        Renderer(protocol_bytes),
        Renderer(bytearray(protocol_bytes)),
        Renderer(memoryview(protocol_bytes)),
        Renderer.from_file(PROTOCOL_PATH),
        Renderer.from_file(Path(PROTOCOL_PATH)),
    ]

    outputs = [renderer.render(STATE) for renderer in renderers]
    assert all(output == outputs[0] for output in outputs)
    assert b"Hello, Ada!" in outputs[0]


def test_rejects_invalid_protocol_and_plugin(protocol_bytes: bytes) -> None:
    with pytest.raises(ProtocolError, match="decode WebUI protocol"):
        Renderer(b"not a protobuf")
    with pytest.raises(TypeError, match="protocol"):
        Renderer("not bytes")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="not-a-plugin"):
        Renderer(protocol_bytes, plugin="not-a-plugin")


def test_missing_protocol_file_uses_normal_os_error(tmp_path: Path) -> None:
    missing = tmp_path / "missing.bin"
    with pytest.raises(FileNotFoundError):
        Renderer.from_file(missing)


def test_mapping_and_serialized_state_paths_match(renderer: Renderer) -> None:
    serialized = json.dumps(STATE, ensure_ascii=False, separators=(",", ":"))
    outputs = [
        renderer.render(STATE),
        renderer.render(MappingProxyType(STATE)),
        renderer.render(serialized),
        renderer.render(serialized.encode()),
        renderer.render(bytearray(serialized.encode())),
        renderer.render(memoryview(serialized.encode())),
    ]

    assert all(output == outputs[0] for output in outputs)
    assert isinstance(outputs[0], bytes)
    assert isinstance(renderer.render_text(STATE), str)


def test_exact_dict_is_serialized_without_copying(
    renderer: Renderer,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    original_dumps = json.dumps
    serialized_inputs: list[object] = []

    def tracking_dumps(value: object, **kwargs: Any) -> str:
        serialized_inputs.append(value)
        return original_dumps(value, **kwargs)

    monkeypatch.setattr("microsoft_webui._api.json.dumps", tracking_dumps)

    renderer.render(STATE)
    renderer.render(MappingProxyType(STATE))

    assert serialized_inputs[0] is STATE
    assert type(serialized_inputs[1]) is dict


def test_renders_unicode_escapes_html_and_accepts_large_state(renderer: Renderer) -> None:
    state: dict[str, Any] = {
        **STATE,
        "name": "Zoë 🐍 <script>\0",
        "unused": "x" * 1_000_000,
    }
    output = renderer.render(state)

    assert "Zoë 🐍".encode() in output
    assert b"&lt;script&gt;" in output
    assert b"<script>\x00" not in output


def test_rejects_wrong_or_invalid_state(renderer: Renderer) -> None:
    with pytest.raises(TypeError, match="state must be"):
        renderer.render(42)  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="JSON compliant"):
        renderer.render({**STATE, "bad": float("nan")})
    with pytest.raises(StateError, match="parse state JSON"):
        renderer.render(b"{not-json")
    with pytest.raises(StateError, match="invalid state JSON"):
        renderer.render_partial(b"{not-json")


def test_invalid_python_string_preserves_unicode_error(renderer: Renderer) -> None:
    with pytest.raises(UnicodeEncodeError):
        renderer.render("\ud800")


def test_missing_entry_is_actionable(renderer: Renderer) -> None:
    with pytest.raises(RenderError, match=r"missing\.html"):
        renderer.render(STATE, entry="missing.html")


@pytest.mark.parametrize("plugin", list(Plugin))
def test_all_plugins_render(protocol_bytes: bytes, plugin: Plugin) -> None:
    output = Renderer(protocol_bytes, plugin=plugin).render(STATE)
    assert isinstance(output, bytes)
    assert b"Hello," in output
    assert b"Ada" in output


def test_nonce_and_trusted_injections(renderer: Renderer) -> None:
    output = renderer.render(
        STATE,
        nonce="test-nonce",
        head_inject='<meta name="python-head">',
        body_inject='<script id="python-body"></script>',
    )

    assert b'nonce="test-nonce"' in output
    assert b'<meta name="python-head">' in output
    assert b'<script id="python-body"></script>' in output


def test_partial_and_component_template_payloads_are_json_bytes(renderer: Renderer) -> None:
    partial_bytes = renderer.render_partial(STATE)
    templates_bytes = renderer.render_component_templates(["greeting-card"])
    single_template_bytes = renderer.render_component_templates("greeting-card")
    partial = json.loads(partial_bytes)
    templates = json.loads(templates_bytes)

    assert isinstance(partial_bytes, bytes)
    assert partial["state"]["name"] == "Ada"
    assert "inventory" in partial
    assert isinstance(templates_bytes, bytes)
    assert "greeting-card" in json.dumps(templates)
    assert "inventory" in templates
    assert single_template_bytes == templates_bytes


def test_tokens_are_immutable_and_in_build_order(renderer: Renderer) -> None:
    assert renderer.tokens == ("brand-color",)
    assert isinstance(renderer.tokens, tuple)


def test_reuses_one_renderer_across_threads(renderer: Renderer) -> None:
    def render(index: int) -> bytes:
        state = {**STATE, "name": f"worker-{index}"}
        return renderer.render(state)

    with ThreadPoolExecutor(max_workers=4) as executor:
        outputs = list(executor.map(render, range(40)))

    for index, output in enumerate(outputs):
        assert f"Hello, worker-{index}!".encode() in output


def test_repeated_render_has_no_cross_request_state(renderer: Renderer) -> None:
    for index in range(100):
        output = renderer.render({**STATE, "name": f"request-{index}"})
        assert f"request-{index}".encode() in output
        if index:
            assert f"request-{index - 1}".encode() not in output
