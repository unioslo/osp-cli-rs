use std::collections::BTreeSet;
use std::ffi::OsString;

use miette::{Result, miette};
use crate::osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};

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

const INVOCATION_COMPLETION_FLAGS: &[&str] = &[
    "--format",
    "--json",
    "--table",
    "--mreg",
    "--value",
    "--md",
    "--mode",
    "--plain",
    "--rich",
    "--color",
    "--unicode",
    "--ascii",
    "--verbose",
    "--quiet",
    "--debug",
    "--cache",
    "--plugin-provider",
];

const FORMAT_COMPLETION_FLAGS: &[&str] =
    &["--format", "--json", "--table", "--mreg", "--value", "--md"];
const MODE_COMPLETION_FLAGS: &[&str] = &["--mode", "--plain", "--rich"];
const UNICODE_COMPLETION_FLAGS: &[&str] = &["--unicode", "--ascii"];

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

pub(crate) fn append_invocation_help_if_verbose(
    help_text: &str,
    invocation: &InvocationOptions,
) -> String {
    if should_show_invocation_help(invocation) {
        append_invocation_help(help_text)
    } else {
        help_text.to_string()
    }
}

pub(crate) fn should_show_invocation_help(invocation: &InvocationOptions) -> bool {
    invocation.verbose > 0
}

