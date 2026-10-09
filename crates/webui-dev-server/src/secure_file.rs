// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Filesystem-root confinement for static file serving.

use std::fs::File;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
#[cfg(any(unix, windows))]
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct SecureRoot {
    path: PathBuf,
    #[cfg(any(unix, windows))]
    directory: Arc<File>,
    #[cfg(unix)]
    directory_identity: (u64, u64),
    #[cfg(windows)]
    final_path: PathBuf,
}

impl SecureRoot {
    #[cfg(unix)]
    pub(crate) fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::new_unix(path, || Ok(()))
    }

    #[cfg(unix)]
    fn new_unix(
        path: PathBuf,
        after_canonicalize: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<Self> {
        let path = std::fs::canonicalize(path)?;
        after_canonicalize()?;
        let directory = open_directory_no_follow(&path)?;
        let directory_identity = directory_identity(&directory.metadata()?)?;
        let directory = Arc::new(directory);
        Ok(Self {
            path,
            directory,
            directory_identity,
        })
    }

    #[cfg(windows)]
    pub(crate) fn new(path: PathBuf) -> std::io::Result<Self> {
        Self::new_windows(path, || Ok(()))
    }

    #[cfg(windows)]
    fn new_windows(
        path: PathBuf,
        after_canonicalize: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<Self> {
        let path = std::fs::canonicalize(path)?;
        after_canonicalize()?;
        let directory = open_node(&path)?;
        let final_path = final_path(&directory)?;
        if final_path != path {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "serving root changed during initialization",
            ));
        }
        Ok(Self {
            path,
            directory: Arc::new(directory),
            final_path,
        })
    }

    #[cfg(all(not(unix), not(windows)))]
    pub(crate) fn new(path: PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            path: std::fs::canonicalize(path)?,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn join(&self, path: &Path) -> PathBuf {
        self.path.join(path)
    }

    #[cfg(unix)]
    pub(crate) fn open(
        &self,
        path: PathBuf,
        detect_directory: bool,
        _known_length: Option<u64>,
    ) -> std::io::Result<OpenedNode> {
        self.validate_directory_identity()?;
        self.open_unix(path, detect_directory, || Ok(()))
    }

    #[cfg(unix)]
    fn validate_directory_identity(&self) -> std::io::Result<()> {
        let current_identity = directory_identity(&std::fs::symlink_metadata(&self.path)?)?;
        if current_identity == self.directory_identity {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "serving root changed after initialization",
            ))
        }
    }

    #[cfg(unix)]
    fn open_unix(
        &self,
        path: PathBuf,
        detect_directory: bool,
        after_metadata_check: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<OpenedNode> {
        use std::os::fd::AsRawFd;
        use std::path::Component;

        let relative = path.strip_prefix(&self.path).map_err(|_| {
            Error::new(ErrorKind::PermissionDenied, "path escapes the serving root")
        })?;
        let mut components = relative.components().peekable();
        let mut parent = None::<File>;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "path contains an unsafe component",
                ));
            };
            let parent_descriptor = parent
                .as_ref()
                .map_or_else(|| self.directory.as_raw_fd(), AsRawFd::as_raw_fd);
            if components.peek().is_none() {
                if detect_directory {
                    let metadata = metadata_at(parent_descriptor, name)?;
                    if metadata.file_type == libc::S_IFLNK {
                        return self.open_following_in_root(path.clone());
                    }
                    if metadata.file_type == libc::S_IFDIR {
                        return Ok(OpenedNode::Directory);
                    }
                    if metadata.file_type != libc::S_IFREG {
                        return Err(Error::new(
                            ErrorKind::InvalidInput,
                            "cannot serve a non-regular file",
                        ));
                    }
                }
                after_metadata_check()?;
                let file = match open_at(parent_descriptor, name, false) {
                    Ok(file) => file,
                    Err(error) if Self::is_link_resolution_error(&error) => {
                        return self.open_following_in_root(path.clone());
                    }
                    Err(error) => return Err(error),
                };
                return opened_file(path, file);
            }
            let child = match open_at(parent_descriptor, name, true) {
                Ok(child) => child,
                Err(error) if Self::is_link_resolution_error(&error) => {
                    return self.open_following_in_root(path.clone());
                }
                Err(error) => return Err(error),
            };
            parent = Some(child);
        }
        Err(Error::new(
            ErrorKind::InvalidInput,
            "path does not identify a file",
        ))
    }

    #[cfg(unix)]
    fn open_following_in_root(&self, response_path: PathBuf) -> std::io::Result<OpenedNode> {
        let checked_path = std::fs::canonicalize(&response_path)?;
        let relative_path = checked_path.strip_prefix(&self.path).map_err(|_| {
            Error::new(ErrorKind::PermissionDenied, "path escapes the serving root")
        })?;
        self.open_canonical_at(response_path, relative_path)
    }

    #[cfg(unix)]
    fn open_canonical_at(
        &self,
        response_path: PathBuf,
        relative_path: &Path,
    ) -> std::io::Result<OpenedNode> {
        use std::os::fd::AsRawFd;
        use std::path::Component;

        let mut components = relative_path.components().peekable();
        let mut parent = None;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "resolved path contains an invalid component",
                ));
            };
            let parent_descriptor = parent
                .as_ref()
                .map_or_else(|| self.directory.as_ref(), |file: &File| file);
            if components.peek().is_none() {
                return opened_file(
                    response_path,
                    open_at(parent_descriptor.as_raw_fd(), name, false)?,
                );
            }
            parent = Some(open_at(parent_descriptor.as_raw_fd(), name, true)?);
        }
        Ok(OpenedNode::Directory)
    }

    #[cfg(windows)]
    pub(crate) fn classify_windows_path(&self, path: &Path) -> std::io::Result<WindowsPathKind> {
        let metadata = windows_node_metadata(path)?;
        Ok(if metadata.is_plain_directory() {
            WindowsPathKind::PlainDirectory
        } else if metadata.is_reparse_point() {
            WindowsPathKind::ReparsePoint
        } else if metadata.is_plain_file() {
            WindowsPathKind::PlainFile(metadata.length)
        } else {
            WindowsPathKind::Other
        })
    }

    #[cfg(windows)]
    pub(crate) fn open(
        &self,
        path: PathBuf,
        detect_directory: bool,
        known_length: Option<u64>,
    ) -> std::io::Result<OpenedNode> {
        self.open_checked_windows(path, detect_directory, known_length)
    }

    #[cfg(windows)]
    fn open_checked_windows(
        &self,
        response_path: PathBuf,
        detect_directory: bool,
        known_length: Option<u64>,
    ) -> std::io::Result<OpenedNode> {
        let relative_path = response_path.strip_prefix(&self.path).map_err(|_| {
            Error::new(ErrorKind::PermissionDenied, "path escapes the serving root")
        })?;
        if let Some(file) = open_relative_no_reparse(&self.directory, relative_path)? {
            return opened_windows_file(response_path, file, known_length);
        }

        let metadata = if detect_directory {
            let metadata = windows_node_metadata(&response_path)?;
            if metadata.is_plain_directory() {
                return Ok(OpenedNode::Directory);
            }
            Some(metadata)
        } else {
            None
        };
        let file = open_node(&response_path)?;
        let opened_path = final_path(&file)?;
        if !opened_path.starts_with(&self.final_path) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "opened file escapes the serving root",
            ));
        }
        if let Some(metadata) = metadata.filter(WindowsNodeMetadata::is_plain_file) {
            return Ok(OpenedNode::File {
                path: response_path,
                file,
                length: metadata.length,
            });
        }
        let opened_metadata = file.metadata()?;
        opened_file_with_metadata(response_path, file, opened_metadata)
    }

    #[cfg(unix)]
    fn is_link_resolution_error(error: &Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(libc::ELOOP) | Some(libc::ENOTDIR)
        )
    }

    #[cfg(all(not(unix), not(windows)))]
    pub(crate) fn open(
        &self,
        path: PathBuf,
        _detect_directory: bool,
        _known_length: Option<u64>,
    ) -> std::io::Result<OpenedNode> {
        let path = std::fs::canonicalize(path)?;
        if !path.starts_with(&self.path) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "path escapes the serving root",
            ));
        }
        let file = File::open(&path)?;
        opened_file(path, file)
    }
}

