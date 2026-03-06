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

struct PaletteSpec<'a> {
    text: &'a str,
    muted: &'a str,
    accent: &'a str,
    info: &'a str,
    warning: &'a str,
    success: &'a str,
    error: &'a str,
    border: &'a str,
    title: &'a str,
}

fn palette(spec: PaletteSpec<'_>) -> ThemePalette {
    ThemePalette {
        text: spec.text.to_string(),
        muted: spec.muted.to_string(),
        accent: spec.accent.to_string(),
        info: spec.info.to_string(),
        warning: spec.warning.to_string(),
        success: spec.success.to_string(),
        error: spec.error.to_string(),
        border: spec.border.to_string(),
        title: spec.title.to_string(),
        selection: spec.accent.to_string(),
        link: spec.accent.to_string(),
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
                palette: palette(PaletteSpec {
                    text: "",
                    muted: "",
                    accent: "",
                    info: "",
                    warning: "",
                    success: "",
                    error: "",
                    border: "",
                    title: "",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "nord".to_string(),
                name: "Nord".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#d8dee9",
                    muted: "#6d7688",
                    accent: "#88c0d0",
                    info: "#81a1c1",
                    warning: "#ebcb8b",
                    success: "#a3be8c",
                    error: "bold #bf616a",
                    border: "#81a1c1",
                    title: "#81a1c1",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "dracula".to_string(),
                name: "Dracula".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#f8f8f2",
                    muted: "#6879ad",
                    accent: "#bd93f9",
                    info: "#8be9fd",
                    warning: "#f1fa8c",
                    success: "#50fa7b",
                    error: "bold #ff5555",
                    border: "#ff79c6",
                    title: "#ff79c6",
                }),
                overrides: ThemeOverrides {
                    value_number: Some("#ff79c6".to_string()),
                    ..ThemeOverrides::default()
                },
            },
            ThemeDefinition {
                id: "gruvbox".to_string(),
                name: "Gruvbox".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#ebdbb2",
                    muted: "#a89984",
                    accent: "#8ec07c",
                    info: "#83a598",
                    warning: "#fe8019",
                    success: "#b8bb26",
                    error: "bold #fb4934",
                    border: "#fabd2f",
                    title: "#fabd2f",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "tokyonight".to_string(),
                name: "Tokyo Night".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#c0caf5",
                    muted: "#9aa5ce",
                    accent: "#7aa2f7",
                    info: "#7dcfff",
                    warning: "#e0af68",
                    success: "#9ece6a",
                    error: "bold #f7768e",
                    border: "#e0af68",
                    title: "#e0af68",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "molokai".to_string(),
                name: "Molokai".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#F8F8F2",
                    muted: "#75715E",
                    accent: "#FD971F",
                    info: "#66D9EF",
                    warning: "#E6DB74",
                    success: "#A6E22E",
                    error: "bold #F92672",
                    border: "#E6DB74",
                    title: "#E6DB74",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "catppuccin".to_string(),
                name: "Catppuccin".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#cdd6f4",
                    muted: "#89b4fa",
                    accent: "#fab387",
                    info: "#89dceb",
                    warning: "#f9e2af",
                    success: "#a6e3a1",
                    error: "bold #f38ba8",
                    border: "#89dceb",
                    title: "#89dceb",
                }),
                overrides: ThemeOverrides::default(),
            },
            ThemeDefinition {
                id: "rose-pine-moon".to_string(),
                name: "Rose Pine Moon".to_string(),
                base: None,
                palette: palette(PaletteSpec {
                    text: "#e0def4",
                    muted: "#908caa",
                    accent: "#c4a7e7",
                    info: "#9ccfd8",
                    warning: "#f6c177",
                    success: "#8bd5ca",
                    error: "bold #eb6f92",
                    border: "#e8dff6",
                    title: "#e8dff6",
                }),
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
