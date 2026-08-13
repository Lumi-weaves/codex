//! Ordered RichCodex target editing reached from the full model picker.
//!
//! Public UI resolves opaque account handles into user-owned labels. Reordering and replacement
//! submit the route's complete target list so the bundled backend can apply one atomic revision.

use super::*;
use crate::app_event::ModelRouteTargetEditorState;
use codex_app_server_protocol::ModelRouteTarget;
use codex_app_server_protocol::ModelRouteTargetInput;
use codex_app_server_protocol::ModelRouteTargetStatus;
use codex_app_server_protocol::ProviderAccount;
use codex_app_server_protocol::ProviderAccountStatus;

impl ChatWidget {
    pub(crate) fn show_model_route_target_editor(&mut self, editor: ModelRouteTargetEditorState) {
        let mut items = Vec::with_capacity(editor.route.targets.len() + 1);
        let editor_for_add = editor.clone();
        items.push(SelectionItem {
            name: "Add target".to_string(),
            description: Some("Append another provider account/model binding".to_string()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenModelRouteTargetAccountChoices {
                    editor: editor_for_add.clone(),
                    replace_index: None,
                });
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        for (target_index, target) in editor.route.targets.iter().enumerate() {
            let account_label = editor
                .accounts
                .iter()
                .find(|account| account.id == target.account_id)
                .map(|account| account.user_label.as_str())
                .unwrap_or("Unavailable account");
            let status = match target.status {
                ModelRouteTargetStatus::Unverified => "unverified",
                ModelRouteTargetStatus::ReauthenticationRequired => "sign-in required",
            };
            let editor_for_action = editor.clone();
            items.push(SelectionItem {
                name: account_label.to_string(),
                description: Some(format!(
                    "priority {} · {} · {} · {status}",
                    target_index + 1,
                    target.provider_id,
                    target.upstream_model_id
                )),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenModelRouteTargetActions {
                        editor: editor_for_action.clone(),
                        target_index,
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Targets for {}", editor.route.display_name)),
            subtitle: Some(format!(
                "tag: {} · earlier targets are tried first",
                editor.route.model_tag
            )),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: false,
            ..Default::default()
        });
    }

    pub(crate) fn show_model_route_target_actions(
        &mut self,
        editor: ModelRouteTargetEditorState,
        target_index: usize,
    ) {
        let Some(target) = editor.route.targets.get(target_index) else {
            self.add_error_message(
                "Could not edit model target: target no longer exists.".to_string(),
            );
            return;
        };
        let target_label = editor
            .accounts
            .iter()
            .find(|account| account.id == target.account_id)
            .map(|account| account.user_label.as_str())
            .unwrap_or("Unavailable account");
        let mut items = Vec::with_capacity(4);

        if target_index > 0 {
            let editor_for_action = editor.clone();
            items.push(target_mutation_item(
                "Move earlier",
                "Raise this target's routing priority",
                editor_for_action,
                move |targets| targets.swap(target_index, target_index - 1),
            ));
        }
        if target_index + 1 < editor.route.targets.len() {
            let editor_for_action = editor.clone();
            items.push(target_mutation_item(
                "Move later",
                "Lower this target's routing priority",
                editor_for_action,
                move |targets| targets.swap(target_index, target_index + 1),
            ));
        }

        let editor_for_replace = editor.clone();
        items.push(SelectionItem {
            name: "Change account or model".to_string(),
            description: Some("Preserve this target's position and identity".to_string()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenModelRouteTargetAccountChoices {
                    editor: editor_for_replace.clone(),
                    replace_index: Some(target_index),
                });
            })],
            dismiss_on_select: true,
            ..Default::default()
        });

        if editor.route.targets.len() > 1 {
            let editor_for_remove = editor.clone();
            items.push(target_mutation_item(
                "Remove target",
                "Remove only this binding; keep the model tag and account",
                editor_for_remove,
                move |targets| {
                    targets.remove(target_index);
                },
            ));
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Edit target {}", target_index + 1)),
            subtitle: Some(format!(
                "{target_label} · {} · {}",
                target.provider_id, target.upstream_model_id
            )),
            footer_hint: Some(standard_popup_hint_line()),
            initial_selected_idx: Some(0),
            items,
            ..Default::default()
        });
    }

