// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-only integrity check for an already-open, staged release artifact.
//!
//! The caller must authenticate release metadata independently, obtain user
//! consent, and derive the policy from the installed host. This module does
//! not authenticate release provenance, open a path, inspect archive contents,
//! verify OS signatures/notarization, or prevent later writes to the file.

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::time::SystemTime;

use sha2::{Digest, Sha256, Sha512};
use thiserror::Error;

const SCAN_BYTES: usize = 64 * 1024;
const MAX_VERSION_BYTES: usize = 62; // Three u64 decimals, separated by two dots.

/// A strict, stable `major.minor.patch` release version (no pre-release or build suffix).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[non_exhaustive]
pub struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl ReleaseVersion {
    /// Parse a canonical three-component numeric version, without leading zeroes.
    pub fn parse(value: &str) -> Result<Self, UpdateError> {
        if value.len() > MAX_VERSION_BYTES {
            return Err(UpdateError::InvalidVersion);
        }
        let mut parts = value.split('.');
        let mut next = || {
            let part = parts.next().ok_or(UpdateError::InvalidVersion)?;
            if part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(UpdateError::InvalidVersion);
            }
            part.parse::<u64>().map_err(|_| UpdateError::InvalidVersion)
        };
        let (major, minor, patch) = (next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(UpdateError::InvalidVersion);
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

/// Installed release trust level, provided by the trusted host, not the renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InstalledTrust {
    /// No installed publisher signature; only authenticated metadata and bytes are checked.
    Unsigned,
    /// Ad-hoc signing is not publisher identity proof.
    AdHoc,
    /// The installed app has a publisher signature; byte-only updates are insufficient.
    PublisherSigned,
    /// The installed app requires notarization; byte-only updates are insufficient.
    Notarized,
}

/// Extra trust proof required by the installed host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RequiredTrust {
    /// Authenticated-release byte integrity only.
    ReleaseIntegrity,
    /// Requires separate OS publisher signature verification (unsupported here).
    PublisherSignature,
    /// Requires separate OS notarization verification (unsupported here).
    Notarization,
}

/// Expected bytes and identity from independently authenticated private release metadata.
#[derive(Debug)]
#[non_exhaustive]
pub struct ExpectedUpdate {
    version: ReleaseVersion,
    app_id: String,
    target_triple: String,
    byte_len: u64,
    sha256: [u8; 32],
}

impl ExpectedUpdate {
    /// Construct expected metadata. The caller must authenticate all fields before calling.
    pub fn new(
        version: ReleaseVersion,
        app_id: &str,
        target_triple: &str,
        byte_len: u64,
        sha256: [u8; 32],
    ) -> Result<Self, UpdateError> {
        validate_identity(app_id, target_triple)?;
        if byte_len == 0 {
            return Err(UpdateError::InvalidLength);
        }
        Ok(Self {
            version,
            app_id: app_id.to_owned(),
            target_triple: target_triple.to_owned(),
            byte_len,
            sha256,
        })
    }
}

/// SHA-512 expectation from independently authenticated, target-bound release metadata.
///
/// Do not construct this from bytes hashed from the staged download itself:
/// that would establish no release provenance.
#[derive(Debug)]
#[non_exhaustive]
pub struct ExpectedUpdateSha512 {
    version: ReleaseVersion,
    app_id: String,
    target_triple: String,
    byte_len: u64,
    sha512: [u8; 64],
}

impl ExpectedUpdateSha512 {
    /// Construct a SHA-512 expectation after authenticating every release field.
    pub fn new(
        version: ReleaseVersion,
        app_id: &str,
        target_triple: &str,
        byte_len: u64,
        sha512: [u8; 64],
    ) -> Result<Self, UpdateError> {
        validate_identity(app_id, target_triple)?;
        if byte_len == 0 {
            return Err(UpdateError::InvalidLength);
        }
        Ok(Self {
            version,
            app_id: app_id.to_owned(),
            target_triple: target_triple.to_owned(),
            byte_len,
            sha512,
        })
    }
}

