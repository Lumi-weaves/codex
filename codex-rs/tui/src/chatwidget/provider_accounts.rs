//! RichCodex provider-account controls reached from the full model picker.
//!
//! The provider plane exposes safe account metadata only. Secrets enter through
//! a masked prompt and immediately cross the app-server write boundary.

use super::*;
use crate::app_event::ProviderApiKey;
use codex_app_server_protocol::ProviderAccountCredentialKind;
use codex_app_server_protocol::ProviderAccountListResponse;
use codex_app_server_protocol::ProviderAccountLogin;
use codex_app_server_protocol::ProviderAccountLoginFailure;
use codex_app_server_protocol::ProviderAccountLoginStatus;
use codex_app_server_protocol::ProviderAccountStatus;

const PROVIDER_OAUTH_LOGIN_VIEW_ID: &str = "provider-oauth-login";

impl ChatWidget {
    pub(crate) fn show_provider_accounts(&mut self, response: ProviderAccountListResponse) {
        let mut items = Vec::with_capacity(response.data.len() + 2);
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
                tx.send(AppEvent::OpenProviderApiKeyLabelPrompt);
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        for account in response.data {
            let credential = match account.credential_kind {
                ProviderAccountCredentialKind::OAuth => "OpenAI OAuth",
                ProviderAccountCredentialKind::ApiKey => "OpenAI API key",
            };
            let status = match account.status {
                ProviderAccountStatus::Ready => "ready",
                ProviderAccountStatus::VerificationRequired => "verification pending",
                ProviderAccountStatus::ReauthenticationRequired => "sign-in required",
            };
            items.push(SelectionItem {
                name: account.user_label,
                description: Some(format!("{credential} · {status}")),
                is_disabled: true,
                disabled_reason: Some(
                    "Account details are read-only in this first provider-plane slice.".to_string(),
                ),
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

    pub(crate) fn open_provider_api_key_label_prompt(&mut self) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Add OpenAI API key — display name".to_string(),
            "Friendly account name".to_string(),
            "OpenAI API".to_string(),
            Some("The secret is entered on the next screen.".to_string()),
            Box::new(move |user_label| {
                tx.send(AppEvent::OpenProviderApiKeySecretPrompt { user_label });
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
                tx.send(AppEvent::SubmitProviderOAuthLogin { user_label });
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

    pub(crate) fn open_provider_api_key_secret_prompt(&mut self, user_label: String) {
        let tx = self.app_event_tx.clone();
        let context_label = Some(format!("Account label: {user_label}"));
        let view = CustomPromptView::new_secret(
            "Add OpenAI API key".to_string(),
            "Paste API key (input is hidden)".to_string(),
            context_label,
            Box::new(move |api_key| {
                tx.send(AppEvent::SubmitProviderApiKey {
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
                    "Added provider account: {} (OpenAI API key) · ready to attach to a model target",
                    response.account.user_label
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not add provider account: {error}"));
            }
        }
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
        Some(ProviderAccountLoginFailure::StoreUnavailable) => {
            "the RichCodex credential store is unavailable"
        }
        None => "unknown login failure",
    }
}
