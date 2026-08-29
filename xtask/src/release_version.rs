// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Release-version parsing shared by versioning, hotfix, and packaging tasks.

use std::fmt;

/// A WebUI release version.
///
/// Releases are either stable (`major.minor.patch`) or hotfix prereleases
/// (`major.minor.patch-hotfix.number`).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    hotfix: Option<u64>,
}

impl ReleaseVersion {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (core, hotfix) = match value.split_once('-') {
            Some((core, suffix)) => {
                let number = suffix.strip_prefix("hotfix.").and_then(parse_number)?;
                if number == 0 {
                    return None;
                }
                (core, Some(number))
            }
            None => (value, None),
        };

        let mut parts = core.split('.');
        let major = parts.next().and_then(parse_number)?;
        let minor = parts.next().and_then(parse_number)?;
        let patch = parts.next().and_then(parse_number)?;
        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            major,
            minor,
            patch,
            hotfix,
        })
    }

    pub(crate) fn parse_tag(value: &str) -> Option<Self> {
        value.strip_prefix('v').and_then(Self::parse)
    }

    pub(crate) fn is_stable(self) -> bool {
        self.hotfix.is_none()
    }

    pub(crate) fn hotfix_number(self) -> Option<u64> {
        self.hotfix
    }

    pub(crate) fn with_hotfix(self, number: u64) -> Option<Self> {
        if number == 0 {
            return None;
        }
        Some(Self {
            hotfix: Some(number),
            ..self
        })
    }

    pub(crate) fn same_base(self, other: Self) -> bool {
        self.major == other.major && self.minor == other.minor && self.patch == other.patch
    }

    pub(crate) fn python_version(self) -> String {
        match self.hotfix {
            Some(number) => format!("{}.{}.{}.post{number}", self.major, self.minor, self.patch),
            None => self.to_string(),
        }
    }

    pub(crate) fn python_cargo_version(self) -> String {
        match self.hotfix {
            Some(number) => format!("{}.{}.{}-post.{number}", self.major, self.minor, self.patch),
            None => self.to_string(),
        }
    }

    pub(crate) fn npm_dist_tag(self) -> String {
        match self.hotfix {
            Some(_) => format!("hotfix-{}.{}.{}", self.major, self.minor, self.patch),
            None => "latest".to_string(),
        }
    }
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(number) = self.hotfix {
            write!(formatter, "-hotfix.{number}")?;
        }
        Ok(())
    }
}

fn parse_number(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_and_hotfix_versions() {
        let stable = ReleaseVersion::parse("12.34.56");
        let hotfix = ReleaseVersion::parse("12.34.56-hotfix.7");

        assert!(stable.is_some_and(ReleaseVersion::is_stable));
        assert_eq!(hotfix.and_then(ReleaseVersion::hotfix_number), Some(7));
    }

    #[test]
    fn rejects_non_release_semver_forms() {
        for invalid in [
            "",
            "1.0",
            "1.0.0.0",
            "01.0.0",
            "1.00.0",
            "1.0.00",
            "1.0.0-hotfix.0",
            "1.0.0-hotfix.01",
            "1.0.0-alpha.1",
            "1.0.0+build.1",
            "+1.0.0",
            "1.+0.0",
            "1.0.+0",
            "1.0.0-hotfix.+1",
            "v1.0.0",
        ] {
            assert!(
                ReleaseVersion::parse(invalid).is_none(),
                "{invalid} should be rejected"
            );
        }
    }

    #[test]
    fn maps_hotfix_to_pep_440_post_release() {
        assert_eq!(
            ReleaseVersion::parse("1.2.3-hotfix.4").map(ReleaseVersion::python_version),
            Some("1.2.3.post4".to_string())
        );
        assert_eq!(
            ReleaseVersion::parse("1.2.3").map(ReleaseVersion::python_version),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            ReleaseVersion::parse("1.2.3-hotfix.4").map(ReleaseVersion::python_cargo_version),
            Some("1.2.3-post.4".to_string())
        );
    }

    #[test]
    fn maps_releases_to_safe_npm_dist_tags() {
        assert_eq!(
            ReleaseVersion::parse("1.2.3-hotfix.4").map(ReleaseVersion::npm_dist_tag),
            Some("hotfix-1.2.3".to_string())
        );
        assert_eq!(
            ReleaseVersion::parse("1.2.3").map(ReleaseVersion::npm_dist_tag),
            Some("latest".to_string())
        );
    }

    #[test]
    fn parses_prefixed_release_tags() {
        assert_eq!(
            ReleaseVersion::parse_tag("v1.2.3-hotfix.4").and_then(ReleaseVersion::hotfix_number),
            Some(4)
        );
        assert!(ReleaseVersion::parse_tag("1.2.3").is_none());
    }
}