#[cfg(windows)]
fn open_relative_no_reparse(root: &File, path: &Path) -> std::io::Result<Option<File>> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{
        RtlNtStatusToDosError, HANDLE, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE,
        STATUS_REPARSE_POINT_ENCOUNTERED, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    let mut encoded = encode_windows_path(path)?;
    let length = encoded
        .len()
        .checked_sub(1)
        .and_then(|length| length.checked_mul(2))
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "path is too long"))?;
    let maximum_length = length
        .checked_add(2)
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "path is too long"))?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: maximum_length,
        Buffer: encoded.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(std::mem::size_of::<OBJECT_ATTRIBUTES>())
            .map_err(|_| Error::new(ErrorKind::InvalidData, "object attributes are too large"))?,
        RootDirectory: root.as_raw_handle(),
        ObjectName: &name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut io_status = IO_STATUS_BLOCK::default();
    // SAFETY: the root owns a live directory handle, `name` and `attributes`
    // remain valid for the call, and successful ownership transfers below.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status == STATUS_REPARSE_POINT_ENCOUNTERED {
        return Ok(None);
    }
    if status < 0 {
        // SAFETY: converting the status does not dereference any pointers.
        let code = unsafe { RtlNtStatusToDosError(status) };
        return Err(Error::from_raw_os_error(code as i32));
    }
    // SAFETY: successful `NtCreateFile` returned an owned, valid handle.
    Ok(Some(unsafe { File::from_raw_handle(handle) }))
}