/// Constraints obtained from the *installed* app, never from an update offer.
#[derive(Debug)]
#[non_exhaustive]
pub struct InstalledUpdatePolicy {
    version: ReleaseVersion,
    app_id: String,
    target_triple: String,
    max_bytes: u64,
    installed_trust: InstalledTrust,
    required_trust: RequiredTrust,
}

impl InstalledUpdatePolicy {
    /// Set installed identity, version, trust and a nonzero maximum staged size.
    pub fn new(
        version: ReleaseVersion,
        app_id: &str,
        target_triple: &str,
        max_bytes: u64,
        installed_trust: InstalledTrust,
    ) -> Result<Self, UpdateError> {
        validate_identity(app_id, target_triple)?;
        if max_bytes == 0 {
            return Err(UpdateError::InvalidLimit);
        }
        Ok(Self {
            version,
            app_id: app_id.to_owned(),
            target_triple: target_triple.to_owned(),
            max_bytes,
            installed_trust,
            required_trust: RequiredTrust::ReleaseIntegrity,
        })
    }

    /// Raise the required proof level; requirements cannot be lowered later.
    pub fn require(mut self, requirement: RequiredTrust) -> Self {
        if trust_rank(requirement) > trust_rank(self.required_trust) {
            self.required_trust = requirement;
        }
        self
    }
}

fn trust_rank(trust: RequiredTrust) -> u8 {
    match trust {
        RequiredTrust::ReleaseIntegrity => 0,
        RequiredTrust::PublisherSignature => 1,
        RequiredTrust::Notarization => 2,
    }
}

#[cold]
#[inline(never)]
fn validate_identity(app_id: &str, target: &str) -> Result<(), UpdateError> {
    if app_id.len() > 255
        || app_id.split('.').count() < 2
        || app_id.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(UpdateError::InvalidAppId);
    }
    if target.len() > 128
        || target.split('-').count() < 3
        || target.split('-').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        return Err(UpdateError::InvalidTarget);
    }
    Ok(())
}

/// An observed digest and metadata for the retained file handle, not install authorization.
///
/// Do not reopen by path, infer archive contents, or assume the bytes remain unchanged
/// after verification. On return, the retained handle's cursor is at offset 0;
/// later reads or writes can move it. No installer or update handoff is provided.
#[derive(Debug)]
#[non_exhaustive]
pub struct StagedIntegrityReceipt {
    file: File,
    version: ReleaseVersion,
    app_id: String,
    target_triple: String,
    byte_len: u64,
    sha256: [u8; 32],
}

impl StagedIntegrityReceipt {
    /// Borrow the exact opened handle, rewound to offset 0 when verified, not a path.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// The authenticated release version associated with these observed bytes.
    pub fn version(&self) -> ReleaseVersion {
        self.version
    }

    /// The authenticated release app identity (not proof of opaque archive contents).
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The authenticated release target (not proof of opaque archive contents).
    pub fn target_triple(&self) -> &str {
        &self.target_triple
    }

    /// Number of bytes read and hashed.
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// SHA-256 observed while scanning the opened handle.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

/// SHA-512 observation over the retained, rewound opened handle, not install authority.
///
/// The file cursor is at offset 0 on return, but subsequent reads or writes can
/// move it or change the bytes. No SHA-256 assertion is made on this path.
#[derive(Debug)]
#[non_exhaustive]
pub struct StagedSha512IntegrityReceipt {
    file: File,
    version: ReleaseVersion,
    app_id: String,
    target_triple: String,
    byte_len: u64,
    sha512: [u8; 64],
}

impl StagedSha512IntegrityReceipt {
    /// Borrow the exact opened handle, rewound to offset 0 when verified, not a path.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Version from the authenticated release expectation.
    pub fn version(&self) -> ReleaseVersion {
        self.version
    }

