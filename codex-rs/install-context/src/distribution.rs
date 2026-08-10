//! Compile-time distribution identity for Codex builds.
//!
//! The distribution is a compile-time property of the build: it is not user
//! configurable and cannot be overridden through wrapper environment
//! variables. Every build from this repository is a Lumi-managed build, so
//! the hard safety boundary below applies to everything compiled here:
//! Lumi builds must never check for updates, update, or fetch announcements
//! through OpenAI's official channels.

/// Which distribution a Codex build belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Distribution {
    /// The upstream OpenAI-distributed build, updated through official
    /// npm/brew/standalone channels.
    Official,
    /// The Lumi fork build, distributed and updated outside OpenAI's
    /// official channels.
    Lumi,
}

impl Distribution {
    pub const fn is_lumi(self) -> bool {
        matches!(self, Self::Lumi)
    }

    pub const fn is_official(self) -> bool {
        matches!(self, Self::Official)
    }
}

/// The distribution of this build, fixed at compile time.
///
/// Builds from this fork are Lumi builds by construction. Keeping the policy
/// in a constant (rather than configuration or environment) means the safety
/// boundary cannot be accidentally disabled by users or wrappers.
pub const DISTRIBUTION: Distribution = Distribution::Lumi;

/// Returns the upstream-compatible release version for `version`, with the
/// Lumi fork prerelease intentionally stripped.
///
/// Upstream model/API surfaces expect the official release version (for
/// example `0.147.0`) and must not observe fork prereleases such as
/// `0.147.0-lumi.1`; stripping the prerelease keeps upstream compatibility
/// intact while the build retains its distinct internal identity. Versions
/// without the Lumi prerelease are returned unchanged.
pub fn upstream_compatible_version(version: &str) -> &str {
    version
        .split_once("-lumi.")
        .map_or(version, |(base, _prerelease)| base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_major_minor_patch(version: &str) -> Option<(u64, u64, u64)> {
        let mut parts = version.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next()?.parse::<u64>().ok()?;
        let patch = parts.next()?.parse::<u64>().ok()?;
        parts.next().is_none().then_some((major, minor, patch))
    }

    #[test]
    fn build_version_carries_lumi_prerelease_identity() {
        let version = env!("CARGO_PKG_VERSION");
        let (base, prerelease) = version
            .split_once('-')
            .unwrap_or_else(|| panic!("expected a prerelease in version {version:?}"));
        assert!(
            plain_major_minor_patch(base).is_some(),
            "version base should be plain x.y.z, got {base:?}"
        );
        assert!(
            prerelease.starts_with("lumi."),
            "fork prerelease should be lumi.*, got {prerelease:?}"
        );
    }

    #[test]
    fn build_version_strips_to_official_release_version() {
        let version = env!("CARGO_PKG_VERSION");
        let (base, _prerelease) = version
            .split_once('-')
            .unwrap_or_else(|| panic!("expected a prerelease in version {version:?}"));
        assert_eq!(upstream_compatible_version(version), base);
    }

    #[test]
    fn upstream_compatible_version_strips_lumi_prerelease() {
        assert_eq!(upstream_compatible_version("0.147.0-lumi.1"), "0.147.0");
        assert_eq!(upstream_compatible_version("0.148.0-lumi.2"), "0.148.0");
    }

    #[test]
    fn upstream_compatible_version_preserves_plain_versions() {
        assert_eq!(upstream_compatible_version("0.147.0"), "0.147.0");
        assert_eq!(
            upstream_compatible_version("0.147.0-alpha.1"),
            "0.147.0-alpha.1"
        );
    }

    #[test]
    fn distribution_helpers_classify_variants() {
        assert!(Distribution::Lumi.is_lumi());
        assert!(!Distribution::Lumi.is_official());
        assert!(Distribution::Official.is_official());
        assert!(!Distribution::Official.is_lumi());
    }
}
