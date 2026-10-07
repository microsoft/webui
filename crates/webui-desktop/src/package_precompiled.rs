// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Resource-only layout for consumer-owned, already-compiled Rust hosts.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::bundle::DesktopPackageTarget;
use crate::error::{DesktopError, Result};
use crate::package::{self, windows};

#[path = "package_binary.rs"]
mod binary;
#[path = "package_output.rs"]
mod output;
#[path = "package_seal.rs"]
mod seal;
use output::PackageOutput;

/// A mapped input's layout class. Destinations are relative to the corresponding
/// executable or data root, not relative to the output directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    /// A native binary under `Contents/MacOS` or the portable root.
    Executable,
    /// An opaque file under `Contents/Resources` or `resources`.
    Data,
}

/// One explicitly mapped file. Directory trees must be enumerated by the consumer.
#[derive(Clone, Debug)]
pub struct PrecompiledResource {
    source: PathBuf,
    destination: PathBuf,
    kind: ResourceKind,
}

impl PrecompiledResource {
    /// Map one existing file into a relative executable or data destination.
    ///
    /// The file must exist when packaging runs; no file is opened by this constructor.
    ///
    /// # Errors
    ///
    /// Rejects an absolute, traversing, or Windows-aliased destination.
    pub fn new(
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
        kind: ResourceKind,
    ) -> Result<Self> {
        let destination = destination.into();
        validate_relative(&destination)?;
        Ok(Self {
            source: source.into(),
            destination,
            kind,
        })
    }
}

/// Inputs for laying out a compiled desktop host, without compiling WebUI sources.
pub struct PrecompiledHostOptions {
    host_exe: PathBuf,
    identity: Option<PackageIdentity>,
    target_triple: Option<String>,
    target: DesktopPackageTarget,
    out_dir: PathBuf,
    icon: Option<PathBuf>,
    resources: Vec<PrecompiledResource>,
}

struct PackageIdentity {
    app_id: String,
    app_name: String,
    version: String,
}

impl PrecompiledHostOptions {
    /// Begin a resource-only layout for an already-compiled native executable.
    /// Identity and target triple are required before packaging.
    ///
    /// # Errors
    ///
    /// Rejects an empty host path or output directory.
    pub fn new(
        host_exe: impl Into<PathBuf>,
        target: DesktopPackageTarget,
        out_dir: impl Into<PathBuf>,
    ) -> Result<Self> {
        let host_exe = host_exe.into();
        let out_dir = out_dir.into();
        if host_exe.as_os_str().is_empty() || out_dir.as_os_str().is_empty() {
            return Err(validation(
                "host executable and output directory must be specified".to_string(),
            ));
        }
        Ok(Self {
            host_exe,
            target,
            out_dir,
            identity: None,
            target_triple: None,
            icon: None,
            resources: Vec::new(),
        })
    }

    /// Set a reverse-DNS ID, display name, and dotted numeric version.
    ///
    /// # Errors
    ///
    /// Rejects invalid or missing identity fields.
    pub fn identity(
        mut self,
        app_id: impl Into<String>,
        app_name: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<Self> {
        let identity = PackageIdentity {
            app_id: app_id.into(),
            app_name: app_name.into(),
            version: version.into(),
        };
        validate_identity(&identity)?;
        self.identity = Some(identity);
        Ok(self)
    }

    /// Select the exact Rust target triple for the host and mapped binaries.
    ///
    /// # Errors
    ///
    /// Rejects a triple not supported by the chosen package layout.
    pub fn target_triple(mut self, triple: impl Into<String>) -> Result<Self> {
        let triple = triple.into();
        binary::parse_target(&triple, package_format(self.target))?;
        self.target_triple = Some(triple);
        Ok(self)
    }

    /// Add an existing icon file; macOS packages require `.icns`.
    ///
    /// # Errors
    ///
    /// Rejects an empty or invalid icon name or a non-`.icns` macOS icon.
    pub fn icon(mut self, icon: impl Into<PathBuf>) -> Result<Self> {
        let icon = icon.into();
        if icon.file_name().and_then(|name| name.to_str()).is_none()
            || (self.target == DesktopPackageTarget::MacosApp
                && icon.extension().and_then(|ext| ext.to_str()) != Some("icns"))
        {
            return Err(validation(format!("invalid icon path {}", icon.display())));
        }
        self.icon = Some(icon);
        Ok(self)
    }

    /// Add one mapped executable or data file without copying it yet.
    ///
    /// # Errors
    ///
    /// Rejects an unsafe destination, including Windows trailing-dot/space aliases.
    pub fn add_resource(mut self, resource: PrecompiledResource) -> Result<Self> {
        validate_relative(&resource.destination)?;
        self.resources.push(resource);
        Ok(self)
    }
}

/// Paths of the created layout for consumer-owned installers or distribution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrecompiledPackageResult {
    /// The `.app` or portable directory.
    pub output_path: PathBuf,
    /// Copied host executable.
    pub host_path: PathBuf,
    /// Copied mapped resources in input order.
    pub resource_paths: Vec<PathBuf>,
    /// Copied icon, if supplied.
    pub icon_path: Option<PathBuf>,
}

