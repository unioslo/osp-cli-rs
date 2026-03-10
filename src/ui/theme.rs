use std::ops::Deref;
use std::sync::{Arc, OnceLock};

/// Palette entries used by the built-in terminal themes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemePalette {
    /// Base foreground color for plain text.
    pub text: String,
    /// Secondary or de-emphasized foreground color.
    pub muted: String,
    /// Accent color used for keys and highlights.
    pub accent: String,
    /// Informational message color.
    pub info: String,
    /// Warning message color.
    pub warning: String,
    /// Success message color.
    pub success: String,
    /// Error message color.
    pub error: String,
    /// Border and chrome color.
    pub border: String,
    /// Title and heading color.
    pub title: String,
    /// Selection/highlight color.
    pub selection: String,
    /// Link color.
    pub link: String,
    /// Optional primary background color.
    pub bg: Option<String>,
    /// Optional alternate background color.
    pub bg_alt: Option<String>,
}

/// Concrete theme data shared by theme handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeData {
    /// Stable theme identifier used for config and lookup.
    pub id: String,
    /// User-facing display name.
    pub name: String,
    /// Optional parent theme identifier.
    pub base: Option<String>,
    /// Core palette values for the theme.
    pub palette: ThemePalette,
    /// Optional overrides for derived semantic tokens.
    pub overrides: ThemeOverrides,
}

/// Shared handle to resolved theme data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDefinition(Arc<ThemeData>);

/// Optional theme-specific overrides for derived style tokens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeOverrides {
    /// Override for numeric values.
    pub value_number: Option<String>,
    /// Override for completion menu text.
    pub repl_completion_text: Option<String>,
    /// Override for completion menu backgrounds.
    pub repl_completion_background: Option<String>,
    /// Override for the active completion entry highlight.
    pub repl_completion_highlight: Option<String>,
}

impl ThemeDefinition {
    /// Builds a theme definition from palette data and optional overrides.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        base: Option<String>,
        palette: ThemePalette,
        overrides: ThemeOverrides,
    ) -> Self {
        Self(Arc::new(ThemeData {
            id: id.into(),
            name: name.into(),
            base,
            palette,
            overrides,
        }))
    }

    /// Returns the style specification used for numeric values.
    pub fn value_number_spec(&self) -> &str {
        self.overrides
            .value_number
            .as_deref()
            .unwrap_or(&self.palette.success)
    }

    /// Returns the style specification used for REPL completion text.
    pub fn repl_completion_text_spec(&self) -> &str {
        self.overrides
            .repl_completion_text
            .as_deref()
            .unwrap_or("#000000")
    }

    /// Returns the style specification used for REPL completion backgrounds.
    pub fn repl_completion_background_spec(&self) -> &str {
        self.overrides
            .repl_completion_background
            .as_deref()
            .unwrap_or(&self.palette.accent)
    }

    /// Returns the style specification used for the highlighted completion entry.
    pub fn repl_completion_highlight_spec(&self) -> &str {
        self.overrides
            .repl_completion_highlight
            .as_deref()
            .unwrap_or(&self.palette.border)
    }

    /// Returns the display name shown to users.
    pub fn display_name(&self) -> &str {
        self.name.as_str()
    }
}

