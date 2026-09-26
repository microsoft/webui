// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const WINDOWS_APP_SDK_FILES: &[&str] = &[
    "Microsoft.WindowsAppRuntime.Bootstrap.dll",
    "Microsoft.WindowsAppSDK.LICENSE.txt",
    "Microsoft.WindowsAppSDK.NOTICES.txt",
    "Microsoft.WindowsAppSDK.PROVENANCE.json",
];

#[derive(Clone, Copy)]
pub(super) struct Plan {
    pub platform: &'static str,
    pub backend: &'static str,
    pub package_target: &'static str,
    pub executable_dir: &'static str,
    pub resources: &'static str,
    suffix: &'static str,
}

impl Plan {
    pub fn for_host() -> Result<Self, String> {
        match std::env::consts::OS {
            "macos" => Self::for_name("darwin"),
            "windows" => Self::for_name("win32"),
            "linux" => Self::for_name("linux"),
            other => Err(format!("unsupported native platform: {other}")),
        }
    }

    pub fn for_name(name: &str) -> Result<Self, String> {
        match name {
            "darwin" => Ok(Self {
                platform: "darwin",
                backend: "WKWebView",
                package_target: "macos-app",
                executable_dir: "Contents/MacOS",
                resources: "Contents/Resources/webui",
                suffix: "",
            }),
            "win32" => Ok(Self {
                platform: "win32",
                backend: "WebView2",
                package_target: "windows-portable",
                executable_dir: "",
                resources: "resources/webui",
                suffix: ".exe",
            }),
            "linux" => Ok(Self {
                platform: "linux",
                backend: "WebKitGTK",
                package_target: "linux-portable",
                executable_dir: "",
                resources: "resources/webui",
                suffix: "",
            }),
            other => Err(format!("unsupported native platform: {other}")),
        }
    }

    pub fn executable(self) -> String {
        let mut name = String::from("webui-native-ipc-fixture");
        name.push_str(self.suffix);
        name
    }

    pub fn cli(self) -> String {
        let mut name = String::from("webui-desktop");
        name.push_str(self.suffix);
        name
    }

    pub fn example(self, name: &str) -> String {
        let mut output = String::with_capacity(name.len() + self.suffix.len());
        output.push_str(name);
        output.push_str(self.suffix);
        output
    }

    pub fn json(self) -> Value {
        super::object([
            ("platform", Value::from(self.platform)),
            ("executable", Value::from(self.executable())),
            ("cli", Value::from(self.cli())),
            ("native_backend", Value::from(self.backend)),
            ("package_target", Value::from(self.package_target)),
            ("executable_dir", Value::from(self.executable_dir)),
            ("resources", Value::from(self.resources)),
        ])
    }

    pub fn is_windows(self) -> bool {
        self.platform == "win32"
    }
}

pub(super) fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(digest, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(digest)
}

pub(super) fn copy_runner(
    source: &Path,
    destination: &Path,
    plan: Plan,
    companions: &Path,
) -> Result<(), String> {
    let mut inputs = Vec::with_capacity(if plan.is_windows() { 5 } else { 1 });
    inputs.push(source.to_path_buf());
    if plan.is_windows() {
        inputs.extend(
            WINDOWS_APP_SDK_FILES
                .iter()
                .map(|name| companions.join(name)),
        );
    }
    for input in &inputs {
        let metadata = fs::metadata(input)
            .map_err(|error| format!("native runner input {}: {error}", input.display()))?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(format!(
                "native runner input is missing or empty: {}",
                input.display()
            ));
        }
    }
    for input in &inputs {
        let target = if input == source {
            destination.to_path_buf()
        } else {
            destination
                .parent()
                .ok_or("runner destination has no parent")?
                .join(input.file_name().ok_or("companion has no filename")?)
        };
        fs::copy(input, &target)
            .map_err(|error| format!("{} -> {}: {error}", input.display(), target.display()))?;
    }
    Ok(())
}

pub(super) fn verify_companions(source: &Path, packaged: &Path) -> Result<(), String> {
    for name in WINDOWS_APP_SDK_FILES {
        let original = source.join(name);
        let bundled = packaged.join(name);
        if sha256(&original)? != sha256(&bundled)? {
            return Err(format!("Windows portable package did not preserve {name}"));
        }
    }
    Ok(())
}

pub(super) fn copy_directory(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir(to).map_err(|error| format!("{}: {error}", to.display()))?;
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, destination)) = stack.pop() {
        let entries =
            fs::read_dir(&source).map_err(|error| format!("{}: {error}", source.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", source.display()))?;
            let ty = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            let target = destination.join(entry.file_name());
            if ty.is_dir() {
                fs::create_dir(&target)
                    .map_err(|error| format!("{}: {error}", target.display()))?;
                stack.push((entry.path(), target));
            } else if ty.is_file() {
                fs::copy(entry.path(), &target)
                    .map_err(|error| format!("{}: {error}", target.display()))?;
            } else {
                return Err(format!(
                    "unsupported fixture input: {}",
                    entry.path().display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{copy_runner, sha256, Plan, WINDOWS_APP_SDK_FILES};
    use std::fs;

    #[test]
    fn windows_copy_requires_all_companions_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("examples").join("fixture.exe");
        let companions = dir.path().join("release");
        let destination = dir.path().join("output").join("fixture.exe");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(&companions).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&source, b"fixture").unwrap();
        for name in &WINDOWS_APP_SDK_FILES[..3] {
            fs::write(companions.join(name), name.as_bytes()).unwrap();
        }
        let plan = Plan::for_name("win32").unwrap();
        assert!(copy_runner(&source, &destination, plan, &companions).is_err());
        assert!(!destination.exists());
        fs::write(companions.join(WINDOWS_APP_SDK_FILES[3]), b"notices").unwrap();
        copy_runner(&source, &destination, plan, &companions).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"fixture");
        assert_eq!(sha256(&destination).unwrap(), sha256(&source).unwrap());
        for name in WINDOWS_APP_SDK_FILES {
            assert_eq!(
                fs::read(destination.parent().unwrap().join(name)).unwrap(),
                fs::read(companions.join(name)).unwrap()
            );
        }
    }
}