#[cfg(windows)]
fn opened_windows_file(
    path: PathBuf,
    file: File,
    known_length: Option<u64>,
) -> std::io::Result<OpenedNode> {
    if let Some(length) = known_length {
        return Ok(OpenedNode::File { path, file, length });
    }
    let metadata = file.metadata()?;
    opened_file_with_metadata(path, file, metadata)
}

#[cfg(windows)]
fn open_node(path: &Path) -> std::io::Result<File> {
    use std::os::windows::io::FromRawHandle;

    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let encoded = encode_windows_path(path)?;
    // SAFETY: `encoded` is NUL-terminated and remains live for the call.
    // A successful handle is transferred immediately into the returned file.
    let handle = unsafe {
        CreateFileW(
            encoded.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::last_os_error());
    }
    // SAFETY: `CreateFileW` returned an owned, valid handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(windows)]
fn encode_windows_path(path: &Path) -> std::io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "path contains an embedded NUL",
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

#[cfg(windows)]
fn windows_node_metadata(path: &Path) -> std::io::Result<WindowsNodeMetadata> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesExW, GetFileExInfoStandard, WIN32_FILE_ATTRIBUTE_DATA,
    };

    let encoded = encode_windows_path(path)?;
    let mut metadata = std::mem::MaybeUninit::<WIN32_FILE_ATTRIBUTE_DATA>::uninit();
    // SAFETY: `encoded` is NUL-terminated and `metadata` provides enough
    // writable storage for the requested standard attribute data.
    if unsafe {
        GetFileAttributesExW(
            encoded.as_ptr(),
            GetFileExInfoStandard,
            metadata.as_mut_ptr().cast(),
        )
    } == 0
    {
        return Err(Error::last_os_error());
    }
    // SAFETY: successful `GetFileAttributesExW` initialized the value.
    let metadata = unsafe { metadata.assume_init() };
    Ok(WindowsNodeMetadata {
        attributes: metadata.dwFileAttributes,
        length: (u64::from(metadata.nFileSizeHigh) << 32) | u64::from(metadata.nFileSizeLow),
    })
}

