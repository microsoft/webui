# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Compare two `webui inspect` JSON dumps semantically.

The committed `protocol.bin` fixture is a build output, so CI rebuilds it and
checks for drift. Bytes cannot be compared directly because protobuf map field
ordering is not stable, so compare the decoded JSON instead.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("committed", type=Path)
    parser.add_argument("generated", type=Path)
    args = parser.parse_args()

    try:
        committed = json.loads(args.committed.read_text(encoding="utf-8"))
        generated = json.loads(args.generated.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    if committed != generated:
        print(
            "error: the Python protocol fixture is stale; regenerate "
            "crates/webui-python/tests/fixtures/protocol.bin",
            file=sys.stderr,
        )
        return 1

    print("protocol fixture is current")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
