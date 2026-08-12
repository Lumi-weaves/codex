use super::*;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::chatwidget::model_popups::ALL_MODELS_POPUP_VIEW_ID;
use codex_app_server_protocol::ModelWorkbenchPublicationStatus;
use codex_app_server_protocol::ModelWorkbenchRetireResponse;
use codex_app_server_protocol::ModelWorkbenchUpsertResponse;

impl ChatWidget {
    pub(super) fn handle_model_workbench_key(&mut self, key_event: KeyEvent) -> bool {
        if self.bottom_pane.active_view_id() != Some(ALL_MODELS_POPUP_VIEW_ID)
            || key_event.kind != KeyEventKind::Press
            || key_event.modifiers != KeyModifiers::NONE
        {
            return false;
        }
        let Some(model) = self.selected_model_workbench_preset() else {
            return false;
        };
        match key_event.code {
            KeyCode::Char('i') => {
                self.open_model_workbench_display_prompt(model.display_name, model.model);
                true
            }
            KeyCode::Char('d') => {
                self.open_model_workbench_retire_confirmation(model.display_name, model.model);
                true
            }
            _ => false,
        }
    }

    fn selected_model_workbench_preset(&self) -> Option<ModelPreset> {
        let selected = self
            .bottom_pane
            .selected_index_for_active_view(ALL_MODELS_POPUP_VIEW_ID)?;
        self.model_catalog
            .try_list_models()
            .ok()?
            .into_iter()
            .filter(|preset| preset.show_in_picker && !Self::is_auto_model(&preset.model))
            .nth(selected)
    }

    fn open_model_workbench_display_prompt(
        &mut self,
        initial_display_name: String,
        initial_model_tag: String,
    ) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            "Model display name".to_string(),
            "Type a friendly name and press Enter".to_string(),
            initial_display_name,
            Some(format!("Stable tag: {initial_model_tag}")),
            Box::new(move |display_name| {
                tx.send(AppEvent::OpenModelWorkbenchTagPrompt {
                    display_name,
                    initial_model_tag: initial_model_tag.clone(),
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_model_workbench_tag_prompt(
        &mut self,
        display_name: String,
        initial_model_tag: String,
    ) {
        let tx = self.app_event_tx.clone();
        let display_name_for_submit = display_name.clone();
        let view = CustomPromptView::new(
            "Stable model tag".to_string(),
            "Type an existing exact tag and press Enter".to_string(),
            initial_model_tag,
            Some(format!("Display name: {display_name}")),
            Box::new(move |model_tag| {
                tx.send(AppEvent::FetchModelWorkbenchUpsert {
                    display_name: display_name_for_submit.clone(),
                    model_tag,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    fn open_model_workbench_retire_confirmation(
        &mut self,
        display_name: String,
        model_tag: String,
    ) {
        let retire_tag = model_tag.clone();
        let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
            tx.send(AppEvent::FetchModelWorkbenchRetire {
                model_tag: retire_tag.clone(),
            });
        })];
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Retire {display_name}?")),
            subtitle: Some(format!(
                "Hide tag {model_tag} from future pickers. Routes, accounts, credentials, and running tasks are unchanged."
            )),
            footer_hint: Some(standard_popup_hint_line()),
            initial_selected_idx: Some(1),
            items: vec![
                SelectionItem {
                    name: "Retire display entry".to_string(),
                    actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Keep entry".to_string(),
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
    }

    pub(crate) fn on_model_workbench_upsert_loaded(
        &mut self,
        result: Result<ModelWorkbenchUpsertResponse, String>,
    ) {
        match result {
            Ok(response) => self.show_model_workbench_receipt(
                if response.changed {
                    "Saved"
                } else {
                    "Already current"
                },
                &response.entry.display_name,
                &response.entry.model_tag,
                response.publication.status,
            ),
            Err(error) => self.add_error_message(format!("Could not save model entry: {error}")),
        }
    }

    pub(crate) fn on_model_workbench_retire_loaded(
        &mut self,
        result: Result<ModelWorkbenchRetireResponse, String>,
    ) {
        match result {
            Ok(response) => self.show_model_workbench_receipt(
                if response.changed {
                    "Retired"
                } else {
                    "Already retired"
                },
                &response.entry.display_name,
                &response.entry.model_tag,
                response.publication.status,
            ),
            Err(error) => self.add_error_message(format!("Could not retire model entry: {error}")),
        }
    }

    fn show_model_workbench_receipt(
        &mut self,
        action: &str,
        display_name: &str,
        model_tag: &str,
        publication: ModelWorkbenchPublicationStatus,
    ) {
        let publication = match publication {
            ModelWorkbenchPublicationStatus::Synchronized => "catalog synchronized",
            ModelWorkbenchPublicationStatus::Pending => "saved; catalog publication pending",
            ModelWorkbenchPublicationStatus::Failed => "saved; catalog publication failed",
        };
        self.add_info_message(
            format!("{action}: {display_name} (tag: {model_tag}) · {publication}"),
            /*hint*/ None,
        );
    }
}