    /// App ID from the authenticated release expectation, not proof of archive contents.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Target from the authenticated release expectation, not proof of archive contents.
    pub fn target_triple(&self) -> &str {
        &self.target_triple
    }

    /// Number of bytes scanned and checked against the expected size.
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Observed SHA-512 digest matched to independently authenticated expected metadata.
    pub fn sha512(&self) -> [u8; 64] {
        self.sha512
    }
}

/// Actionable failure in policy validation or staged-file verification.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UpdateError {
    /// Version must be a canonical numeric triplet.
    #[error("invalid version: use canonical major.minor.patch numeric components")]
    InvalidVersion,
    /// Application identity must be a bounded reverse-DNS identifier.
    #[error("invalid app ID: use a bounded dotted ASCII identifier")]
    InvalidAppId,
    /// Target must be a bounded target triple.
    #[error("invalid target triple: use a bounded ASCII target triple")]
    InvalidTarget,
    /// Staged artifacts cannot have zero expected bytes.
    #[error("expected update size is zero: provide an authenticated nonempty artifact size")]
    InvalidLength,
    /// Installed host must set a positive size cap.
    #[error("installed update size cap is zero: configure a positive maximum")]
    InvalidLimit,
    /// Authenticated size exceeds the installed host's cap.
    #[error("update exceeds installed size cap: reject the offer or raise the trusted host limit")]
    SizeLimitExceeded,
    /// Update is not newer than the installed version.
    #[error("update version is not newer than installed version: reject replay or downgrade")]
    VersionNotNewer,
    /// Release identity does not match the installed application.
    #[error("update app ID does not match installed app ID: reject the offer")]
    AppIdMismatch,
    /// Release target does not match the installed host target.
    #[error("update target triple does not match installed host target: reject the offer")]
    TargetMismatch,
    /// A signature or notarization proof cannot be established by byte hashing.
    #[error("installed policy requires publisher signature or notarization: use a separate OS verification path")]
    UnverifiableTrust,
    /// The supplied handle is not a regular file.
    #[error("staged handle is not a regular file: pass an already-open regular file")]
    NotRegularFile,
    /// The opened file changed while it was scanned.
    #[error("staged file changed during verification: discard the artifact and retry from a fresh staged file")]
    FileChanged,
    /// Observed and authenticated lengths differ.
    #[error("staged file length differs from authenticated metadata: discard the artifact")]
    LengthMismatch,
    /// Observed and authenticated hashes differ.
    #[error("staged file SHA-256 differs from authenticated metadata: discard the artifact")]
    DigestMismatch,
    /// Observed and authenticated SHA-512 hashes differ.
    #[error("staged file SHA-512 differs from authenticated metadata: discard the artifact")]
    Sha512Mismatch,
    /// Metadata, seek, or read operation failed.
    #[error("failed {context} on staged handle")]
    Io {
        /// Operation that failed.
        context: &'static str,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
}

#[cold]
#[inline(never)]
fn io(context: &'static str, source: std::io::Error) -> UpdateError {
    UpdateError::Io { context, source }
}

/// Verify a staged *opened handle* against trusted expected metadata and installed policy.
///
/// Reads from the start with one 64 KiB buffer. File metadata is compared before
/// and after scanning, but this is not a lock or immutable snapshot: a concurrent
/// writer can evade timestamp checks or modify bytes after return. Do not treat
/// this receipt as permission to install an archive, or as OS signature proof.
/// The same opened handle is rewound to offset 0 before it is returned.
pub fn verify_staged(
    file: File,
    expected: &ExpectedUpdate,
    policy: &InstalledUpdatePolicy,
) -> Result<StagedIntegrityReceipt, UpdateError> {
    verify_with_chunk(file, expected, policy, |_| {})
}

fn verify_with_chunk(
    file: File,
    expected: &ExpectedUpdate,
    policy: &InstalledUpdatePolicy,
    after_chunk: impl FnMut(u64),
) -> Result<StagedIntegrityReceipt, UpdateError> {
    let (file, byte_len, digest) = scan_with_chunk::<Sha256>(
        file,
        ExpectedFields {
            version: expected.version,
            app_id: &expected.app_id,
            target_triple: &expected.target_triple,
            byte_len: expected.byte_len,
        },
        DigestExpectation {
            bytes: &expected.sha256,
            mismatch: UpdateError::DigestMismatch,
        },
        policy,
        after_chunk,
    )?;
    Ok(StagedIntegrityReceipt {
        file,
        version: expected.version,
        app_id: expected.app_id.clone(),
        target_triple: expected.target_triple.clone(),
        byte_len,
        sha256: digest.into(),
    })
}

/// Verify SHA-512 bytes of an already-open file against independently authenticated
/// release metadata and an installed-host policy.
///
/// The receipt retains the same opened handle, rewound to offset 0. This does
/// not authenticate the release metadata, prove archive contents/signatures,
/// or install an update. Never use a hash computed from the downloaded file as
/// the expected hash: the caller must authenticate its target-bound provenance.
pub fn verify_staged_sha512(
    file: File,
    expected: &ExpectedUpdateSha512,
    policy: &InstalledUpdatePolicy,
) -> Result<StagedSha512IntegrityReceipt, UpdateError> {
    verify_sha512_with_chunk(file, expected, policy, |_| {})
}

fn verify_sha512_with_chunk(
    file: File,
    expected: &ExpectedUpdateSha512,
    policy: &InstalledUpdatePolicy,
    after_chunk: impl FnMut(u64),
) -> Result<StagedSha512IntegrityReceipt, UpdateError> {
    let (file, byte_len, digest) = scan_with_chunk::<Sha512>(
        file,
        ExpectedFields {
            version: expected.version,
            app_id: &expected.app_id,
            target_triple: &expected.target_triple,
            byte_len: expected.byte_len,
        },
        DigestExpectation {
            bytes: &expected.sha512,
            mismatch: UpdateError::Sha512Mismatch,
        },
        policy,
        after_chunk,
    )?;
    Ok(StagedSha512IntegrityReceipt {
        file,
        version: expected.version,
        app_id: expected.app_id.clone(),
        target_triple: expected.target_triple.clone(),
        byte_len,
        sha512: digest.into(),
    })
}

struct ExpectedFields<'a> {
    version: ReleaseVersion,
    app_id: &'a str,
    target_triple: &'a str,
    byte_len: u64,
}

