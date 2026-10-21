# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Compare two ``webui inspect`` JSON dumps semantically.

Committed protocol fixtures are build outputs, so CI rebuilds them and checks
for drift. Bytes cannot be compared directly because protobuf map field ordering
is not stable, so compare the decoded JSON instead.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

DEFAULT_FIXTURE = Path("crates/webui-python/tests/fixtures/protocol.bin")


def _ordinary_dependencies(protocol: dict[str, Any], owner: str) -> list[str] | None:
    records = protocol.get("fragments", {})
    known = {
        name
        for name, component in protocol.get("components", {}).items()
        if component.get("template_json")
    }
    pending = [owner]
    visited: set[str] = set()
    dependencies: set[str] = set()
    while pending:
        name = pending.pop()
        if name in visited:
            continue
        visited.add(name)
        record = records.get(name)
        if record is None:
            return None
        for item in record["fragments"]:
            instruction = item.get("fragment") or {}
            if "Component" in instruction:
                target = instruction["Component"]["fragment_id"]
                if target in known:
                    dependencies.add(target)
                else:
                    pending.append(target)
            elif "ForLoop" in instruction:
                pending.append(instruction["ForLoop"]["fragment_id"])
            elif "IfCond" in instruction:
                if not instruction["IfCond"].get("deferred", False):
                    pending.append(instruction["IfCond"]["fragment_id"])
            elif "Attribute" in instruction and instruction["Attribute"].get("template"):
                pending.append(instruction["Attribute"]["template"])
    return sorted(dependencies)


def normalize_legacy_metadata(
    committed: dict[str, Any], generated: dict[str, Any]
) -> None:
    """Keep legacy binaries as compatibility inputs, validating additive metadata."""
    identity = generated.get("build_id", "")
    if (
        not committed.get("build_id")
        and identity.startswith("sha256:")
        and len(identity) == 71
        and all(char in "0123456789abcdef" for char in identity[7:])
    ):
        if "build_id" in committed:
            generated["build_id"] = committed["build_id"]
        else:
            generated.pop("build_id")
    previous = committed.get("components", {})
    for name, component in generated.get("components", {}).items():
        original = previous.get(name, {}).get("template_json", "")
        current = component.get("template_json", "")
        if not original or not current:
            continue
        old = json.loads(original)
        new = json.loads(current)
        if "v" in old or new.get("v") != 1:
            continue
        if new.get("d", []) != _ordinary_dependencies(committed, name):
            continue
        new.pop("v")
        new.pop("d", None)
        if old == new:
            component["template_json"] = original


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("committed", type=Path)
    parser.add_argument("generated", type=Path)
    parser.add_argument(
        "fixture",
        nargs="?",
        type=Path,
        default=DEFAULT_FIXTURE,
        help="committed binary to name when the semantic comparison fails",
    )
    args = parser.parse_args()

    try:
        committed = json.loads(args.committed.read_text(encoding="utf-8"))
        generated = json.loads(args.generated.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    if isinstance(committed, dict) and isinstance(generated, dict):
        normalize_legacy_metadata(committed, generated)
    if committed != generated:
        print(
            f"error: the Python protocol fixture is stale; regenerate {args.fixture}",
            file=sys.stderr,
        )
        return 1

    print("protocol fixture is current")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
