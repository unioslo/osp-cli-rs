use std::ffi::OsString;

use miette::{Result, miette};
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};

pub(crate) const INVOCATION_HELP_SECTION: &str = r#"Common Invocation Options:
  --format <auto|json|table|mreg|value|md>  Format this invocation only
  --json | --table | --mreg | --value | --md
                                            Convenience aliases for --format
  --mode <auto|plain|rich>                  Render mode for this invocation
  --plain | --rich                          Convenience aliases for --mode
  --color <auto|always|never>               Color policy for this invocation
  --unicode <auto|always|never>             Unicode policy for this invocation
  --ascii                                   Alias for --unicode never
  -v, --verbose                             Increase message verbosity
  -q, --quiet                               Decrease message verbosity
  -d, --debug                               Increase developer log verbosity
  --cache                                   Reuse identical result in this REPL session
  --plugin-provider <PLUGIN_ID>             Select provider for this invocation

These flags may appear anywhere before `--` and affect only the current command.
`--cache` is available only inside the interactive REPL."#;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InvocationOptions {
    pub(crate) format: Option<OutputFormat>,
    pub(crate) mode: Option<RenderMode>,
    pub(crate) color: Option<ColorMode>,
    pub(crate) unicode: Option<UnicodeMode>,
    pub(crate) verbose: u8,
    pub(crate) quiet: u8,
    pub(crate) debug: u8,
    pub(crate) cache: bool,
    pub(crate) plugin_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedCliArgs {
    pub(crate) argv: Vec<OsString>,
    pub(crate) invocation: InvocationOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedCommandTokens {
    pub(crate) tokens: Vec<String>,
    pub(crate) invocation: InvocationOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScannedCommandTokenTrace {
    pub(crate) tokens: Vec<String>,
    pub(crate) invocation: InvocationOptions,
    pub(crate) kept_indices: Vec<usize>,
}

pub(crate) fn scan_cli_argv(argv: &[OsString]) -> Result<ScannedCliArgs> {
    if argv.is_empty() {
        return Ok(ScannedCliArgs {
            argv: Vec::new(),
            invocation: InvocationOptions::default(),
        });
    }

    let head = argv[0].clone();
    let tail = argv[1..]
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let scanned = scan_command_tokens(&tail)?;
    let mut cleaned = Vec::with_capacity(scanned.tokens.len() + 1);
    cleaned.push(head);
    cleaned.extend(scanned.tokens.into_iter().map(OsString::from));

    Ok(ScannedCliArgs {
        argv: cleaned,
        invocation: scanned.invocation,
    })
}

pub(crate) fn scan_command_tokens(tokens: &[String]) -> Result<ScannedCommandTokens> {
    let traced = scan_command_tokens_with_trace(tokens)?;
    Ok(ScannedCommandTokens {
        tokens: traced.tokens,
        invocation: traced.invocation,
    })
}

pub(crate) fn scan_command_tokens_with_trace(
    tokens: &[String],
) -> Result<ScannedCommandTokenTrace> {
    let mut cleaned = Vec::with_capacity(tokens.len());
    let mut kept_indices = Vec::with_capacity(tokens.len());
    let mut invocation = InvocationOptions::default();
    let mut index = 0usize;
    let mut passthrough = false;

    while index < tokens.len() {
        let token = &tokens[index];
        if passthrough {
            cleaned.push(token.clone());
            kept_indices.push(index);
            index += 1;
            continue;
        }

        if token == "--" {
            passthrough = true;
            cleaned.push(token.clone());
            kept_indices.push(index);
            index += 1;
            continue;
        }

        if let Some(kind) = FormatAlias::parse(token) {
            set_format(&mut invocation, kind.format(), token)?;
            index += 1;
            continue;
        }

        if let Some(kind) = RenderAlias::parse(token) {
            set_mode(&mut invocation, kind.mode(), token)?;
            index += 1;
            continue;
        }

        if token == "--ascii" {
            set_unicode(&mut invocation, UnicodeMode::Never, token)?;
            index += 1;
            continue;
        }

        if token == "--verbose" {
            invocation.verbose = invocation.verbose.saturating_add(1);
            index += 1;
            continue;
        }

        if token == "--quiet" {
            invocation.quiet = invocation.quiet.saturating_add(1);
            index += 1;
            continue;
        }

        if token == "--debug" {
            invocation.debug = invocation.debug.saturating_add(1);
            index += 1;
            continue;
        }

        if token == "--cache" {
            invocation.cache = true;
            index += 1;
            continue;
        }

        if is_short_count_cluster(token) {
            for ch in token.chars().skip(1) {
                match ch {
                    'v' => invocation.verbose = invocation.verbose.saturating_add(1),
                    'q' => invocation.quiet = invocation.quiet.saturating_add(1),
                    'd' => invocation.debug = invocation.debug.saturating_add(1),
                    _ => unreachable!("short cluster guard should only allow v/q/d"),
                }
            }
            index += 1;
            continue;
        }

        if let Some(value) = token.strip_prefix("--format=") {
            let format = OutputFormat::parse(value)
                .ok_or_else(|| miette!("unknown output format: {value}"))?;
            set_format(&mut invocation, format, "--format")?;
            index += 1;
            continue;
        }

        if token == "--format" {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| miette!("`--format` expects a value"))?;
            let format = OutputFormat::parse(value)
                .ok_or_else(|| miette!("unknown output format: {value}"))?;
            set_format(&mut invocation, format, "--format")?;
            index += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--mode=") {
            let mode =
                RenderMode::parse(value).ok_or_else(|| miette!("unknown render mode: {value}"))?;
            set_mode(&mut invocation, mode, "--mode")?;
            index += 1;
            continue;
        }

        if token == "--mode" {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| miette!("`--mode` expects a value"))?;
            let mode =
                RenderMode::parse(value).ok_or_else(|| miette!("unknown render mode: {value}"))?;
            set_mode(&mut invocation, mode, "--mode")?;
            index += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--color=") {
            let color =
                ColorMode::parse(value).ok_or_else(|| miette!("unknown color mode: {value}"))?;
            set_color(&mut invocation, color, "--color")?;
            index += 1;
            continue;
        }

        if token == "--color" {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| miette!("`--color` expects a value"))?;
            let color =
                ColorMode::parse(value).ok_or_else(|| miette!("unknown color mode: {value}"))?;
            set_color(&mut invocation, color, "--color")?;
            index += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--unicode=") {
            let unicode = UnicodeMode::parse(value)
                .ok_or_else(|| miette!("unknown unicode mode: {value}"))?;
            set_unicode(&mut invocation, unicode, "--unicode")?;
            index += 1;
            continue;
        }

        if token == "--unicode" {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| miette!("`--unicode` expects a value"))?;
            let unicode = UnicodeMode::parse(value)
                .ok_or_else(|| miette!("unknown unicode mode: {value}"))?;
            set_unicode(&mut invocation, unicode, "--unicode")?;
            index += 2;
            continue;
        }

        if let Some(value) = token.strip_prefix("--plugin-provider=") {
            set_plugin_provider(&mut invocation, value, "--plugin-provider")?;
            index += 1;
            continue;
        }

        if token == "--plugin-provider" {
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| miette!("`--plugin-provider` expects a value"))?;
            set_plugin_provider(&mut invocation, value, "--plugin-provider")?;
            index += 2;
            continue;
        }

        cleaned.push(token.clone());
        kept_indices.push(index);
        index += 1;
    }

    Ok(ScannedCommandTokenTrace {
        tokens: cleaned,
        invocation,
        kept_indices,
    })
}