struct DigestExpectation<'a> {
    bytes: &'a [u8],
    mismatch: UpdateError,
}

fn scan_with_chunk<D: Digest>(
    mut file: File,
    expected: ExpectedFields<'_>,
    digest_expectation: DigestExpectation<'_>,
    policy: &InstalledUpdatePolicy,
    mut after_chunk: impl FnMut(u64),
) -> Result<(File, u64, sha2::digest::Output<D>), UpdateError> {
    if expected.byte_len > policy.max_bytes {
        return Err(UpdateError::SizeLimitExceeded);
    }
    if expected.version <= policy.version {
        return Err(UpdateError::VersionNotNewer);
    }
    if expected.app_id != policy.app_id {
        return Err(UpdateError::AppIdMismatch);
    }
    if expected.target_triple != policy.target_triple {
        return Err(UpdateError::TargetMismatch);
    }
    if policy.required_trust != RequiredTrust::ReleaseIntegrity
        || matches!(
            policy.installed_trust,
            InstalledTrust::PublisherSigned | InstalledTrust::Notarized
        )
    {
        return Err(UpdateError::UnverifiableTrust);
    }

    let before = file
        .metadata()
        .map_err(|err| io("reading initial metadata", err))?;
    if !before.is_file() {
        return Err(UpdateError::NotRegularFile);
    }
    let initial_modified = before
        .modified()
        .map_err(|err| io("reading initial modification time", err))?;
    if before.len() != expected.byte_len {
        return Err(UpdateError::LengthMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|err| io("seeking staged handle", err))?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; SCAN_BYTES];
    let mut count = 0u64;
    while count < expected.byte_len {
        let remaining = expected.byte_len - count;
        let take = match usize::try_from(remaining) {
            Ok(value) => value.min(SCAN_BYTES),
            // On a narrow pointer width, a larger remaining count still
            // requires only one fixed-size chunk at a time.
            Err(_) => SCAN_BYTES,
        };
        let read = file
            .read(&mut buf[..take])
            .map_err(|err| io("reading staged bytes", err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        count += read as u64;
        after_chunk(count);
    }
    let mut extra = [0u8; 1];
    let has_extra = if count == expected.byte_len {
        file.read(&mut extra)
            .map_err(|err| io("checking staged EOF", err))?
            != 0
    } else {
        false
    };
    let after = file
        .metadata()
        .map_err(|err| io("reading final metadata", err))?;
    let final_modified = after
        .modified()
        .map_err(|err| io("reading final modification time", err))?;
    if !same_snapshot(&before, initial_modified, &after, final_modified) {
        return Err(UpdateError::FileChanged);
    }
    if count != expected.byte_len || has_extra {
        return Err(UpdateError::LengthMismatch);
    }
    let digest = hasher.finalize();
    let observed: &[u8] = digest.as_ref();
    if observed != digest_expectation.bytes {
        return Err(digest_expectation.mismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|err| io("rewinding verified staged handle", err))?;
    Ok((file, count, digest))
}

fn same_snapshot(
    before: &Metadata,
    initial: SystemTime,
    after: &Metadata,
    final_time: SystemTime,
) -> bool {
    before.is_file() && after.is_file() && before.len() == after.len() && initial == final_time
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, SystemTime};

    use tempfile::tempdir;

    const ID: &str = "com.example.desktop";
    const TARGET: &str = "aarch64-apple-darwin";

    fn version(text: &str) -> ReleaseVersion {
        ReleaseVersion::parse(text).unwrap()
    }

    fn expected(bytes: &[u8]) -> ExpectedUpdate {
        ExpectedUpdate::new(
            version("2.0.0"),
            ID,
            TARGET,
            bytes.len() as u64,
            Sha256::digest(bytes).into(),
        )
        .unwrap()
    }

    fn policy() -> InstalledUpdatePolicy {
        InstalledUpdatePolicy::new(
            version("1.0.0"),
            ID,
            TARGET,
            1024 * 1024,
            InstalledTrust::Unsigned,
        )
        .unwrap()
    }

    #[test]
    fn canonical_version_checked_and_ordered() {
        assert!(version("1.0.10") > version("1.0.9"));
        assert!(matches!(
            ReleaseVersion::parse(&"9".repeat(1024 * 1024)),
            Err(UpdateError::InvalidVersion)
        ));
        for invalid in [
            "",
            "1",
            "1.2",
            "01.2.3",
            "1.2.3-beta",
            "1.2.3.4",
            "1.2.-1",
            "18446744073709551616.0.0",
        ] {
            assert!(matches!(
                ReleaseVersion::parse(invalid),
                Err(UpdateError::InvalidVersion)
            ));
        }
    }

    #[test]
    fn small_staged_handle_checks_bytes_and_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact.zip");
        std::fs::write(&path, b"opaque bytes").unwrap();
        let receipt = verify_staged(
            File::open(&path).unwrap(),
            &expected(b"opaque bytes"),
            &policy(),
        )
        .unwrap();
        let mut retained = receipt.file();
        let mut bytes = Vec::new();
        retained.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"opaque bytes");
        assert_eq!(receipt.version(), version("2.0.0"));
        assert_eq!(receipt.app_id(), ID);
        assert_eq!(receipt.target_triple(), TARGET);
        assert_eq!(receipt.byte_len(), 12);
        let digest: [u8; 32] = Sha256::digest(b"opaque bytes").into();
        assert_eq!(receipt.sha256(), digest);
        assert_eq!(receipt.file().metadata().unwrap().len(), 12);

        let wrong_digest = ExpectedUpdate::new(version("2.0.0"), ID, TARGET, 12, [0; 32]).unwrap();
        assert!(matches!(
            verify_staged(File::open(&path).unwrap(), &wrong_digest, &policy()),
            Err(UpdateError::DigestMismatch)
        ));
        let wrong_size = ExpectedUpdate::new(version("2.0.0"), ID, TARGET, 13, [0; 32]).unwrap();
        assert!(matches!(
            verify_staged(File::open(&path).unwrap(), &wrong_size, &policy()),
            Err(UpdateError::LengthMismatch)
        ));
    }

    #[test]
    fn policy_rejects_empty_oversized_downgrade_identity_target_and_unproven_trust() {
        assert!(matches!(
            ExpectedUpdate::new(version("2.0.0"), ID, TARGET, 0, [0; 32]),
            Err(UpdateError::InvalidLength)
        ));
        assert!(matches!(
            InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 0, InstalledTrust::Unsigned),
            Err(UpdateError::InvalidLimit)
        ));
        assert!(matches!(
            ExpectedUpdate::new(version("2.0.0"), "invalid", TARGET, 1, [0; 32]),
            Err(UpdateError::InvalidAppId)
        ));
        assert!(matches!(
            ExpectedUpdate::new(version("2.0.0"), ID, "unknown", 1, [0; 32]),
            Err(UpdateError::InvalidTarget)
        ));

        let dir = tempdir().unwrap();
        let path = dir.path().join("payload");
        std::fs::write(&path, b"contents").unwrap();
        let check = |e: &ExpectedUpdate, p: &InstalledUpdatePolicy| {
            verify_staged(File::open(&path).unwrap(), e, p)
        };
        let oversized =
            InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 1, InstalledTrust::Unsigned)
                .unwrap();
        assert!(matches!(
            check(&expected(b"contents"), &oversized),
            Err(UpdateError::SizeLimitExceeded)
        ));
        for v in ["1.0.0", "0.9.9"] {
            let e = ExpectedUpdate::new(
                version(v),
                ID,
                TARGET,
                8,
                Sha256::digest(b"contents").into(),
            )
            .unwrap();
            assert!(matches!(
                check(&e, &policy()),
                Err(UpdateError::VersionNotNewer)
            ));
        }
        let app =
            ExpectedUpdate::new(version("2.0.0"), "com.other.desktop", TARGET, 8, [0; 32]).unwrap();
        assert!(matches!(
            check(&app, &policy()),
            Err(UpdateError::AppIdMismatch)
        ));
        for target in ["x86_64-apple-darwin", "aarch64-pc-windows-msvc"] {
            let arch = ExpectedUpdate::new(version("2.0.0"), ID, target, 8, [0; 32]).unwrap();
            assert!(matches!(
                check(&arch, &policy()),
                Err(UpdateError::TargetMismatch)
            ));
        }
        for trust in [InstalledTrust::PublisherSigned, InstalledTrust::Notarized] {
            let signed =
                InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 100, trust).unwrap();
            assert!(matches!(
                check(&expected(b"contents"), &signed),
                Err(UpdateError::UnverifiableTrust)
            ));
        }
        for requirement in [
            RequiredTrust::PublisherSignature,
            RequiredTrust::Notarization,
        ] {
            let p = policy()
                .require(requirement)
                .require(RequiredTrust::ReleaseIntegrity);
            assert!(matches!(
                check(&expected(b"contents"), &p),
                Err(UpdateError::UnverifiableTrust)
            ));
        }
        let ad_hoc =
            InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 100, InstalledTrust::AdHoc)
                .unwrap();
        assert!(check(&expected(b"contents"), &ad_hoc).is_ok());
    }

    #[test]
    fn path_swap_does_not_change_open_file_or_create_install_authority() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("staged");
        let replacement = dir.path().join("replacement");
        std::fs::write(&path, b"original").unwrap();
        std::fs::write(&replacement, b"malicious").unwrap();
        let open = File::open(&path).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let receipt = verify_staged(open, &expected(b"original"), &policy()).unwrap();
        let mut original_handle = receipt.file();
        let mut bytes = Vec::new();
        original_handle.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"original");
        assert_eq!(std::fs::read(&path).unwrap(), b"malicious");
        // The receipt is only a byte observation. It exposes neither an install
        // method nor an archive-identity / publisher-signature assertion.
    }

    #[test]
    fn same_handle_mutation_while_reading_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("staged");
        let bytes = vec![0x61; SCAN_BYTES * 3];
        std::fs::write(&path, &bytes).unwrap();
        let expected = expected(&bytes);
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = barrier.clone();
        let writer = std::thread::spawn(move || {
            let mut writer = OpenOptions::new().write(true).open(path).unwrap();
            writer_barrier.wait();
            // Change bytes already scanned, keeping the same length. The first
            // chunk's original hash alone cannot catch this in-place rewrite.
            writer.write_all(b"changed").unwrap();
            writer.flush().unwrap();
            writer
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000))
                .unwrap();
            writer_barrier.wait();
        });
        let result = verify_with_chunk(
            File::open(dir.path().join("staged")).unwrap(),
            &expected,
            &policy(),
            |count| {
                if count == SCAN_BYTES as u64 {
                    barrier.wait();
                    barrier.wait();
                }
            },
        );
        writer.join().unwrap();
        assert!(matches!(result, Err(UpdateError::FileChanged)));
    }

    fn expected_sha512(bytes: &[u8]) -> ExpectedUpdateSha512 {
        ExpectedUpdateSha512::new(
            version("2.0.0"),
            ID,
            TARGET,
            bytes.len() as u64,
            Sha512::digest(bytes).into(),
        )
        .unwrap()
    }

    #[test]
    fn sha512_receipt_retains_rewound_handle_and_sha512_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact.zip");
        std::fs::write(&path, b"opaque bytes").unwrap();
        let receipt = verify_staged_sha512(
            File::open(&path).unwrap(),
            &expected_sha512(b"opaque bytes"),
            &policy(),
        )
        .unwrap();
        assert_eq!(receipt.version(), version("2.0.0"));
        assert_eq!(receipt.app_id(), ID);
        assert_eq!(receipt.target_triple(), TARGET);
        assert_eq!(receipt.byte_len(), 12);
        let expected_digest: [u8; 64] = Sha512::digest(b"opaque bytes").into();
        assert_eq!(receipt.sha512(), expected_digest);
        let mut file = receipt.file();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"opaque bytes");
    }

    #[test]
    fn sha512_rejects_wrong_digest_size_identity_version_and_unproven_signature() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, b"contents").unwrap();
        let check = |expected: &ExpectedUpdateSha512, policy: &InstalledUpdatePolicy| {
            verify_staged_sha512(File::open(&path).unwrap(), expected, policy)
        };
        assert!(matches!(
            ExpectedUpdateSha512::new(version("2.0.0"), ID, TARGET, 0, [0; 64]),
            Err(UpdateError::InvalidLength)
        ));
        assert!(matches!(
            ExpectedUpdateSha512::new(version("2.0.0"), "invalid", TARGET, 8, [0; 64]),
            Err(UpdateError::InvalidAppId)
        ));
        let wrong_digest =
            ExpectedUpdateSha512::new(version("2.0.0"), ID, TARGET, 8, [0; 64]).unwrap();
        assert!(matches!(
            check(&wrong_digest, &policy()),
            Err(UpdateError::Sha512Mismatch)
        ));
        let wrong_size =
            ExpectedUpdateSha512::new(version("2.0.0"), ID, TARGET, 9, [0; 64]).unwrap();
        assert!(matches!(
            check(&wrong_size, &policy()),
            Err(UpdateError::LengthMismatch)
        ));
        let oversized =
            InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 1, InstalledTrust::Unsigned)
                .unwrap();
        assert!(matches!(
            check(&expected_sha512(b"contents"), &oversized),
            Err(UpdateError::SizeLimitExceeded)
        ));
        for old in ["1.0.0", "0.9.9"] {
            let e = ExpectedUpdateSha512::new(version(old), ID, TARGET, 8, [0; 64]).unwrap();
            assert!(matches!(
                check(&e, &policy()),
                Err(UpdateError::VersionNotNewer)
            ));
        }
        let app = ExpectedUpdateSha512::new(version("2.0.0"), "com.other.app", TARGET, 8, [0; 64])
            .unwrap();
        assert!(matches!(
            check(&app, &policy()),
            Err(UpdateError::AppIdMismatch)
        ));
        for target in ["x86_64-apple-darwin", "aarch64-pc-windows-msvc"] {
            let arch = ExpectedUpdateSha512::new(version("2.0.0"), ID, target, 8, [0; 64]).unwrap();
            assert!(matches!(
                check(&arch, &policy()),
                Err(UpdateError::TargetMismatch)
            ));
        }
        for trust in [InstalledTrust::PublisherSigned, InstalledTrust::Notarized] {
            let signed =
                InstalledUpdatePolicy::new(version("1.0.0"), ID, TARGET, 100, trust).unwrap();
            assert!(matches!(
                check(&expected_sha512(b"contents"), &signed),
                Err(UpdateError::UnverifiableTrust)
            ));
        }
        for required in [
            RequiredTrust::PublisherSignature,
            RequiredTrust::Notarization,
        ] {
            assert!(matches!(
                check(&expected_sha512(b"contents"), &policy().require(required)),
                Err(UpdateError::UnverifiableTrust)
            ));
        }
    }

    #[test]
    fn sha512_path_swap_keeps_the_original_opened_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("staged");
        let replacement = dir.path().join("replacement");
        std::fs::write(&path, b"original").unwrap();
        std::fs::write(&replacement, b"malicious").unwrap();
        let open = File::open(&path).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let receipt = verify_staged_sha512(open, &expected_sha512(b"original"), &policy()).unwrap();
        let mut file = receipt.file();
        let mut retained = Vec::new();
        file.read_to_end(&mut retained).unwrap();
        assert_eq!(retained, b"original");
        assert_eq!(std::fs::read(&path).unwrap(), b"malicious");
    }

    #[test]
    fn sha512_detects_in_place_mutation_after_first_chunk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("staged");
        let bytes = vec![0x61; SCAN_BYTES * 3];
        std::fs::write(&path, &bytes).unwrap();
        let expected = expected_sha512(&bytes);
        let barrier = Arc::new(Barrier::new(2));
        let writer_barrier = barrier.clone();
        let writer = std::thread::spawn(move || {
            let mut writer = OpenOptions::new().write(true).open(path).unwrap();
            writer_barrier.wait();
            writer.write_all(b"changed").unwrap();
            writer.flush().unwrap();
            writer
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000))
                .unwrap();
            writer_barrier.wait();
        });
        let result = verify_sha512_with_chunk(
            File::open(dir.path().join("staged")).unwrap(),
            &expected,
            &policy(),
            |count| {
                if count == SCAN_BYTES as u64 {
                    barrier.wait();
                    barrier.wait();
                }
            },
        );
        writer.join().unwrap();
        assert!(matches!(result, Err(UpdateError::FileChanged)));
    }
}
