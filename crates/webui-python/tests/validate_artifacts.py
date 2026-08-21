# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

from __future__ import annotations

import argparse
import sys
import tarfile
import zipfile
from email.message import Message
from email.parser import BytesParser
from email.policy import default
from pathlib import Path, PurePosixPath

EXPECTED_LICENSE_EXPRESSION = "MIT"
EXPECTED_LICENSE_PATH = "LICENSE"
METADATA_VERSION = "2.4"
PROJECT_ROOT = Path(__file__).parents[1]


def _read_metadata(contents: bytes, source: str) -> Message:
    metadata = BytesParser(policy=default).parsebytes(contents)
    if metadata.get_all("Metadata-Version", []) != [METADATA_VERSION]:
        raise ValueError(f"{source}: expected one Metadata-Version: {METADATA_VERSION}")
    if metadata.get_all("License-Expression", []) != [EXPECTED_LICENSE_EXPRESSION]:
        raise ValueError(
            f"{source}: expected one License-Expression: {EXPECTED_LICENSE_EXPRESSION}"
        )
    if metadata.get_all("License", []):
        raise ValueError(f"{source}: stale legacy License metadata is present")

    license_files = metadata.get_all("License-File", [])
    if license_files != [EXPECTED_LICENSE_PATH]:
        raise ValueError(f"{source}: expected one License-File: {EXPECTED_LICENSE_PATH}")
    return metadata


def _validate_license_path(path: str, source: str) -> None:
    normalized = PurePosixPath(path)
    if normalized.is_absolute() or ".." in normalized.parts or normalized.as_posix() != path:
        raise ValueError(f"{source}: License-File must be a normalized relative path: {path}")


def _expected_license() -> bytes:
    return (PROJECT_ROOT / EXPECTED_LICENSE_PATH).read_bytes()


def _validate_sdist(path: Path) -> None:
    with tarfile.open(path, "r:*") as archive:
        members = archive.getmembers()
        metadata_members = [member for member in members if member.name.endswith("/PKG-INFO")]
        if len(metadata_members) != 1:
            raise ValueError(f"{path}: expected exactly one PKG-INFO")

        metadata_member = metadata_members[0]
        metadata_file = archive.extractfile(metadata_member)
        if metadata_file is None:
            raise ValueError(f"{path}: PKG-INFO is not a regular file")
        metadata = _read_metadata(metadata_file.read(), metadata_member.name)
        root = PurePosixPath(metadata_member.name).parent
        names = [member.name for member in members]

        for license_path in metadata.get_all("License-File", []):
            _validate_license_path(license_path, metadata_member.name)
            expected = (root / license_path).as_posix()
            if names.count(expected) != 1:
                raise ValueError(f"{path}: declared license is missing or duplicated: {expected}")
            license_file = archive.extractfile(expected)
            if license_file is None or license_file.read() != _expected_license():
                raise ValueError(
                    f"{path}: declared license does not contain the full license: {expected}"
                )


def _validate_wheel(path: Path) -> None:
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        metadata_names = [name for name in names if name.endswith(".dist-info/METADATA")]
        if len(metadata_names) != 1:
            raise ValueError(f"{path}: expected exactly one .dist-info/METADATA")

        metadata_name = metadata_names[0]
        metadata = _read_metadata(archive.read(metadata_name), metadata_name)
        dist_info = PurePosixPath(metadata_name).parent

        for license_path in metadata.get_all("License-File", []):
            _validate_license_path(license_path, metadata_name)
            expected = (dist_info / "licenses" / license_path).as_posix()
            if names.count(expected) != 1:
                raise ValueError(f"{path}: declared license is missing or duplicated: {expected}")
            if archive.read(expected) != _expected_license():
                raise ValueError(
                    f"{path}: declared license does not contain the full license: {expected}"
                )


def validate_artifact(path: Path) -> None:
    if path.name.endswith(".tar.gz"):
        _validate_sdist(path)
    elif path.suffix == ".whl":
        _validate_wheel(path)
    else:
        raise ValueError(f"{path}: expected a .tar.gz sdist or .whl wheel")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate PEP 639 license metadata and files in Python distributions."
    )
    parser.add_argument("artifacts", nargs="+", type=Path)
    args = parser.parse_args()

    failed = False
    for artifact in args.artifacts:
        try:
            validate_artifact(artifact)
        except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
            failed = True
            print(f"error: {error}", file=sys.stderr)
        else:
            print(f"validated {artifact}")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
