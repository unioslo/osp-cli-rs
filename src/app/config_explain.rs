use crate::config::{
    BootstrapScopeRule, ConfigExplain, ConfigResolver, ConfigSchema, ConfigValue, ResolveOptions,
    ResolvedConfig, RuntimeConfigPaths, RuntimeDefaults, bootstrap_key_spec,
    build_runtime_pipeline, is_bootstrap_only_key,
};
use crate::core::output::OutputFormat;
use crate::ui::messages::MessageBuffer;
use crate::ui::theme::DEFAULT_THEME_NAME;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::app::{RuntimeContext, UiState};
use crate::cli::ConfigExplainArgs;
use crate::ui::presentation::{build_presentation_defaults_layer, explain_presentation_effect};

use super::{
    CliCommandResult, DEFAULT_REPL_PROMPT, RuntimeConfigRequest, document_from_json,
    document_from_text,
};

pub(crate) struct ConfigExplainContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) session_layer: &'a crate::config::ConfigLayer,
    pub(crate) runtime_load: crate::config::RuntimeLoadOptions,
}

pub(crate) fn config_explain_result(
    context: &ConfigExplainContext<'_>,
    args: ConfigExplainArgs,
) -> Result<CliCommandResult> {
    let explain = explain_runtime_config(
        RuntimeConfigRequest::new(
            context.context.profile_override().map(str::to_owned),
            Some(context.context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(context.runtime_load)
        .with_session_layer(Some(context.session_layer.clone())),
        &args.key,
    )?;

    if explain.final_entry.is_none() && explain.layers.is_empty() {
        let suggestions = suggest_config_keys(context.config, &args.key);
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        if !suggestions.is_empty() {
            messages.info(format!("did you mean: {}", suggestions.join(", ")));
        }
        return Ok(CliCommandResult {
            exit_code: 1,
            messages,
            output: None,
            stderr_text: None,
            failure_report: None,
        });
    }

    if matches!(context.ui.render_settings.format, OutputFormat::Json) {
        let payload = config_explain_json(&explain, context.config, args.show_secrets);
        return Ok(CliCommandResult::document(document_from_json(payload)));
    }

    Ok(CliCommandResult::document(document_from_text(
        &render_config_explain_text(&explain, context.config, args.show_secrets),
    )))
}

pub(crate) fn config_value_to_json(value: &ConfigValue) -> serde_json::Value {
    if value.is_secret() {
        return "[REDACTED]".into();
    }
    config_value_to_json_exposed(value)
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
    let defaults = RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
    let paths = RuntimeConfigPaths::discover();
    let base_pipeline = build_runtime_pipeline(
        defaults.to_layer(),
        None,
        &paths,
        request.runtime_load,
        None,
        request.session_layer.clone(),
    );
    // Explain mirrors the runtime bootstrap path. We resolve once to discover the selected
    // presentation preset, synthesize that into a normal config layer, and then explain against
    // the fully shaped config so `config explain` matches the runtime users actually get.
    let base_config = base_pipeline
        .resolve(ResolveOptions {
            profile_override: request.profile_override.clone(),
            terminal: request.terminal.clone(),
        })
        .into_diagnostic()
        .wrap_err("failed to resolve config before explaining presentation defaults")?;
    let pipeline = build_runtime_pipeline(
        defaults.to_layer(),
        Some(build_presentation_defaults_layer(&base_config)),
        &paths,
        request.runtime_load,
        None,
        request.session_layer,
    );

    let layers = pipeline
        .load_layers()
        .into_diagnostic()
        .wrap_err("failed to load config layers (file, env, secrets)")?;
    let resolver = ConfigResolver::from_loaded_layers(layers);
    resolver
        .explain_key(
            key,
            ResolveOptions {
                profile_override: request.profile_override,
                terminal: request.terminal,
            },
        )
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to explain config key `{key}`"))
}

pub(crate) fn render_config_explain_text(
    explain: &ConfigExplain,
    config: &ResolvedConfig,
    show_secrets: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("key: {}\n", explain.key));
    out.push_str(&format!(
        "phase: {}\n",
        if is_bootstrap_only_key(&explain.key) {
            "bootstrap"
        } else {
            "runtime"
        }
    ));

    if let Some(final_entry) = &explain.final_entry {
        let value_display = display_value(&explain.key, &final_entry.value, show_secrets);
        out.push_str(&format!(
            "value: {} ({})\n\n",
            value_display,
            config_value_type(&final_entry.value)
        ));
        out.push_str("winner:\n");
        out.push_str(&format!("  source: {}\n", final_entry.source));
        out.push_str(&format!("  scope: {}\n", format_scope(&final_entry.scope)));
        out.push_str(&format!(
            "  origin: {}\n\n",
            final_entry.origin.as_deref().unwrap_or("-")
        ));
    } else {
        out.push_str("value: not set\n\n");
    }

    if let Some(effect) = explain_presentation_effect(config, &explain.key) {
        out.push_str("presentation:\n");
        out.push_str(&format!("  preset: {}\n", effect.preset.as_config_value()));
        out.push_str(&format!("  preset_source: {}\n", effect.preset_source));
        out.push_str(&format!(
            "  preset_scope: {}\n",
            format_scope(&effect.preset_scope)
        ));
        out.push_str(&format!(
            "  preset_origin: {}\n",
            effect.preset_origin.as_deref().unwrap_or("-")
        ));
        out.push_str(&format!(
            "  seeded_value: {} ({})\n",
            display_value(&explain.key, &effect.seeded_value, show_secrets),
            config_value_type(&effect.seeded_value)
        ));
        out.push_str(
            "  note: key kept its builtin default, so ui.presentation seeds the resolved UI value\n\n",
        );
    }

    out.push_str("context:\n");
    out.push_str(&format!("  active_profile: {}\n", explain.active_profile));
    // Runtime and bootstrap explains both resolve through the same active
    // profile decision, so the source is always part of the contract now.
    out.push_str(&format!(
        "  active_profile_source: {}\n",
        explain.active_profile_source.as_str()
    ));
    out.push_str(&format!(
        "  terminal: {}\n\n",
        explain.terminal.as_deref().unwrap_or("none")
    ));

    if let Some(policy) = bootstrap_scope_policy(&explain.key) {
        out.push_str(&format!("bootstrap_scope_policy: {policy}\n\n"));
    }

    let precedence = precedence_chain(explain);
    if !precedence.is_empty() {
        out.push_str("candidates (in priority order):\n");
        for (is_winner, source, scope, origin, value) in precedence {
            let marker = if is_winner { "  ✅" } else { "   " };
            out.push_str(&format!(
                "{marker} {source} ({scope}) = {}",
                display_value(&explain.key, &value, show_secrets),
            ));
            if let Some(origin_hint) = origin {
                out.push_str(&format!(" [{origin_hint}]"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if let Some(interpolation) = &explain.interpolation {
        out.push_str("interpolation:\n");
        out.push_str(&format!(
            "  template: {}\n",
            display_value(
                &explain.key,
                &ConfigValue::String(interpolation.template.clone()),
                show_secrets
            )
        ));
        for step in &interpolation.steps {
            out.push_str(&format!(
                "  ${{{}}} raw={} final={} (from {}, {})\n",
                step.placeholder,
                display_value(&step.placeholder, &step.raw_value, show_secrets),
                display_value(&step.placeholder, &step.value, show_secrets),
                step.source,
                format_scope(&step.scope),
            ));
        }
        if !show_secrets && contains_sensitive_values(explain) {
            out.push_str("  note: some values are redacted; pass --show-secrets to display them\n");
        }
    }

    out
}

pub(crate) fn config_explain_json(
    explain: &ConfigExplain,
    config: &ResolvedConfig,
    show_secrets: bool,
) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    root.insert("key".to_string(), explain.key.clone().into());
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
            redact_value_json(
                &explain.key,
                &ConfigValue::String(interpolation.template.clone()),
                show_secrets,
            ),
        );
        let mut steps = Vec::new();
        for step in &interpolation.steps {
            let mut item = serde_json::Map::new();
            item.insert("placeholder".to_string(), step.placeholder.clone().into());
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

fn display_value(key: &str, value: &ConfigValue, show_secrets: bool) -> String {
    if value.is_secret() {
        return if show_secrets {
            match value.reveal() {
                ConfigValue::String(v) => v.clone(),
                _ => config_value_to_json_exposed(value).to_string(),
            }
        } else {
            "[REDACTED]".to_string()
        };
    }

    if show_secrets || !is_sensitive_key(key) {
        return match value.reveal() {
            ConfigValue::String(v) => v.clone(),
            _ => config_value_to_json_exposed(value).to_string(),
        };
    }

    "[REDACTED]".to_string()
}

pub(crate) fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("apikey")
        || normalized.contains("api_key")
        || normalized.contains("access_key")
        || normalized.contains("private_key")
        || normalized.contains("ssh_key")
        || normalized.contains("client_secret")
        || normalized.contains("bearer")
        || normalized.contains("jwt")
        || normalized.ends_with(".key")
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

fn suggest_config_keys(config: &ResolvedConfig, key: &str) -> Vec<String> {
    let key_lc = key.to_ascii_lowercase();
    let schema = ConfigSchema::default();
    let schema_keys = schema.entries().map(|(key, _)| key.to_string());
    let all_keys = config
        .values()
        .keys()
        .chain(config.aliases().keys())
        .cloned()
        .chain(schema_keys)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<String>>();

    let mut prefix_matches = all_keys
        .iter()
        .filter(|candidate| candidate.starts_with(&key_lc) || candidate.contains(&key_lc))
        .take(5)
        .cloned()
        .collect::<Vec<String>>();

    if prefix_matches.is_empty() {
        prefix_matches = all_keys
            .iter()
            .filter(|candidate| {
                let left = candidate.split('.').next().unwrap_or_default();
                let right = key_lc.split('.').next().unwrap_or_default();
                left == right
            })
            .take(5)
            .cloned()
            .collect();
    }

    prefix_matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ActiveProfileSource, ConfigLayer, ConfigResolver, ConfigSource, ConfigValue,
        ExplainInterpolation, ExplainInterpolationStep, ResolveOptions, ResolvedValue, Scope,
    };
    use std::collections::BTreeSet;

    fn resolved_config_and_explain(
        key: &str,
        defaults: &[(&str, &str)],
        file: &[(&str, &str)],
        session: &[(&str, &str)],
    ) -> (ResolvedConfig, ConfigExplain) {
        let mut resolver = ConfigResolver::default();

        let mut defaults_layer = ConfigLayer::default();
        defaults_layer.set("profile.default", "default");
        for (entry_key, value) in defaults {
            defaults_layer.set(*entry_key, *value);
        }
        resolver.set_defaults(defaults_layer);

        let mut file_layer = ConfigLayer::default();
        for (entry_key, value) in file {
            file_layer.set(*entry_key, *value);
        }
        resolver.set_file(file_layer);

        let mut session_layer = ConfigLayer::default();
        for (entry_key, value) in session {
            session_layer.set(*entry_key, *value);
        }
        resolver.set_session(session_layer);

        let options = ResolveOptions::default().with_terminal("repl");
        let base = resolver
            .resolve(options.clone())
            .expect("base config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        let config = resolver
            .resolve(options.clone())
            .expect("config should resolve");
        let explain = resolver
            .explain_key(key, options)
            .expect("config explain should resolve");
        (config, explain)
    }

    #[test]
    fn config_explain_text_surfaces_presentation_effect_unit() {
        let (config, explain) = resolved_config_and_explain(
            "ui.chrome.frame",
            &[("ui.presentation", "expressive")],
            &[],
            &[("ui.presentation", "austere")],
        );

        let rendered = render_config_explain_text(&explain, &config, false);

        assert!(rendered.contains("value: none (string)"));
        assert!(rendered.contains("presentation:"));
        assert!(rendered.contains("preset: austere"));
        assert!(rendered.contains("preset_source: session"));
        assert!(rendered.contains("seeded_value: none (string)"));
    }

    #[test]
    fn config_explain_json_surfaces_presentation_effect_unit() {
        let (config, explain) = resolved_config_and_explain(
            "ui.chrome.frame",
            &[("ui.presentation", "expressive")],
            &[],
            &[("ui.presentation", "austere")],
        );

        let payload = config_explain_json(&explain, &config, false);

        assert_eq!(payload["value"], "none");
        assert_eq!(payload["presentation"]["preset"], "austere");
        assert_eq!(payload["presentation"]["preset_source"], "session");
        assert_eq!(payload["presentation"]["seeded_value"], "none");
        assert_eq!(payload["presentation"]["seeded_value_type"], "string");
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
                template: "${auth.api_token}".to_string(),
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
    }
}
