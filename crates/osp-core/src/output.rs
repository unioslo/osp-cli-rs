#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Auto,
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