pub(crate) fn hidden_invocation_completion_flags(
    invocation: &InvocationOptions,
) -> BTreeSet<String> {
    if !should_show_invocation_help(invocation) {
        return INVOCATION_COMPLETION_FLAGS
            .iter()
            .map(|flag| (*flag).to_string())
            .collect();
    }

    let mut hidden = BTreeSet::new();

    if invocation.format.is_some() {
        hidden.extend(
            FORMAT_COMPLETION_FLAGS
                .iter()
                .map(|flag| (*flag).to_string()),
        );
    }
    if invocation.mode.is_some() {
        hidden.extend(MODE_COMPLETION_FLAGS.iter().map(|flag| (*flag).to_string()));
    }
    if invocation.color.is_some() {
        hidden.insert("--color".to_string());
    }
    if invocation.unicode.is_some() {
        hidden.extend(
            UNICODE_COMPLETION_FLAGS
                .iter()
                .map(|flag| (*flag).to_string()),
        );
    }
    if invocation.cache {
        hidden.insert("--cache".to_string());
    }
    if invocation.plugin_provider.is_some() {
        hidden.insert("--plugin-provider".to_string());
    }

    hidden
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
    use super::{
        INVOCATION_HELP_SECTION, InvocationOptions, append_invocation_help,
        append_invocation_help_if_verbose, hidden_invocation_completion_flags, scan_cli_argv,
        scan_command_tokens, scan_command_tokens_with_trace, should_show_invocation_help,
    };
    use crate::osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use std::ffi::OsString;

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

    #[test]
    fn invocation_help_requires_verbose_unit() {
        assert!(!should_show_invocation_help(&InvocationOptions::default()));
        assert!(should_show_invocation_help(&InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        }));
        assert_eq!(
            append_invocation_help_if_verbose(
                "Usage: osp [COMMAND]\n",
                &InvocationOptions::default()
            ),
            "Usage: osp [COMMAND]\n"
        );
        assert!(
            append_invocation_help_if_verbose(
                "Usage: osp [COMMAND]\n",
                &InvocationOptions {
                    verbose: 1,
                    ..InvocationOptions::default()
                }
            )
            .contains("Common Invocation Options:")
        );
    }

    #[test]
    fn hidden_completion_flags_follow_verbose_and_used_one_shots_unit() {
        let hidden = hidden_invocation_completion_flags(&InvocationOptions::default());
        assert!(hidden.contains("--json"));
        assert!(hidden.contains("--plugin-provider"));
        assert!(hidden.contains("--debug"));

        let hidden = hidden_invocation_completion_flags(&InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        });
        assert!(!hidden.contains("--json"));
        assert!(!hidden.contains("--plugin-provider"));
        assert!(!hidden.contains("--debug"));

        let hidden = hidden_invocation_completion_flags(&InvocationOptions {
            verbose: 1,
            format: Some(OutputFormat::Json),
            cache: true,
            plugin_provider: Some("ldap".to_string()),
            ..InvocationOptions::default()
        });
        assert!(hidden.contains("--format"));
        assert!(hidden.contains("--json"));
        assert!(hidden.contains("--table"));
        assert!(hidden.contains("--cache"));
        assert!(hidden.contains("--plugin-provider"));
        assert!(!hidden.contains("--debug"));
    }

    #[test]
    fn trace_scanner_reports_kept_token_indices() {
        let traced = scan_command_tokens_with_trace(&[
            "ldap".to_string(),
            "--json".to_string(),
            "user".to_string(),
            "alice".to_string(),
        ])
        .expect("trace scan should succeed");

        assert_eq!(traced.tokens, vec!["ldap", "user", "alice"]);
        assert_eq!(traced.kept_indices, vec![0, 2, 3]);
    }

    #[test]
    fn duplicate_plugin_provider_is_rejected() {
        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--plugin-provider".to_string(),
            "one".to_string(),
            "--plugin-provider=two".to_string(),
        ])
        .expect_err("duplicate provider should fail");
        assert!(err.to_string().contains("specified more than once"));
    }

    #[test]
    fn invalid_mode_and_empty_provider_value_are_rejected() {
        let err =
            scan_command_tokens(&["ldap".to_string(), "--mode".to_string(), "wat".to_string()])
                .expect_err("invalid mode should fail");
        assert!(err.to_string().contains("unknown render mode"));

        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--plugin-provider".to_string(),
            "   ".to_string(),
        ])
        .expect_err("empty provider should fail");
        assert!(err.to_string().contains("expects a non-empty value"));
    }

    #[test]
    fn invocation_help_section_mentions_cache_scope() {
        assert!(INVOCATION_HELP_SECTION.contains("--cache"));
        assert!(INVOCATION_HELP_SECTION.contains("interactive REPL"));
    }

    #[test]
    fn scan_cli_argv_preserves_binary_and_strips_invocation_flags() {
        let scanned = scan_cli_argv(&[
            OsString::from("osp"),
            OsString::from("--json"),
            OsString::from("config"),
            OsString::from("show"),
        ])
        .expect("cli argv scan should succeed");

        assert_eq!(
            scanned.argv,
            vec![
                OsString::from("osp"),
                OsString::from("config"),
                OsString::from("show")
            ]
        );
        assert_eq!(scanned.invocation.format, Some(OutputFormat::Json));
    }

    #[test]
    fn scan_cli_argv_handles_empty_input() {
        let scanned = scan_cli_argv(&[]).expect("empty argv should scan");
        assert!(scanned.argv.is_empty());
        assert_eq!(scanned.invocation, InvocationOptions::default());
    }

    #[test]
    fn supports_inline_assignment_forms_for_render_settings() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "--format=value".to_string(),
            "--mode=rich".to_string(),
            "--color=always".to_string(),
            "--unicode=always".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(scanned.invocation.format, Some(OutputFormat::Value));
        assert_eq!(scanned.invocation.mode, Some(RenderMode::Rich));
        assert_eq!(scanned.invocation.color, Some(ColorMode::Always));
        assert_eq!(scanned.invocation.unicode, Some(UnicodeMode::Always));
    }

    #[test]
    fn short_clusters_accumulate_verbose_quiet_and_debug_counts() {
        let scanned =
            scan_command_tokens(&["ldap".to_string(), "-vvqd".to_string(), "user".to_string()])
                .expect("scan should succeed");

        assert_eq!(scanned.invocation.verbose, 2);
        assert_eq!(scanned.invocation.quiet, 1);
        assert_eq!(scanned.invocation.debug, 1);
        assert_eq!(scanned.tokens, vec!["ldap".to_string(), "user".to_string()]);
    }

    #[test]
    fn missing_values_for_named_flags_are_rejected() {
        let err = scan_command_tokens(&["ldap".to_string(), "--format".to_string()])
            .expect_err("missing format value should fail");
        assert!(err.to_string().contains("`--format` expects a value"));

        let err = scan_command_tokens(&["ldap".to_string(), "--color".to_string()])
            .expect_err("missing color value should fail");
        assert!(err.to_string().contains("`--color` expects a value"));
    }

    #[test]
    fn scan_cli_argv_can_strip_only_invocation_flags() {
        let scanned = scan_cli_argv(&[
            OsString::from("osp"),
            OsString::from("--json"),
            OsString::from("-vv"),
        ])
        .expect("cli argv scan should succeed");

        assert_eq!(scanned.argv, vec![OsString::from("osp")]);
        assert_eq!(scanned.invocation.format, Some(OutputFormat::Json));
        assert_eq!(scanned.invocation.verbose, 2);
    }

    #[test]
    fn non_host_short_flags_are_left_in_command_tokens() {
        let scanned = scan_command_tokens(&[
            "ldap".to_string(),
            "-x".to_string(),
            "--json".to_string(),
            "alice".to_string(),
        ])
        .expect("scan should succeed");

        assert_eq!(
            scanned.tokens,
            vec!["ldap".to_string(), "-x".to_string(), "alice".to_string()]
        );
        assert_eq!(scanned.invocation.format, Some(OutputFormat::Json));
    }

    #[test]
    fn conflicting_mode_color_and_unicode_flags_are_rejected() {
        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--plain".to_string(),
            "--mode".to_string(),
            "rich".to_string(),
        ])
        .expect_err("conflicting mode flags should fail");
        assert!(err.to_string().contains("conflicting render mode flags"));

        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--color".to_string(),
            "never".to_string(),
            "--color=always".to_string(),
        ])
        .expect_err("conflicting color flags should fail");
        assert!(err.to_string().contains("conflicting color mode flags"));

        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--ascii".to_string(),
            "--unicode".to_string(),
            "always".to_string(),
        ])
        .expect_err("conflicting unicode flags should fail");
        assert!(err.to_string().contains("conflicting unicode mode flags"));
    }

    #[test]
    fn duplicate_format_aliases_are_rejected_even_when_equivalent() {
        let err = scan_command_tokens(&[
            "ldap".to_string(),
            "--json".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .expect_err("duplicate format selectors should fail");
        assert!(err.to_string().contains("conflicting output format flags"));
    }
}
