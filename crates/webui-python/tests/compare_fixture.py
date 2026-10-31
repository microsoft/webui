# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Compare current-format ``webui inspect`` dumps without discarding metadata."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

DEFAULT_FIXTURE = Path("target/python-fixtures/app/protocol.bin")


def validate_metadata(protocol: dict[str, Any]) -> None:
    if not isinstance(protocol, dict):
        raise ValueError("protocol fixture must be an object")
    identity = protocol.get("build_id", "")
    if not (
        isinstance(identity, str)
        and identity.startswith("sha256:")
        and len(identity) == 71
        and all(char in "0123456789abcdef" for char in identity[7:])
    ):
        raise ValueError("protocol fixture must have a current build identity")
    for group in ("components", "client_fragments"):
        for name, record in protocol.get(group, {}).items():
            template = record.get("template_json")
            if template and json.loads(template).get("v") != 3:
                raise ValueError(f"protocol fixture metadata for {name} must use v3")


def compare_protocols(expected: dict[str, Any], actual: dict[str, Any]) -> None:
    validate_metadata(expected)
    validate_metadata(actual)
    if expected != actual:
        raise ValueError("protocol fixtures differ; inspect the current compiler output")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("expected", type=Path)
    parser.add_argument("generated", type=Path)
    parser.add_argument(
        "fixture",
        nargs="?",
        type=Path,
        default=DEFAULT_FIXTURE,
        help="fixture to name when current-format validation or semantic comparison fails",
    )
    args = parser.parse_args()

    try:
        expected = json.loads(args.expected.read_text(encoding="utf-8"))
        generated = json.loads(args.generated.read_text(encoding="utf-8"))
        compare_protocols(expected, generated)
    except (OSError, ValueError) as error:
        print(f"error: {args.fixture}: {error}", file=sys.stderr)
        return 1

    print("current protocol fixtures match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
