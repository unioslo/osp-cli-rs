#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub info: &'static str,
    pub warning: &'static str,
    pub success: &'static str,
    pub error: &'static str,
    pub border: &'static str,
    pub title: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeDefinition {
    pub name: &'static str,
    pub palette: ThemePalette,
    pub overrides: ThemeOverrides,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    pub value_number: Option<&'static str>,
    pub repl_completion_text: Option<&'static str>,
    pub repl_completion_background: Option<&'static str>,
    pub repl_completion_highlight: Option<&'static str>,
}

impl ThemeDefinition {
    pub fn value_number_spec(&self) -> &'static str {
        self.overrides.value_number.unwrap_or(self.palette.success)
    }

    pub fn repl_completion_text_spec(&self) -> &'static str {
        self.overrides.repl_completion_text.unwrap_or("#000000")
    }

    pub fn repl_completion_background_spec(&self) -> &'static str {
        self.overrides
            .repl_completion_background
            .unwrap_or(self.palette.accent)
    }

    pub fn repl_completion_highlight_spec(&self) -> &'static str {
        self.overrides
            .repl_completion_highlight
            .unwrap_or(self.palette.border)
    }
}

pub const DEFAULT_THEME_NAME: &str = "rose-pine-moon";
const NO_THEME_OVERRIDES: ThemeOverrides = ThemeOverrides {
    value_number: None,
    repl_completion_text: None,
    repl_completion_background: None,
    repl_completion_highlight: None,
};

const THEMES: &[ThemeDefinition] = &[
    ThemeDefinition {
        name: "plain",
        palette: ThemePalette {
            text: "",
            muted: "",
            accent: "",
            info: "",
            warning: "",
            success: "",
            error: "",
            border: "",
            title: "",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "nord",
        palette: ThemePalette {
            text: "#d8dee9",
            muted: "#6d7688",
            accent: "#88c0d0",
            info: "#81a1c1",
            warning: "#ebcb8b",
            success: "#a3be8c",
            error: "bold #bf616a",
            border: "#81a1c1",
            title: "#81a1c1",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "dracula",
        palette: ThemePalette {
            text: "#f8f8f2",
            muted: "#6879ad",
            accent: "#bd93f9",
            info: "#8be9fd",
            warning: "#f1fa8c",
            success: "#50fa7b",
            error: "bold #ff5555",
            border: "#ff79c6",
            title: "#ff79c6",
        },
        overrides: ThemeOverrides {
            value_number: Some("#ff79c6"),
            ..NO_THEME_OVERRIDES
        },
    },
    ThemeDefinition {
        name: "gruvbox",
        palette: ThemePalette {
            text: "#ebdbb2",
            muted: "#a89984",
            accent: "#8ec07c",
            info: "#83a598",
            warning: "#fe8019",
            success: "#b8bb26",
            error: "bold #fb4934",
            border: "#fabd2f",
            title: "#fabd2f",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "tokyonight",
        palette: ThemePalette {
            text: "#c0caf5",
            muted: "#9aa5ce",
            accent: "#7aa2f7",
            info: "#7dcfff",
            warning: "#e0af68",
            success: "#9ece6a",
            error: "bold #f7768e",
            border: "#e0af68",
            title: "#e0af68",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "molokai",
        palette: ThemePalette {
            text: "#F8F8F2",
            muted: "#75715E",
            accent: "#FD971F",
            info: "#66D9EF",
            warning: "#E6DB74",
            success: "#A6E22E",
            error: "bold #F92672",
            border: "#E6DB74",
            title: "#E6DB74",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "catppuccin",
        palette: ThemePalette {
            text: "#cdd6f4",
            muted: "#89b4fa",
            accent: "#fab387",
            info: "#89dceb",
            warning: "#f9e2af",
            success: "#a6e3a1",
            error: "bold #f38ba8",
            border: "#89dceb",
            title: "#89dceb",
        },
        overrides: NO_THEME_OVERRIDES,
    },
    ThemeDefinition {
        name: "rose-pine-moon",
        palette: ThemePalette {
            text: "#e0def4",
            muted: "#908caa",
            accent: "#c4a7e7",
            info: "#9ccfd8",
            warning: "#f6c177",
            success: "#8bd5ca",
            error: "bold #eb6f92",
            border: "#e8dff6",
            title: "#e8dff6",
        },
        overrides: NO_THEME_OVERRIDES,
    },
];

pub fn normalize_theme_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub fn all_themes() -> &'static [ThemeDefinition] {
    THEMES
}

pub fn available_theme_names() -> Vec<&'static str> {
    let mut names = THEMES.iter().map(|theme| theme.name).collect::<Vec<_>>();
    names.sort_unstable();
    names
}

pub fn find_theme(name: &str) -> Option<&'static ThemeDefinition> {
    let normalized = normalize_theme_name(name);
    THEMES.iter().find(|theme| theme.name == normalized)
}

pub fn resolve_theme(name: &str) -> &'static ThemeDefinition {
    find_theme(name).unwrap_or_else(|| {
        THEMES
            .iter()
            .find(|theme| theme.name == DEFAULT_THEME_NAME)
            .expect("default theme must exist")
    })
}

pub fn is_known_theme(name: &str) -> bool {
    find_theme(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::{find_theme, resolve_theme};

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
}
