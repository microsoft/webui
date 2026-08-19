# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Assert every CI definition agrees with the xtask Python release contract.

``xtask/src/publish.rs`` owns the release contract: the distribution name, the
interpreter/ABI tags, and one platform tag per supported target. Those same tags
are restated in the Azure build pipeline and both GitHub workflows, where a typo
or a forgotten target only surfaces as a failed release. This script fails fast
when any of those restatements drift from the contract.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parents[3]
CONTRACT_SOURCE = Path("xtask/src/publish.rs")

# Restatements of the contract. Each entry maps a file to the extractor that
# recovers the platform tags it pins.
WHEEL_TAG_SOURCES = (
    Path(".ado/pipelines/azure-pipelines-build.yml"),
    Path(".github/workflows/publish-pypi.yml"),
)
MATRIX_TAG_SOURCE = Path(".github/workflows/pr-python.yml")

CONTRACT_TAG = re.compile(r'python_platform_tag:\s*"([^"]+)"')
CONTRACT_CONST = re.compile(r'const PYTHON_(\w+):\s*&str\s*=\s*"([^"]+)"')
WHEEL_TAG = re.compile(r"cp311-abi3-([A-Za-z0-9_.]+)")
MATRIX_TAG = re.compile(r"^\s*platform_tag:\s*(\S+)\s*$", re.MULTILINE)


def _read(relative: Path) -> str:
    return (REPO_ROOT / relative).read_text(encoding="utf-8")


def _contract_tags() -> set[str]:
    source = _read(CONTRACT_SOURCE)
    constants = dict(CONTRACT_CONST.findall(source))
    expected_constants = {
        "DISTRIBUTION_NAME": "microsoft_webui",
        "INTERPRETER_TAG": "cp311",
        "ABI_TAG": "abi3",
    }
    for name, value in expected_constants.items():
        if constants.get(name) != value:
            raise ValueError(
                f"{CONTRACT_SOURCE}: PYTHON_{name} is {constants.get(name)!r}, "
                f"expected {value!r}. Update every CI restatement together with "
                f"the contract."
            )

    tags = CONTRACT_TAG.findall(source)
    unique = set(tags)
    if len(unique) != 6:
        raise ValueError(
            f"{CONTRACT_SOURCE}: expected 6 unique python_platform_tag values, "
            f"found {sorted(unique)}"
        )
    return unique


def _restated_tags(relative: Path, pattern: re.Pattern[str]) -> set[str]:
    return {tag.removesuffix(".whl") for tag in pattern.findall(_read(relative))}


def main() -> int:
    try:
        contract = _contract_tags()
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    failed = False
    restatements = [(source, WHEEL_TAG) for source in WHEEL_TAG_SOURCES]
    restatements.append((MATRIX_TAG_SOURCE, MATRIX_TAG))

    for relative, pattern in restatements:
        try:
            restated = _restated_tags(relative, pattern)
        except OSError as error:
            print(f"error: {error}", file=sys.stderr)
            failed = True
            continue

        if restated != contract:
            failed = True
            print(
                f"error: {relative} does not match the {CONTRACT_SOURCE} release "
                f"contract.\n"
                f"  missing: {sorted(contract - restated) or 'none'}\n"
                f"  unexpected: {sorted(restated - contract) or 'none'}",
                file=sys.stderr,
            )
        else:
            print(f"validated {relative}")

    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
