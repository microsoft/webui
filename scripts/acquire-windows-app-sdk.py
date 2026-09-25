# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Restore the pinned, unmodified Windows App SDK bootstrap redistributables."""

import hashlib
import json
from pathlib import Path
import urllib.request
import zipfile


ROOT = Path(__file__).resolve().parent.parent
VERSION = "1.8.260803002"
PACKAGE = f"Microsoft.WindowsAppSDK.Foundation.{VERSION}.nupkg"
URL = f"https://www.nuget.org/api/v2/package/Microsoft.WindowsAppSDK.Foundation/{VERSION}"
SHA256 = "b9232041afd605b606c6f78f442d92ead0076453f1f2a3260d2b7f8089bcab0e"
DEST = ROOT / "crates" / "webui-desktop" / "runtime"
DLL = "Microsoft.WindowsAppRuntime.Bootstrap.dll"


def write_if_changed(path, data):
    if path.is_file() and path.read_bytes() == data:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def main():
    archive = ROOT / "target" / "windows-app-sdk" / PACKAGE
    if not archive.is_file():
        archive.parent.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(URL, archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise SystemExit(f"SHA256 mismatch: remove {archive} and rerun to download the pinned archive.")

    files = []
    with zipfile.ZipFile(archive) as package:
        entries = [
            (f"runtimes/win-{arch}/native/{DLL}", f"win-{arch}/{DLL}")
            for arch in ("x64", "arm64", "x86")
        ]
        entries.append(("license.txt", "Microsoft.WindowsAppSDK.LICENSE.txt"))
        for source, destination in entries:
            data = package.read(source)
            write_if_changed(DEST / destination, data)
            files.append({
                "source": source,
                "path": destination,
                "sha256": hashlib.sha256(data).hexdigest(),
                "size": len(data),
            })
    manifest = {
        "package": "Microsoft.WindowsAppSDK.Foundation",
        "version": VERSION,
        "url": URL,
        "sha256": SHA256,
        "files": files,
    }
    write_if_changed(
        DEST / "Microsoft.WindowsAppSDK.PROVENANCE.json",
        (json.dumps(manifest, indent=2) + "\n").encode("utf-8"),
    )
    print(f"Verified {PACKAGE}; restored {len(files)} bootstrap/license assets.")


if __name__ == "__main__":
    main()
