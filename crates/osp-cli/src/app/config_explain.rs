use miette::{IntoDiagnostic, Result, WrapErr};
use osp_config::{
    ConfigExplain, ConfigResolver, ConfigValue, ResolveOptions, ResolvedConfig, RuntimeConfigPaths,
    RuntimeDefaults, build_runtime_pipeline, is_bootstrap_only_key,
};
use osp_core::output::OutputFormat;
use osp_ui::messages::MessageBuffer;
use osp_ui::theme::DEFAULT_THEME_NAME;

use crate::cli::ConfigExplainArgs;
use crate::state::AppState;

use super::{DEFAULT_REPL_PROMPT, RuntimeConfigRequest, emit_messages};

pub(crate) fn config_explain_output(
    state: &AppState,
    args: ConfigExplainArgs,
) -> Result<Option<String>> {
    let explain = explain_runtime_config(
        RuntimeConfigRequest::new(
            Some(state.config.resolved().active_profile().to_string()),
            state.config.resolved().terminal(),
        )
        .with_runtime_load(state.launch.runtime_load)
        .with_session_layer(Some(state.session.config_overrides.clone())),
        &args.key,
    )?;

    if explain.final_entry.is_none() && explain.layers.is_empty() {
        let suggestions = suggest_config_keys(state.config.resolved(), &args.key);
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        if !suggestions.is_empty() {
            messages.info(format!("did you mean: {}", suggestions.join(", ")));
        }
        emit_messages(state, &messages);
        return Ok(None);
    }

    if matches!(state.ui.render_settings.format, OutputFormat::Json) {
        let payload = config_explain_json(&explain, args.show_secrets);
        return Ok(Some(format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).into_diagnostic()?
        )));
    }

    Ok(Some(render_config_explain_text(
        &explain,
        args.show_secrets,
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
    let pipeline = build_runtime_pipeline(
        defaults.to_layer(),
        &paths,
        request.runtime_load,
        None,
        request.session_layer,
    );

    let layers = pipeline
        .load_layers()
        .into_diagnostic()
        .wrap_err("config layer loading failed")?;
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
        .wrap_err("config explain failed")
}

pub(crate) fn render_config_explain_text(explain: &ConfigExplain, show_secrets: bool) -> String {
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

    out.push_str("context:\n");
    out.push_str(&format!("  active_profile: {}\n", explain.active_profile));
    out.push_str(&format!(
        "  terminal: {}\n\n",
        explain.terminal.as_deref().unwrap_or("none")
    ));

    let precedence = effective_precedence_chain(explain);
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
                "  ${{{}}} -> {} (from {}, {})\n",
                step.placeholder,
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
    for (is_winner, source, scope, origin, value) in effective_precedence_chain(explain) {
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

    serde_json::Value::Object(root)
}

fn effective_precedence_chain(
    explain: &ConfigExplain,
) -> Vec<(bool, String, String, Option<String>, ConfigValue)> {
    let winner_source = explain.final_entry.as_ref().map(|entry| entry.source);
    let mut chain = Vec::new();

    for layer in &explain.layers {
        let mut candidates = layer
            .candidates
            .iter()
            .filter_map(|candidate| candidate.rank.map(|rank| (rank, candidate)))
            .collect::<Vec<(u8, &osp_config::ExplainCandidate)>>();
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

pub(crate) fn format_scope(scope: &osp_config::Scope) -> String {
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
        trace
            .steps
            .iter()
            .any(|step| step.value.is_secret() || is_sensitive_key(&step.placeholder))
    })
}

fn suggest_config_keys(config: &ResolvedConfig, key: &str) -> Vec<String> {
    let key_lc = key.to_ascii_lowercase();
    let mut prefix_matches = config
        .values()
        .keys()
        .filter(|candidate| candidate.starts_with(&key_lc) || candidate.contains(&key_lc))
        .take(5)
        .cloned()
        .collect::<Vec<String>>();

    if prefix_matches.is_empty() {
        prefix_matches = config
            .values()
            .keys()
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
