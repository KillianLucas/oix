//! Model, collaboration, and reasoning popups for `ChatWidget`.
//!
//! These surfaces are tightly related because changing one often redirects
//! into another, especially while Plan mode is active.

use super::*;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::bundled_provider_catalog_entry;
use codex_model_provider_info::default_harness_for_provider_model;

impl ChatWidget {
    /// Open a popup to choose a quick auto model. Selecting "All models"
    /// opens the full picker with every available preset.
    pub(crate) fn open_model_popup(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Model selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        self.open_model_provider_popup();
    }

    pub(crate) fn open_current_harness_popup(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Harness selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        self.open_harness_popup(
            self.current_model().to_string(),
            self.effective_reasoning_effort(),
        );
    }

    fn open_model_provider_popup(&mut self) {
        let mut providers: Vec<_> = self
            .config
            .model_providers
            .iter()
            .map(|(provider_id, provider)| (provider_id.clone(), provider.clone()))
            .collect();
        providers.sort_by(|left, right| {
            let left_current = left.0 == self.config.model_provider_id;
            let right_current = right.0 == self.config.model_provider_id;
            right_current
                .cmp(&left_current)
                .then_with(|| {
                    provider_sort_priority(left.0.as_str())
                        .cmp(&provider_sort_priority(right.0.as_str()))
                })
                .then_with(|| {
                    left.1
                        .name
                        .to_ascii_lowercase()
                        .cmp(&right.1.name.to_ascii_lowercase())
                })
        });

        let items: Vec<SelectionItem> = providers
            .into_iter()
            .map(|(provider_id, provider)| {
                let provider_name = provider.name.clone();
                let description = provider_description(provider_id.as_str(), &provider);
                let search_value = Some(format!("{provider_id} {provider_name} {description}"));
                SelectionItem {
                    name: provider_name.clone(),
                    description: Some(description),
                    is_current: provider_id == self.config.model_provider_id,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::LoadProviderModels {
                            provider_id: provider_id.clone(),
                            provider_name: provider_name.clone(),
                        });
                    })],
                    dismiss_on_select: true,
                    search_value,
                    ..Default::default()
                }
            })
            .collect();

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some("Select Provider".to_string()),
            subtitle: Some("Choose a provider for the next chat.".to_string()),
            footer_hint: Some("Type to filter • Enter to continue • Esc to dismiss".into()),
            items,
            is_searchable: true,
            search_placeholder: Some("Filter providers".to_string()),
            ..Default::default()
        });
    }

    fn model_menu_header(&self, title: &str, subtitle: &str) -> Box<dyn Renderable> {
        let title = title.to_string();
        let subtitle = subtitle.to_string();
        let mut header = ColumnRenderable::new();
        header.push(Line::from(title.bold()));
        header.push(Line::from(subtitle.dim()));
        if let Some(warning) = self.model_menu_warning_line() {
            header.push(warning);
        }
        Box::new(header)
    }

    fn model_menu_warning_line(&self) -> Option<Line<'static>> {
        let base_url = self.custom_openai_base_url()?;
        let warning = format!(
            "Warning: OpenAI base URL is overridden to {base_url}. Selecting models may not be supported or work properly."
        );
        Some(Line::from(warning.red()))
    }

    fn custom_openai_base_url(&self) -> Option<String> {
        if !self.config.model_provider.is_openai() {
            return None;
        }

        let base_url = self.config.model_provider.base_url.as_ref()?;
        let trimmed = base_url.trim();
        if trimmed.is_empty() {
            return None;
        }

        let normalized = trimmed.trim_end_matches('/');
        if normalized == DEFAULT_OPENAI_BASE_URL {
            return None;
        }

        Some(trimmed.to_string())
    }

    #[allow(dead_code)]
    pub(crate) fn open_model_popup_with_presets(&mut self, presets: Vec<ModelPreset>) {
        let presets: Vec<ModelPreset> = presets
            .into_iter()
            .filter(|preset| preset.show_in_picker)
            .collect();

        let current_model = self.current_model();
        let current_label = presets
            .iter()
            .find(|preset| preset.model.as_str() == current_model)
            .map(|preset| preset.model.to_string())
            .unwrap_or_else(|| self.model_display_name().to_string());

        let (mut auto_presets, other_presets): (Vec<ModelPreset>, Vec<ModelPreset>) = presets
            .into_iter()
            .partition(|preset| Self::is_auto_model(&preset.model));

        if auto_presets.is_empty() {
            self.open_all_models_popup(other_presets);
            return;
        }

        auto_presets.sort_by_key(|preset| Self::auto_model_order(&preset.model));
        let mut items: Vec<SelectionItem> = auto_presets
            .into_iter()
            .map(|preset| {
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.clone());
                let model = preset.model.clone();
                let should_prompt_plan_mode_scope = self.should_prompt_plan_mode_reasoning_scope(
                    model.as_str(),
                    Some(preset.default_reasoning_effort),
                );
                let actions = Self::model_selection_actions(
                    model.clone(),
                    Some(preset.default_reasoning_effort),
                    should_prompt_plan_mode_scope,
                );
                SelectionItem {
                    name: model.clone(),
                    description,
                    is_current: model.as_str() == current_model,
                    is_default: preset.is_default,
                    actions,
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();

        if !other_presets.is_empty() {
            let all_models = other_presets;
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenAllModelsPopup {
                    models: all_models.clone(),
                });
            })];

            let is_current = !items.iter().any(|item| item.is_current);
            let description = Some(format!(
                "Choose a specific model and reasoning level (current: {current_label})"
            ));

            items.push(SelectionItem {
                name: "All models".to_string(),
                description,
                is_current,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            "Select Model",
            "Pick a quick auto mode or browse all models.",
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line()),
            items,
            header,
            ..Default::default()
        });
    }

    #[allow(dead_code)]
    fn is_auto_model(model: &str) -> bool {
        model.starts_with("codex-auto-")
    }

    #[allow(dead_code)]
    fn auto_model_order(model: &str) -> usize {
        match model {
            "codex-auto-fast" => 0,
            "codex-auto-balanced" => 1,
            "codex-auto-thorough" => 2,
            _ => 3,
        }
    }

    pub(crate) fn open_all_models_popup(&mut self, presets: Vec<ModelPreset>) {
        if presets.is_empty() {
            self.add_info_message(
                "No additional models are available right now.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        let mut items: Vec<SelectionItem> = Vec::new();
        for preset in presets.into_iter() {
            let description =
                (!preset.description.is_empty()).then_some(preset.description.to_string());
            let is_current = preset.model.as_str() == self.current_model();
            let single_supported_effort = preset.supported_reasoning_efforts.len() == 1;
            let preset_for_action = preset.clone();
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                let preset_for_event = preset_for_action.clone();
                tx.send(AppEvent::OpenReasoningPopup {
                    model: preset_for_event,
                });
            })];
            items.push(SelectionItem {
                name: preset.model.clone(),
                description,
                is_current,
                is_default: preset.is_default,
                actions,
                dismiss_on_select: single_supported_effort,
                dismiss_parent_on_child_accept: !single_supported_effort,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            "Select Model and Effort",
            "Access legacy models by running codex -m <model_name> or in your config.toml",
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(self.bottom_pane.standard_popup_hint_line()),
            items,
            header,
            ..Default::default()
        });
    }

    #[allow(dead_code)]
    fn model_selection_actions(
        model_for_action: String,
        effort_for_action: Option<ReasoningEffortConfig>,
        should_prompt_plan_mode_scope: bool,
    ) -> Vec<SelectionAction> {
        vec![Box::new(move |tx| {
            if should_prompt_plan_mode_scope {
                tx.send(AppEvent::OpenPlanReasoningScopePrompt {
                    model: model_for_action.clone(),
                    effort: effort_for_action,
                });
                return;
            }

            tx.send(AppEvent::OpenHarnessPopup {
                model: model_for_action.clone(),
                effort: effort_for_action,
            });
        })]
    }

    fn should_prompt_plan_mode_reasoning_scope(
        &self,
        selected_model: &str,
        selected_effort: Option<ReasoningEffortConfig>,
    ) -> bool {
        if !self.collaboration_modes_enabled()
            || self.active_mode_kind() != ModeKind::Plan
            || selected_model != self.current_model()
        {
            return false;
        }

        // Prompt whenever the selection is not a true no-op for both:
        // 1) the active Plan-mode effective reasoning, and
        // 2) the stored global defaults that would be updated by the fallback path.
        selected_effort != self.effective_reasoning_effort()
            || selected_model != self.current_collaboration_mode.model()
            || selected_effort != self.current_collaboration_mode.reasoning_effort()
    }

    pub(crate) fn open_plan_reasoning_scope_prompt(
        &mut self,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) {
        let reasoning_phrase = match effort {
            Some(ReasoningEffortConfig::None) => "no reasoning".to_string(),
            Some(selected_effort) => {
                format!(
                    "{} reasoning",
                    Self::reasoning_effort_label(selected_effort).to_lowercase()
                )
            }
            None => "the selected reasoning".to_string(),
        };
        let plan_only_description = format!("Always use {reasoning_phrase} in Plan mode.");
        let plan_reasoning_source = if let Some(plan_override) =
            self.config.plan_mode_reasoning_effort
        {
            format!(
                "user-chosen Plan override ({})",
                Self::reasoning_effort_label(plan_override).to_lowercase()
            )
        } else if let Some(plan_mask) = collaboration_modes::plan_mask(self.model_catalog.as_ref())
        {
            match plan_mask.reasoning_effort.flatten() {
                Some(plan_effort) => format!(
                    "built-in Plan default ({})",
                    Self::reasoning_effort_label(plan_effort).to_lowercase()
                ),
                None => "built-in Plan default (no reasoning)".to_string(),
            }
        } else {
            "built-in Plan default".to_string()
        };
        let all_modes_description = format!(
            "Set the global default reasoning level and the Plan mode override. This replaces the current {plan_reasoning_source}."
        );
        let subtitle = format!("Choose where to apply {reasoning_phrase}.");

        let plan_only_actions: Vec<SelectionAction> = vec![Box::new({
            let model = model.clone();
            move |tx| {
                tx.send(AppEvent::UpdateModel(model.clone()));
                tx.send(AppEvent::UpdatePlanModeReasoningEffort(effort));
                tx.send(AppEvent::PersistPlanModeReasoningEffort(effort));
            }
        })];
        let all_modes_actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
            tx.send(AppEvent::UpdateModel(model.clone()));
            tx.send(AppEvent::UpdateReasoningEffort(effort));
            tx.send(AppEvent::UpdatePlanModeReasoningEffort(effort));
            tx.send(AppEvent::PersistPlanModeReasoningEffort(effort));
            tx.send(AppEvent::PersistModelSelection {
                model: model.clone(),
                effort,
            });
        })];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(PLAN_MODE_REASONING_SCOPE_TITLE.to_string()),
            subtitle: Some(subtitle),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![
                SelectionItem {
                    name: PLAN_MODE_REASONING_SCOPE_PLAN_ONLY.to_string(),
                    description: Some(plan_only_description),
                    actions: plan_only_actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: PLAN_MODE_REASONING_SCOPE_ALL_MODES.to_string(),
                    description: Some(all_modes_description),
                    actions: all_modes_actions,
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        self.notify(Notification::PlanModePrompt {
            title: PLAN_MODE_REASONING_SCOPE_TITLE.to_string(),
        });
    }

    /// Open a popup to choose the reasoning effort (stage 2) for the given model.
    pub(crate) fn open_reasoning_popup(&mut self, preset: ModelPreset) {
        let default_effort: ReasoningEffortConfig = preset.default_reasoning_effort;
        let supported = preset.supported_reasoning_efforts;
        let in_plan_mode =
            self.collaboration_modes_enabled() && self.active_mode_kind() == ModeKind::Plan;

        let warn_effort = if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::XHigh)
        {
            Some(ReasoningEffortConfig::XHigh)
        } else if supported
            .iter()
            .any(|option| option.effort == ReasoningEffortConfig::High)
        {
            Some(ReasoningEffortConfig::High)
        } else {
            None
        };
        let warning_text = warn_effort.map(|effort| {
            let effort_label = Self::reasoning_effort_label(effort);
            format!("⚠ {effort_label} reasoning effort can quickly consume Plus plan rate limits.")
        });
        let warn_for_model = preset.model.starts_with("gpt-5.1-codex")
            || preset.model.starts_with("gpt-5.1-codex-max")
            || preset.model.starts_with("gpt-5.2");

        struct EffortChoice {
            stored: Option<ReasoningEffortConfig>,
            display: ReasoningEffortConfig,
        }
        let mut choices: Vec<EffortChoice> = Vec::new();
        for effort in ReasoningEffortConfig::iter() {
            if supported.iter().any(|option| option.effort == effort) {
                choices.push(EffortChoice {
                    stored: Some(effort),
                    display: effort,
                });
            }
        }
        if choices.is_empty() {
            choices.push(EffortChoice {
                stored: Some(default_effort),
                display: default_effort,
            });
        }

        if choices.len() == 1 {
            let selected_effort = choices.first().and_then(|c| c.stored);
            let selected_model = preset.model;
            if self.should_prompt_plan_mode_reasoning_scope(&selected_model, selected_effort) {
                self.app_event_tx
                    .send(AppEvent::OpenPlanReasoningScopePrompt {
                        model: selected_model,
                        effort: selected_effort,
                    });
            } else {
                self.apply_model_and_effort(selected_model, selected_effort);
            }
            return;
        }

        let default_choice: Option<ReasoningEffortConfig> = choices
            .iter()
            .any(|choice| choice.stored == Some(default_effort))
            .then_some(Some(default_effort))
            .flatten()
            .or_else(|| choices.iter().find_map(|choice| choice.stored))
            .or(Some(default_effort));

        let model_slug = preset.model.to_string();
        let is_current_model = self.current_model() == preset.model.as_str();
        let highlight_choice = if is_current_model {
            if in_plan_mode {
                self.config
                    .plan_mode_reasoning_effort
                    .or(self.effective_reasoning_effort())
            } else {
                self.effective_reasoning_effort()
            }
        } else {
            default_choice
        };
        let selection_choice = highlight_choice.or(default_choice);
        let initial_selected_idx = choices
            .iter()
            .position(|choice| choice.stored == selection_choice)
            .or_else(|| {
                selection_choice
                    .and_then(|effort| choices.iter().position(|choice| choice.display == effort))
            });
        let mut items: Vec<SelectionItem> = Vec::new();
        for choice in choices.iter() {
            let effort = choice.display;
            let mut effort_label = Self::reasoning_effort_label(effort).to_string();
            if choice.stored == default_choice {
                effort_label.push_str(" (default)");
            }

            let description = choice
                .stored
                .and_then(|effort| {
                    supported
                        .iter()
                        .find(|option| option.effort == effort)
                        .map(|option| option.description.to_string())
                })
                .filter(|text| !text.is_empty());

            let show_warning = warn_for_model && warn_effort == Some(effort);
            let selected_description = if show_warning {
                warning_text.as_ref().map(|warning_message| {
                    description.as_ref().map_or_else(
                        || warning_message.clone(),
                        |d| format!("{d}\n{warning_message}"),
                    )
                })
            } else {
                None
            };

            let model_for_action = model_slug.clone();
            let choice_effort = choice.stored;
            let should_prompt_plan_mode_scope =
                self.should_prompt_plan_mode_reasoning_scope(model_slug.as_str(), choice_effort);
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                if should_prompt_plan_mode_scope {
                    tx.send(AppEvent::OpenPlanReasoningScopePrompt {
                        model: model_for_action.clone(),
                        effort: choice_effort,
                    });
                } else {
                    tx.send(AppEvent::OpenHarnessPopup {
                        model: model_for_action.clone(),
                        effort: choice_effort,
                    });
                }
            })];

            items.push(SelectionItem {
                name: effort_label,
                description,
                selected_description,
                is_current: is_current_model && choice.stored == highlight_choice,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let mut header = ColumnRenderable::new();
        header.push(Line::from(
            format!("Select Reasoning Level for {model_slug}").bold(),
        ));

        self.bottom_pane.show_selection_view(SelectionViewParams {
            header: Box::new(header),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        });
    }

    pub(super) fn reasoning_effort_label(effort: ReasoningEffortConfig) -> &'static str {
        match effort {
            ReasoningEffortConfig::None => "None",
            ReasoningEffortConfig::Minimal => "Minimal",
            ReasoningEffortConfig::Low => "Low",
            ReasoningEffortConfig::Medium => "Medium",
            ReasoningEffortConfig::High => "High",
            ReasoningEffortConfig::XHigh => "Extra high",
        }
    }

    pub(super) fn apply_model_and_effort_without_persist(
        &self,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) {
        self.app_event_tx.send(AppEvent::UpdateModel(model));
        self.app_event_tx
            .send(AppEvent::UpdateReasoningEffort(effort));
    }

    fn apply_model_and_effort(&self, model: String, effort: Option<ReasoningEffortConfig>) {
        self.apply_model_and_effort_without_persist(model.clone(), effort);
        self.app_event_tx
            .send(AppEvent::PersistModelSelection { model, effort });
    }

    pub(crate) fn open_model_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        presets: Vec<ModelPreset>,
    ) {
        let mut presets: Vec<ModelPreset> = presets
            .into_iter()
            .filter(|preset| preset.show_in_picker)
            .collect();
        presets.sort_by(|left, right| left.model.cmp(&right.model));

        let mut items: Vec<SelectionItem> = presets
            .into_iter()
            .map(|preset| {
                let provider_id_for_action = provider_id.clone();
                let provider_name_for_action = provider_name.clone();
                let single_supported_effort = single_supported_reasoning_effort(&preset);
                let actions: Vec<SelectionAction> = if let Some(effort) = single_supported_effort {
                    let model = preset.model.clone();
                    vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenHarnessPopupForProvider {
                            provider_id: provider_id_for_action.clone(),
                            provider_name: provider_name_for_action.clone(),
                            model: model.clone(),
                            effort,
                        });
                    })]
                } else {
                    let preset_for_action = preset.clone();
                    vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenReasoningPopupForProvider {
                            provider_id: provider_id_for_action.clone(),
                            provider_name: provider_name_for_action.clone(),
                            model: preset_for_action.clone(),
                        });
                    })]
                };
                SelectionItem {
                    name: preset.model.clone(),
                    description: provider_model_description(
                        self.config.model_providers.get(provider_id.as_str()),
                        provider_id.as_str(),
                        provider_name.as_str(),
                        preset.model.as_str(),
                        preset.description.as_str(),
                    ),
                    is_current: provider_id == self.config.model_provider_id
                        && preset.model == self.current_model(),
                    is_default: preset.is_default,
                    actions,
                    dismiss_on_select: single_supported_effort.is_some(),
                    search_value: Some(format!(
                        "{} {} {}",
                        preset.model, preset.display_name, preset.description
                    )),
                    ..Default::default()
                }
            })
            .collect();

        let provider_id_for_action = provider_id;
        let provider_name_for_action = provider_name.clone();
        items.push(SelectionItem {
            name: "Custom model name".to_string(),
            description: Some("Type a model id that this provider accepts.".to_string()),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenCustomProviderModelPrompt {
                    provider_id: provider_id_for_action.clone(),
                    provider_name: provider_name_for_action.clone(),
                    initial_text: None,
                });
            })],
            dismiss_on_select: true,
            keep_visible_during_search: true,
            search_value: Some("custom manual typed model".to_string()),
            ..Default::default()
        });

        let header = self.model_menu_header(
            &format!("Select Model for {provider_name}"),
            "Choose a listed model or type a model id.",
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some("Type to filter • Enter to select • Esc to dismiss".into()),
            items,
            header,
            is_searchable: true,
            search_placeholder: Some("Filter models".to_string()),
            ..Default::default()
        });
    }

    pub(crate) fn open_custom_model_prompt_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        initial_text: Option<String>,
    ) {
        let tx = self.app_event_tx.clone();
        let view = CustomPromptView::new(
            format!("{provider_name} model name"),
            "Type any model id, then press Enter".to_string(),
            initial_text.unwrap_or_default(),
            Some("This will start a new chat with the selected provider.".to_string()),
            Box::new(move |model: String| {
                let model = model.trim().to_string();
                if model.is_empty() {
                    return;
                }
                tx.send(AppEvent::OpenHarnessPopupForProvider {
                    provider_id: provider_id.clone(),
                    provider_name: provider_name.clone(),
                    model,
                    effort: None,
                });
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_reasoning_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        preset: ModelPreset,
    ) {
        let default_effort = preset.default_reasoning_effort;
        let supported = preset.supported_reasoning_efforts;
        if supported.is_empty() {
            self.app_event_tx
                .send(AppEvent::OpenHarnessPopupForProvider {
                    provider_id,
                    provider_name,
                    model: preset.model,
                    effort: None,
                });
            return;
        }

        let choices: Vec<_> = ReasoningEffortConfig::iter()
            .filter(|effort| supported.iter().any(|option| option.effort == *effort))
            .map(|effort| {
                let mut label = Self::reasoning_effort_label(effort).to_string();
                if effort == default_effort {
                    label.push_str(" (default)");
                }
                let description = supported
                    .iter()
                    .find(|option| option.effort == effort)
                    .map(|option| option.description.to_string())
                    .filter(|text| !text.is_empty());
                (label, Some(effort), description)
            })
            .collect();
        let initial_selected_idx = choices
            .iter()
            .position(|(_, effort, _)| *effort == Some(default_effort));

        let mut items = Vec::new();
        for (label, effort, description) in choices {
            let provider_id_for_action = provider_id.clone();
            let provider_name_for_action = provider_name.clone();
            let model = preset.model.clone();
            items.push(SelectionItem {
                name: label,
                description,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenHarnessPopupForProvider {
                        provider_id: provider_id_for_action.clone(),
                        provider_name: provider_name_for_action.clone(),
                        model: model.clone(),
                        effort,
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = self.model_menu_header(
            &format!("Select Reasoning Level for {}", preset.model),
            &format!("{provider_name} will start a new chat with this selection."),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            header,
            footer_hint: Some(standard_popup_hint_line()),
            items,
            initial_selected_idx,
            ..Default::default()
        });
    }

    pub(crate) fn open_harness_popup(
        &mut self,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) {
        let provider_id = self.config.model_provider_id.clone();
        let provider_name = self
            .config
            .model_providers
            .get(provider_id.as_str())
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| provider_id.clone());
        let provider = self.config.model_providers.get(provider_id.as_str());
        let items = harness_selection_items(
            provider_id,
            provider_name.clone(),
            provider,
            model.clone(),
            effort,
            /*include_all_harnesses*/ true,
        );
        let header = self.model_menu_header(
            "Select Tool Harness",
            &format!("{provider_name} / {model} will start a new chat."),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line()),
            items,
            header,
            ..Default::default()
        });
    }

    pub(crate) fn open_harness_popup_for_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        model: String,
        effort: Option<ReasoningEffortConfig>,
    ) {
        let provider = self.config.model_providers.get(provider_id.as_str());
        let items = harness_selection_items(
            provider_id,
            provider_name.clone(),
            provider,
            model.clone(),
            effort,
            /*include_all_harnesses*/ false,
        );
        let header = self.model_menu_header(
            "Select Tool Harness",
            &format!("{provider_name} / {model} will start a new chat."),
        );
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line()),
            items,
            header,
            ..Default::default()
        });
    }
}

