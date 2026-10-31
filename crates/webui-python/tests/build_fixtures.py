# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Build source fixtures with the current compiler; never check protocol binaries in."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from compare_fixture import compare_protocols

ROOT = Path(__file__).resolve().parents[3]
SOURCES = Path(__file__).parent / "fixtures"
DEFAULT_OUTPUT = ROOT / "target" / "python-fixtures"
NAMES = ("app", "streaming-app", "lazy-app")


def cli_command() -> list[str]:
    executable = os.environ.get("WEBUI_BINARY_PATH")
    return [executable] if executable else [
        "cargo", "run", "--quiet", "-p", "microsoft-webui-cli", "--",
    ]


def inspect_protocol(command: list[str], path: Path) -> dict[str, Any]:
    result = subprocess.run(
        [*command, "inspect", str(path)],
        cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8",
    )
    protocol: dict[str, Any] = json.loads(result.stdout)
    if not isinstance(protocol, dict):
        raise ValueError(f"{path}: inspected protocol must be an object")
    return protocol


def build_fixtures(output: Path) -> Path:
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    command = cli_command()
    with tempfile.TemporaryDirectory(prefix="verify-", dir=output) as scratch:
        for name in NAMES:
            destination = output / name
            repeated = Path(scratch) / name
            for directory in (destination, repeated):
                subprocess.run(
                    [
                        *command, "build", str(SOURCES / name), "--plugin", "webui",
                        "--out", str(directory),
                    ],
                    cwd=ROOT, check=True,
                )
            # Protobuf map bytes may differ; every decoded field must match.
            compare_protocols(
                inspect_protocol(command, destination / "protocol.bin"),
                inspect_protocol(command, repeated / "protocol.bin"),
            )
        if (output / "app" / "greeting-card.css").read_bytes() != (
            SOURCES / "app" / "greeting-card.css"
        ).read_bytes():
            raise ValueError("greeting-card.css differs from the authored fixture stylesheet")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    try:
        output = build_fixtures(args.out)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: could not build current Python fixtures: {error}", file=sys.stderr)
        return 1
    print(f"current Python fixtures verified: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