/// Lay out one already-compiled host and each explicitly mapped resource once.
///
/// The source bundle and server are owned by the caller; no protocol, browser
/// bundle, Node host, or renderer is generated. Windows portable layouts also
/// require the native build's App SDK bootstrap DLL and notices beside the host.
/// This operation does not sign, archive, install, or publish the result.
///
/// # Errors
///
/// Rejects invalid identity, triple, native binary headers, symlink inputs,
/// unsafe or colliding destination paths, overlapping output and inputs, and
/// existing output paths **before writing anything**. I/O errors during copying
/// can leave a partial *new* output directory; callers may inspect/remove it.
/// On Windows, use an output parent protected from concurrent untrusted
/// modification: standard path-based directory creation cannot anchor each
/// destination to an open root handle if another process swaps directories.
#[must_use = "packaging errors and layout paths must be handled"]
pub fn package_precompiled_host(
    options: PrecompiledHostOptions,
) -> Result<PrecompiledPackageResult> {
    let identity = options.identity.as_ref().ok_or_else(|| {
        validation("app identity is required; call .identity(id, name, version)".to_string())
    })?;
    validate_identity(identity)?;
    let triple = options.target_triple.as_deref().ok_or_else(|| {
        validation("target triple is required; call .target_triple(triple)".to_string())
    })?;
    let target = binary::parse_target(triple, package_format(options.target))?;
    let safe_name = package::safe_package_name(&identity.app_name);
    let name = match options.target {
        DesktopPackageTarget::MacosApp => format!("{safe_name}.app"),
        DesktopPackageTarget::WindowsPortable => format!("{safe_name}-windows-portable"),
        DesktopPackageTarget::LinuxPortable => format!("{safe_name}-linux-portable"),
    };
    let output_path = options.out_dir.join(name);
    let (exec_root, data_root) = match options.target {
        DesktopPackageTarget::MacosApp => (
            output_path.join("Contents/MacOS"),
            output_path.join("Contents/Resources"),
        ),
        _ => (output_path.clone(), output_path.join("resources")),
    };
    let mut host = open_input(&options.host_exe, &output_path, "host")?;
    binary::validate_binary(&mut host, &options.host_exe, target)?;
    let host_name = options
        .host_exe
        .file_name()
        .ok_or_else(|| validation("host executable has no file name".to_string()))?;
    if options.target == DesktopPackageTarget::WindowsPortable
        && !host_name
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".exe")
    {
        return Err(validation(
            "Windows host must have an .exe file name".to_string(),
        ));
    }
    let host_path = exec_root.join(host_name);
    let mut claimed = Claims {
        files: HashSet::with_capacity(options.resources.len() + 8),
        dirs: HashSet::with_capacity(options.resources.len() + 8),
    };
    claim(&mut claimed, &output_path, &host_path)?;
    let mut runtime_files = if options.target == DesktopPackageTarget::WindowsPortable {
        let files = windows::validate_inputs(&options.host_exe, &output_path)?;
        let mut opened = Vec::with_capacity(files.len());
        for file in &files {
            let mut handle = open_input(file, &output_path, "Windows App SDK deployment file")?;
            if handle
                .metadata()
                .map_err(|source| DesktopError::Io {
                    context: format!("checking Windows deployment file {}", file.display()),
                    source,
                })?
                .len()
                == 0
            {
                return Err(validation(format!(
                    "empty Windows deployment file {}",
                    file.display()
                )));
            }
            if file
                .file_name()
                .is_some_and(|name| name == windows::bootstrap_name())
            {
                binary::validate_binary(&mut handle, file, target)?;
            }
            if let Some(name) = file.file_name() {
                claim(&mut claimed, &output_path, &exec_root.join(name))?;
            }
            opened.push((file.clone(), handle));
        }
        opened
    } else {
        Vec::new()
    };
    let (icon_path, mut icon_handle) = if let Some(icon) = &options.icon {
        let handle = open_input(icon, &output_path, "icon")?;
        let icon_name = match options.target {
            DesktopPackageTarget::MacosApp => {
                if icon.extension().and_then(|v| v.to_str()) != Some("icns") {
                    return Err(validation("macOS icons must be .icns files".to_string()));
                }
                "AppIcon.icns"
            }
            _ => icon
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| validation("icon file name must be valid UTF-8".to_string()))?,
        };
        let path = data_root.join(icon_name);
        claim(&mut claimed, &output_path, &path)?;
        (Some(path), Some(handle))
    } else {
        (None, None)
    };
    let mut resource_paths = Vec::with_capacity(options.resources.len());
    let mut resource_seals = Vec::with_capacity(options.resources.len());
    for resource in &options.resources {
        validate_relative(&resource.destination)?;
        let mut handle = open_input(&resource.source, &output_path, "resource")?;
        if resource.kind == ResourceKind::Executable {
            binary::validate_binary(&mut handle, &resource.source, target)?;
            #[cfg(unix)]
            require_executable(&handle, &resource.source)?;
        } else {
            binary::validate_data(&mut handle, &resource.source, target)?;
        }
        let root = if resource.kind == ResourceKind::Executable {
            &exec_root
        } else {
            &data_root
        };
        let path = root.join(&resource.destination);
        claim(&mut claimed, &output_path, &path)?;
        resource_paths.push(path);
        resource_seals.push(seal::ResourceSeal::from_handle(
            &mut handle,
            &resource.source,
        )?);
    }
    #[cfg(unix)]
    require_executable(&host, &options.host_exe)?;
    if fs::symlink_metadata(&output_path).is_ok() {
        return Err(validation(format!(
            "output {} already exists; refusing to replace user data",
            output_path.display()
        )));
    }
    drop(claimed);

    // The Unix output root is an owned directory fd. All subsequent writes
    // are handle-relative and exclusive, never path-based fs::copy.
    let mut output = PackageOutput::create(
        &options.out_dir,
        output_path
            .file_name()
            .ok_or_else(|| validation("invalid package output name".to_string()))?,
    )?;
    copy_bound(
        &mut host,
        &mut output,
        CopySpec::new(&options.host_exe, &host_path, &output_path, target, true)?,
    )?;
    for (file, handle) in &mut runtime_files {
        if let Some(name) = file.file_name() {
            let destination = exec_root.join(name);
            copy_bound(
                handle,
                &mut output,
                CopySpec::new(
                    file,
                    &destination,
                    &output_path,
                    target,
                    name == windows::bootstrap_name(),
                )?,
            )?;
        }
    }
    if let (Some(source), Some(dest), Some(handle)) = (&options.icon, &icon_path, &mut icon_handle)
    {
        copy_bound(
            handle,
            &mut output,
            CopySpec::new(source, dest, &output_path, target, false)?,
        )?;
    }
    for ((resource, dest), resource_seal) in options
        .resources
        .iter()
        .zip(&resource_paths)
        .zip(&resource_seals)
    {
        seal::copy_sealed(
            &mut output,
            CopySpec::new(
                &resource.source,
                dest,
                &output_path,
                target,
                resource.kind == ResourceKind::Executable,
            )?,
            resource_seal,
        )?;
    }
    if options.target == DesktopPackageTarget::MacosApp {
        let executable_name = host_name.to_string_lossy();
        let plist = package::info_plist_fields(
            package::PlistIdentity {
                app_id: &identity.app_id,
                app_name: &identity.app_name,
                version: &identity.version,
            },
            &executable_name,
            icon_path.as_ref().map(|_| "AppIcon.icns"),
        );
        let mut dest = output.create_file(Path::new("Contents/Info.plist"))?;
        std::io::Write::write_all(&mut dest, plist.as_bytes()).map_err(|source| {
            DesktopError::Io {
                context: "writing macOS package Info.plist".to_string(),
                source,
            }
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            dest.set_permissions(fs::Permissions::from_mode(0o644))
                .map_err(|source| DesktopError::Io {
                    context: "setting macOS package Info.plist permissions".to_string(),
                    source,
                })?;
        }
    }
    output.finish()?;
    Ok(PrecompiledPackageResult {
        output_path,
        host_path,
        resource_paths,
        icon_path,
    })
}