fn provider_sort_priority(provider_id: &str) -> u16 {
    bundled_provider_catalog_entry(provider_id).map_or(u16::MAX, |entry| entry.sort_priority)
}

fn single_supported_reasoning_effort(
    preset: &ModelPreset,
) -> Option<Option<ReasoningEffortConfig>> {
    if preset.supported_reasoning_efforts.is_empty() {
        return Some(None);
    }

    let mut choices = ReasoningEffortConfig::iter().filter(|effort| {
        preset
            .supported_reasoning_efforts
            .iter()
            .any(|option| option.effort == *effort)
    });
    let first = choices.next()?;
    if choices.next().is_none() {
        Some(Some(first))
    } else {
        None
    }
}

fn provider_description(
    provider_id: &str,
    provider: &codex_model_provider_info::ModelProviderInfo,
) -> String {
    let description = if provider.requires_openai_auth {
        "Sign in with ChatGPT".to_string()
    } else if let Some(env_key) = provider.env_key.as_deref() {
        format!("Use {env_key} or paste a key")
    } else if provider.auth.is_some() || provider.experimental_bearer_token.is_some() {
        "Auth configured".to_string()
    } else {
        match provider.wire_api {
            WireApi::Responses => "No API key required".to_string(),
            WireApi::Chat => "Chat-compatible endpoint".to_string(),
            WireApi::Messages => "Anthropic Messages endpoint".to_string(),
        }
    };
    let harness = default_harness_for_provider_model(provider_id, provider, None);
    harness.map_or(description.clone(), |harness| {
        format!("{description} | Harness: {harness}")
    })
}