#[cfg(windows)]
struct WindowsNodeMetadata {
    attributes: u32,
    length: u64,
}

#[cfg(windows)]
impl WindowsNodeMetadata {
    fn is_plain_directory(&self) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        };

        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
            && self.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }

    fn is_plain_file(&self) -> bool {
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        };

        self.attributes
            & (FILE_ATTRIBUTE_DEVICE | FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            == 0
    }

    fn is_reparse_point(&self) -> bool {
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

#[cfg(windows)]
pub(crate) enum WindowsPathKind {
    PlainDirectory,
    PlainFile(u64),
    ReparsePoint,
    Other,
}

#[cfg(windows)]
fn final_path(file: &File) -> std::io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

    let mut buffer = vec![0_u16; 260];
    loop {
        // SAFETY: the file owns a valid handle and `buffer` provides the
        // writable capacity reported to `GetFinalPathNameByHandleW`.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).map_err(|_| {
                    Error::new(ErrorKind::InvalidData, "resolved path exceeds u32 length")
                })?,
                0,
            )
        };
        if length == 0 {
            return Err(Error::last_os_error());
        }

        let length = usize::try_from(length)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "resolved path is too long"))?;
        if length < buffer.len() {
            return Ok(PathBuf::from(OsString::from_wide(&buffer[..length])));
        }
        buffer.resize(
            length
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::InvalidData, "resolved path is too long"))?,
            0,
        );
    }
}

#[cfg(not(windows))]
fn opened_file(path: PathBuf, file: File) -> std::io::Result<OpenedNode> {
    let metadata = file.metadata()?;
    opened_file_with_metadata(path, file, metadata)
}

fn opened_file_with_metadata(
    path: PathBuf,
    file: File,
    metadata: std::fs::Metadata,
) -> std::io::Result<OpenedNode> {
    if metadata.is_dir() {
        return Ok(OpenedNode::Directory);
    }
    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "cannot serve a non-regular file",
        ));
    }
    Ok(OpenedNode::File {
        path,
        file,
        length: metadata.len(),
    })
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::fd::AsRawFd;
    use std::path::Component;

    let mut directory = File::open(Path::new("/"))?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = open_at(directory.as_raw_fd(), name, true)?;
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "serving root contains an invalid component",
                ));
            }
        }
    }
    Ok(directory)
}

#[cfg(unix)]
fn directory_identity(metadata: &std::fs::Metadata) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    if !metadata.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "serving root is not a directory",
        ));
    }
    Ok((metadata.dev(), metadata.ino()))
}

pub(crate) enum OpenedNode {
    File {
        path: PathBuf,
        file: File,
        length: u64,
    },
    Directory,
}

#[cfg(unix)]
struct EntryMetadata {
    file_type: libc::mode_t,
}

