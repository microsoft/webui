# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import json

import pytest
from microsoft_webui import DeferredOptions, Renderer, RenderError, StateError


def test_deferred_resources_and_terminal_state(deferred_renderer: Renderer) -> None:
    prepared = deferred_renderer.prepare_deferred_resources("test-panel", "details")
    metadata = json.loads(prepared.json)
    assert metadata["type"] == "webui-deferred"
    assert "state" not in metadata
    assert prepared.options.inventory == metadata["inventory"]
    assert prepared.json == deferred_renderer.render_deferred_resources("test-panel", "details")

    state = {"message": "Fresh", "items": ["one"], "unused": "not sent"}
    outputs = [
        deferred_renderer.render_deferred_partial(
            value, "test-panel", "details", options=prepared.options
        )
        for value in [state, json.dumps(state), json.dumps(state).encode()]
    ]
    assert outputs[0] == outputs[1] == outputs[2]
    payload = json.loads(outputs[0])
    assert payload["state"] == {"message": "Fresh", "items": ["one"]}
    assert "templates" not in payload
    assert "componentStyles" not in payload


def test_deferred_errors_remain_explicit(deferred_renderer: Renderer) -> None:
    with pytest.raises(RenderError, match="deferred-not-found"):
        deferred_renderer.render_deferred_resources("test-panel", "missing")
    with pytest.raises(RenderError, match="deferred-build-mismatch"):
        deferred_renderer.render_deferred_resources(
            "test-panel", "details", options=DeferredOptions(build_id="old")
        )
    with pytest.raises(StateError):
        deferred_renderer.render_deferred_partial("{", "test-panel", "details")
