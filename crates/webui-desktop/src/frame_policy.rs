// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::{Arc, Mutex, Weak};

use crate::{DesktopError, HostLifetime, Result};

#[cfg(not(target_os = "linux"))]
const MAX_FRAME_GRANTS: usize = 32;

fn valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// An exact, unprivileged HTTP origin under the reserved `.localhost` domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpFrameOrigin(String);

impl HttpFrameOrigin {
    /// Construct an origin from a canonical lowercase localhost subdomain and port.
    ///
    /// # Errors
    ///
    /// Rejects bare localhost, non-local domains, malformed labels, and port zero.
    pub fn from_localhost_subdomain(host: &str, port: u16) -> Result<Self> {
        let Some(prefix) = host.strip_suffix(".localhost") else {
            return Err(invalid_origin());
        };
        if prefix.is_empty() || host.len() > 253 || port == 0 || !prefix.split('.').all(valid_label)
        {
            return Err(invalid_origin());
        }
        Ok(Self(format!("http://{host}:{port}")))
    }

    /// Borrow the canonical scheme and authority, without a trailing slash.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(not(target_os = "linux"))]
    fn allows(&self, url: &str) -> bool {
        url.strip_prefix(&self.0).is_some_and(|path| {
            (path.is_empty() || path.starts_with('/'))
                && !url.contains('\\')
                && !url.bytes().any(|byte| byte.is_ascii_control())
        })
    }
}

#[cold]
#[inline(never)]
fn invalid_origin() -> DesktopError {
    DesktopError::UnsupportedRuntime {
        message: "unprivileged frame origin must be an exact lowercase HTTP .localhost subdomain with a nonzero port".to_string(),
        help: "Use the current preview lease hostname and bound port; do not grant a wildcard, remote host, or the main application origin".to_string(),
    }
}

struct Grants {
    entries: Vec<(HttpFrameOrigin, usize)>,
    closed: bool,
}

pub(crate) struct FramePolicy {
    lifetime: HostLifetime,
    grants: Mutex<Grants>,
}

impl FramePolicy {
    pub(crate) fn new(lifetime: HostLifetime) -> Arc<Self> {
        Arc::new(Self {
            lifetime,
            grants: Mutex::new(Grants {
                entries: Vec::new(),
                closed: false,
            }),
        })
    }

    pub(crate) fn handle(self: &Arc<Self>) -> FramePolicyHandle {
        FramePolicyHandle(Arc::downgrade(self))
    }

    #[cfg(not(target_os = "linux"))]
    pub(crate) fn allows(&self, url: &str) -> bool {
        if !self.lifetime.is_active() {
            return false;
        }
        match self.grants.lock() {
            Ok(grants) => {
                !grants.closed && grants.entries.iter().any(|(origin, _)| origin.allows(url))
            }
            Err(_) => {
                eprintln!("WebUI: frame policy lock is unavailable; denying subframe navigation");
                false
            }
        }
    }

    pub(crate) fn close(&self) {
        match self.grants.lock() {
            Ok(mut grants) => {
                grants.closed = true;
                grants.entries.clear();
            }
            Err(_) => {
                eprintln!("WebUI: frame policy lock is unavailable during frame close");
            }
        }
    }
}

/// Weak, host-owned handle for granting exact unprivileged subframe origins.
#[derive(Clone)]
pub struct FramePolicyHandle(Weak<FramePolicy>);