impl Deref for ThemeDefinition {
    type Target = ThemeData;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

/// Default theme identifier used when no valid theme is configured.
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

fn builtin_theme(
    id: &'static str,
    name: &'static str,
    palette: ThemePalette,
    overrides: ThemeOverrides,
) -> ThemeDefinition {
    ThemeDefinition::new(id, name, None, palette, overrides)
}

fn builtin_theme_defs() -> &'static [ThemeDefinition] {
    static THEMES: OnceLock<Vec<ThemeDefinition>> = OnceLock::new();
    THEMES.get_or_init(|| {
        vec![
            builtin_theme(
                "plain",
                "Plain",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "nord",
                "Nord",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "dracula",
                "Dracula",
                palette(PaletteSpec {
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
                ThemeOverrides {
                    value_number: Some("#ff79c6".to_string()),
                    ..ThemeOverrides::default()
                },
            ),
            builtin_theme(
                "gruvbox",
                "Gruvbox",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "tokyonight",
                "Tokyo Night",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "molokai",
                "Molokai",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "catppuccin",
                "Catppuccin",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
            builtin_theme(
                "rose-pine-moon",
                "Rose Pine Moon",
                palette(PaletteSpec {
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
                ThemeOverrides::default(),
            ),
        ]
    })
}

/// Returns the built-in theme catalog.
pub fn builtin_themes() -> Vec<ThemeDefinition> {
    builtin_theme_defs().to_vec()
}

/// Normalizes a theme name for lookup.
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

/// Converts a theme identifier into a user-facing display name.
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

/// Returns all available themes.
pub fn all_themes() -> Vec<ThemeDefinition> {
    builtin_theme_defs().to_vec()
}

/// Returns the identifiers of all available themes.
pub fn available_theme_names() -> Vec<String> {
    all_themes()
        .into_iter()
        .map(|theme| theme.id.clone())
        .collect()
}

/// Finds a built-in theme by name after normalization.
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

/// Finds a theme by name after normalization.
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

/// Resolves a theme by name, falling back to the default theme.
pub fn resolve_theme(name: &str) -> ThemeDefinition {
    find_theme(name).unwrap_or_else(|| {
        builtin_theme_defs()
            .iter()
            .find(|theme| theme.id == DEFAULT_THEME_NAME)
            .expect("default theme must exist")
            .clone()
    })
}

/// Returns whether a theme name resolves to a known theme.
pub fn is_known_theme(name: &str) -> bool {
    find_theme(name).is_some()
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use super::{
        DEFAULT_THEME_NAME, all_themes, available_theme_names, builtin_themes,
        display_name_from_id, find_builtin_theme, find_theme, is_known_theme, resolve_theme,
    };

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
    fn repl_completion_text_defaults_to_black_for_all_themes() {
        for theme_id in ["rose-pine-moon", "dracula", "tokyonight", "catppuccin"] {
            let theme = resolve_theme(theme_id);
            assert_eq!(theme.repl_completion_text_spec(), "#000000");
        }
    }

    #[test]
    fn display_name_from_id_formats_title_case() {
        assert_eq!(display_name_from_id("rose-pine-moon"), "Rose Pine Moon");
        assert_eq!(display_name_from_id("solarized-dark"), "Solarized Dark");
    }

    #[test]
    fn display_name_and_lookup_helpers_cover_normalization_edges() {
        let rose = find_theme(" Rose_Pine Moon ").expect("theme lookup should normalize");
        assert_eq!(black_box(rose.display_name()), "Rose Pine Moon");

        let builtin =
            black_box(find_builtin_theme(" TOKYONIGHT ")).expect("builtin theme should normalize");
        assert_eq!(builtin.id, "tokyonight");

        assert_eq!(black_box(display_name_from_id("--")), "");
        assert_eq!(
            black_box(display_name_from_id("-already-title-")),
            "Already Title"
        );
        assert!(black_box(find_theme("   ")).is_none());
        assert!(black_box(find_builtin_theme("   ")).is_none());
    }

    #[test]
    fn theme_catalog_helpers_expose_defaults_and_fallbacks() {
        let names = black_box(available_theme_names());
        assert!(names.contains(&DEFAULT_THEME_NAME.to_string()));
        assert_eq!(
            black_box(all_themes()).len(),
            black_box(builtin_themes()).len()
        );
        assert!(black_box(is_known_theme("nord")));
        assert!(!black_box(is_known_theme("missing-theme")));

        let fallback = black_box(resolve_theme("missing-theme"));
        assert_eq!(fallback.id, DEFAULT_THEME_NAME);
    }
}