pub(crate) fn append_invocation_help(help_text: &str) -> String {
    if help_text.contains("Common Invocation Options:") {
        return help_text.to_string();
    }

    let trimmed = help_text.trim_end_matches('\n');
    format!("{trimmed}\n\n{INVOCATION_HELP_SECTION}\n")
}

fn is_short_count_cluster(token: &str) -> bool {
    token.starts_with('-')
        && !token.starts_with("--")
        && token.len() > 1
        && token
            .chars()
            .skip(1)
            .all(|ch| matches!(ch, 'v' | 'q' | 'd'))
}

fn set_format(options: &mut InvocationOptions, format: OutputFormat, source: &str) -> Result<()> {
    if let Some(existing) = options.format {
        return Err(miette!(
            "conflicting output format flags: existing `{}` and `{source}`",
            existing.as_str()
        ));
    }
    options.format = Some(format);
    Ok(())
}

fn set_mode(options: &mut InvocationOptions, mode: RenderMode, source: &str) -> Result<()> {
    if let Some(existing) = options.mode {
        return Err(miette!(
            "conflicting render mode flags: existing `{}` and `{source}`",
            existing.as_str()
        ));
    }
    options.mode = Some(mode);
    Ok(())
}

fn set_color(options: &mut InvocationOptions, color: ColorMode, source: &str) -> Result<()> {
    if let Some(existing) = options.color {
        return Err(miette!(
            "conflicting color mode flags: existing `{}` and `{source}`",
            existing.as_str()
        ));
    }
    options.color = Some(color);
    Ok(())
}

