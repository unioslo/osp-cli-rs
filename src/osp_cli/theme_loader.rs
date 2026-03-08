use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::osp_config::{ConfigSource, ResolvedConfig};
use crate::osp_ui::style::is_valid_style_spec;
use crate::osp_ui::theme::{
    ThemeDefinition, ThemeOverrides, ThemePalette, builtin_themes, display_name_from_id,
    find_builtin_theme, normalize_theme_name,
};

#[derive(Debug, Clone)]
pub(crate) struct ThemeLoadIssue {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ThemeCatalog {
    pub(crate) entries: BTreeMap<String, ThemeEntry>,
    pub(crate) issues: Vec<ThemeLoadIssue>,
}

impl ThemeCatalog {
    pub(crate) fn ids(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub(crate) fn resolve(&self, input: &str) -> Option<&ThemeEntry> {
        let normalized = normalize_theme_name(input);
        if normalized.is_empty() {
            return None;
        }
        self.entries.get(&normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeSource {
    Builtin,
    Custom,
}

#[derive(Debug, Clone)]
pub(crate) struct ThemeEntry {
    pub(crate) theme: ThemeDefinition,
    pub(crate) source: ThemeSource,
    pub(crate) origin: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
struct CustomThemeLoad {
    themes: Vec<ThemeDefinition>,
    origins: BTreeMap<String, PathBuf>,
    issues: Vec<ThemeLoadIssue>,
}

#[derive(Debug, Clone)]
struct ThemeSpec {
    id: String,
    name: String,
    base: Option<String>,
    palette: ThemePaletteFile,
    overrides: ThemeOverrides,
}

struct ThemePathSelection {
    paths: Vec<PathBuf>,
    explicit: bool,
}

pub(crate) fn load_theme_catalog(config: &ResolvedConfig) -> ThemeCatalog {
    let custom = load_custom_themes(config);
    let mut entries: BTreeMap<String, ThemeEntry> = BTreeMap::new();
    for theme in builtin_themes() {
        entries.insert(
            theme.id.clone(),
            ThemeEntry {
                theme,
                source: ThemeSource::Builtin,
                origin: None,
            },
        );
    }

    let mut issues = custom.issues;
    for theme in custom.themes {
        let origin = custom.origins.get(&theme.id).cloned();
        if let Some(path) = origin.clone()
            && entries.contains_key(&theme.id)
        {
            issues.push(ThemeLoadIssue {
                path,
                message: format!("custom theme overrides builtin: {}", theme.id),
            });
        }
        entries.insert(
            theme.id.clone(),
            ThemeEntry {
                theme,
                source: ThemeSource::Custom,
                origin,
            },
        );
    }

    ThemeCatalog { entries, issues }
}

fn load_custom_themes(config: &ResolvedConfig) -> CustomThemeLoad {
    let mut issues = Vec::new();
    let mut specs: BTreeMap<String, ThemeSpec> = BTreeMap::new();
    let mut origins: BTreeMap<String, PathBuf> = BTreeMap::new();

    let selection = resolve_theme_paths(config);
    for dir in selection.paths {
        if !dir.is_dir() {
            if selection.explicit {
                issues.push(ThemeLoadIssue {
                    path: dir,
                    message: "theme path is not a directory".to_string(),
                });
            }
            continue;
        }

        let mut entries = match fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .collect::<Vec<_>>(),
            Err(err) => {
                if selection.explicit {
                    issues.push(ThemeLoadIssue {
                        path: dir,
                        message: format!("failed to read theme directory: {err}"),
                    });
                }
                continue;
            }
        };
        entries.sort();

        for path in entries {
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }

            match parse_theme_spec(&path) {
                Ok(spec) => {
                    let theme = match resolve_theme_spec(
                        &spec.id,
                        &specs,
                        &mut BTreeMap::new(),
                        &mut Vec::new(),
                    ) {
                        Ok(theme) => apply_theme_overrides(theme, &spec),
                        Err(_) => {
                            // The full recursive resolve runs after all specs are known.
                            // Validate only the direct patch values at parse time here.
                            apply_theme_overrides(
                                empty_theme(&spec.id, &spec.name, spec.base.clone()),
                                &spec,
                            )
                        }
                    };
                    for message in validate_theme_specs(&theme) {
                        issues.push(ThemeLoadIssue {
                            path: path.clone(),
                            message,
                        });
                    }
                    if let Some(existing) = origins.get(&spec.id) {
                        issues.push(ThemeLoadIssue {
                            path: path.clone(),
                            message: format!(
                                "theme id collision: {} overridden (previous: {})",
                                spec.id,
                                existing.display()
                            ),
                        });
                    }
                    origins.insert(spec.id.clone(), path.clone());
                    specs.insert(spec.id.clone(), spec);
                }
                Err(err) => {
                    issues.push(ThemeLoadIssue { path, message: err });
                }
            }
        }
    }

    let mut resolved = BTreeMap::new();
    for id in specs.keys().cloned().collect::<Vec<_>>() {
        let mut stack = Vec::new();
        match resolve_theme_spec(&id, &specs, &mut resolved, &mut stack) {
            Ok(_) => {}
            Err(message) => {
                if let Some(path) = origins.get(&id).cloned() {
                    issues.push(ThemeLoadIssue { path, message });
                }
            }
        }
    }

    CustomThemeLoad {
        themes: resolved.into_values().collect(),
        origins,
        issues,
    }
}

pub(crate) fn log_theme_issues(issues: &[ThemeLoadIssue]) {
    for issue in issues {
        tracing::warn!(path = %issue.path.display(), "{message}", message = issue.message);
    }
}

fn resolve_theme_paths(config: &ResolvedConfig) -> ThemePathSelection {
    if let Some(paths) = config.get_string_list("theme.path") {
        let explicit = config
            .get_value_entry("theme.path")
            .map(|entry| {
                !matches!(
                    entry.source,
                    ConfigSource::BuiltinDefaults | ConfigSource::Derived
                )
            })
            .unwrap_or(false);
        return ThemePathSelection {
            paths: normalize_theme_paths(paths),
            explicit,
        };
    }
    ThemePathSelection {
        paths: default_theme_paths(),
        explicit: false,
    }
}

fn normalize_theme_paths(paths: Vec<String>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter_map(|raw| expand_theme_path(&raw))
        .collect()
}

fn expand_theme_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return Some(PathBuf::from(home));
    }

    if let Some(stripped) = trimmed.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return Some(PathBuf::from(home).join(stripped));
    }

