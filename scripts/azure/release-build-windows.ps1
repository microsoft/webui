# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

if (-not $env:BUILD_ARTIFACT_STAGING_DIRECTORY) {
    throw 'BUILD_ARTIFACT_STAGING_DIRECTORY is required.'
}

if (-not (Get-Command protoc -ErrorAction SilentlyContinue)) {
    $version = '29.5'
    $expectedSha256 = '633d3e555fc97f0a1f55b4adb03256cd94b8059e51e7abbae98ff39e58a9dfa5'
    $zip = "$env:AGENT_TEMP_DIRECTORY\protoc.zip"
    $dest = "$env:AGENT_TEMP_DIRECTORY\protoc"
    Invoke-WebRequest `
        -Uri "https://github.com/protocolbuffers/protobuf/releases/download/v$version/protoc-$version-win64.zip" `
        -OutFile $zip
    $actualSha256 = (Get-FileHash -Path $zip -Algorithm SHA256).Hash
    if (-not $actualSha256.Equals($expectedSha256, [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item $zip -Force
        throw "protoc archive SHA-256 mismatch: expected $expectedSha256, received $actualSha256."
    }
    Expand-Archive -Path $zip -DestinationPath $dest -Force
    $env:PATH = "$dest\bin;$env:PATH"
}

rustup toolchain install 1.93 `
    --profile minimal `
    --target x86_64-pc-windows-msvc,aarch64-pc-windows-msvc
rustup default 1.93

cargo build --release `
    --target x86_64-pc-windows-msvc `
    -p microsoft-webui-cli `
    -p microsoft-webui-ffi `
    -p microsoft-webui-node
cargo build --release `
    --target aarch64-pc-windows-msvc `
    -p microsoft-webui-cli `
    -p microsoft-webui-ffi `
    -p microsoft-webui-node
cargo xtask publish-stage `
    --target all `
    --profile release `
    --native-only

$stage = "$env:BUILD_ARTIFACT_STAGING_DIRECTORY\stage-windows"
New-Item -ItemType Directory -Force `
    "$stage\publish", "$stage\packages", "$stage\dotnet\runtimes" | Out-Null
Copy-Item publish\native -Destination "$stage\publish" -Recurse
Copy-Item packages\webui-win32-x64, packages\webui-win32-arm64 `
    -Destination "$stage\packages" -Recurse
Copy-Item dotnet\runtimes\win-x64, dotnet\runtimes\win-arm64 `
    -Destination "$stage\dotnet\runtimes" -Recurse