fn provider_model_description(
    provider: Option<&codex_model_provider_info::ModelProviderInfo>,
    provider_id: &str,
    provider_name: &str,
    model: &str,
    description: &str,
) -> Option<String> {
    let harness = provider.and_then(|provider| {
        default_harness_for_provider_model(provider_id, provider, Some(model))
    });
    match (description.is_empty(), harness) {
        (true, None) => None,
        (true, Some(harness)) => Some(format!("Harness: {harness}")),
        (false, None) => Some(description.to_string()),
        (false, Some(harness)) => Some(format!("{description} | Harness: {harness}")),
    }
    .or_else(|| Some(format!("Provider: {provider_name}")))
}

fn harness_selection_items(
    provider_id: String,
    provider_name: String,
    provider: Option<&codex_model_provider_info::ModelProviderInfo>,
    model: String,
    effort: Option<ReasoningEffortConfig>,
    include_all_harnesses: bool,
) -> Vec<SelectionItem> {
    let choices = harness_choices_for_provider_model(
        provider_id.as_str(),
        provider,
        model.as_str(),
        include_all_harnesses,
    );
    choices
        .into_iter()
        .map(|choice| {
            let provider_id = provider_id.clone();
            let provider_name = provider_name.clone();
            let model = model.clone();
            let harness = choice.stored.clone();
            let actions: Vec<SelectionAction> = vec![Box::new(move |tx| {
                tx.send(AppEvent::PersistProviderModelSelection {
                    provider_id: provider_id.clone(),
                    provider_name: provider_name.clone(),
                    model: model.clone(),
                    effort,
                    harness: harness.clone(),
                });
            })];
            SelectionItem {
                name: choice.label,
                description: Some(choice.description),
                is_current: false,
                actions,
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect()
}

struct HarnessChoice {
    stored: Option<String>,
    label: String,
    description: String,
}

fn harness_choices_for_provider_model(
    provider_id: &str,
    provider: Option<&codex_model_provider_info::ModelProviderInfo>,
    model: &str,
    include_all_harnesses: bool,
) -> Vec<HarnessChoice> {
    let wire_api = provider
        .map(|provider| provider.wire_api)
        .unwrap_or_default();
    let recommended = provider
        .and_then(|provider| default_harness_for_provider_model(provider_id, provider, Some(model)))
        .unwrap_or("");
    let all_harnesses = [
        "",
        "claude-code",
        "claude-code-bare",
        "kimi-cli",
        "qwen-code",
        "deepseek-tui",
        "mini-swe-agent",
        "opencode",
        "swe-agent",
        "terminus-2",
        "minimal",
    ];
    let mut choices = if include_all_harnesses {
        all_harnesses.to_vec()
    } else {
        match wire_api {
            WireApi::Messages => vec!["claude-code", "claude-code-bare"],
            WireApi::Chat => all_harnesses.to_vec(),
            WireApi::Responses => vec![""],
        }
    };
    choices.sort_by_key(|harness| usize::from(*harness != recommended));
    choices
        .into_iter()
        .map(|harness| harness_choice(harness, harness == recommended))
        .collect()
}

fn harness_choice(harness: &str, is_recommended: bool) -> HarnessChoice {
    let base_label = match harness {
        "" => "Codex",
        "claude-code" => "Claude Code",
        "claude-code-bare" => "Claude Code Bare",
        "kimi-cli" => "Kimi CLI",
        "qwen-code" => "Qwen Code",
        "deepseek-tui" => "DeepSeek TUI",
        "mini-swe-agent" => "mini-swe-agent",
        "opencode" => "opencode",
        "swe-agent" => "SWE-agent",
        "terminus-2" => "Terminus 2",
        "minimal" => "Minimal",
        other => other,
    };
    let label = if is_recommended {
        format!("{base_label} (recommended)")
    } else {
        base_label.to_string()
    };
    let description = match harness {
        "" => "Use the native Codex tool harness.",
        "claude-code" => "Use the Claude Code-style tool harness.",
        "claude-code-bare" => "Use the lean Claude Code-style harness.",
        "kimi-cli" => "Use the Kimi CLI-style tool harness.",
        "qwen-code" => "Use the Qwen Code-style tool harness.",
        "deepseek-tui" => "Use the DeepSeek TUI-style tool harness.",
        "mini-swe-agent" => "Use the mini-swe-agent-style tool harness.",
        "opencode" => "Use the opencode-style tool harness.",
        "swe-agent" => "Use the SWE-agent-style tool harness.",
        "terminus-2" => "Use the Terminus 2-style terminal harness.",
        "minimal" => "Use a minimal shell-oriented tool harness.",
        _ => "Use this configured tool harness.",
    }
    .to_string();
    HarnessChoice {
        stored: (!harness.is_empty()).then(|| harness.to_string()),
        label,
        description,
    }
}

#[cfg(test)]
mod tests {
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::WireApi;

    use super::harness_choices_for_provider_model;

    #[test]
    fn standalone_harness_picker_shows_all_harnesses_for_responses_models() {
        let provider = ModelProviderInfo {
            wire_api: WireApi::Responses,
            ..Default::default()
        };

        let choices = harness_choices_for_provider_model(
            "openai",
            Some(&provider),
            "gpt-5.5",
            /*include_all_harnesses*/ true,
        )
        .into_iter()
        .map(|choice| choice.label)
        .collect::<Vec<_>>();

        assert_eq!(
            choices,
            vec![
                "Codex (recommended)",
                "Claude Code",
                "Claude Code Bare",
                "Kimi CLI",
                "Qwen Code",
                "DeepSeek TUI",
                "mini-swe-agent",
                "opencode",
                "SWE-agent",
                "Terminus 2",
                "Minimal",
            ]
        );
    }

    #[test]
    fn provider_scoped_harness_picker_keeps_responses_native_only() {
        let provider = ModelProviderInfo {
            wire_api: WireApi::Responses,
            ..Default::default()
        };

        let choices = harness_choices_for_provider_model(
            "openai",
            Some(&provider),
            "gpt-5.5",
            /*include_all_harnesses*/ false,
        );

        assert_eq!(
            choices
                .into_iter()
                .map(|choice| choice.label)
                .collect::<Vec<_>>(),
            vec!["Codex (recommended)"]
        );
    }
}
