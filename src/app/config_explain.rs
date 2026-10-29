//! Config explain rendering and suggestion helpers.
//!
//! `config explain` is a user-facing introspection surface built on top of the
//! lower-level config resolver. This module owns the last-mile conversion from
//! explanation data into human/JSON output, plus the fuzzy key-recovery hints
//! shown when a requested key does not exist.

use crate::config::is_sensitive_key;
use crate::config::{
    BootstrapScopeRule, ConfigExplain, ConfigSchema, ConfigValue, ResolvedConfig,
    bootstrap_key_spec, is_bootstrap_only_key,
};
use crate::core::fuzzy::{config_fuzzy_matcher, fold_case};
use crate::core::output::OutputFormat;
use crate::ui::messages::{MessageBuffer, MessageLevel};
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::app::UiState;
use crate::cli::ConfigExplainArgs;
use crate::ui::explain_presentation_effect;

use super::{CommandOutput, RuntimeConfigRequest, prepare_runtime_config};

pub(crate) fn config_explain_result(
    config: &ResolvedConfig,
    ui: &UiState,
    request: RuntimeConfigRequest,
    args: ConfigExplainArgs,
) -> Result<CommandOutput> {
    let explain = explain_runtime_config(request, &config.display_key(&args.key))?;

    if explain.final_entry.is_none() && explain.layers.is_empty() {
        let mut messages = MessageBuffer::default();
        let error = push_missing_config_key_messages(&mut messages, config, &args.key);
        let report = if error.action.is_some() {
            miette::Report::new(crate::core::CommandHint::new("config list", error))
        } else {
            miette::Report::new(error)
        };
        return Ok(CommandOutput {
            exit_code: 1,
            messages,
            body: crate::app::CommandBody::None,
            stderr_text: String::new(),
            failure_report: Some(report),
            ..CommandOutput::exit(0)
        });
    }

    if matches!(ui.render_settings.requested_format(), OutputFormat::Json) {
        let payload = config_explain_json(&explain, config, args.show_secrets);
        return Ok(CommandOutput::json(payload));
    }

    Ok(CommandOutput::text(render_config_explain_text(
        &explain,
        config,
        args.show_secrets,
    )))
}

pub(crate) fn config_value_to_json_for_key(key: &str, value: &ConfigValue) -> serde_json::Value {
    redact_value_json(key, value, false)
}

fn config_value_to_json_exposed(value: &ConfigValue) -> serde_json::Value {
    match value {
        ConfigValue::Secret(secret) => config_value_to_json_exposed(secret.expose()),
        ConfigValue::String(v) => v.clone().into(),
        ConfigValue::Bool(v) => (*v).into(),
        ConfigValue::Integer(v) => (*v).into(),
        ConfigValue::Float(v) => (*v).into(),
        ConfigValue::List(values) => {
            serde_json::Value::Array(values.iter().map(config_value_to_json_exposed).collect())
        }
    }
}

pub(crate) fn explain_runtime_config(
    request: RuntimeConfigRequest,
    key: &str,
) -> Result<ConfigExplain> {
    let key = request.product_defaults.key_map.display(key);
    prepare_runtime_config(request, false)
        .and_then(|prepared| prepared.explain_key(&key))
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to explain config key `{key}`"))
}

