use std::collections::BTreeSet;

use crate::config::explain::selected_value;
use crate::config::{
    ActiveProfileSource, BootstrapConfigExplain, ConfigError, ConfigExplain, ConfigSource,
    ConfigValue, ResolveOptions, ResolvedValue, Scope, normalize_identifier,
    validate_bootstrap_value,
};

use crate::config::selector::{LayerRef, ScopeSelector};

/// One resolution request always runs against a single active profile/terminal
/// pair. Bootstrap computes this frame once up front so runtime resolution and
/// explain both describe the same view of the world.
#[derive(Debug, Clone)]
pub(crate) struct ResolutionFrame {
    pub(crate) active_profile: String,
    pub(crate) active_profile_source: ActiveProfileSource,
    pub(crate) terminal: Option<String>,
    pub(crate) known_profiles: BTreeSet<String>,
}

pub(crate) fn prepare_resolution(
    layers: [LayerRef<'_>; 7],
    options: ResolveOptions,
) -> Result<ResolutionFrame, ConfigError> {
    validate_layers(layers)?;
    let terminal = options.terminal.map(|value| normalize_identifier(&value));
    let profile_override = options
        .profile_override
        .map(|value| normalize_identifier(&value));
    let known_profiles = collect_known_profiles(layers);
    let profile_selection = resolve_active_profile(
        layers,
        profile_override.as_deref(),
        terminal.as_deref(),
        &known_profiles,
    )?;

    tracing::debug!(
        active_profile = %profile_selection.profile,
        active_profile_source = %profile_selection.source.as_str(),
        terminal = ?terminal,
        known_profiles = known_profiles.len(),
        "prepared config resolution frame"
    );

    Ok(ResolutionFrame {
        active_profile: profile_selection.profile,
        active_profile_source: profile_selection.source,
        terminal,
        known_profiles,
    })
}

pub(crate) fn explain_default_profile_key(
    layers: [LayerRef<'_>; 7],
    options: ResolveOptions,
) -> Result<ConfigExplain, ConfigError> {
    Ok(explain_default_profile_bootstrap(layers, options)?.into())
}

pub(crate) fn explain_default_profile_bootstrap(
    layers: [LayerRef<'_>; 7],
    options: ResolveOptions,
) -> Result<BootstrapConfigExplain, ConfigError> {
    let frame = prepare_resolution(layers, options)?;
    // Default-profile lookup is a bootstrap pass, so it must not see any scope
    // that already depends on the active profile being chosen.
    let selector = ScopeSelector::global(frame.terminal.as_deref());
    let explain_layers = layers
        .into_iter()
        .filter_map(|layer| selector.explain_layer(layer, "profile.default"))
        .collect::<Vec<_>>();

    let final_entry = select_default_profile_across_layers(layers, selector)
        .map(|selected| selected_value(&selected))
        .or_else(|| {
            Some(ResolvedValue {
                raw_value: ConfigValue::String("default".to_string()),
                value: ConfigValue::String("default".to_string()),
                source: ConfigSource::Derived,
                scope: Scope::global(),
                origin: None,
            })
        });

    Ok(BootstrapConfigExplain {
        key: "profile.default".to_string(),
        active_profile: frame.active_profile,
        active_profile_source: frame.active_profile_source,
        terminal: frame.terminal,
        known_profiles: frame.known_profiles,
        layers: explain_layers,
        final_entry,
    })
}

impl From<BootstrapConfigExplain> for ConfigExplain {
    fn from(value: BootstrapConfigExplain) -> Self {
        Self {
            key: value.key,
            active_profile: value.active_profile,
            active_profile_source: value.active_profile_source,
            terminal: value.terminal,
            known_profiles: value.known_profiles,
            layers: value.layers,
            final_entry: value.final_entry,
            interpolation: None,
        }
    }
}

fn validate_layers(layers: [LayerRef<'_>; 7]) -> Result<(), ConfigError> {
    for layer in layers {
        layer.layer.validate_entries()?;
    }

    Ok(())
}

fn collect_known_profiles(layers: [LayerRef<'_>; 7]) -> BTreeSet<String> {
    let mut known = BTreeSet::new();

    for layer in layers {
        for entry in &layer.layer.entries {
            if let Some(profile) = entry.scope.profile.as_deref() {
                known.insert(profile.to_string());
            }
        }
    }

    known
}

fn resolve_active_profile(
    layers: [LayerRef<'_>; 7],
    explicit: Option<&str>,
    terminal: Option<&str>,
    known_profiles: &BTreeSet<String>,
) -> Result<ActiveProfileSelection, ConfigError> {
    tracing::debug!(
        explicit_profile = ?explicit,
        terminal = ?terminal,
        known_profiles = known_profiles.len(),
        "resolving active profile"
    );
    let selection = if let Some(profile) = explicit {
        ActiveProfileSelection {
            profile: normalize_identifier(profile),
            source: ActiveProfileSource::Override,
        }
    } else {
        ActiveProfileSelection {
            profile: resolve_default_profile(layers, terminal)?,
            source: ActiveProfileSource::DefaultProfile,
        }
    };

    if selection.profile.trim().is_empty() {
        return Err(ConfigError::MissingDefaultProfile);
    }

    if !known_profiles.is_empty() && !known_profiles.contains(&selection.profile) {
        tracing::warn!(
            active_profile = %selection.profile,
            known_profiles = known_profiles.len(),
            "resolved unknown active profile"
        );
        return Err(ConfigError::UnknownProfile {
            profile: selection.profile,
            known: known_profiles.iter().cloned().collect::<Vec<String>>(),
        });
    }

    tracing::debug!(
        active_profile = %selection.profile,
        active_profile_source = %selection.source.as_str(),
        "resolved active profile"
    );

    Ok(selection)
}

fn resolve_default_profile(
    layers: [LayerRef<'_>; 7],
    terminal: Option<&str>,
) -> Result<String, ConfigError> {
    let mut picked: Option<ConfigValue> = None;
    // Bootstrap selection is layer-wide but scope-restricted: later layers may
    // still override earlier ones, but profile-scoped values are invisible.
    let selector = ScopeSelector::global(terminal);

    for layer in layers {
        if let Some(selected) = selector.select(layer, "profile.default") {
            picked = Some(selected.entry.value.clone());
        }
    }

    match picked {
        None => {
            tracing::debug!(terminal = ?terminal, "using implicit default profile");
            Ok("default".to_string())
        }
        Some(value) => {
            validate_bootstrap_value("profile.default", &value)?;
            match value.reveal() {
                ConfigValue::String(profile) => {
                    let normalized = normalize_identifier(profile);
                    tracing::debug!(
                        terminal = ?terminal,
                        selected_profile = %normalized,
                        "resolved profile.default from loaded layers"
                    );
                    Ok(normalized)
                }
                other => Err(ConfigError::InvalidBootstrapValue {
                    key: "profile.default".to_string(),
                    reason: format!("expected string, got {other:?}"),
                }),
            }
        }
    }
}

fn select_default_profile_across_layers<'a>(
    layers: [LayerRef<'a>; 7],
    selector: ScopeSelector<'a>,
) -> Option<crate::config::selector::SelectedLayerEntry<'a>> {
    let mut selected = None;

    for layer in layers {
        if let Some(entry) = selector.select(layer, "profile.default") {
            selected = Some(entry);
        }
    }

    selected
}

#[derive(Debug, Clone)]
struct ActiveProfileSelection {
    profile: String,
    source: ActiveProfileSource,
}
