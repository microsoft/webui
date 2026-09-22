// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

use prost::Message as _;
use prost_reflect::DescriptorPool;
use sha2::{Digest, Sha256};

use crate::{
    emit,
    error::{io, schema},
    evolution, model, GenerateConfig, GenerateError, GeneratedFiles,
};

const DESCRIPTOR_LIMIT: u64 = 16 * 1024 * 1024;
const TS_OPTIONS: &str = "forceLong=bigint,useMapType=true,outputServices=none,oneof=unions-value,useOptionals=messages,outputJsonMethods=false,outputPartialMethods=false,useExactTypes=true,esModuleInterop=true,importSuffix=.js,env=browser,annotateFilesWithVersion=false";
static NEXT: AtomicU64 = AtomicU64::new(0);

mod tools;

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn generate(config: &GenerateConfig) -> Result<GeneratedFiles, GenerateError> {
    crate::artifacts::validate_paths(config)?;
    if config.roots.is_empty() {
        return Err(schema(
            "ipc-roots",
            "roots",
            "no schema roots supplied",
            "provide at least one application .proto file",
        ));
    }
    let base = config
        .lock_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !base.is_dir() {
        return Err(schema(
            "ipc-output",
            base.display().to_string(),
            "lock directory does not exist",
            "create the application schema directory first",
        ));
    }
    let scratch = Scratch(base.join(format!(
        ".webui-ipc-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir(&scratch.0).map_err(|e| io(&scratch.0, e))?;
    let rust_stage = scratch.0.join("rust");
    let ts_stage = scratch.0.join("ts");
    fs::create_dir(&rust_stage).map_err(|e| io(&rust_stage, e))?;
    fs::create_dir(&ts_stage).map_err(|e| io(&ts_stage, e))?;
    let roots = canonical(&config.roots)?;
    let sdk_include = scratch.0.join("include");
    let sdk_options = sdk_include.join("webui/ipc/options.proto");
    if let Some(parent) = sdk_options.parent() {
        fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    fs::write(
        &sdk_options,
        include_str!("../proto/webui/ipc/options.proto"),
    )
    .map_err(|e| io(&sdk_options, e))?;
    let mut includes = vec![fs::canonicalize(&sdk_include).map_err(|e| io(&sdk_include, e))?];
    includes.extend(canonical(&config.includes)?);
    for root in &roots {
        if fs::metadata(root).map_err(|e| io(root, e))?.len() > DESCRIPTOR_LIMIT {
            return Err(schema(
                "ipc-schema-limit",
                root.display().to_string(),
                "root schema exceeds 16 MiB",
                "split the application schema",
            ));
        }
        if !includes.iter().any(|dir| root.starts_with(dir)) {
            if let Some(parent) = root.parent() {
                includes.push(parent.into());
            }
        }
    }
    let descriptor = scratch.0.join("descriptor.bin");
    let protoc = config
        .protoc
        .clone()
        .or_else(|| std::env::var_os("PROTOC").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("protoc"));
    let protoc = tools::program(&protoc)?;
    let mut cmd = proto_command(&protoc, &includes)?;
    cmd.arg("--include_imports").arg("--include_source_info");
    tools::path_option(&mut cmd, "--descriptor_set_out=", &descriptor)?;
    for root in &roots {
        cmd.arg(tools::protoc_path(root)?);
    }
    run(&mut cmd, "protoc", "install protoc and set GenerateConfig.protoc; check the schema file/line in the compiler diagnostic")?;
    let bytes = bounded_read(&descriptor)?;
    let pool = DescriptorPool::decode(bytes.as_slice()).map_err(|e| {
        schema(
            "ipc-descriptor",
            "protoc output",
            e.to_string(),
            "compile the schema with a supported protoc",
        )
    })?;
    for file in pool.files() {
        if file.name().starts_with('-') {
            return Err(schema(
                "ipc-input-path",
                file.name(),
                "protobuf file name would be interpreted as a compiler option",
                "rename protobuf files so their import paths do not start with a dash",
            ));
        }
    }
    let contract = model::build(&pool)?;
    let normalized = normalized(&contract)?;
    let mut hash = String::with_capacity(64);
    for byte in Sha256::digest(&normalized) {
        write!(hash, "{byte:02x}").ok();
    }
    let old = match bounded_read(&config.lock_file) {
        Ok(bytes) => Some(bytes),
        Err(GenerateError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            None
        }
        Err(e) => return Err(e),
    };
    let lock = evolution::update(&contract, &hash, old.as_deref())?;
    let mut fds = prost_types::FileDescriptorSet::decode(bytes.as_slice()).map_err(|e| {
        schema(
            "ipc-descriptor",
            "protoc output",
            e.to_string(),
            "regenerate the descriptor set",
        )
    })?;
    for file in &mut fds.file {
        file.source_code_info = None;
    }
    fds.file.sort_by(|a, b| a.name.cmp(&b.name));
    let ts_descriptor = scratch.0.join("normalized.bin");
    fs::write(&ts_descriptor, fds.encode_to_vec()).map_err(|e| io(&ts_descriptor, e))?;
    fds.file.retain(|f| model::application_file(f.name()));
    let mut prost = prost_build::Config::new();
    prost
        .out_dir(&rust_stage)
        .include_file("ipc_messages.rs")
        .btree_map(["."]);
    prost.compile_fds(fds).map_err(|e| io(&rust_stage, e))?;
    let plugin = find_plugin(config.ts_proto_plugin.as_deref())?;
    verify_plugin(&plugin)?;
    let mut cmd = Command::new(&protoc);
    tools::path_option(&mut cmd, "--descriptor_set_in=", &ts_descriptor)?;
    tools::configure_plugin(
        &mut cmd,
        &plugin,
        &fs::canonicalize(&scratch.0).map_err(|e| io(&scratch.0, e))?,
    )?;
    tools::path_option(&mut cmd, "--ts_proto_out=", &ts_stage)?;
    cmd.arg(format!("--ts_proto_opt={TS_OPTIONS}"));
    // Generate every application import, not just root files.
    for file in pool.files().filter(|f| model::application_file(f.name())) {
        cmd.arg(file.name());
    }
    // Explicit Empty generation is needed even when only imported.
    if contract.messages.iter().any(|m| m.name == model::EMPTY) {
        cmd.arg("google/protobuf/empty.proto");
    }
    run(&mut cmd, "ts-proto", "install ts-proto@2.12.3 and @bufbuild/protobuf@2.15.0; set GenerateConfig.ts_proto_plugin to its protoc-gen-ts_proto executable")?;
    let mut artifacts = BTreeMap::new();
    collect(&rust_stage, &config.rust_out, &mut artifacts)?;
    collect(&ts_stage, &config.ts_out, &mut artifacts)?;
    let rust_wrapper = config.rust_out.join("ipc.rs");
    let ts_wrapper = config.ts_out.join("ipc.ts");
    let ts_runtime = config.ts_out.join("ipc-runtime.ts");
    if [&rust_wrapper, &ts_wrapper, &ts_runtime]
        .iter()
        .any(|path| artifacts.contains_key(*path))
    {
        return Err(schema(
            "ipc-output-collision",
            "ipc",
            "schema codec collides with the generated IPC wrapper",
            "rename the ipc.proto or ipc-runtime.proto file, or ipc package",
        ));
    }
    artifacts.insert(rust_wrapper, emit::rust(&contract, &hash)?.into_bytes());
    let (facade, runtime) = emit::typescript(&contract, &hash)?;
    artifacts.insert(ts_wrapper, facade.into_bytes());
    artifacts.insert(ts_runtime, runtime.into_bytes());
    let manifest = config.lock_file.with_file_name("ipc-schema.json");
    if config.lock_file == manifest
        || artifacts.contains_key(&manifest)
        || artifacts.contains_key(&config.lock_file)
    {
        return Err(schema(
            "ipc-output-collision",
            config.lock_file.display().to_string(),
            "schema metadata overlaps another generated artifact",
            "choose a distinct compatibility lock path",
        ));
    }
    artifacts.insert(manifest.clone(), normalized);
    artifacts.insert(config.lock_file.clone(), json(&lock)?);
    let result = GeneratedFiles {
        rust: artifacts
            .keys()
            .filter(|p| p.starts_with(&config.rust_out))
            .cloned()
            .collect(),
        typescript: artifacts
            .keys()
            .filter(|p| p.starts_with(&config.ts_out))
            .cloned()
            .collect(),
        manifest,
        inventory: config.lock_file.with_file_name("ipc-generated-files.json"),
        lock_file: config.lock_file.clone(),
        schema_hash: hash,
    };
    crate::artifacts::persist(config, artifacts)?;
    Ok(result)
}

fn canonical(paths: &[PathBuf]) -> Result<Vec<PathBuf>, GenerateError> {
    paths
        .iter()
        .map(|p| fs::canonicalize(p).map_err(|e| io(p, e)))
        .collect()
}

fn proto_command(protoc: &Path, includes: &[PathBuf]) -> Result<Command, GenerateError> {
    let mut cmd = Command::new(protoc);
    for include in includes {
        cmd.arg("-I").arg(tools::protoc_path(include)?);
    }
    Ok(cmd)
}

fn run(cmd: &mut Command, tool: &str, help: &str) -> Result<(), GenerateError> {
    let failure = |message: String| GenerateError::Tool {
        tool: tool.into(),
        message,
        help: help.into(),
    };
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| failure(e.to_string()))?;
    let mut diagnostic = Vec::with_capacity(4096);
    if let Some(stderr) = child.stderr.take() {
        if let Err(error) = stderr.take(16_385).read_to_end(&mut diagnostic) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure(error.to_string()));
        }
    }
    if diagnostic.len() > 16_384 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure(
            "compiler diagnostic exceeded 16 KiB; fix the first schema errors and retry".into(),
        ));
    }
    if !child.wait().map_err(|e| failure(e.to_string()))?.success() {
        return Err(failure(String::from_utf8_lossy(&diagnostic).into_owned()));
    }
    Ok(())
}

