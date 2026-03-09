#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Auto,
    Guide,
    Json,
    Table,
    Markdown,
    Mreg,
    Value,
}

impl OutputFormat {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Auto,
    Plain,
    Rich,
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RenderMode::Auto => "auto",
            RenderMode::Plain => "plain",
            RenderMode::Rich => "rich",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(RenderMode::Auto),
            "plain" => Some(RenderMode::Plain),
            "rich" => Some(RenderMode::Rich),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Always => "always",
            ColorMode::Never => "never",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ColorMode::Auto),
            "always" => Some(ColorMode::Always),
            "never" => Some(ColorMode::Never),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnicodeMode {
    Auto,
    Always,
    Never,
}

impl UnicodeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            UnicodeMode::Auto => "auto",
            UnicodeMode::Always => "always",
            UnicodeMode::Never => "never",
        }
    }

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