pub(crate) fn render_config_explain_text(
    explain: &ConfigExplain,
    config: &ResolvedConfig,
    show_secrets: bool,
) -> String {
    let facts = config_explain_json(explain, config, show_secrets);
    let text = |value: &serde_json::Value| value.as_str().unwrap_or("").to_string();
    let optional =
        |value: &serde_json::Value, fallback: &str| value.as_str().unwrap_or(fallback).to_string();
    let mut out = format!(
        "key: {}\nphase: {}\n",
        text(&facts["key"]),
        text(&facts["phase"])
    );
    if let Some(doc) = facts["description"].as_str() {
        out.push_str(&format!("description: {doc}\n"));
    }
    out.push('\n');
    if facts["source"].is_null() {
        out.push_str("value: not set\n\n");
    } else {
        out.push_str(&format!(
            "value: {} ({})\n\nwinner:\n  source: {}\n  scope: {}\n  origin: {}\n\n",
            display_json_value(&facts["value"]),
            text(&facts["value_type"]),
            text(&facts["source"]),
            text(&facts["scope"]),
            optional(&facts["origin"], "-")
        ));
    }
    if let Some(effect) = facts.get("presentation") {
        out.push_str(&format!("presentation:\n  preset: {}\n  preset_source: {}\n  preset_scope: {}\n  preset_origin: {}\n  seeded_value: {} ({})\n  note: {}\n\n",
            text(&effect["preset"]), text(&effect["preset_source"]), text(&effect["preset_scope"]),
            optional(&effect["preset_origin"], "-"), display_json_value(&effect["seeded_value"]),
            text(&effect["seeded_value_type"]), text(&effect["note"])));
    }
    out.push_str(&format!(
        "context:\n  active_profile: {}\n  active_profile_source: {}\n  terminal: {}\n\n",
        text(&facts["active_profile"]),
        text(&facts["active_profile_source"]),
        optional(&facts["terminal"], "none")
    ));
    if let Some(policy) = facts["bootstrap_scope_policy"].as_str() {
        out.push_str(&format!("bootstrap_scope_policy: {policy}\n\n"));
    }
    if let Some(candidates) = facts["candidates"]
        .as_array()
        .filter(|candidates| !candidates.is_empty())
    {
        out.push_str("candidates (in priority order):\n");
        for candidate in candidates {
            let marker = if candidate["winner"] == true {
                "  ✅"
            } else {
                "   "
            };
            out.push_str(&format!(
                "{marker} {} ({}) = {}",
                text(&candidate["source"]),
                text(&candidate["scope"]),
                display_json_value(&candidate["value"])
            ));
            if let Some(origin) = candidate["origin"].as_str() {
                out.push_str(&format!(" [{origin}]"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    if let Some(interpolation) = facts.get("interpolation") {
        out.push_str(&format!(
            "interpolation:\n  template: {}\n",
            display_json_value(&interpolation["template"])
        ));
        for step in interpolation["steps"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  ${{{}}} raw={} final={} (from {}, {})\n",
                text(&step["placeholder"]),
                display_json_value(&step["raw_value"]),
                display_json_value(&step["value"]),
                text(&step["source"]),
                text(&step["scope"])
            ));
        }
        if !show_secrets && contains_sensitive_values(explain) {
            out.push_str("  note: some values are redacted; pass --show-secrets to display them\n");
        }
    }
    out
}

fn display_json_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(crate) fn config_explain_json(
    explain: &ConfigExplain,
    config: &ResolvedConfig,
    show_secrets: bool,
) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    root.insert("key".to_string(), config.display_key(&explain.key).into());
    // Keep the resolution phase explicit in output so bootstrap-only keys such
    // as `profile.default` are not mistaken for ordinary runtime config.
    root.insert(
        "phase".to_string(),
        if is_bootstrap_only_key(&explain.key) {
            "bootstrap"
        } else {
            "runtime"
        }
        .into(),
    );
    root.insert(
        "description".to_string(),
        config_key_doc(&explain.key).map_or(serde_json::Value::Null, Into::into),
    );
    root.insert(
        "active_profile".to_string(),
        explain.active_profile.clone().into(),
    );
    root.insert(
        "active_profile_source".to_string(),
        explain.active_profile_source.as_str().into(),
    );
    root.insert(
        "bootstrap_scope_policy".to_string(),
        bootstrap_scope_policy(&explain.key).map_or(serde_json::Value::Null, Into::into),
    );
    root.insert(
        "terminal".to_string(),
        explain
            .terminal
            .clone()
            .map_or(serde_json::Value::Null, Into::into),
    );

    if let Some(final_entry) = &explain.final_entry {
        root.insert(
            "value".to_string(),
            redact_value_json(&explain.key, &final_entry.value, show_secrets),
        );
        root.insert(
            "value_type".to_string(),
            config_value_type(&final_entry.value).to_string().into(),
        );
        root.insert("source".to_string(), final_entry.source.to_string().into());
        root.insert("scope".to_string(), format_scope(&final_entry.scope).into());
        root.insert(
            "origin".to_string(),
            final_entry
                .origin
                .clone()
                .map_or(serde_json::Value::Null, Into::into),
        );
    } else {
        root.insert("value".to_string(), serde_json::Value::Null);
        root.insert("value_type".to_string(), "none".into());
        root.insert("source".to_string(), serde_json::Value::Null);
        root.insert("scope".to_string(), serde_json::Value::Null);
        root.insert("origin".to_string(), serde_json::Value::Null);
    }

    let mut candidates = Vec::new();
    for (is_winner, source, scope, origin, value) in precedence_chain(explain) {
        let mut row = serde_json::Map::new();
        row.insert("winner".to_string(), is_winner.into());
        row.insert("source".to_string(), source.to_string().into());
        row.insert("scope".to_string(), scope.into());
        row.insert(
            "origin".to_string(),
            origin.map_or(serde_json::Value::Null, Into::into),
        );
        row.insert(
            "value".to_string(),
            redact_value_json(&explain.key, &value, show_secrets),
        );
        candidates.push(serde_json::Value::Object(row));
    }
    root.insert(
        "candidates".to_string(),
        serde_json::Value::Array(candidates),
    );

    if let Some(interpolation) = &explain.interpolation {
        let mut section = serde_json::Map::new();
        section.insert(
            "template".to_string(),
            redact_value_json(&explain.key, &interpolation.template, show_secrets),
        );
        let mut steps = Vec::new();
        for step in &interpolation.steps {
            let mut item = serde_json::Map::new();
            item.insert(
                "placeholder".to_string(),
                config.display_key(&step.placeholder).into(),
            );
            item.insert(
                "raw_value".to_string(),
                redact_value_json(&step.placeholder, &step.raw_value, show_secrets),
            );
            item.insert(
                "value".to_string(),
                redact_value_json(&step.placeholder, &step.value, show_secrets),
            );
            item.insert("source".to_string(), step.source.to_string().into());
            item.insert("scope".to_string(), format_scope(&step.scope).into());
            item.insert(
                "origin".to_string(),
                step.origin
                    .clone()
                    .map_or(serde_json::Value::Null, Into::into),
            );
            steps.push(serde_json::Value::Object(item));
        }
        section.insert("steps".to_string(), serde_json::Value::Array(steps));
        root.insert(
            "interpolation".to_string(),
            serde_json::Value::Object(section),
        );
    }

    if let Some(effect) = explain_presentation_effect(config, &explain.key) {
        let mut section = serde_json::Map::new();
        section.insert("preset".to_string(), effect.preset.as_config_value().into());
        section.insert(
            "preset_source".to_string(),
            effect.preset_source.to_string().into(),
        );
        section.insert(
            "preset_scope".to_string(),
            format_scope(&effect.preset_scope).into(),
        );
        section.insert(
            "preset_origin".to_string(),
            effect
                .preset_origin
                .map_or(serde_json::Value::Null, Into::into),
        );
        section.insert(
            "seeded_value".to_string(),
            redact_value_json(&explain.key, &effect.seeded_value, show_secrets),
        );
        section.insert(
            "seeded_value_type".to_string(),
            config_value_type(&effect.seeded_value).to_string().into(),
        );
        section.insert(
            "note".to_string(),
            "key kept its builtin default, so ui.presentation seeds the resolved UI value".into(),
        );
        root.insert(
            "presentation".to_string(),
            serde_json::Value::Object(section),
        );
    }

    serde_json::Value::Object(root)
}

fn precedence_chain(
    explain: &ConfigExplain,
) -> Vec<(bool, String, String, Option<String>, ConfigValue)> {
    let winner_source = explain.final_entry.as_ref().map(|entry| entry.source);
    let mut chain = Vec::new();

    for layer in &explain.layers {
        let mut candidates = layer
            .candidates
            .iter()
            .filter_map(|candidate| candidate.rank.map(|rank| (rank, candidate)))
            .collect::<Vec<(u8, &crate::config::ExplainCandidate)>>();
        if candidates.is_empty() {
            continue;
        }

        candidates.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| right.entry_index.cmp(&left.entry_index))
        });

        for (_rank, candidate) in candidates {
            let is_winner = winner_source == Some(layer.source)
                && layer.selected_entry_index == Some(candidate.entry_index);
            chain.push((
                is_winner,
                layer.source.to_string(),
                format_scope(&candidate.scope),
                candidate.origin.clone(),
                candidate.value.clone(),
            ));
        }
    }

    chain
}

