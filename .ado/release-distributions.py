# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Identify optional distributions from the selected release build's artifacts."""

import json
import sys

artifacts = {artifact["name"] for artifact in json.load(sys.stdin)["value"]}
patterns = []
for artifact, variable, assets in [
    (
        "standalone_native_toolchains",
        "toolchainsAvailable",
        ["toolchains/*.tar.gz", "toolchains/*.tar.gz.sha256"],
    ),
    (
        "standalone_sdk_archives",
        "sdkAvailable",
        ["sdk/*.tar.gz", "sdk/*.tar.gz.sha256"],
    ),
    ("go_module_proxy", "goAvailable", ["go/webui-go-proxy-*.tar.gz"]),
]:
    available = artifact in artifacts
    print(f"##vso[task.setvariable variable={variable};isOutput=true]{str(available).lower()}")
    if available:
        patterns.extend(
            "$(Build.SourcesDirectory)/publish_artifacts_standalone/" + asset
            for asset in assets
        )
print(
    "##vso[task.setvariable variable=releaseAssets;isOutput=true]"
    + "%0A".join(patterns)
)