fn bounded_read(path: &Path) -> Result<Vec<u8>, GenerateError> {
    if fs::metadata(path).map_err(|e| io(path, e))?.len() > DESCRIPTOR_LIMIT {
        return Err(schema(
            "ipc-schema-limit",
            path.display().to_string(),
            "descriptor exceeds 16 MiB",
            "split the application schema",
        ));
    }
    fs::read(path).map_err(|e| io(path, e))
}

fn find_plugin(explicit: Option<&Path>) -> Result<PathBuf, GenerateError> {
    if let Some(path) = explicit {
        return fs::canonicalize(path).map_err(|e| io(path, e));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(if cfg!(windows) {
                "protoc-gen-ts_proto.cmd"
            } else {
                "protoc-gen-ts_proto"
            });
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(GenerateError::Tool { tool: "ts-proto".into(), message: "protoc-gen-ts_proto is not on PATH".into(), help: "install ts-proto@2.12.3 and pass GenerateConfig.ts_proto_plugin (for pnpm: node_modules/.bin/protoc-gen-ts_proto)".into() })
}

fn verify_plugin(plugin: &Path) -> Result<(), GenerateError> {
    for parent in plugin.ancestors().take(5) {
        for path in [
            parent.join("package.json"),
            parent.join("ts-proto/package.json"),
            parent.join("node_modules/ts-proto/package.json"),
        ] {
            let Ok(bytes) = fs::read(path) else { continue };
            let Ok(package) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if package["name"] == "ts-proto" {
                if package["version"] == "2.12.3" {
                    return verify_runtime(plugin);
                }
                return Err(schema("ipc-codegen-version", plugin.display().to_string(), "ts-proto version must be 2.12.3", "install the pinned compiler ts-proto@2.12.3 and runtime @bufbuild/protobuf@2.15.0"));
            }
        }
    }
    Err(schema(
        "ipc-codegen-version",
        plugin.display().to_string(),
        "cannot verify ts-proto package version",
        "point ts_proto_plugin at the installed ts-proto executable, not an opaque wrapper",
    ))
}

fn verify_runtime(plugin: &Path) -> Result<(), GenerateError> {
    for parent in plugin.ancestors().take(7) {
        for path in [
            parent.join("@bufbuild/protobuf/package.json"),
            parent.join("node_modules/@bufbuild/protobuf/package.json"),
        ] {
            let Ok(bytes) = fs::read(path) else { continue };
            let Ok(package) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if package["name"] == "@bufbuild/protobuf" && package["version"] == "2.15.0" {
                return Ok(());
            }
        }
    }
    Err(schema(
        "ipc-runtime-version",
        plugin.display().to_string(),
        "cannot verify @bufbuild/protobuf 2.15.0 alongside ts-proto",
        "install @bufbuild/protobuf@2.15.0 in the application containing the ts-proto plugin",
    ))
}

fn collect(
    stage: &Path,
    destination: &Path,
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), GenerateError> {
    let mut dirs = vec![stage.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| io(&dir, e))? {
            let path = entry.map_err(|e| io(&dir, e))?.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let relative = path.strip_prefix(stage).map_err(|e| {
                    schema(
                        "ipc-output",
                        path.display().to_string(),
                        e.to_string(),
                        "use disjoint generation directories",
                    )
                })?;
                if relative == Path::new("google/protobuf/descriptor.ts")
                    || relative == Path::new("webui/ipc/options.ts")
                {
                    continue;
                }
                let mut content = String::from("// Copyright (c) Microsoft Corporation.\n// Licensed under the MIT license.\n\n");
                content.push_str(&fs::read_to_string(&path).map_err(|e| io(&path, e))?);
                files.insert(destination.join(relative), content.into_bytes());
            }
        }
    }
    Ok(())
}

fn json(value: &impl serde::Serialize) -> Result<Vec<u8>, GenerateError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| {
        schema(
            "ipc-manifest",
            "generated contract",
            e.to_string(),
            "report this generator error",
        )
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn normalized(contract: &model::Contract) -> Result<Vec<u8>, GenerateError> {
    let mut value = serde_json::to_value(contract).map_err(|e| {
        schema(
            "ipc-manifest",
            &contract.name,
            e.to_string(),
            "report this generator error",
        )
    })?;
    if let Some(messages) = value
        .get_mut("messages")
        .and_then(serde_json::Value::as_array_mut)
    {
        for message in messages {
            if let Some(object) = message.as_object_mut() {
                object.remove("file");
                object.remove("rust");
                object.remove("ts");
            }
        }
    }
    json(&value)
}
