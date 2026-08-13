//! RichCodex provider-account controls reached from the full model picker.
//!
//! The provider plane exposes safe account metadata only. Secrets enter through
//! a masked prompt and immediately cross the app-server write boundary.

use super::*;
use crate::app_event::ProviderApiKey;
use crate::app_event::ProviderApiKeyConfig;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountCredentialKind;
use codex_app_server_protocol::ProviderAccountListResponse;
use codex_app_server_protocol::ProviderAccountLogin;
use codex_app_server_protocol::ProviderAccountLoginFailure;
use codex_app_server_protocol::ProviderAccountLoginStatus;
use codex_app_server_protocol::ProviderAccountRemovalPreviewResponse;
use codex_app_server_protocol::ProviderAccountRemoveResponse;
use codex_app_server_protocol::ProviderAccountReplaceApiKeyResponse;
use codex_app_server_protocol::ProviderAccountStatus;

const PROVIDER_OAUTH_LOGIN_VIEW_ID: &str = "provider-oauth-login";

impl ChatWidget {
    pub(crate) fn show_provider_accounts(&mut self, response: ProviderAccountListResponse) {
        let mut items = Vec::with_capacity(response.data.len() + 3);
        items.push(SelectionItem {
            name: "Sign in with OpenAI".to_string(),
            description: Some("Add another ChatGPT/Codex account by device code".to_string()),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenProviderOAuthLabelPrompt);
            })],
            dismiss_on_select: true,
            ..Default::default()
        });
        items.push(SelectionItem {
            name: "Add OpenAI API key".to_string(),
            description: Some("Create a write-only provider account".to_string()),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenProviderApiKeyLabelPrompt {
                    config: openai_api_key_config(),
                });
            })],
            dismiss_on_select: true,
            ..Default::default()
        });
        items.push(SelectionItem {
            name: "Add compatible API provider".to_string(),
            description: Some(
                "Configure an HTTPS OpenAI-compatible Responses endpoint".to_string(),
            ),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::OpenCompatibleProviderIdPrompt);
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        for account in response.data {
            let provider_name = response
                .providers
                .iter()
                .find(|provider| provider.id == account.provider_id)
                .map(|provider| provider.display_name.as_str())
                .unwrap_or(account.provider_id.as_str());
            let credential = match account.credential_kind {
                ProviderAccountCredentialKind::OAuth => "OpenAI OAuth".to_string(),
                ProviderAccountCredentialKind::ApiKey => format!("{provider_name} API key"),
            };
            let status = match account.status {
                ProviderAccountStatus::Ready => "ready",
                ProviderAccountStatus::VerificationRequired => "verification pending",
                ProviderAccountStatus::ReauthenticationRequired => "sign-in required",
            };
            let action_account = account.clone();
            let expected_revision = response.desired_state_revision.clone();
            items.push(SelectionItem {
                name: account.user_label,
                description: Some(format!("{credential} · {status}")),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenProviderAccountActions {
                        account: action_account.clone(),
                        expected_revision: expected_revision.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Provider accounts".to_string()),
            subtitle: Some(
                "Credentials stay in the bundled backend; model routes use opaque account handles."
                    .to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: false,
            ..Default::default()
        });
    }

    pub(crate) fn open_provider_account_actions(
        &mut self,
        account: ProviderAccount,
        expected_revision: String,
    ) {
        let reauth_account = account.clone();
        let reauth_revision = expected_revision;
        let remove_account_id = account.id.clone();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(account.user_label.clone()),
            subtitle: Some(format!("{} · opaque account handle", account.provider_id)),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: "Reauthenticate".to_string(),
                    description: Some(
                        "Replace credentials without changing model targets".to_string(),
                    ),
                    actions: vec![Box::new(move |tx| match reauth_account.credential_kind {
                        ProviderAccountCredentialKind::OAuth => {
                            tx.send(AppEvent::SubmitProviderOAuthLogin {
                                user_label: reauth_account.user_label.clone(),
                                account_id: Some(reauth_account.id.clone()),
                            })
                        }
                        ProviderAccountCredentialKind::ApiKey => {
                            tx.send(AppEvent::OpenProviderApiKeyReplacementPrompt {
                                account: reauth_account.clone(),
                                expected_revision: reauth_revision.clone(),
                            })
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Remove account".to_string(),
                    description: Some("Preview every affected model target first".to_string()),
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::PreviewProviderAccountRemoval {
                            account_id: remove_account_id.clone(),
                        });
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            is_searchable: false,
            ..Default::default()
        });
    }

    pub(crate) fn open_provider_api_key_replacement_prompt(
        &mut self,
        account: ProviderAccount,
        expected_revision: String,
    ) {
        let tx = self.app_event_tx.clone();
        let account_id = account.id;
        let view = CustomPromptView::new_secret(
            format!("Reauthenticate {}", account.user_label),
            "Paste replacement API key (input is hidden)".to_string(),
            Some("The opaque account handle and every model target stay unchanged.".to_string()),
            Box::new(move |api_key| {
                tx.send(AppEvent::SubmitProviderApiKeyReplacement {
                    account_id: account_id.clone(),
                    expected_revision: expected_revision.clone(),
                    api_key: ProviderApiKey::new(api_key),
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn finish_provider_api_key_replacement(
        &mut self,
        result: Result<ProviderAccountReplaceApiKeyResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!(
                    "Reauthenticated provider account: {} · model targets unchanged",
                    response.account.user_label
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not replace provider API key: {error}"))
            }
        }
    }

    pub(crate) fn show_provider_account_removal_preview(
        &mut self,
        result: Result<ProviderAccountRemovalPreviewResponse, String>,
    ) {
        let preview = match result {
            Ok(preview) => preview,
            Err(error) => {
                self.add_error_message(format!(
                    "Could not preview provider account removal: {error}"
                ));
                return;
            }
        };
        if !preview.can_remove {
            let affected = preview
                .affected_targets
                .iter()
                .map(|target| format!("{} → {}", target.display_name, target.upstream_model_id))
                .collect::<Vec<_>>()
                .join(", ");
            self.add_error_message(format!(
                "Cannot remove {}: update these model targets first: {affected}",
                preview.account.user_label,
            ));
            return;
        }
        let account_id = preview.account.id.clone();
        let expected_revision = preview.desired_state_revision.clone();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Remove {}?", preview.account.user_label)),
            subtitle: Some(
                "No model targets reference this account. Credentials will be deleted.".to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![SelectionItem {
                name: "Remove account".to_string(),
                description: Some("This cannot be undone".to_string()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::SubmitProviderAccountRemoval {
                        account_id: account_id.clone(),
                        expected_revision: expected_revision.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            }],
            is_searchable: false,
            ..Default::default()
        });
    }

    pub(crate) fn finish_provider_account_removal(
        &mut self,
        result: Result<ProviderAccountRemoveResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!("Removed provider account: {}", response.account.user_label),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not remove provider account: {error}"))
            }
        }
    }

    pub(crate) fn open_compatible_provider_id_prompt(&mut self) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Add compatible provider — ID".to_string(),
            "Stable lowercase ID (letters, digits, '.', '_', '-')".to_string(),
            String::new(),
            Some("Targets bind to this identifier; choose it once.".to_string()),
            Box::new(move |provider_id| {
                tx.send(AppEvent::OpenCompatibleProviderDisplayNamePrompt { provider_id });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_compatible_provider_display_name_prompt(&mut self, provider_id: String) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Add compatible provider — display name".to_string(),
            "Friendly provider name".to_string(),
            String::new(),
            Some(format!("Provider ID: {provider_id}")),
            Box::new(move |provider_display_name| {
                tx.send(AppEvent::OpenCompatibleProviderBaseUrlPrompt {
                    provider_id: provider_id.clone(),
                    provider_display_name,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_compatible_provider_base_url_prompt(
        &mut self,
        provider_id: String,
        provider_display_name: String,
    ) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Add compatible provider — API base URL".to_string(),
            "HTTPS base without /responses".to_string(),
            "https://example.com/v1".to_string(),
            Some(format!("{provider_display_name} · ID: {provider_id}")),
            Box::new(move |api_base_url| {
                tx.send(AppEvent::OpenProviderApiKeyLabelPrompt {
                    config: ProviderApiKeyConfig {
                        provider_id: provider_id.clone(),
                        provider_display_name: provider_display_name.clone(),
                        api_base_url,
                    },
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_provider_api_key_label_prompt(&mut self, config: ProviderApiKeyConfig) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            format!(
                "Add {} API key — account name",
                config.provider_display_name
            ),
            "Friendly account name".to_string(),
            format!("{} API", config.provider_display_name),
            Some("The secret is entered on the next screen.".to_string()),
            Box::new(move |user_label| {
                tx.send(AppEvent::OpenProviderApiKeySecretPrompt {
                    config: config.clone(),
                    user_label,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_provider_oauth_label_prompt(&mut self) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Sign in with OpenAI — display name".to_string(),
            "Friendly account name".to_string(),
            "OpenAI Codex".to_string(),
            Some("Device authorization opens on the next screen.".to_string()),
            Box::new(move |user_label| {
                tx.send(AppEvent::SubmitProviderOAuthLogin {
                    user_label,
                    account_id: None,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn show_provider_oauth_login(
        &mut self,
        result: Result<ProviderAccountLogin, String>,
    ) {
        let login = match result {
            Ok(login) => login,
            Err(error) => {
                self.add_error_message(format!("Could not start OpenAI login: {error}"));
                return;
            }
        };
        let (Some(verification_url), Some(user_code)) = (
            login.verification_url.as_deref(),
            login.user_code.as_deref(),
        ) else {
            self.add_error_message(
                "Could not start OpenAI login: backend returned no device code.".to_string(),
            );
            return;
        };
        let login_id = login.login_id.clone();
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(PROVIDER_OAUTH_LOGIN_VIEW_ID),
            title: Some("Sign in with OpenAI".to_string()),
            subtitle: Some(format!(
                "Open {verification_url} and enter code {user_code}; RichCodex is waiting in the background."
            )),
            footer_hint: Some(standard_popup_hint_line()),
            initial_selected_idx: Some(0),
            items: vec![SelectionItem {
                name: "Cancel login".to_string(),
                description: Some("Stop this device authorization attempt".to_string()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::CancelProviderOAuthLogin {
                        login_id: login_id.clone(),
                    });
                })],
                dismiss_on_select: false,
                ..Default::default()
            }],
            ..Default::default()
        });
    }

    pub(crate) fn finish_provider_oauth_login(
        &mut self,
        result: Result<ProviderAccountLogin, String>,
    ) {
        self.bottom_pane
            .dismiss_active_view_if_id(PROVIDER_OAUTH_LOGIN_VIEW_ID);
        match result {
            Ok(login) => match login.status {
                ProviderAccountLoginStatus::Completed => {
                    let Some(account) = login.account else {
                        self.add_error_message(
                            "OpenAI login completed without an account receipt.".to_string(),
                        );
                        return;
                    };
                    self.add_info_message(
                        format!(
                            "Added provider account: {} (OpenAI OAuth) · ready to attach to a model target",
                            account.user_label
                        ),
                        None,
                    );
                }
                ProviderAccountLoginStatus::Cancelled => {
                    self.add_info_message("OpenAI provider login cancelled.".to_string(), None)
                }
                ProviderAccountLoginStatus::Failed => self.add_error_message(format!(
                    "OpenAI provider login failed: {}.",
                    provider_login_failure_label(login.failure)
                )),
                ProviderAccountLoginStatus::AwaitingUser
                | ProviderAccountLoginStatus::Exchanging => self.add_error_message(
                    "OpenAI provider login ended in a non-terminal state.".to_string(),
                ),
            },
            Err(error) => {
                self.add_error_message(format!("Could not monitor OpenAI login: {error}"));
            }
        }
    }

    pub(crate) fn open_provider_api_key_secret_prompt(
        &mut self,
        config: ProviderApiKeyConfig,
        user_label: String,
    ) {
        let tx = self.app_event_tx.clone();
        let context_label = Some(format!("Account label: {user_label}"));
        let view = CustomPromptView::new_secret(
            format!("Add {} API key", config.provider_display_name),
            "Paste API key (input is hidden)".to_string(),
            context_label,
            Box::new(move |api_key| {
                tx.send(AppEvent::SubmitProviderApiKey {
                    config: config.clone(),
                    user_label: user_label.clone(),
                    api_key: ProviderApiKey::new(api_key),
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn finish_provider_api_key_add(
        &mut self,
        result: Result<codex_app_server_protocol::ProviderAccountAddApiKeyResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!(
                    "Added provider account: {} ({} API key) · ready to attach to a model target",
                    response.account.user_label, response.account.provider_id
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not add provider account: {error}"));
            }
        }
    }
}

fn openai_api_key_config() -> ProviderApiKeyConfig {
    ProviderApiKeyConfig {
        provider_id: "openai".to_string(),
        provider_display_name: "OpenAI".to_string(),
        api_base_url: "https://api.openai.com/v1".to_string(),
    }
}

fn provider_login_failure_label(failure: Option<ProviderAccountLoginFailure>) -> &'static str {
    match failure {
        Some(ProviderAccountLoginFailure::Expired) => "device code expired",
        Some(ProviderAccountLoginFailure::Unavailable) => "OpenAI login is unavailable",
        Some(ProviderAccountLoginFailure::InvalidCredential) => {
            "OpenAI returned an invalid credential"
        }
        Some(ProviderAccountLoginFailure::AccountAlreadyExists) => {
            "that OpenAI account is already configured"
        }
        Some(ProviderAccountLoginFailure::AccountLimitReached) => "provider account limit reached",
        Some(ProviderAccountLoginFailure::AccountNotFound) => "provider account no longer exists",
        Some(ProviderAccountLoginFailure::CredentialKindMismatch) => {
            "provider account cannot use OpenAI device login"
        }
        Some(ProviderAccountLoginFailure::AccountIdentityMismatch) => {
            "signed-in OpenAI account does not match this account"
        }
        Some(ProviderAccountLoginFailure::StoreUnavailable) => {
            "the RichCodex credential store is unavailable"
        }
        None => "unknown login failure",
    }
}
