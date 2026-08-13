//! RichCodex model-route controls hosted by the full model picker.
//!
//! The picker is the user-facing control plane: `i` creates a stable model tag bound to one
//! provider account, while `d` retires only that tag. Credentials stay behind opaque account IDs
//! in the bundled backend.

use super::model_popups::ALL_MODELS_POPUP_VIEW_ID;
use super::*;
use crate::app_event::ModelRouteAccountChoices;
use crate::app_event::ModelRouteDraft;
use codex_app_server_protocol::ProviderAccountStatus;

impl ChatWidget {
    pub(super) fn handle_model_route_key(&mut self, key_event: KeyEvent) -> bool {
        let action = match key_event {
            KeyEvent {
                code: KeyCode::Char('i'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                ..
            } => Some(true),
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                ..
            } => Some(false),
            _ => None,
        };
        let Some(is_create) = action else {
            return false;
        };
        let Some(selected_model) = self.selected_all_models_preset() else {
            return false;
        };

        self.bottom_pane
            .dismiss_active_view_if_id(ALL_MODELS_POPUP_VIEW_ID);
        if is_create {
            self.open_model_route_display_name_prompt(selected_model);
        } else {
            self.open_model_route_retire_confirmation(selected_model);
        }
        true
    }

    fn selected_all_models_preset(&self) -> Option<ModelPreset> {
        let selected_idx = self
            .bottom_pane
            .selected_index_for_active_view(ALL_MODELS_POPUP_VIEW_ID)?;
        self.model_catalog
            .try_list_models()
            .ok()?
            .into_iter()
            .filter(|preset| preset.show_in_picker && !Self::is_auto_model(&preset.model))
            .nth(selected_idx)
    }

    fn open_model_route_display_name_prompt(&mut self, selected_model: ModelPreset) {
        let tx = self.app_event_tx.clone();
        let initial_text = selected_model.display_name.clone();
        let context_label = Some(format!("Semantic model: {}", selected_model.model));
        let view = CustomPromptView::new(
            "Add RichCodex model route — display name".to_string(),
            "Friendly name shown in the model picker".to_string(),
            initial_text,
            context_label,
            Box::new(move |display_name| {
                tx.send(AppEvent::OpenModelRouteTagPrompt {
                    display_name,
                    selected_model: selected_model.clone(),
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_model_route_tag_prompt(
        &mut self,
        display_name: String,
        selected_model: ModelPreset,
    ) {
        let tx = self.app_event_tx.clone();
        let initial_text = selected_model.model.clone();
        let context_label = Some(format!("Display name: {display_name}"));
        let view = CustomPromptView::new(
            "Add RichCodex model route — stable tag".to_string(),
            "Stable tag used by conversations and subagents".to_string(),
            initial_text,
            context_label,
            Box::new(move |model_tag| {
                tx.send(AppEvent::BeginModelRouteCreate {
                    draft: ModelRouteDraft {
                        display_name: display_name.clone(),
                        model_tag,
                        selected_model: selected_model.clone(),
                    },
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn show_model_route_account_choices(
        &mut self,
        draft: ModelRouteDraft,
        choices: ModelRouteAccountChoices,
    ) {
        if choices.accounts.is_empty() {
            self.add_error_message(
                "Could not add model route: no RichCodex provider accounts are available."
                    .to_string(),
            );
            return;
        }

        let mut items = Vec::with_capacity(choices.accounts.len());
        for account in choices.accounts.iter().cloned() {
            let disabled = matches!(
                account.status,
                ProviderAccountStatus::ReauthenticationRequired
            );
            let status = match account.status {
                ProviderAccountStatus::Ready => "ready",
                ProviderAccountStatus::VerificationRequired => "verification pending",
                ProviderAccountStatus::ReauthenticationRequired => "sign-in required",
            };
            let draft = draft.clone();
            let choices = choices.clone();
            let account_for_action = account.clone();
            items.push(SelectionItem {
                name: account.user_label,
                description: Some(format!("{} · {status}", account.provider_id)),
                is_disabled: disabled,
                disabled_reason: disabled.then(|| {
                    "Reauthenticate this account before routing traffic to it.".to_string()
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::SubmitModelRouteCreate {
                        draft: draft.clone(),
                        choices: choices.clone(),
                        account: account_for_action.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Choose a provider account".to_string()),
            subtitle: Some(format!(
                "{} → {} · upstream model {}",
                draft.display_name, draft.model_tag, choices.upstream_model_id
            )),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: choices.accounts.len() > 8,
            search_placeholder: Some("Search provider accounts".to_string()),
            ..Default::default()
        });
    }

    fn open_model_route_retire_confirmation(&mut self, selected_model: ModelPreset) {
        let model_tag = selected_model.model;
        let retire_tag = model_tag.clone();
        let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
            tx.send(AppEvent::BeginModelRouteRetire {
                model_tag: retire_tag.clone(),
            });
        })];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Retire model route `{model_tag}`?")),
            subtitle: Some(
                "Only the tag is hidden; accounts, targets, credentials, and running tasks are preserved."
                    .to_string(),
            ),
            footer_hint: Some(standard_popup_hint_line()),
            initial_selected_idx: Some(1),
            items: vec![
                SelectionItem {
                    name: "Retire route".to_string(),
                    description: Some("Hide this managed model tag".to_string()),
                    actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Keep route".to_string(),
                    description: Some("Return without changing the model plane".to_string()),
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
    }

    pub(crate) fn finish_model_route_create(
        &mut self,
        result: Result<codex_app_server_protocol::ModelRouteCreateResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!(
                    "Added: {} (tag: {}) · catalog synchronized",
                    response.route.display_name, response.route.model_tag
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not add model route: {error}"));
            }
        }
    }

    pub(crate) fn finish_model_route_retire(
        &mut self,
        result: Result<codex_app_server_protocol::ModelRouteRetireResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!(
                    "Retired: {} (tag: {}) · catalog synchronized",
                    response.route.display_name, response.route.model_tag
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not retire model route: {error}"));
            }
        }
    }
}
