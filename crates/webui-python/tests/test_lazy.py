# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import json

import pytest
from microsoft_webui import LazyOptions, Renderer, RenderError, StateError


def test_lazy_resources_and_terminal_state(lazy_renderer: Renderer) -> None:
    prepared = lazy_renderer.prepare_lazy_resources("test-panel", "details")
    metadata = json.loads(prepared.json)
    assert metadata["type"] == "webui-lazy"
    assert "state" not in metadata
    assert prepared.options.inventory == metadata["inventory"]
    assert prepared.json == lazy_renderer.render_lazy_resources("test-panel", "details")

    state = {"message": "Fresh", "items": ["one"], "unused": "not sent"}
    outputs = [
        lazy_renderer.render_lazy_partial(
            value, "test-panel", "details", options=prepared.options
        )
        for value in [state, json.dumps(state), json.dumps(state).encode()]
    ]
    assert outputs[0] == outputs[1] == outputs[2]
    payload = json.loads(outputs[0])
    assert payload["state"] == {"message": "Fresh", "items": ["one"]}
    assert "templates" not in payload
    assert "componentStyles" not in payload


def test_lazy_errors_remain_explicit(lazy_renderer: Renderer) -> None:
    with pytest.raises(RenderError, match="lazy-not-found"):
        lazy_renderer.render_lazy_resources("test-panel", "missing")
    with pytest.raises(RenderError, match="lazy-build-mismatch"):
        lazy_renderer.render_lazy_resources(
            "test-panel", "details", options=LazyOptions(build_id="old")
        )
    with pytest.raises(StateError):
        lazy_renderer.render_lazy_partial("{", "test-panel", "details")