    pub(crate) fn show_model_route_target_account_choices(
        &mut self,
        editor: ModelRouteTargetEditorState,
        replace_index: Option<usize>,
    ) {
        if editor.accounts.is_empty() {
            self.add_error_message(
                "Could not edit model target: no RichCodex provider accounts are available."
                    .to_string(),
            );
            return;
        }

        let title = if replace_index.is_some() {
            "Choose replacement account"
        } else {
            "Choose account for new target"
        };
        let mut items = Vec::with_capacity(editor.accounts.len());
        for account in editor.accounts.iter().cloned() {
            let disabled = matches!(
                account.status,
                ProviderAccountStatus::ReauthenticationRequired
            );
            let status = match account.status {
                ProviderAccountStatus::Ready => "ready",
                ProviderAccountStatus::VerificationRequired => "verification pending",
                ProviderAccountStatus::ReauthenticationRequired => "sign-in required",
            };
            let editor_for_action = editor.clone();
            let account_for_action = account.clone();
            items.push(SelectionItem {
                name: account.user_label,
                description: Some(format!("{} · {status}", account.provider_id)),
                is_disabled: disabled,
                disabled_reason: disabled.then(|| {
                    "Reauthenticate this account before routing traffic to it.".to_string()
                }),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenModelRouteTargetUpstreamPrompt {
                        editor: editor_for_action.clone(),
                        replace_index,
                        account: account_for_action.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(title.to_string()),
            subtitle: Some(format!("Model tag: {}", editor.route.model_tag)),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            is_searchable: editor.accounts.len() > 8,
            search_placeholder: Some("Search provider accounts".to_string()),
            ..Default::default()
        });
    }

    pub(crate) fn open_model_route_target_upstream_prompt(
        &mut self,
        editor: ModelRouteTargetEditorState,
        replace_index: Option<usize>,
        account: ProviderAccount,
    ) {
        let initial_text = replace_index
            .and_then(|target_index| editor.route.targets.get(target_index))
            .map(|target| target.upstream_model_id.clone())
            .unwrap_or_else(|| editor.route.semantic_model.clone());
        let tx = self.app_event_tx.clone();
        let account_label = account.user_label.clone();
        let view = CustomPromptView::new(
            "Set upstream model ID".to_string(),
            "Provider-native model name".to_string(),
            initial_text,
            Some(format!(
                "{} · account: {account_label}",
                editor.route.model_tag
            )),
            Box::new(move |upstream_model_id| {
                let mut targets = route_target_inputs(&editor.route.targets);
                let replacement = ModelRouteTargetInput {
                    id: replace_index
                        .and_then(|target_index| targets.get(target_index))
                        .and_then(|target| target.id.clone()),
                    provider_id: account.provider_id.clone(),
                    account_id: account.id.clone(),
                    upstream_model_id,
                };
                if let Some(target_index) = replace_index {
                    if let Some(target) = targets.get_mut(target_index) {
                        *target = replacement;
                    } else {
                        return;
                    }
                } else {
                    targets.push(replacement);
                }
                tx.send(AppEvent::SubmitModelRouteTargets {
                    editor: editor.clone(),
                    targets,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn finish_model_route_targets(
        &mut self,
        result: Result<codex_app_server_protocol::ModelRouteSetTargetsResponse, String>,
    ) {
        match result {
            Ok(response) => self.add_info_message(
                format!(
                    "Updated targets: {} (tag: {}) · {} target(s) · catalog synchronized",
                    response.route.display_name,
                    response.route.model_tag,
                    response.route.targets.len()
                ),
                None,
            ),
            Err(error) => {
                self.add_error_message(format!("Could not update model targets: {error}"));
            }
        }
    }
}

fn route_target_inputs(targets: &[ModelRouteTarget]) -> Vec<ModelRouteTargetInput> {
    targets
        .iter()
        .map(|target| ModelRouteTargetInput {
            id: Some(target.id.clone()),
            provider_id: target.provider_id.clone(),
            account_id: target.account_id.clone(),
            upstream_model_id: target.upstream_model_id.clone(),
        })
        .collect()
}

fn target_mutation_item(
    name: &str,
    description: &str,
    editor: ModelRouteTargetEditorState,
    mutate: impl Fn(&mut Vec<ModelRouteTargetInput>) + Send + Sync + 'static,
) -> SelectionItem {
    SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        actions: vec![Box::new(move |tx| {
            let mut targets = route_target_inputs(&editor.route.targets);
            mutate(&mut targets);
            tx.send(AppEvent::SubmitModelRouteTargets {
                editor: editor.clone(),
                targets,
            });
        })],
        dismiss_on_select: true,
        ..Default::default()
    }
}