fn config_value_type(value: &ConfigValue) -> &'static str {
    match value.reveal() {
        ConfigValue::String(_) => "string",
        ConfigValue::Bool(_) => "bool",
        ConfigValue::Integer(_) => "integer",
        ConfigValue::Float(_) => "float",
        ConfigValue::List(_) => "list",
        ConfigValue::Secret(_) => "string",
    }
}

fn config_key_doc(key: &str) -> Option<&'static str> {
    ConfigSchema::default().doc_for_key(key)
}

fn redact_value_json(key: &str, value: &ConfigValue, show_secrets: bool) -> serde_json::Value {
    if value.is_secret() {
        return if show_secrets {
            config_value_to_json_exposed(value)
        } else {
            "[REDACTED]".into()
        };
    }
    if show_secrets || !is_sensitive_key(key) {
        return config_value_to_json_exposed(value);
    }

    "[REDACTED]".into()
}

pub(crate) fn format_scope(scope: &crate::config::Scope) -> String {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (Some(profile), Some(terminal)) => format!("profile:{profile} terminal:{terminal}"),
        (Some(profile), None) => format!("profile:{profile}"),
        (None, Some(terminal)) => format!("terminal:{terminal}"),
        (None, None) => "global".to_string(),
    }
}

fn contains_sensitive_values(explain: &ConfigExplain) -> bool {
    if is_sensitive_key(&explain.key) {
        return true;
    }

    if explain
        .final_entry
        .as_ref()
        .is_some_and(|entry| entry.value.is_secret())
    {
        return true;
    }

    explain.interpolation.as_ref().is_some_and(|trace| {
        trace.steps.iter().any(|step| {
            step.raw_value.is_secret()
                || step.value.is_secret()
                || is_sensitive_key(&step.placeholder)
        })
    })
}

