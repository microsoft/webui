# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Assert a built wheel has the exact expected filename and metadata tags.

maturin derives a wheel's name from the workspace version and the compilation
target. This checks that what landed in the output directory is the one wheel
the release contract expects, including the compressed manylinux platform tag
in the filename and its expanded form in ``.dist-info/WHEEL``.
"""

from __future__ import annotations

import argparse
import sys
import tomllib
from pathlib import Path
from zipfile import ZipFile

REPO_ROOT = Path(__file__).parents[3]
DISTRIBUTION_NAME = "microsoft_webui"
INTERPRETER_ABI = "cp311-abi3"


def workspace_version() -> str:
    manifest = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return str(manifest["workspace"]["package"]["version"])


def validate(output: Path, platform_tag: str) -> Path:
    expected_name = (
        f"{DISTRIBUTION_NAME}-{workspace_version()}-{INTERPRETER_ABI}-{platform_tag}.whl"
    )
    wheels = sorted(path.name for path in output.glob("*.whl"))
    if wheels != [expected_name]:
        raise ValueError(f"expected exactly {expected_name}, found {wheels}")

    wheel = output / expected_name
    with ZipFile(wheel) as archive:
        metadata_names = [
            name for name in archive.namelist() if name.endswith(".dist-info/WHEEL")
        ]
        if len(metadata_names) != 1:
            raise ValueError(f"expected one .dist-info/WHEEL, found {metadata_names}")
        metadata = archive.read(metadata_names[0]).decode("utf-8")

    # A compressed filename tag such as `manylinux_2_17_x86_64.manylinux2014_x86_64`
    # must expand to one `Tag:` line per component in the wheel metadata.
    expected_tags = {f"{INTERPRETER_ABI}-{part}" for part in platform_tag.split(".")}
    tags = {
        line.removeprefix("Tag: ").strip()
        for line in metadata.splitlines()
        if line.startswith("Tag: ")
    }
    if tags != expected_tags:
        raise ValueError(
            f"expected wheel metadata tags {sorted(expected_tags)}, found {sorted(tags)}"
        )
    return wheel


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="directory maturin wrote the wheel to")
    parser.add_argument("platform_tag", help="expected platform tag, e.g. win_amd64")
    args = parser.parse_args()

    try:
        wheel = validate(args.output, args.platform_tag)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(f"validated {wheel.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
