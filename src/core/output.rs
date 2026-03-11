//! Output-mode enums shared across config parsing, CLI flags, and the UI.
//!
//! These types exist so the rest of the crate can talk about rendering intent
//! without passing around raw strings. They define the stable vocabulary for
//! format, richness, color, and unicode behavior.
//!
//! Contract:
//!
//! - these enums should stay small and broadly reusable
//! - parsing here should accept the user-facing spellings supported by config
//!   and CLI flags
//! - higher-level modules may interpret `Auto`, but the canonical labels live
//!   here

/// Supported output formats for rendered command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Chooses a format based on the command result and terminal context.
    Auto,
    /// Renders guidance-oriented documents.
    Guide,
    /// Emits JSON output.
    Json,
    /// Renders tabular output.
    Table,
    /// Renders Markdown tables or documents.
    Markdown,
    /// Renders the legacy mreg text format.
    Mreg,
    /// Emits a scalar or compact value view.
    Value,
}

impl OutputFormat {
    /// Returns the canonical config and CLI spelling for this format.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::OutputFormat;
    ///
    /// assert_eq!(OutputFormat::Markdown.as_str(), "md");
    /// assert_eq!(OutputFormat::Json.as_str(), "json");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Auto => "auto",
            OutputFormat::Guide => "guide",
            OutputFormat::Json => "json",
            OutputFormat::Table => "table",
            OutputFormat::Markdown => "md",
            OutputFormat::Mreg => "mreg",
            OutputFormat::Value => "value",
        }
    }

    /// Parses the user-facing output format spellings accepted by config and
    /// CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::OutputFormat;
    ///
    /// assert_eq!(OutputFormat::parse("guide"), Some(OutputFormat::Guide));
    /// assert_eq!(OutputFormat::parse(" markdown "), Some(OutputFormat::Markdown));
    /// assert_eq!(OutputFormat::parse("MD"), Some(OutputFormat::Markdown));
    /// assert_eq!(OutputFormat::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(OutputFormat::Auto),
            "guide" => Some(OutputFormat::Guide),
            "json" => Some(OutputFormat::Json),
            "table" => Some(OutputFormat::Table),
            "md" | "markdown" => Some(OutputFormat::Markdown),
            "mreg" => Some(OutputFormat::Mreg),
            "value" => Some(OutputFormat::Value),
            _ => None,
        }
    }
}

/// Rendering mode preference for terminal output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Chooses plain or rich rendering automatically.
    Auto,
    /// Forces plain rendering.
    Plain,
    /// Forces rich rendering.
    Rich,
}

impl RenderMode {
    /// Returns the canonical config and CLI spelling for this render mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::RenderMode;
    ///
    /// assert_eq!(RenderMode::Rich.as_str(), "rich");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            RenderMode::Auto => "auto",
            RenderMode::Plain => "plain",
            RenderMode::Rich => "rich",
        }
    }

    /// Parses the render-mode spellings accepted by config and CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::RenderMode;
    ///
    /// assert_eq!(RenderMode::parse("RICH"), Some(RenderMode::Rich));
    /// assert_eq!(RenderMode::parse(" plain "), Some(RenderMode::Plain));
    /// assert_eq!(RenderMode::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(RenderMode::Auto),
            "plain" => Some(RenderMode::Plain),
            "rich" => Some(RenderMode::Rich),
            _ => None,
        }
    }
}

/// Color handling policy for rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Chooses color usage from terminal capabilities.
    Auto,
    /// Always emits color sequences.
    Always,
    /// Never emits color sequences.
    Never,
}

impl ColorMode {
    /// Returns the canonical config and CLI spelling for this color mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::ColorMode;
    ///
    /// assert_eq!(ColorMode::Always.as_str(), "always");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Always => "always",
            ColorMode::Never => "never",
        }
    }

    /// Parses the color-mode spellings accepted by config and CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::ColorMode;
    ///
    /// assert_eq!(ColorMode::parse("never"), Some(ColorMode::Never));
    /// assert_eq!(ColorMode::parse(" AUTO "), Some(ColorMode::Auto));
    /// assert_eq!(ColorMode::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ColorMode::Auto),
            "always" => Some(ColorMode::Always),
            "never" => Some(ColorMode::Never),
            _ => None,
        }
    }
}