fn bootstrap_scope_policy(key: &str) -> Option<&'static str> {
    let spec = bootstrap_key_spec(key)?;
    Some(match spec.scope_rule {
        BootstrapScopeRule::GlobalOnly => {
            "global only; terminal and profile scopes are ignored during bootstrap"
        }
        BootstrapScopeRule::GlobalOrTerminal => {
            "global and terminal-only; profile scopes are ignored during bootstrap"
        }
    })
}

pub(crate) fn push_missing_config_key_messages(
    messages: &mut MessageBuffer,
    config: &ResolvedConfig,
    key: &str,
) -> crate::core::CommandError {
    let key = config.display_key(key);
    let key = key.as_str();
    let (title, error) = if let Some((_, entry)) = ConfigSchema::default()
        .entries()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
    {
        let mut message = format!("{key} is not set");
        if let Some(allowed) = entry.allowed_values() {
            message.push_str(&format!(" (allowed: {})", allowed.join(", ")));
        }
        (
            "Configuration value is not set",
            crate::core::CommandError::new(message),
        )
    } else {
        let _ = suggest_config_keys(config, key);
        (
            "Unknown configuration key",
            crate::core::CommandError::new(format!("Unknown config key '{key}'.")).with_action(
                format!(
                    "Run {} to see the keys.",
                    crate::core::CommandScope::OneShot.command("config list")
                ),
            ),
        )
    };
    messages.push_titled(MessageLevel::Error, title, error.render());
    error
}

