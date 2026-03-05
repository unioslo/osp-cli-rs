use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use osp_config::{ConfigSource, ResolvedConfig};
use osp_ui::theme::{
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
        if let Some(path) = origin.clone() {
            if entries.contains_key(&theme.id) {
                issues.push(ThemeLoadIssue {
                    path,
                    message: format!("custom theme overrides builtin: {}", theme.id),
                });
            }
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
    let mut catalog: BTreeMap<String, ThemeDefinition> = BTreeMap::new();
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

            match parse_theme_file(&path) {
                Ok(theme) => {
                    for message in validate_theme_specs(&theme) {
                        issues.push(ThemeLoadIssue {
                            path: path.clone(),
                            message,
                        });
                    }
                    if let Some(existing) = origins.get(&theme.id) {
                        issues.push(ThemeLoadIssue {
                            path: path.clone(),
                            message: format!(
                                "theme id collision: {} overridden (previous: {})",
                                theme.id,
                                existing.display()
                            ),
                        });
                    }
                    origins.insert(theme.id.clone(), path.clone());
                    catalog.insert(theme.id.clone(), theme);
                }
                Err(err) => {
                    issues.push(ThemeLoadIssue {
                        path,
                        message: err,
                    });
                }
            }
        }
    }

    CustomThemeLoad {
        themes: catalog.into_values().collect(),
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

    if trimmed == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home));
        }
    }

    if let Some(stripped) = trimmed.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home).join(stripped));
        }
    }

    Some(PathBuf::from(trimmed))
}

fn default_theme_paths() -> Vec<PathBuf> {
    osp_config::default_config_root_dir()
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

#[derive(Debug, Deserialize)]
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

fn parse_theme_file(path: &Path) -> Result<ThemeDefinition, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read theme file: {err}"))?;
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

    let base_id = parsed
        .base
        .as_deref()
        .map(normalize_theme_name)
        .filter(|value| !value.is_empty())
        .filter(|value| value != "none");
    let mut palette = match base_id.as_deref() {
        Some(base) => find_builtin_theme(base)
            .map(|theme| theme.palette)
            .ok_or_else(|| format!("unknown base theme: {base}"))?,
        None => empty_palette(),
    };
    if let Some(file_palette) = parsed.palette {
        if let Some(value) = file_palette.text {
            palette.text = value;
        }
        if let Some(value) = file_palette.muted {
            palette.muted = value;
        }
        if let Some(value) = file_palette.accent {
            palette.accent = value;
        }
        if let Some(value) = file_palette.info {
            palette.info = value;
        }
        if let Some(value) = file_palette.warning {
            palette.warning = value;
        }
        if let Some(value) = file_palette.success {
            palette.success = value;
        }
        if let Some(value) = file_palette.error {
            palette.error = value;
        }
        if let Some(value) = file_palette.border {
            palette.border = value;
        }
        if let Some(value) = file_palette.title {
            palette.title = value;
        }
        if let Some(value) = file_palette.selection {
            palette.selection = value;
        }
        if let Some(value) = file_palette.link {
            palette.link = value;
        }
        if let Some(value) = file_palette.bg {
            palette.bg = Some(value);
        }
        if let Some(value) = file_palette.bg_alt {
            palette.bg_alt = Some(value);
        }
    }

    let overrides = ThemeOverrides {
        value_number: parsed.overrides.value_number,
        repl_completion_text: parsed.overrides.repl_completion_text,
        repl_completion_background: parsed.overrides.repl_completion_background,
        repl_completion_highlight: parsed.overrides.repl_completion_highlight,
    };

    Ok(ThemeDefinition {
        id,
        name,
        base: base_id,
        palette,
        overrides,
    })
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
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }

    for raw in trimmed.split_whitespace() {
        let token = raw.trim().to_ascii_lowercase();
        if is_known_style_token(&token) {
            continue;
        }
        if parse_hex_rgb(&token).is_some() {
            continue;
        }
        return false;
    }
    true
}

fn is_known_style_token(token: &str) -> bool {
    matches!(
        token,
        "bold"
            | "dim"
            | "italic"
            | "underline"
            | "black"
            | "red"
            | "green"
            | "yellow"
            | "blue"
            | "magenta"
            | "cyan"
            | "white"
            | "bright-black"
            | "bright-red"
            | "bright-green"
            | "bright-yellow"
            | "bright-blue"
            | "bright-magenta"
            | "bright-cyan"
            | "bright-white"
    )
}

fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    if !value.starts_with('#') || value.len() != 7 {
        return None;
    }
    let r = u8::from_str_radix(&value[1..3], 16).ok()?;
    let g = u8::from_str_radix(&value[3..5], 16).ok()?;
    let b = u8::from_str_radix(&value[5..7], 16).ok()?;
    Some((r, g, b))
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
    use super::{is_valid_color_spec, parse_theme_file};
    use std::fs;
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

        let theme = parse_theme_file(&path).expect("theme should parse");
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

        let theme = parse_theme_file(&path).expect("theme should parse");
        assert_eq!(theme.palette.accent, "#123456");
        assert_eq!(theme.palette.text, "#f8f8f2");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
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
}
