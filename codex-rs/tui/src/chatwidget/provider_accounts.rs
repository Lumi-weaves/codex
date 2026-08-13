//! RichCodex provider-account controls reached from the full model picker.
//!
//! The provider plane exposes safe account metadata only. Secrets enter through
//! a masked prompt and immediately cross the app-server write boundary.

use super::*;
use crate::app_event::ProviderApiKey;
use codex_app_server_protocol::ProviderAccountCredentialKind;
use codex_app_server_protocol::ProviderAccountListResponse;
use codex_app_server_protocol::ProviderAccountStatus;

impl ChatWidget {
    pub(crate) fn show_provider_accounts(&mut self, response: ProviderAccountListResponse) {
        let mut items = Vec::with_capacity(response.data.len() + 1);
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