    Some(PathBuf::from(trimmed))
}

fn default_theme_paths() -> Vec<PathBuf> {
    crate::osp_config::default_config_root_dir()
        .map(|mut root| {
            root.push("themes");
            root
        })
        .into_iter()
        .collect()
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    base: Option<String>,
    id: Option<String>,
    name: Option<String>,
    palette: Option<ThemePaletteFile>,
    #[serde(default)]
    overrides: ThemeOverridesFile,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ThemePaletteFile {
    text: Option<String>,
    muted: Option<String>,
    accent: Option<String>,
    info: Option<String>,
    warning: Option<String>,
    success: Option<String>,
    error: Option<String>,
    border: Option<String>,
    title: Option<String>,
    selection: Option<String>,
    link: Option<String>,
    bg: Option<String>,
    bg_alt: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ThemeOverridesFile {
    value_number: Option<String>,
    repl_completion_text: Option<String>,
    repl_completion_background: Option<String>,
    repl_completion_highlight: Option<String>,
}

fn parse_theme_spec(path: &Path) -> Result<ThemeSpec, String> {
    let raw =
        fs::read_to_string(path).map_err(|err| format!("failed to read theme file: {err}"))?;
    let parsed: ThemeFile =
        toml::from_str(&raw).map_err(|err| format!("failed to parse toml: {err}"))?;

    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let mut id = parsed
        .id
        .as_deref()
        .map(normalize_theme_name)
        .unwrap_or_default();
    if id.is_empty() {
        id = normalize_theme_name(stem);
    }
    if id.is_empty() {
        return Err("theme id is empty".to_string());
    }

    let name = parsed
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| display_name_from_id(&id));

    let base = parsed
        .base
        .as_deref()
        .map(normalize_theme_name)
        .filter(|value| !value.is_empty())
        .filter(|value| value != "none");

    let overrides = ThemeOverrides {
        value_number: parsed.overrides.value_number,
        repl_completion_text: parsed.overrides.repl_completion_text,
        repl_completion_background: parsed.overrides.repl_completion_background,
        repl_completion_highlight: parsed.overrides.repl_completion_highlight,
    };

    Ok(ThemeSpec {
        id,
        name,
        base,
        palette: parsed.palette.unwrap_or_default(),
        overrides,
    })
}

fn resolve_theme_spec(
    id: &str,
    specs: &BTreeMap<String, ThemeSpec>,
    resolved: &mut BTreeMap<String, ThemeDefinition>,
    stack: &mut Vec<String>,
) -> Result<ThemeDefinition, String> {
    if let Some(theme) = resolved.get(id) {
        return Ok(theme.clone());
    }
    if stack.iter().any(|entry| entry == id) {
        stack.push(id.to_string());
        return Err(format!("theme base cycle detected: {}", stack.join(" -> ")));
    }

    let spec = specs
        .get(id)
        .ok_or_else(|| format!("theme missing during resolution: {id}"))?;
    stack.push(id.to_string());

    let base_theme = match spec.base.as_deref() {
        Some(base) if specs.contains_key(base) => {
            Some(resolve_theme_spec(base, specs, resolved, stack)?)
        }
        Some(base) => find_builtin_theme(base)
            .ok_or_else(|| format!("unknown base theme: {base}"))
            .map(Some)?,
        None => None,
    };

    let theme = apply_theme_overrides(
        base_theme.unwrap_or_else(|| empty_theme(&spec.id, &spec.name, spec.base.clone())),
        spec,
    );
    stack.pop();
    resolved.insert(id.to_string(), theme.clone());
    Ok(theme)
}

fn empty_theme(id: &str, name: &str, base: Option<String>) -> ThemeDefinition {
    ThemeDefinition::new(id, name, base, empty_palette(), ThemeOverrides::default())
}

fn apply_theme_overrides(theme: ThemeDefinition, spec: &ThemeSpec) -> ThemeDefinition {
    let mut palette = theme.palette.clone();
    if let Some(value) = spec.palette.text.as_ref() {
        palette.text = value.clone();
    }
    if let Some(value) = spec.palette.muted.as_ref() {
        palette.muted = value.clone();
    }
    if let Some(value) = spec.palette.accent.as_ref() {
        palette.accent = value.clone();
    }
    if let Some(value) = spec.palette.info.as_ref() {
        palette.info = value.clone();
    }
    if let Some(value) = spec.palette.warning.as_ref() {
        palette.warning = value.clone();
    }
    if let Some(value) = spec.palette.success.as_ref() {
        palette.success = value.clone();
    }
    if let Some(value) = spec.palette.error.as_ref() {
        palette.error = value.clone();
    }
    if let Some(value) = spec.palette.border.as_ref() {
        palette.border = value.clone();
    }
    if let Some(value) = spec.palette.title.as_ref() {
        palette.title = value.clone();
    }
    if let Some(value) = spec.palette.selection.as_ref() {
        palette.selection = value.clone();
    }
    if let Some(value) = spec.palette.link.as_ref() {
        palette.link = value.clone();
    }
    if let Some(value) = spec.palette.bg.as_ref() {
        palette.bg = Some(value.clone());
    }
    if let Some(value) = spec.palette.bg_alt.as_ref() {
        palette.bg_alt = Some(value.clone());
    }

    ThemeDefinition::new(
        spec.id.clone(),
        spec.name.clone(),
        spec.base.clone(),
        palette,
        spec.overrides.clone(),
    )
}

fn validate_theme_specs(theme: &ThemeDefinition) -> Vec<String> {
    let mut issues = Vec::new();

    check_spec(&mut issues, "palette.text", &theme.palette.text);
    check_spec(&mut issues, "palette.muted", &theme.palette.muted);
    check_spec(&mut issues, "palette.accent", &theme.palette.accent);
    check_spec(&mut issues, "palette.info", &theme.palette.info);
    check_spec(&mut issues, "palette.warning", &theme.palette.warning);
    check_spec(&mut issues, "palette.success", &theme.palette.success);
    check_spec(&mut issues, "palette.error", &theme.palette.error);
    check_spec(&mut issues, "palette.border", &theme.palette.border);
    check_spec(&mut issues, "palette.title", &theme.palette.title);
    check_spec(&mut issues, "palette.selection", &theme.palette.selection);
    check_spec(&mut issues, "palette.link", &theme.palette.link);
    if let Some(value) = &theme.palette.bg {
        check_spec(&mut issues, "palette.bg", value);
    }
    if let Some(value) = &theme.palette.bg_alt {
        check_spec(&mut issues, "palette.bg_alt", value);
    }
    if let Some(value) = &theme.overrides.value_number {
        check_spec(&mut issues, "overrides.value_number", value);
    }
    if let Some(value) = &theme.overrides.repl_completion_text {
        check_spec(&mut issues, "overrides.repl_completion_text", value);
    }
    if let Some(value) = &theme.overrides.repl_completion_background {
        check_spec(&mut issues, "overrides.repl_completion_background", value);
    }
    if let Some(value) = &theme.overrides.repl_completion_highlight {
        check_spec(&mut issues, "overrides.repl_completion_highlight", value);
    }

    issues
}

fn check_spec(issues: &mut Vec<String>, key: &str, value: &str) {
    if is_valid_color_spec(value) {
        return;
    }
    issues.push(format!("invalid color spec for {key}: {value}"));
}

fn is_valid_color_spec(value: &str) -> bool {
    is_valid_style_spec(value)
}

fn empty_palette() -> ThemePalette {
    ThemePalette {
        text: String::new(),
        muted: String::new(),
        accent: String::new(),
        info: String::new(),
        warning: String::new(),
        success: String::new(),
        error: String::new(),
        border: String::new(),
        title: String::new(),
        selection: String::new(),
        link: String::new(),
        bg: None,
        bg_alt: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ThemeCatalog, ThemePaletteFile, ThemeSource, ThemeSpec, apply_theme_overrides,
        default_theme_paths, empty_theme, expand_theme_path, is_valid_color_spec,
        load_theme_catalog, log_theme_issues, normalize_theme_paths, parse_theme_spec,
        resolve_theme_spec,
    };
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn resolved_config_with_theme_paths(paths: Vec<String>) -> crate::osp_config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut file = ConfigLayer::default();
        file.set("theme.path", paths);

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver.set_file(file);
        resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("theme test config should resolve")
    }

    #[test]
    fn theme_file_defaults_id_and_name_from_file_stem() {
        let dir = std::env::temp_dir().join("osp-theme-loader-test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("solarized-dark.toml");
        fs::write(
            &path,
            r##"
[palette]
text = "#eee8d5"
muted = "#93a1a1"
accent = "#268bd2"
info = "#2aa198"
warning = "#b58900"
success = "#859900"
error = "bold #dc322f"
border = "#586e75"
title = "#586e75"
"##,
        )
        .expect("theme file should be written");

        let spec = parse_theme_spec(&path).expect("theme should parse");
        let theme =
            apply_theme_overrides(empty_theme(&spec.id, &spec.name, spec.base.clone()), &spec);
        assert_eq!(theme.id, "solarized-dark");
        assert_eq!(theme.name, "Solarized Dark");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_file_inherits_from_base() {
        let dir = std::env::temp_dir().join("osp-theme-loader-test-base");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("custom.toml");
        fs::write(
            &path,
            r##"
base = "dracula"

[palette]
accent = "#123456"
"##,
        )
        .expect("theme file should be written");

        let spec = parse_theme_spec(&path).expect("theme should parse");
        let mut specs = BTreeMap::new();
        specs.insert(spec.id.clone(), spec);
        let theme = resolve_theme_spec("custom", &specs, &mut BTreeMap::new(), &mut Vec::new())
            .expect("theme should resolve");
        assert_eq!(theme.palette.accent, "#123456");
        assert_eq!(theme.palette.text, "#f8f8f2");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_theme_can_inherit_from_custom_base() {
        let mut specs = BTreeMap::new();
        specs.insert(
            "brand-base".to_string(),
            ThemeSpec {
                id: "brand-base".to_string(),
                name: "Brand Base".to_string(),
                base: Some("nord".to_string()),
                palette: ThemePaletteFile {
                    accent: Some("#123456".to_string()),
                    ..ThemePaletteFile::default()
                },
                overrides: Default::default(),
            },
        );
        specs.insert(
            "brand-child".to_string(),
            ThemeSpec {
                id: "brand-child".to_string(),
                name: "Brand Child".to_string(),
                base: Some("brand-base".to_string()),
                palette: ThemePaletteFile {
                    warning: Some("#abcdef".to_string()),
                    ..ThemePaletteFile::default()
                },
                overrides: Default::default(),
            },
        );

        let theme =
            resolve_theme_spec("brand-child", &specs, &mut BTreeMap::new(), &mut Vec::new())
                .expect("custom base chain should resolve");

        assert_eq!(theme.palette.accent, "#123456");
        assert_eq!(theme.palette.warning, "#abcdef");
        assert_eq!(theme.palette.text, "#d8dee9");
    }

    #[test]
    fn color_spec_validation_accepts_known_tokens() {
        assert!(is_valid_color_spec(""));
        assert!(is_valid_color_spec("bold #ff00ff"));
        assert!(is_valid_color_spec("bright-blue"));
    }

    #[test]
    fn color_spec_validation_rejects_unknown_tokens() {
        assert!(!is_valid_color_spec("nope"));
        assert!(!is_valid_color_spec("#12345"));
    }

    #[test]
    fn theme_catalog_resolve_normalizes_input_and_rejects_blank_unit() {
        let mut catalog = ThemeCatalog::default();
        catalog.entries.insert(
            "rose-pine".to_string(),
            super::ThemeEntry {
                theme: empty_theme("rose-pine", "Rose Pine", None),
                source: ThemeSource::Builtin,
                origin: None,
            },
        );

        assert!(catalog.resolve("  ").is_none());
        assert!(catalog.resolve("Rose Pine").is_some());
        assert_eq!(catalog.ids(), vec!["rose-pine".to_string()]);
    }

    #[test]
    fn theme_path_helpers_expand_home_and_drop_blank_entries_unit() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let original = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", "/tmp/theme-home") };

        assert_eq!(expand_theme_path("   "), None);
        assert_eq!(
            expand_theme_path("~"),
            Some(std::path::PathBuf::from("/tmp/theme-home"))
        );
        assert_eq!(
            expand_theme_path("~/themes"),
            Some(std::path::PathBuf::from("/tmp/theme-home/themes"))
        );
        assert_eq!(
            normalize_theme_paths(vec![" ".to_string(), "~/themes".to_string()]),
            vec![std::path::PathBuf::from("/tmp/theme-home/themes")]
        );

        match original {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn theme_catalog_load_reports_invalid_specs_and_preserves_custom_origins_unit() {
        let root = unique_temp_dir("osp-theme-loader-catalog");
        let themes_dir = root.join("themes");
        let missing_dir = root.join("missing");
        let dracula_path = themes_dir.join("dracula.toml");
        let broken_path = themes_dir.join("broken.toml");
        let cycle_a_path = themes_dir.join("cycle-a.toml");
        let cycle_b_path = themes_dir.join("cycle-b.toml");
        let dupe_a_path = themes_dir.join("dupe-a.toml");
        let dupe_b_path = themes_dir.join("dupe-b.toml");
        fs::create_dir_all(&themes_dir).expect("themes dir should be created");

        fs::write(
            &dracula_path,
            r##"
[palette]
accent = "#123456"
"##,
        )
        .expect("override theme should be written");
        fs::write(&broken_path, "not = [valid").expect("broken theme writes");
        fs::write(
            &cycle_a_path,
            r##"
id = "cycle-a"
base = "cycle-b"
"##,
        )
        .expect("cycle a writes");
        fs::write(
            &cycle_b_path,
            r##"
id = "cycle-b"
base = "cycle-a"
"##,
        )
        .expect("cycle b writes");
        fs::write(
            &dupe_a_path,
            r##"
id = "dupe"
[palette]
text = "bogus"
selection = "#111111"
link = "#222222"
bg = "#000000"
bg_alt = "#010101"

[overrides]
value_number = "broken"
repl_completion_text = "#eeeeee"
repl_completion_background = "#111111"
repl_completion_highlight = "bad"
"##,
        )
        .expect("dupe a writes");
        fs::write(
            &dupe_b_path,
            r##"
id = "dupe"
name = "Dupe Final"
base = "none"
[palette]
text = "#ffffff"
"##,
        )
        .expect("dupe b writes");

        let config = resolved_config_with_theme_paths(vec![
            missing_dir.display().to_string(),
            themes_dir.display().to_string(),
        ]);
        let catalog = load_theme_catalog(&config);

        let dracula = catalog
            .resolve("dracula")
            .expect("custom builtin override should resolve");
        assert_eq!(dracula.source, ThemeSource::Custom);
        assert_eq!(dracula.theme.palette.accent, "#123456");
        assert_eq!(dracula.origin.as_deref(), Some(dracula_path.as_path()));

        let dupe = catalog
            .resolve("dupe")
            .expect("latest duplicate should win");
        assert_eq!(dupe.theme.name, "Dupe Final");
        assert_eq!(dupe.origin.as_deref(), Some(dupe_b_path.as_path()));

        let messages = catalog
            .issues
            .iter()
            .map(|issue| issue.message.clone())
            .collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("theme path is not a directory"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("custom theme overrides builtin: dracula"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("failed to parse toml"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("theme id collision: dupe overridden"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("theme base cycle detected"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("invalid color spec for palette.text"))
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("invalid color spec for overrides.value_number"))
        );

        log_theme_issues(&catalog.issues);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_theme_paths_tracks_home_config_root_unit() {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let original_home = std::env::var("HOME").ok();
        let original_xdg_config_home = std::env::var("XDG_CONFIG_HOME").ok();
        unsafe { std::env::set_var("HOME", "/tmp/osp-theme-loader-home") };
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };

        assert_eq!(
            default_theme_paths(),
            vec![Path::new("/tmp/osp-theme-loader-home/.config/osp/themes").to_path_buf()]
        );

        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/osp-theme-loader-xdg") };
        assert_eq!(
            default_theme_paths(),
            vec![Path::new("/tmp/osp-theme-loader-xdg/osp/themes").to_path_buf()]
        );

        match original_home {
            Some(value) => unsafe { std::env::set_var("HOME", value) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        match original_xdg_config_home {
            Some(value) => unsafe { std::env::set_var("XDG_CONFIG_HOME", value) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
    }
}
