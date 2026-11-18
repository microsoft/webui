// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Synthetic archive-layout fixtures only, never genuine artifact evidence.

use super::contract;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

pub(super) fn stage(root: &Path, version: &str) -> Result<(), String> {
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let producer = temporary.path().join("producer");
    fs::create_dir(&producer).map_err(|e| e.to_string())?;
    fs::write(producer.join("LICENSE"), b"synthetic contract fixture")
        .map_err(|e| e.to_string())?;
    for triple in super::release::TARGETS {
        let target = contract::target(triple).ok_or("missing target")?;
        let source = temporary.path().join(triple);
        fs::create_dir_all(source.join("lib")).map_err(|e| e.to_string())?;
        let package = Value::Object(Map::from_iter([
            (
                "name".to_owned(),
                Value::from(format!(
                    "@typescript/typescript-{}-{}",
                    target.os, target.cpu
                )),
            ),
            (
                "version".to_owned(),
                Value::from(contract::TYPESCRIPT_VERSION),
            ),
            ("os".to_owned(), Value::Array(vec![Value::from(target.os)])),
            (
                "cpu".to_owned(),
                Value::Array(vec![Value::from(target.cpu)]),
            ),
        ]));
        fs::write(
            source.join("package.json"),
            serde_json::to_vec(&package).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        for name in contract::libraries() {
            fs::write(source.join("lib").join(name), b"synthetic static contract")
                .map_err(|e| e.to_string())?;
        }
        for name in ["LICENSE", "NOTICE.txt"] {
            fs::write(source.join(name), b"synthetic contract notice")
                .map_err(|e| e.to_string())?;
        }
        let header = header_fixture(&target);
        let cli = source.join(target.cli);
        fs::write(&cli, &header).map_err(|e| e.to_string())?;
        fs::write(source.join("lib").join(target.compiler), header).map_err(|e| e.to_string())?;
        super::stage(&super::StageOptions {
            root: &producer,
            version,
            target: triple,
            checker_dir: &source,
            cli: &cli,
        })?;
    }
    // Export only this fixture's archive/checksum pairs. Do not alter the
    // caller's LICENSE, Cargo.toml, SDK, Go sources, or other distributions.
    let destination = super::output_directory(root)?;
    for entry in fs::read_dir(producer.join("publish/toolchains")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        fs::copy(entry.path(), destination.join(entry.file_name())).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn header_fixture(target: &contract::Target) -> Vec<u8> {
    let mut bytes = vec![0_u8; 80];
    let arm = target.cpu == "arm64";
    match target.os {
        "darwin" => {
            bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
            bytes[4..8]
                .copy_from_slice(&(if arm { 0x0100_000c_u32 } else { 0x0100_0007 }).to_le_bytes());
        }
        "linux" => {
            bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
            bytes[18..20].copy_from_slice(&(if arm { 183_u16 } else { 62 }).to_le_bytes());
        }
        _ => {
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
            bytes[64..68].copy_from_slice(b"PE\0\0");
            bytes[68..70].copy_from_slice(&(if arm { 0xaa64_u16 } else { 0x8664 }).to_le_bytes());
        }
    }
    bytes
}
