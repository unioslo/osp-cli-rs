use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemePalette {
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub info: String,
    pub warning: String,
    pub success: String,
    pub error: String,
    pub border: String,
    pub title: String,
    pub selection: String,
    pub link: String,
    pub bg: Option<String>,
    pub bg_alt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub base: Option<String>,
    pub palette: ThemePalette,
    pub overrides: ThemeOverrides,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    pub value_number: Option<String>,
    pub repl_completion_text: Option<String>,
    pub repl_completion_background: Option<String>,
    pub repl_completion_highlight: Option<String>,
}

impl ThemeDefinition {
    pub fn value_number_spec(&self) -> &str {
        self.overrides
            .value_number
            .as_deref()
            .unwrap_or(&self.palette.success)
    }

    pub fn repl_completion_text_spec(&self) -> &str {
        self.overrides
            .repl_completion_text
            .as_deref()
            .unwrap_or("#000000")
    }

    pub fn repl_completion_background_spec(&self) -> &str {
        self.overrides
            .repl_completion_background
            .as_deref()
            .unwrap_or(&self.palette.accent)
    }

    pub fn repl_completion_highlight_spec(&self) -> &str {
        self.overrides
            .repl_completion_highlight
            .as_deref()
            .unwrap_or(&self.palette.border)
    }

    pub fn display_name(&self) -> &str {
        self.name.as_str()
    }
}

pub const DEFAULT_THEME_NAME: &str = "rose-pine-moon";

fn palette(
    text: &str,
    muted: &str,
    accent: &str,
    info: &str,
    warning: &str,
    success: &str,
    error: &str,
    border: &str,
    title: &str,
) -> ThemePalette {
    ThemePalette {
        text: text.to_string(),
        muted: muted.to_string(),
        accent: accent.to_string(),
        info: info.to_string(),
        warning: warning.to_string(),
        success: success.to_string(),
        error: error.to_string(),
        border: border.to_string(),
        title: title.to_string(),
        selection: accent.to_string(),
        link: accent.to_string(),
        bg: None,
        bg_alt: None,
    }
}

fn builtin_theme_defs() -> &'static [ThemeDefinition] {
    static THEMES: OnceLock<Vec<ThemeDefinition>> = OnceLock::new();
    THEMES.get_or_init(|| {
        vec![
            ThemeDefinition {
                id: "plain".to_string(),
                name: "Plain".to_string(),
                base: None,
                palette: palette("", "", "", "", "", "", "", "", ""),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "nord".to_string(),
                name: "Nord".to_string(),
                base: None,
                palette: palette(
                    "#d8dee9",
                    "#6d7688",
                    "#88c0d0",
                    "#81a1c1",
                    "#ebcb8b",
                    "#a3be8c",
                    "bold #bf616a",
                    "#81a1c1",
                    "#81a1c1",
                ),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "dracula".to_string(),
                name: "Dracula".to_string(),
                base: None,
                palette: palette(
                    "#f8f8f2",
                    "#6879ad",
                    "#bd93f9",
                    "#8be9fd",
                    "#f1fa8c",
                    "#50fa7b",
                    "bold #ff5555",
                    "#ff79c6",
                    "#ff79c6",
                ),
                overrides: ThemeOverrides {
                    value_number: Some("#ff79c6".to_string()),
                    ..ThemeOverrides::default()
                },
            },
            ThemeDefinition {
                id: "gruvbox".to_string(),
                name: "Gruvbox".to_string(),
                base: None,
                palette: palette(
                    "#ebdbb2",
                    "#a89984",
                    "#8ec07c",
                    "#83a598",
                    "#fe8019",
                    "#b8bb26",
                    "bold #fb4934",
                    "#fabd2f",
                    "#fabd2f",
                ),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "tokyonight".to_string(),
                name: "Tokyo Night".to_string(),
                base: None,
                palette: palette(
                    "#c0caf5",
                    "#9aa5ce",
                    "#7aa2f7",
                    "#7dcfff",
                    "#e0af68",
                    "#9ece6a",
                    "bold #f7768e",
                    "#e0af68",
                    "#e0af68",
                ),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "molokai".to_string(),
                name: "Molokai".to_string(),
                base: None,
                palette: palette(
                    "#F8F8F2",
                    "#75715E",
                    "#FD971F",
                    "#66D9EF",
                    "#E6DB74",
                    "#A6E22E",
                    "bold #F92672",
                    "#E6DB74",
                    "#E6DB74",
                ),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "catppuccin".to_string(),
                name: "Catppuccin".to_string(),
                base: None,
                palette: palette(
                    "#cdd6f4",
                    "#89b4fa",
                    "#fab387",
                    "#89dceb",
                    "#f9e2af",
                    "#a6e3a1",
                    "bold #f38ba8",
                    "#89dceb",
                    "#89dceb",
                ),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "rose-pine-moon".to_string(),
                name: "Rose Pine Moon".to_string(),
                base: None,
                palette: palette(
                    "#e0def4",
                    "#908caa",
                    "#c4a7e7",
                    "#9ccfd8",
                    "#f6c177",
                    "#8bd5ca",
                    "bold #eb6f92",
                    "#e8dff6",
                    "#e8dff6",
                ),
                overrides: ThemeOverrides::default(),
            },
        ]
    })
}

