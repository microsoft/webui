# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

import json
import re
from typing import Any

from microsoft_webui import Renderer

REGION = "test-panel::lazy:@0"


def _bootstrap(html: bytes) -> dict[str, Any]:
    match = re.search(rb'id="webui-data"[^>]*>(.*?)</script>', html, re.S)
    assert match, html
    data: dict[str, Any] = json.loads(match.group(1))
    return data


def test_closed_regions_reference_their_build_time_pack(lazy_renderer: Renderer) -> None:
    html = lazy_renderer.render({"show": False, "message": "Fresh", "items": ["one"]})
    assert b"Lazy inline UI" not in html
    templates = _bootstrap(html)["templates"]
    owner = templates["test-panel"]
    url = owner["lp"][REGION]
    assert url.startswith("_webui/packs/test-panel.")
    assert url.endswith(".json")
    trigger = next(entry[3] for entry in owner["c"] if entry[1] == REGION)
    assert trigger == 2
    assert REGION not in templates


def test_open_regions_render_inline_with_their_template(lazy_renderer: Renderer) -> None:
    html = lazy_renderer.render({"show": True, "message": "Fresh", "items": ["one"]})
    assert b"Lazy inline UI" in html
    assert b"Fresh" in html
    fragment = _bootstrap(html)["templates"][REGION]
    assert fragment["f"] == REGION
    assert fragment["o"] == "test-panel"
    assert fragment["e"] == 1
    assert fragment["r"] == [["items", "item", -1, [3, 0]]]