/// Unicode handling policy for rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnicodeMode {
    /// Chooses Unicode usage from terminal capabilities.
    Auto,
    /// Always emits Unicode output.
    Always,
    /// Never emits Unicode output.
    Never,
}

impl UnicodeMode {
    /// Returns the canonical config and CLI spelling for this unicode mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::UnicodeMode;
    ///
    /// assert_eq!(UnicodeMode::Never.as_str(), "never");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            UnicodeMode::Auto => "auto",
            UnicodeMode::Always => "always",
            UnicodeMode::Never => "never",
        }
    }

    /// Parses the unicode-mode spellings accepted by config and CLI flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::UnicodeMode;
    ///
    /// assert_eq!(UnicodeMode::parse("always"), Some(UnicodeMode::Always));
    /// assert_eq!(UnicodeMode::parse(" Auto "), Some(UnicodeMode::Auto));
    /// assert_eq!(UnicodeMode::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(UnicodeMode::Auto),
            "always" => Some(UnicodeMode::Always),
            "never" => Some(UnicodeMode::Never),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorMode, OutputFormat, RenderMode, UnicodeMode};

    #[test]
    fn output_format_round_trips_known_values_and_aliases() {
        assert_eq!(OutputFormat::Auto.as_str(), "auto");
        assert_eq!(OutputFormat::Guide.as_str(), "guide");
        assert_eq!(OutputFormat::Json.as_str(), "json");
        assert_eq!(OutputFormat::Markdown.as_str(), "md");
        assert_eq!(OutputFormat::parse("guide"), Some(OutputFormat::Guide));
        assert_eq!(OutputFormat::parse(" json "), Some(OutputFormat::Json));
        assert_eq!(
            OutputFormat::parse("markdown"),
            Some(OutputFormat::Markdown)
        );
        assert_eq!(OutputFormat::parse("md"), Some(OutputFormat::Markdown));
        assert_eq!(OutputFormat::parse("wat"), None);
    }

    #[test]
    fn render_color_and_unicode_modes_parse_case_insensitively() {
        assert_eq!(RenderMode::Auto.as_str(), "auto");
        assert_eq!(RenderMode::parse("RICH"), Some(RenderMode::Rich));
        assert_eq!(RenderMode::parse("wat"), None);

        assert_eq!(ColorMode::Always.as_str(), "always");
        assert_eq!(ColorMode::parse(" never "), Some(ColorMode::Never));
        assert_eq!(ColorMode::parse("wat"), None);

        assert_eq!(UnicodeMode::Always.as_str(), "always");
        assert_eq!(UnicodeMode::parse("AUTO"), Some(UnicodeMode::Auto));
        assert_eq!(UnicodeMode::parse("wat"), None);
    }

    #[test]
    fn output_modes_cover_remaining_variants() {
        assert_eq!(OutputFormat::Table.as_str(), "table");
        assert_eq!(OutputFormat::Mreg.as_str(), "mreg");
        assert_eq!(OutputFormat::Value.as_str(), "value");
        assert_eq!(OutputFormat::parse("auto"), Some(OutputFormat::Auto));
        assert_eq!(OutputFormat::parse("mreg"), Some(OutputFormat::Mreg));
        assert_eq!(OutputFormat::parse(" value "), Some(OutputFormat::Value));

        assert_eq!(RenderMode::Plain.as_str(), "plain");
        assert_eq!(RenderMode::Rich.as_str(), "rich");
        assert_eq!(RenderMode::parse("plain"), Some(RenderMode::Plain));

        assert_eq!(ColorMode::Auto.as_str(), "auto");
        assert_eq!(ColorMode::parse("always"), Some(ColorMode::Always));

        assert_eq!(UnicodeMode::Never.as_str(), "never");
        assert_eq!(UnicodeMode::parse("never"), Some(UnicodeMode::Never));
    }
}