pub fn builtin_themes() -> Vec<ThemeDefinition> {
    builtin_theme_defs().to_vec()
}

pub fn normalize_theme_name(value: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn display_name_from_id(value: &str) -> String {
    let trimmed = value.trim_matches('-');
    let mut out = String::new();
    for segment in trimmed.split(['-', '_']) {
        if segment.is_empty() {
            continue;
        }
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push(first.to_ascii_uppercase());
            for ch in chars {
                out.push(ch.to_ascii_lowercase());
            }
        }
    }
    if out.is_empty() {
        trimmed.to_string()
    } else {
        out
    }
}

pub fn all_themes() -> Vec<ThemeDefinition> {
    builtin_theme_defs().to_vec()
}

pub fn available_theme_names() -> Vec<String> {
    all_themes().into_iter().map(|theme| theme.id).collect()
}

pub fn find_builtin_theme(name: &str) -> Option<ThemeDefinition> {
    let normalized = normalize_theme_name(name);
    if normalized.is_empty() {
        return None;
    }
    builtin_theme_defs()
        .iter()
        .find(|theme| theme.id == normalized)
        .cloned()
}

pub fn find_theme(name: &str) -> Option<ThemeDefinition> {
    let normalized = normalize_theme_name(name);
    if normalized.is_empty() {
        return None;
    }
    builtin_theme_defs()
        .iter()
        .find(|theme| theme.id == normalized)
        .cloned()
}

pub fn resolve_theme(name: &str) -> ThemeDefinition {
    find_theme(name).unwrap_or_else(|| {
        builtin_theme_defs()
            .iter()
            .find(|theme| theme.id == DEFAULT_THEME_NAME)
            .expect("default theme must exist")
            .clone()
    })
}

pub fn is_known_theme(name: &str) -> bool {
    find_theme(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::{display_name_from_id, find_theme, resolve_theme};

    #[test]
    fn dracula_number_override_matches_python_theme_preset() {
        let dracula = find_theme("dracula").expect("dracula theme should exist");
        assert_eq!(dracula.value_number_spec(), "#ff79c6");
    }

    #[test]
    fn repl_completion_defaults_follow_python_late_defaults() {
        let theme = resolve_theme("rose-pine-moon");
        assert_eq!(theme.repl_completion_text_spec(), "#000000");
        assert_eq!(
            theme.repl_completion_background_spec(),
            theme.palette.accent
        );
        assert_eq!(theme.repl_completion_highlight_spec(), theme.palette.border);
    }

    #[test]
    fn display_name_from_id_formats_title_case() {
        assert_eq!(display_name_from_id("rose-pine-moon"), "Rose Pine Moon");
        assert_eq!(display_name_from_id("solarized-dark"), "Solarized Dark");
    }
}
