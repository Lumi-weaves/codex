#[cfg(any(not(debug_assertions), test))]
use codex_install_context::DISTRIBUTION;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::Distribution;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::InstallContext;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::InstallMethod;
#[cfg(any(not(debug_assertions), test))]
use codex_install_context::StandalonePlatform;

/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Update via `npm install -g @openai/codex@latest`.
    NpmGlobalLatest,
    /// Update via `bun install -g @openai/codex@latest`.
    BunGlobalLatest,
    /// Update via `pnpm add -g @openai/codex@latest`.
    PnpmGlobalLatest,
    /// Update via `brew upgrade codex`.
    BrewUpgrade,
    /// Update via `curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh`.
    StandaloneUnix,
    /// Update via `$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex`.
    StandaloneWindows,
}

impl UpdateAction {
    #[cfg(any(not(debug_assertions), test))]
    pub(crate) fn from_install_context(context: &InstallContext) -> Option<Self> {
        Self::from_install_context_for_distribution(DISTRIBUTION, context)
    }

    /// Maps an install context to the official-channel update action for the
    /// given distribution.
    ///
    /// Lumi builds refuse every official update action: the distribution
    /// policy is fixed at compile time and is never user configurable.
    #[cfg(any(not(debug_assertions), test))]
    pub(crate) fn from_install_context_for_distribution(
        distribution: Distribution,
        context: &InstallContext,
    ) -> Option<Self> {
        if distribution.is_lumi() {
            return None;
        }
        match &context.method {
            InstallMethod::Npm => Some(UpdateAction::NpmGlobalLatest),
            InstallMethod::Bun => Some(UpdateAction::BunGlobalLatest),
            InstallMethod::Pnpm => Some(UpdateAction::PnpmGlobalLatest),
            InstallMethod::Brew => Some(UpdateAction::BrewUpgrade),
            InstallMethod::Standalone { platform, .. } => Some(match platform {
                StandalonePlatform::Unix => UpdateAction::StandaloneUnix,
                StandalonePlatform::Windows => UpdateAction::StandaloneWindows,
            }),
            InstallMethod::Other => None,
        }
    }

    /// Returns the list of command-line arguments for invoking the update.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        match self {
            UpdateAction::NpmGlobalLatest => ("npm", &["install", "-g", "@openai/codex"]),
            UpdateAction::BunGlobalLatest => ("bun", &["install", "-g", "@openai/codex"]),
            UpdateAction::PnpmGlobalLatest => ("pnpm", &["add", "-g", "@openai/codex"]),
            UpdateAction::BrewUpgrade => ("brew", &["upgrade", "--cask", "codex"]),
            UpdateAction::StandaloneUnix => (
                "sh",
                &[
                    "-c",
                    "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh",
                ],
            ),
            UpdateAction::StandaloneWindows => (
                "powershell",
                &[
                    "-ExecutionPolicy",
                    "Bypass",
                    "-c",
                    "$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex",
                ],
            ),
        }
    }

    /// Returns string representation of the command-line arguments for invoking the update.
    pub fn command_str(self) -> String {
        let (command, args) = self.command_args();
        shlex::try_join(std::iter::once(command).chain(args.iter().copied()))
            .unwrap_or_else(|_| format!("{command} {}", args.join(" ")))
    }
}

#[cfg(not(debug_assertions))]
pub fn get_update_action() -> Option<UpdateAction> {
    UpdateAction::from_install_context(InstallContext::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;

    #[test]
    fn maps_install_context_to_update_action() {
        let native_release_dir =
            AbsolutePathBuf::from_absolute_path(std::env::temp_dir().join("native-release"))
                .expect("temp dir path should be absolute");

        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Other,
                    package_layout: None,
                },
            ),
            None
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Npm,
                    package_layout: None,
                },
            ),
            Some(UpdateAction::NpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Bun,
                    package_layout: None,
                },
            ),
            Some(UpdateAction::BunGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Pnpm,
                    package_layout: None,
                },
            ),
            Some(UpdateAction::PnpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Brew,
                    package_layout: None,
                },
            ),
            Some(UpdateAction::BrewUpgrade)
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Standalone {
                        platform: StandalonePlatform::Unix,
                        release_dir: native_release_dir.clone(),
                        resources_dir: Some(native_release_dir.join("codex-resources")),
                    },
                    package_layout: None,
                },
            ),
            Some(UpdateAction::StandaloneUnix)
        );
        assert_eq!(
            UpdateAction::from_install_context_for_distribution(
                Distribution::Official,
                &InstallContext {
                    method: InstallMethod::Standalone {
                        platform: StandalonePlatform::Windows,
                        release_dir: native_release_dir.clone(),
                        resources_dir: Some(native_release_dir.join("codex-resources")),
                    },
                    package_layout: None,
                },
            ),
            Some(UpdateAction::StandaloneWindows)
        );
    }

    #[test]
    fn lumi_distribution_refuses_every_official_update_action() {
        let native_release_dir =
            AbsolutePathBuf::from_absolute_path(std::env::temp_dir().join("native-release"))
                .expect("temp dir path should be absolute");
        let contexts = [
            InstallContext {
                method: InstallMethod::Npm,
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Bun,
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Pnpm,
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Brew,
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Unix,
                    release_dir: native_release_dir.clone(),
                    resources_dir: None,
                },
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Windows,
                    release_dir: native_release_dir,
                    resources_dir: None,
                },
                package_layout: None,
            },
            InstallContext {
                method: InstallMethod::Other,
                package_layout: None,
            },
        ];

        for context in &contexts {
            assert_eq!(
                UpdateAction::from_install_context_for_distribution(Distribution::Lumi, context,),
                None,
                "Lumi builds must refuse the official update action for {:?}",
                context.method
            );
        }
    }

    #[test]
    fn lumi_builds_never_report_an_update_action() {
        let context = InstallContext {
            method: InstallMethod::Npm,
            package_layout: None,
        };
        assert_eq!(UpdateAction::from_install_context(&context), None);
    }

    #[test]
    fn standalone_update_commands_rerun_latest_installer() {
        assert_eq!(
            UpdateAction::StandaloneUnix.command_args(),
            (
                "sh",
                &[
                    "-c",
                    "curl -fsSL https://chatgpt.com/codex/install.sh | CODEX_NON_INTERACTIVE=1 sh"
                ][..],
            )
        );
        assert_eq!(
            UpdateAction::StandaloneWindows.command_args(),
            (
                "powershell",
                &[
                    "-ExecutionPolicy",
                    "Bypass",
                    "-c",
                    "$env:CODEX_NON_INTERACTIVE=1; irm https://chatgpt.com/codex/install.ps1 | iex"
                ][..],
            )
        );
    }
}