fn set_unicode(options: &mut InvocationOptions, unicode: UnicodeMode, source: &str) -> Result<()> {
    if let Some(existing) = options.unicode {
        return Err(miette!(
            "conflicting unicode mode flags: existing `{}` and `{source}`",
            existing.as_str()
        ));
    }
    options.unicode = Some(unicode);
    Ok(())
}

fn set_plugin_provider(options: &mut InvocationOptions, value: &str, source: &str) -> Result<()> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(miette!("{source} expects a non-empty value"));
    }
    if options.plugin_provider.is_some() {
        return Err(miette!("`{source}` specified more than once"));
    }
    options.plugin_provider = Some(normalized.to_string());
    Ok(())
}

#[derive(Clone, Copy)]
enum FormatAlias {
    Json,
    Table,
    Value,
    Markdown,
    Mreg,
}

impl FormatAlias {
    fn parse(token: &str) -> Option<Self> {
        match token {
            "--json" => Some(Self::Json),
            "--table" => Some(Self::Table),
            "--value" => Some(Self::Value),
            "--md" => Some(Self::Markdown),
            "--mreg" => Some(Self::Mreg),
            _ => None,
        }
    }

    fn format(self) -> OutputFormat {
        match self {
            Self::Json => OutputFormat::Json,
            Self::Table => OutputFormat::Table,
            Self::Value => OutputFormat::Value,
            Self::Markdown => OutputFormat::Markdown,
            Self::Mreg => OutputFormat::Mreg,
        }
    }
}

#[derive(Clone, Copy)]
enum RenderAlias {
    Rich,
    Plain,
}

impl RenderAlias {
    fn parse(token: &str) -> Option<Self> {
        match token {
            "--rich" => Some(Self::Rich),
            "--plain" => Some(Self::Plain),
            _ => None,
        }
    }

    fn mode(self) -> RenderMode {
        match self {
            Self::Rich => RenderMode::Rich,
            Self::Plain => RenderMode::Plain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InvocationOptions, append_invocation_help, scan_command_tokens};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};

    #[test]
    fn strips_invocation_flags_and_preserves_command_tokens() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "user".to_string(),
            "oistes".to_string(),
            "--json".to_string(),
            "-vv".to_string(),
            "--plugin-provider".to_string(),
            "uio-ldap".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(
            scanned.tokens,
            vec!["ldap".to_string(), "user".to_string(), "oistes".to_string(),]
        );
        assert_eq!(
            scanned.invocation,
            InvocationOptions {
                format: Some(OutputFormat::Json),
                mode: None,
                color: None,
                unicode: None,
                verbose: 2,
                quiet: 0,
                debug: 0,
                cache: false,
                plugin_provider: Some("uio-ldap".to_string()),
            }
        );
    }

    #[test]
    fn stops_host_scanning_after_double_dash() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "--".to_string(),
            "--json".to_string(),
            "-vv".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(
            scanned.tokens,
            vec![
                "ldap".to_string(),
                "--".to_string(),
                "--json".to_string(),
                "-vv".to_string(),
            ]
        );
        assert_eq!(scanned.invocation, InvocationOptions::default());
    }

    #[test]
    fn rejects_conflicting_format_flags() {
        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--json".to_string(),
            "--format".to_string(),
            "table".to_string(),
        ])
        .expect_err("conflicting flags should fail");
        assert!(err.to_string().contains("conflicting output format flags"));
    }

    #[test]
    fn supports_explicit_render_and_ascii_aliases() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "--plain".to_string(),
            "--color".to_string(),
            "never".to_string(),
            "--ascii".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(scanned.invocation.mode, Some(RenderMode::Plain));
        assert_eq!(scanned.invocation.color, Some(ColorMode::Never));
        assert_eq!(scanned.invocation.unicode, Some(UnicodeMode::Never));
    }

    #[test]
    fn parses_repl_cache_flag_without_touching_command_tokens() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "user".to_string(),
            "alice".to_string(),
            "--cache".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(
            scanned.tokens,
            vec!["ldap".to_string(), "user".to_string(), "alice".to_string()]
        );
        assert!(scanned.invocation.cache);
    }

    #[test]
    fn appends_invocation_help_once() {
        let rendered = append_invocation_help("Usage: osp [COMMAND]\n");
        assert!(rendered.contains("Common Invocation Options:"));

        let twice = append_invocation_help(&rendered);
        assert_eq!(rendered, twice);
    }
}