impl FramePolicyHandle {
    /// Allow one exact local preview origin until the returned grant is dropped.
    ///
    /// This never grants main-document navigation, native IPC, or window
    /// controls. The host must remove the iframe as it releases preview demand;
    /// dropping a grant does not undo a network request already admitted.
    ///
    /// # Errors
    ///
    /// Fails for a retired frame, exhausted grant capacity, or an unavailable
    /// policy lock.
    pub fn allow_unprivileged_origin(&self, origin: HttpFrameOrigin) -> Result<FrameGrant> {
        #[cfg(target_os = "linux")]
        {
            let _ = origin;
            if self
                .0
                .upgrade()
                .is_none_or(|policy| !policy.lifetime.is_active())
            {
                return Err(retired_policy());
            }
            Err(DesktopError::UnsupportedRuntime {
                message: "unprivileged frame navigation grants are unavailable on Linux"
                    .to_string(),
                help: "The Linux local-server frame currently denies subframe documents"
                    .to_string(),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let policy = self.0.upgrade().ok_or_else(retired_policy)?;
            if !policy.lifetime.is_active() {
                return Err(retired_policy());
            }
            let mut grants = policy.grants.lock().map_err(|_| retired_policy())?;
            if grants.closed {
                return Err(retired_policy());
            }
            if let Some((_, count)) = grants
                .entries
                .iter_mut()
                .find(|(entry, _)| *entry == origin)
            {
                *count = count.checked_add(1).ok_or_else(grant_capacity)?;
            } else {
                if grants.entries.len() >= MAX_FRAME_GRANTS {
                    return Err(grant_capacity());
                }
                grants.entries.push((origin.clone(), 1));
            }
            drop(grants);
            Ok(FrameGrant {
                policy: Arc::downgrade(&policy),
                origin,
            })
        }
    }
}

/// Scoped permission for an unprivileged subframe origin.
#[must_use = "keep the grant while the matching preview iframe is in use"]
pub struct FrameGrant {
    policy: Weak<FramePolicy>,
    origin: HttpFrameOrigin,
}

impl Drop for FrameGrant {
    fn drop(&mut self) {
        let Some(policy) = self.policy.upgrade() else {
            return;
        };
        let Ok(mut grants) = policy.grants.lock() else {
            eprintln!("WebUI: frame policy lock is unavailable during grant revocation");
            return;
        };
        if let Some(index) = grants
            .entries
            .iter()
            .position(|(origin, _)| *origin == self.origin)
        {
            if grants.entries[index].1 == 1 {
                grants.entries.swap_remove(index);
            } else {
                grants.entries[index].1 -= 1;
            }
        }
    }
}

#[cold]
#[inline(never)]
fn retired_policy() -> DesktopError {
    DesktopError::UnsupportedRuntime {
        message: "unprivileged frame grants are unavailable after host retirement".to_string(),
        help: "Create a new frame after verifying its replacement server".to_string(),
    }
}

#[cold]
#[inline(never)]
#[cfg(not(target_os = "linux"))]
fn grant_capacity() -> DesktopError {
    DesktopError::UnsupportedRuntime {
        message: "too many unprivileged frame grants".to_string(),
        help: "Release inactive preview grants before opening another frame origin".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(host: &str, port: u16) -> HttpFrameOrigin {
        match HttpFrameOrigin::from_localhost_subdomain(host, port) {
            Ok(origin) => origin,
            Err(error) => panic!("valid local frame origin: {error}"),
        }
    }

    #[test]
    fn validates_exact_localhost_subdomains() {
        assert_eq!(
            origin("p-lease.preview.localhost", 4312).as_str(),
            "http://p-lease.preview.localhost:4312"
        );
        for host in [
            "localhost",
            ".localhost",
            "p-.localhost",
            "-lease.localhost",
            "p-.preview.localhost",
            "p-lease.preview.localhost.evil.com",
            "P-lease.preview.localhost",
            "p_lease.preview.localhost",
            "p-lease..preview.localhost",
        ] {
            assert!(
                HttpFrameOrigin::from_localhost_subdomain(host, 4312).is_err(),
                "{host}"
            );
        }
        assert!(HttpFrameOrigin::from_localhost_subdomain("p-lease.preview.localhost", 0).is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn grant_is_exact_and_released_without_promoting_another_origin() {
        let (owner, lifetime) = HostLifetime::new();
        let policy = FramePolicy::new(lifetime);
        let handle = policy.handle();
        let preview = origin("p-alpha.preview.localhost", 4312);
        let grant = handle.allow_unprivileged_origin(preview.clone());
        assert!(grant.is_ok());
        let duplicate = handle.allow_unprivileged_origin(preview);
        assert!(duplicate.is_ok());
        assert!(policy.allows("http://p-alpha.preview.localhost:4312/deep?lease=alpha"));
        for forbidden in [
            "http://p-alpha.preview.localhost:4312.evil/deep",
            "http://p-alpha.preview.localhost:4313/deep",
            "http://p-beta.preview.localhost:4312/deep",
            "http://127.0.0.1:4312/deep",
            "https://p-alpha.preview.localhost:4312/deep",
            "http://p-alpha.preview.localhost:4312@evil.com/deep",
            "http://p-alpha.preview.localhost:4312\\evil",
        ] {
            assert!(!policy.allows(forbidden), "{forbidden}");
        }
        drop(grant);
        assert!(policy.allows("http://p-alpha.preview.localhost:4312/deep"));
        drop(duplicate);
        assert!(!policy.allows("http://p-alpha.preview.localhost:4312/deep"));
        drop(owner);
        assert!(handle
            .allow_unprivileged_origin(origin("p-beta.preview.localhost", 4312))
            .is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn capacity_and_frame_close_are_fail_closed() {
        let (_owner, lifetime) = HostLifetime::new();
        let policy = FramePolicy::new(lifetime);
        let handle = policy.handle();
        let grants: Vec<_> = (0..MAX_FRAME_GRANTS)
            .map(|index| {
                handle.allow_unprivileged_origin(origin(&format!("p-{index}.localhost"), 1))
            })
            .collect();
        assert!(grants.iter().all(Result::is_ok));
        assert!(handle
            .allow_unprivileged_origin(origin("p-extra.localhost", 1))
            .is_err());
        policy.close();
        assert!(!policy.allows("http://p-0.localhost:1/"));
        assert!(handle
            .allow_unprivileged_origin(origin("p-extra.localhost", 1))
            .is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_rejects_unavailable_subframe_grants() {
        let (_owner, lifetime) = HostLifetime::new();
        let policy = FramePolicy::new(lifetime);
        assert!(policy
            .handle()
            .allow_unprivileged_origin(origin("p-alpha.preview.localhost", 4312))
            .is_err());
    }
}