fn package_format(target: DesktopPackageTarget) -> binary::Format {
    match target {
        DesktopPackageTarget::MacosApp => binary::Format::Mach,
        DesktopPackageTarget::WindowsPortable => binary::Format::Pe,
        DesktopPackageTarget::LinuxPortable => binary::Format::Elf,
    }
}

fn validate_identity(identity: &PackageIdentity) -> Result<()> {
    if identity.app_name.trim().is_empty()
        || identity.app_name.chars().any(char::is_control)
        || !identity
            .app_name
            .bytes()
            .any(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        || identity.app_id.split('.').count() < 2
        || identity.app_id.split('.').any(|part| {
            part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        || identity
            .version
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(validation(
            "app name, reverse-DNS ID, or dotted numeric version is invalid".to_string(),
        ));
    }
    Ok(())
}

fn open_input(source: &Path, output: &Path, label: &'static str) -> Result<File> {
    package::validate_input_overlap(output, source, label)?;
    if fs::symlink_metadata(source)
        .map_err(|source_error| DesktopError::Io {
            context: format!("checking package {label} {}", source.display()),
            source: source_error,
        })?
        .file_type()
        .is_symlink()
    {
        return Err(validation(format!(
            "package {label} {} is a symlink",
            source.display()
        )));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(source)
        .map_err(|source_error| DesktopError::Io {
            context: format!("checking package {label} {}", source.display()),
            source: source_error,
        })?;
    let metadata = file.metadata().map_err(|source_error| DesktopError::Io {
        context: format!("checking package {label} metadata {}", source.display()),
        source: source_error,
    })?;
    if !metadata.is_file() {
        return Err(validation(format!(
            "{label} {} must be a regular file, not a symlink or directory",
            source.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(validation(format!(
                "package {label} {} is a reparse point",
                source.display()
            )));
        }
    }
    Ok(file)
}

fn validate_relative(path: &Path) -> Result<()> {
    let safe = path.to_str().is_some_and(|text| {
        !text.is_empty()
            && text.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part.ends_with(['.', ' '])
                    && !part.contains(['\\', ':', '\0'])
            })
    });
    if !safe || path.is_absolute() {
        return Err(validation(format!(
            "unsafe resource destination {}",
            path.display()
        )));
    }
    Ok(())
}

struct Claims {
    files: HashSet<String>,
    dirs: HashSet<String>,
}

fn claim(claimed: &mut Claims, output: &Path, path: &Path) -> Result<()> {
    let relative = path.strip_prefix(output).map_err(|_| {
        validation(format!(
            "resource destination {} escapes output",
            path.display()
        ))
    })?;
    let text = relative.to_str().ok_or_else(|| {
        validation(format!(
            "package destination {} must be valid UTF-8",
            path.display()
        ))
    })?;
    // These paths include host-native separators added by Path::join.
    // Caller-provided mappings still pass strict validation before joining.
    let mut key = text.replace(std::path::MAIN_SEPARATOR, "/");
    validate_relative(Path::new(&key))?;
    key.make_ascii_lowercase();
    if key == "resources"
        || key == "contents"
        || key == "contents/info.plist"
        || key.split('/').any(|part| {
            let stem = part.split('.').next().unwrap_or_default();
            matches!(stem, "con" | "prn" | "aux" | "nul")
                || (stem.len() == 4
                    && (stem.starts_with("com") || stem.starts_with("lpt"))
                    && stem.as_bytes()[3].is_ascii_digit()
                    && stem.as_bytes()[3] != b'0')
        })
        || claimed.files.contains(&key)
        || claimed.dirs.contains(&key)
    {
        return Err(validation(format!(
            "colliding or reserved package destination {}",
            path.display()
        )));
    }
    let mut end = key.len();
    while let Some(index) = key[..end].rfind('/') {
        if claimed.files.contains(&key[..index]) {
            return Err(validation(format!(
                "package file conflicts with resource directory {}",
                path.display()
            )));
        }
        claimed.dirs.insert(key[..index].to_string());
        end = index;
    }
    claimed.files.insert(key);
    Ok(())
}

#[cfg(unix)]
fn require_executable(file: &File, path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = file
        .metadata()
        .map_err(|source| DesktopError::Io {
            context: format!("checking executable permissions {}", path.display()),
            source,
        })?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        return Err(validation(format!(
            "binary {} has no executable bit",
            path.display()
        )));
    }
    Ok(())
}

struct CopySpec<'a> {
    source: &'a Path,
    destination: &'a Path,
    root: &'a Path,
    relative: &'a Path,
    target: binary::Target,
    executable: bool,
}

impl<'a> CopySpec<'a> {
    fn new(
        source: &'a Path,
        destination: &'a Path,
        root: &'a Path,
        target: binary::Target,
        executable: bool,
    ) -> Result<Self> {
        let relative = destination.strip_prefix(root).map_err(|_| {
            validation(format!(
                "package file escapes output: {}",
                destination.display()
            ))
        })?;
        Ok(Self {
            source,
            destination,
            root,
            relative,
            target,
            executable,
        })
    }
}

fn copy_bound(input: &mut File, output: &mut PackageOutput, spec: CopySpec<'_>) -> Result<()> {
    input
        .seek(SeekFrom::Start(0))
        .map_err(|source| DesktopError::Io {
            context: format!("seeking package input {}", spec.source.display()),
            source,
        })?;
    let mut dest = output.create_file(spec.relative)?;
    std::io::copy(input, &mut dest).map_err(|source| DesktopError::Io {
        context: format!("copying package input into {}", spec.destination.display()),
        source,
    })?;
    let permissions = input
        .metadata()
        .map_err(|source| DesktopError::Io {
            context: format!("reading permissions for {}", spec.destination.display()),
            source,
        })?
        .permissions();
    dest.set_permissions(permissions)
        .map_err(|source| DesktopError::Io {
            context: format!("setting permissions on {}", spec.destination.display()),
            source,
        })?;
    // Source and destination are still the *same open handles* used during
    // preflight and copy. Catch in-place mutation and truncated/corrupt writes.
    input
        .seek(SeekFrom::Start(0))
        .map_err(|source| DesktopError::Io {
            context: format!("rewinding package input {}", spec.source.display()),
            source,
        })?;
    dest.seek(SeekFrom::Start(0))
        .map_err(|source| DesktopError::Io {
            context: format!("rewinding package output {}", spec.destination.display()),
            source,
        })?;
    let mut left = [0u8; 16 * 1024];
    let mut right = [0u8; 16 * 1024];
    loop {
        let count = input.read(&mut left).map_err(|source| DesktopError::Io {
            context: format!("verifying package input {}", spec.source.display()),
            source,
        })?;
        if count == 0 {
            let extra = dest
                .read(&mut right[..1])
                .map_err(|source| DesktopError::Io {
                    context: format!(
                        "checking package output length {}",
                        spec.destination.display()
                    ),
                    source,
                })?;
            if extra == 0 {
                break;
            }
            return Err(validation(format!(
                "package output has extra content: {}",
                spec.destination.display()
            )));
        }
        dest.read_exact(&mut right[..count])
            .map_err(|source| DesktopError::Io {
                context: format!("verifying package output {}", spec.destination.display()),
                source,
            })?;
        if left[..count] != right[..count] {
            return Err(validation(format!(
                "package input changed during copy or output differs: {}",
                spec.source.display()
            )));
        }
    }
    if spec.executable {
        binary::validate_binary(&mut dest, spec.destination, spec.target)?;
    } else {
        binary::validate_data(&mut dest, spec.destination, spec.target)?;
    }
    Ok(())
}

fn validation(message: String) -> DesktopError {
    DesktopError::PackageValidation {
        message,
        help: "Fix the package identity, native input, destination mapping, or choose a new output directory; existing packages are never overwritten",
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn native_joined_paths_keep_portable_collision_keys() {
        let root = Path::new("package");
        let mut claims = Claims {
            files: HashSet::new(),
            dirs: HashSet::new(),
        };
        claim(&mut claims, root, &root.join("Contents/MacOS").join("host")).unwrap();
        claim(
            &mut claims,
            root,
            &root.join("resources").join("sealed/webui.bundle"),
        )
        .unwrap();
        assert!(claims.files.contains("contents/macos/host"));
        assert!(claims.files.contains("resources/sealed/webui.bundle"));
        assert!(claim(
            &mut claims,
            root,
            &root.join("resources/SEALED").join("WEBUI.BUNDLE"),
        )
        .is_err());
        assert!(claim(&mut claims, root, &root.join("resources").join("sealed")).is_err());
        assert!(claim(&mut claims, root, &root.join("resources").join("../escape"),).is_err());
        assert!(claim(&mut claims, root, Path::new("outside/resource")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn literal_unix_backslashes_are_not_native_separators() {
        let root = Path::new("package");
        let mut claims = Claims {
            files: HashSet::new(),
            dirs: HashSet::new(),
        };
        assert!(claim(&mut claims, root, &root.join(r"resources\payload")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn source_name_swap_cannot_change_copied_bytes_or_escape_output_root() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sealed");
        let replacement = dir.path().join("replacement");
        fs::write(&source, b"original sealed data").unwrap();
        fs::write(&replacement, b"changed data").unwrap();
        let root = dir.path().join("New.app");
        let mut input = open_input(&source, &root, "resource").unwrap();
        fs::rename(&source, dir.path().join("former-source")).unwrap();
        symlink(&replacement, &source).unwrap();
        let mut output = PackageOutput::create(dir.path(), root.file_name().unwrap()).unwrap();
        let dest = root.join("Contents/Resources/sealed");
        let target = binary::parse_target("x86_64-apple-darwin", binary::Format::Mach).unwrap();
        copy_bound(
            &mut input,
            &mut output,
            CopySpec::new(&source, &dest, &root, target, false).unwrap(),
        )
        .unwrap();
        output.finish().unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"original sealed data");
        assert_eq!(fs::read(replacement).unwrap(), b"changed data");
    }

    #[cfg(unix)]
    #[test]
    fn replacing_output_path_cannot_redirect_anchored_writes() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let root = dir.path().join("New.app");
        let mut output = PackageOutput::create(dir.path(), root.file_name().unwrap()).unwrap();
        let renamed = dir.path().join("renamed-package");
        fs::rename(&root, &renamed).unwrap();
        symlink(&outside, &root).unwrap();
        let mut dest = output
            .create_file(Path::new("Contents/Resources/sealed"))
            .unwrap();
        std::io::Write::write_all(&mut dest, b"sealed").unwrap();
        assert!(!outside.join("Contents").exists());
        assert_eq!(
            fs::read(renamed.join("Contents/Resources/sealed")).unwrap(),
            b"sealed"
        );
        assert!(matches!(
            output.finish(),
            Err(DesktopError::PackageValidation { .. })
        ));
    }
}