pub(crate) fn suggest_config_keys(config: &ResolvedConfig, key: &str) -> Vec<String> {
    let key_lc = fold_case(key);
    let schema = ConfigSchema::default();
    let schema_keys = schema.entries().map(|(key, _)| key.to_string());
    let all_keys = config
        .values()
        .keys()
        .chain(config.aliases().keys())
        .cloned()
        .chain(schema_keys)
        .map(|key| config.display_key(&key))
        .filter(|candidate| fold_case(candidate) != key_lc)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<String>>();

    let mut prefix_matches = all_keys
        .iter()
        .filter(|candidate| {
            let candidate_lc = fold_case(candidate);
            candidate_lc.starts_with(&key_lc) || candidate_lc.contains(&key_lc)
        })
        .take(5)
        .cloned()
        .collect::<Vec<String>>();

    if prefix_matches.is_empty() {
        let key_root = key_lc.split('.').next().unwrap_or_default();
        let key_leaf = key_lc.rsplit('.').next().unwrap_or_default();
        let matcher = config_fuzzy_matcher();
        let mut fuzzy_matches = all_keys
            .iter()
            .filter_map(|candidate| {
                let candidate_lc = fold_case(candidate);
                let candidate_root = candidate_lc.split('.').next().unwrap_or_default();
                let candidate_leaf = candidate_lc.rsplit('.').next().unwrap_or_default();
                let same_root = candidate_root == key_root;
                if !same_root {
                    return None;
                }
                let leaf_score = same_root
                    .then(|| matcher.fuzzy_match(candidate_leaf, key_leaf))
                    .flatten();
                let full_score = matcher.fuzzy_match(&candidate_lc, &key_lc);
                let score = leaf_score.or(full_score)?;
                Some((
                    !same_root,
                    leaf_score.is_none(),
                    std::cmp::Reverse(leaf_score.unwrap_or(i64::MIN)),
                    std::cmp::Reverse(full_score.unwrap_or(score)),
                    candidate.matches('.').count(),
                    candidate.len(),
                    candidate.clone(),
                ))
            })
            .collect::<Vec<_>>();
        fuzzy_matches.sort();
        prefix_matches = fuzzy_matches
            .into_iter()
            .take(5)
            .map(|(_, _, _, _, _, _, candidate)| candidate)
            .collect();
    }

    if prefix_matches.is_empty() {
        let key_root = key_lc.split('.').next().unwrap_or_default();
        prefix_matches = all_keys
            .iter()
            .filter(|candidate| {
                fold_case(candidate).split('.').next().unwrap_or_default() == key_root
            })
            .take(5)
            .cloned()
            .collect();
    }

    prefix_matches
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::app::resolve_runtime_config;
    use crate::config::{
        ActiveProfileSource, ConfigLayer, ConfigResolver, ConfigSource, ConfigValue,
        ExplainInterpolation, ExplainInterpolationStep, ResolveOptions, ResolvedValue,
        RuntimeLoadOptions, Scope,
    };
    use crate::ui::build_presentation_defaults_layer;
    use std::collections::BTreeSet;

    fn resolved_config_and_explain(
        key: &str,
        defaults: &[(&str, &str)],
        file: &[(&str, &str)],
        session: &[(&str, &str)],
    ) -> (ResolvedConfig, ConfigExplain) {
        let mut layers = crate::config::LoadedLayers::default();

        let mut defaults_layer = ConfigLayer::default();
        defaults_layer.set("profile.default", "default");
        defaults_layer.set("repl.history.path", "/tmp/osp-config-explain-history.jsonl");
        defaults_layer.set("theme.path", Vec::<String>::new());
        for (entry_key, value) in defaults {
            defaults_layer.set(*entry_key, *value);
        }
        layers.defaults = defaults_layer;

        let mut file_layer = ConfigLayer::default();
        for (entry_key, value) in file {
            file_layer.set(*entry_key, *value);
        }
        layers.file = file_layer;

        let mut session_layer = ConfigLayer::default();
        for (entry_key, value) in session {
            session_layer.set(*entry_key, *value);
        }
        layers.session = session_layer;

        let options = ResolveOptions::default().with_terminal("repl");
        let base = ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .resolve(options.clone())
        .expect("base config should resolve");
        layers.presentation = build_presentation_defaults_layer(&base);
        let config = ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .resolve(options.clone())
        .expect("config should resolve");
        let explain = ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .explain_key(key, options)
        .expect("config explain should resolve");
        (config, explain)
    }

    #[test]
    fn config_explain_redacts_sensitive_values_in_text_and_json_unit() {
        let (config, _) = resolved_config_and_explain("ui.format", &[], &[], &[]);
        let explain = ConfigExplain {
            key: "auth.api_token".to_string(),
            active_profile: "default".to_string(),
            active_profile_source: ActiveProfileSource::DefaultProfile,
            terminal: Some("repl".to_string()),
            known_profiles: BTreeSet::from(["default".to_string()]),
            layers: Vec::new(),
            final_entry: Some(ResolvedValue {
                raw_value: ConfigValue::String("secret-token".to_string()).into_secret(),
                value: ConfigValue::String("secret-token".to_string()).into_secret(),
                source: ConfigSource::Secrets,
                scope: Scope::global(),
                origin: Some("secrets.toml".to_string()),
            }),
            interpolation: Some(ExplainInterpolation {
                template: ConfigValue::String("${auth.api_token}".to_string()),
                steps: vec![ExplainInterpolationStep {
                    placeholder: "auth.api_token".to_string(),
                    raw_value: ConfigValue::String("secret-token".to_string()).into_secret(),
                    value: ConfigValue::String("secret-token".to_string()).into_secret(),
                    source: ConfigSource::Secrets,
                    scope: Scope::global(),
                    origin: Some("secrets.toml".to_string()),
                }],
            }),
        };

        let text = render_config_explain_text(&explain, &config, false);
        let json = config_explain_json(&explain, &config, false);

        assert!(text.contains("[REDACTED]"));
        assert!(text.contains("note: some values are redacted"));
        assert_eq!(json["value"], "[REDACTED]");
        assert_eq!(json["interpolation"]["steps"][0]["value"], "[REDACTED]");
    }

    #[test]
    fn config_explain_helpers_cover_scope_policy_and_key_suggestions_unit() {
        let (config, _) = resolved_config_and_explain(
            "ui.format",
            &[("alias.lookup", "ldap user"), ("ui.format", "json")],
            &[],
            &[],
        );

        assert_eq!(format_scope(&Scope::global()), "global");
        assert_eq!(
            format_scope(&Scope::profile_terminal("ops", "repl")),
            "profile:ops terminal:repl"
        );
        assert!(
            bootstrap_scope_policy("profile.default")
                .is_some_and(|value| value.contains("terminal-only"))
        );
        assert!(bootstrap_scope_policy("ui.format").is_none());

        let suggestions = suggest_config_keys(&config, "ui.for");
        assert!(suggestions.iter().any(|candidate| candidate == "ui.format"));
        let typo_suggestions = suggest_config_keys(&config, "ui.formt");
        assert!(
            typo_suggestions
                .iter()
                .any(|candidate| candidate == "ui.format")
        );
        assert_eq!(config_key_doc("ui.format"), Some("Default output format"));
        assert_eq!(config_key_doc("extensions.demo.token"), None);
    }

    #[test]
    fn config_explain_includes_schema_description_in_text_and_json_unit() {
        let (config, explain) =
            resolved_config_and_explain("ui.format", &[("ui.format", "json")], &[], &[]);

        let text = render_config_explain_text(&explain, &config, false);
        let json = config_explain_json(&explain, &config, false);

        assert!(text.contains("description: Default output format"));
        assert_eq!(json["description"], "Default output format");
    }

    #[test]
    fn explain_runtime_config_matches_resolved_runtime_presentation_defaults_unit() {
        let mut product_defaults = ConfigLayer::default();
        product_defaults.set("profile.default", "default");
        product_defaults.set("repl.history.path", "/tmp/osp-config-explain-history.jsonl");
        product_defaults.set("theme.path", Vec::<String>::new());
        product_defaults.set("ui.presentation", "compact");
        let request = RuntimeConfigRequest::new(None, Some("repl"))
            .with_runtime_load(RuntimeLoadOptions::defaults_only())
            .with_product_defaults(product_defaults);

        let resolved = resolve_runtime_config(request.clone())
            .expect("runtime config should resolve through the shared bootstrap path");
        let resolved_entry = resolved
            .get_value_entry("repl.intro")
            .expect("presentation shaping should seed repl.intro");
        let explain = explain_runtime_config(request, "repl.intro")
            .expect("config explain should use the same shaped runtime path");
        let final_entry = explain
            .final_entry
            .expect("repl.intro should have a winning explained value");

        assert_eq!(final_entry.value.reveal(), resolved_entry.value.reveal());
        assert_eq!(final_entry.source, resolved_entry.source);
    }
}