#[cfg(unix)]
fn open_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
    directory: bool,
) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(name.as_bytes())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "path component contains NUL"))?;
    let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    // SAFETY: `parent` is a live directory descriptor, `name` is a
    // NUL-terminated single component, and no pointer outlives this call.
    let descriptor = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: `openat` returned a new owned descriptor that is transferred
    // exactly once into `File`.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn metadata_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::OsStr,
) -> std::io::Result<EntryMetadata> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let name = CString::new(name.as_bytes())
        .map_err(|_| Error::new(ErrorKind::InvalidInput, "path component contains NUL"))?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `parent` is a live directory descriptor, `name` is a
    // NUL-terminated single component, and `metadata` points to enough
    // writable memory for `fstatat` to initialize.
    let result = unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: successful `fstatat` initialized the complete `stat` value.
    let metadata = unsafe { metadata.assume_init() };
    Ok(EntryMetadata {
        file_type: metadata.st_mode & libc::S_IFMT,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn rejects_file_replaced_after_metadata_check() -> Result<(), Box<dyn std::error::Error>> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("asset.js");
        std::fs::write(&path, b"safe")?;
        let root = SecureRoot::new(directory.path().to_path_buf())?;

        let result = root.open_unix(path.clone(), true, || {
            std::fs::remove_file(&path)?;
            let path = CString::new(path.as_os_str().as_bytes())?;
            // SAFETY: `path` is NUL-terminated and points to writable
            // filesystem storage owned by this test.
            if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
                return Err(Error::last_os_error());
            }
            Ok(())
        });

        assert!(result.is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_root_replaced_after_canonicalize() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let root_path = directory.path().join("root");
        let displaced = directory.path().join("displaced");
        std::fs::create_dir(&root_path)?;

        let result = SecureRoot::new_unix(root_path.clone(), || {
            std::fs::rename(&root_path, &displaced)?;
            symlink(Path::new("/"), &root_path)?;
            Ok(())
        });

        assert!(result.is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_root_directory_replaced_after_canonicalize() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let root_path = directory.path().join("root");
        let displaced = directory.path().join("displaced");
        let replacement = directory.path().join("replacement");
        std::fs::create_dir(&root_path)?;
        std::fs::create_dir(&replacement)?;
        std::fs::write(root_path.join("asset.js"), b"original")?;
        std::fs::write(replacement.join("asset.js"), b"replacement")?;

        let root = SecureRoot::new_unix(root_path.clone(), || {
            std::fs::rename(&root_path, &displaced)?;
            std::fs::rename(&replacement, &root_path)?;
            Ok(())
        })?;
        std::fs::rename(&root_path, &replacement)?;
        std::fs::rename(&displaced, &root_path)?;

        assert!(matches!(
            root.open(root_path.join("asset.js"), true, None),
            Err(error) if error.kind() == ErrorKind::PermissionDenied
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_canonical_target_replaced_by_symlink() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let outside = tempfile::NamedTempFile::new()?;
        let target = directory.path().join("target");
        let alias = directory.path().join("asset.js");
        std::fs::write(&target, b"safe")?;
        symlink(&target, &alias)?;
        let root = SecureRoot::new(directory.path().to_path_buf())?;
        let checked = std::fs::canonicalize(&alias)?;
        let relative = checked.strip_prefix(root.path())?;

        std::fs::remove_file(&target)?;
        symlink(outside.path(), &target)?;

        assert!(root.open_canonical_at(alias, relative).is_err());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn rejects_root_junction_replaced_after_canonicalize() -> Result<(), Box<dyn std::error::Error>>
    {
        use std::os::windows::fs::symlink_dir;

        let directory = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let root_path = directory.path().join("root");
        let displaced = directory.path().join("displaced");
        std::fs::create_dir(&root_path)?;

        let result = SecureRoot::new_windows(root_path.clone(), || {
            std::fs::rename(&root_path, &displaced)?;
            symlink_dir(outside.path(), &root_path)?;
            Ok(())
        });

        assert!(matches!(
            result,
            Err(error) if error.kind() == ErrorKind::PermissionDenied
        ));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn rejects_junction_swap_after_path_check() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::windows::fs::symlink_dir;

        let directory = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let candidate = directory.path().join("candidate");
        let displaced = directory.path().join("displaced");
        std::fs::create_dir(&candidate)?;
        std::fs::write(candidate.join("asset.js"), b"safe")?;
        std::fs::write(outside.path().join("asset.js"), b"outside")?;
        let root = SecureRoot::new(directory.path().to_path_buf())?;
        let checked = std::fs::canonicalize(candidate.join("asset.js"))?;
        assert!(checked.starts_with(root.path()));

        std::fs::rename(&candidate, displaced)?;
        symlink_dir(outside.path(), &candidate)?;

        let result = root.open_checked_windows(checked, false, None);
        assert!(matches!(
            result,
            Err(error) if error.kind() == ErrorKind::PermissionDenied
        ));
        Ok(())
    }
}
