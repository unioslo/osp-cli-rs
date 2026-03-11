use crate::config::{ConfigValue, ResolvedConfig};
use anyhow::Result;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginCommandState {
    Enabled,
    Disabled,
}

impl PluginCommandState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    fn from_config_value(value: &ConfigValue) -> Option<Self> {
        match value.reveal() {
            ConfigValue::String(raw) if raw.eq_ignore_ascii_case("enabled") => Some(Self::Enabled),
            ConfigValue::String(raw) if raw.eq_ignore_ascii_case("disabled") => {
                Some(Self::Disabled)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PluginCommandPreferences {
    pub(crate) command_states: BTreeMap<String, PluginCommandState>,
    pub(crate) preferred_providers: BTreeMap<String, String>,
}

impl PluginCommandPreferences {
    pub(crate) fn from_resolved(config: &ResolvedConfig) -> Self {
        let mut preferences = Self::default();
        for (key, entry) in config.values() {
            let Some((command, field)) = plugin_command_config_field(key) else {
                continue;
            };
            match field {
                PluginCommandConfigField::State => {
                    if let Some(state) = PluginCommandState::from_config_value(&entry.value) {
                        preferences.command_states.insert(command, state);
                    }
                }
                PluginCommandConfigField::Provider => {
                    if let ConfigValue::String(provider) = entry.value.reveal() {
                        let provider = provider.trim();
                        if !provider.is_empty() {
                            preferences
                                .preferred_providers
                                .insert(command, provider.to_string());
                        }
                    }
                }
            }
        }
        preferences
    }

    pub(crate) fn state_for(&self, command: &str) -> Option<PluginCommandState> {
        self.command_states.get(command).copied()
    }

    pub(crate) fn preferred_provider_for(&self, command: &str) -> Option<&str> {
        self.preferred_providers.get(command).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn set_state(&mut self, command: &str, state: PluginCommandState) {
        self.command_states.insert(command.to_string(), state);
    }

    pub(crate) fn set_provider(&mut self, command: &str, plugin_id: &str) {
        self.preferred_providers
            .insert(command.to_string(), plugin_id.to_string());
    }

    pub(crate) fn clear_provider(&mut self, command: &str) -> bool {
        self.preferred_providers.remove(command).is_some()
    }
}

enum PluginCommandConfigField {
    State,
    Provider,
}

fn plugin_command_config_field(key: &str) -> Option<(String, PluginCommandConfigField)> {
    let normalized = key.trim().to_ascii_lowercase();
    let remainder = normalized.strip_prefix("plugins.")?;
    let (command, field) = remainder.rsplit_once('.')?;
    if command.trim().is_empty() {
        return None;
    }
    let field = match field {
        "state" => PluginCommandConfigField::State,
        "provider" => PluginCommandConfigField::Provider,
        _ => return None,
    };
    Some((command.to_string(), field))
}

pub(super) fn write_text_atomic(path: &std::path::Path, payload: &str) -> Result<()> {
    crate::config::write_text_atomic(path, payload.as_bytes(), false).map_err(Into::into)
}

pub(super) fn merge_issue(target: &mut Option<String>, message: String) {
    if message.trim().is_empty() {
        return;
    }

    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&message);
        }
        None => *target = Some(message),
    }
}
