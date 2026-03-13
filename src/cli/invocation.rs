use std::collections::BTreeSet;
use std::ffi::OsString;

use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::guide::GuideView;
use crate::ui::presentation::HelpLevel;
use miette::{Result, miette};

pub(crate) const INVOCATION_HELP_SECTION: &str = r#"Common Invocation Options:
  --format <auto|guide|json|table|mreg|value|md>
                                            Format this invocation only
  --guide | --json | --table | --mreg | --value | --md
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
    "--guide",
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

const FORMAT_COMPLETION_FLAGS: &[&str] = &[
    "--format", "--guide", "--json", "--table", "--mreg", "--value", "--md",
];
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
                    _ => {}
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

pub(crate) fn invocation_help_view() -> GuideView {
    GuideView::from_text(INVOCATION_HELP_SECTION)
}

pub(crate) fn extend_with_invocation_help(view: &mut GuideView, level: HelpLevel) {
    if level >= HelpLevel::Verbose {
        view.merge(invocation_help_view());
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
    Guide,
    Json,
    Table,
    Value,
    Markdown,
    Mreg,
}

impl FormatAlias {
    fn parse(token: &str) -> Option<Self> {
        match token {
            "--guide" => Some(Self::Guide),
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
            Self::Guide => OutputFormat::Guide,
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
        INVOCATION_HELP_SECTION, InvocationOptions, extend_with_invocation_help,
        hidden_invocation_completion_flags, invocation_help_view, scan_cli_argv,
        scan_command_tokens, scan_command_tokens_with_trace, should_show_invocation_help,
    };
    use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use crate::guide::GuideView;
    use crate::ui::presentation::HelpLevel;
    use std::ffi::OsString;

    fn scan(tokens: &[&str]) -> super::ScannedCommandTokens {
        scan_command_tokens(
            &tokens
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>(),
        )
        .expect("scan should succeed")
    }

    fn scan_error(tokens: &[&str]) -> String {
        scan_command_tokens(
            &tokens
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>(),
        )
        .expect_err("scan should fail")
        .to_string()
    }

    #[test]
    fn scan_command_tokens_covers_host_flags_aliases_clusters_and_trace_unit() {
        let scanned = scan(&[
            "ldap",
            "user",
            "oistes",
            "--json",
            "-vv",
            "--plugin-provider",
            "uio-ldap",
        ]);
        assert_eq!(scanned.tokens, vec!["ldap", "user", "oistes"]);
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

        let passthrough = scan(&["ldap", "--", "--json", "-vv"]);
        assert_eq!(passthrough.tokens, vec!["ldap", "--", "--json", "-vv"]);
        assert_eq!(passthrough.invocation, InvocationOptions::default());

        let cache = scan(&["ldap", "user", "alice", "--cache"]);
        assert_eq!(cache.tokens, vec!["ldap", "user", "alice"]);
        assert!(cache.invocation.cache);

        let non_host_short = scan(&["ldap", "-x", "--json", "alice"]);
        assert_eq!(non_host_short.tokens, vec!["ldap", "-x", "alice"]);
        assert_eq!(non_host_short.invocation.format, Some(OutputFormat::Json));

        let traced = scan_command_tokens_with_trace(
            &["ldap", "--json", "user", "alice"]
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>(),
        )
        .expect("trace scan should succeed");
        assert_eq!(traced.tokens, vec!["ldap", "user", "alice"]);
        assert_eq!(traced.kept_indices, vec![0, 2, 3]);

        let explicit_aliases = scan(&["ldap", "--plain", "--color", "never", "--ascii"]);
        assert_eq!(explicit_aliases.invocation.mode, Some(RenderMode::Plain));
        assert_eq!(explicit_aliases.invocation.color, Some(ColorMode::Never));
        assert_eq!(
            explicit_aliases.invocation.unicode,
            Some(UnicodeMode::Never)
        );

        let inline = scan(&[
            "ldap",
            "--format=value",
            "--mode=rich",
            "--color=always",
            "--unicode=always",
        ]);
        assert_eq!(inline.invocation.format, Some(OutputFormat::Value));
        assert_eq!(inline.invocation.mode, Some(RenderMode::Rich));
        assert_eq!(inline.invocation.color, Some(ColorMode::Always));
        assert_eq!(inline.invocation.unicode, Some(UnicodeMode::Always));

        let clustered = scan(&["ldap", "-vvqd", "user"]);
        assert_eq!(clustered.invocation.verbose, 2);
        assert_eq!(clustered.invocation.quiet, 1);
        assert_eq!(clustered.invocation.debug, 1);
        assert_eq!(clustered.tokens, vec!["ldap", "user"]);

        let guide = scan(&["ldap", "--guide"]);
        assert_eq!(guide.invocation.format, Some(OutputFormat::Guide));
    }

    #[test]
    fn scan_command_tokens_report_conflicts_and_missing_values_unit() {
        for (tokens, expected) in [
            (
                vec!["ldap", "--json", "--format", "table"],
                "conflicting output format flags",
            ),
            (
                vec!["ldap", "--json", "--format", "json"],
                "conflicting output format flags",
            ),
            (
                vec!["ldap", "--plugin-provider", "one", "--plugin-provider=two"],
                "specified more than once",
            ),
            (vec!["ldap", "--mode", "wat"], "unknown render mode"),
            (
                vec!["ldap", "--plugin-provider", "   "],
                "expects a non-empty value",
            ),
            (vec!["ldap", "--format"], "`--format` expects a value"),
            (vec!["ldap", "--color"], "`--color` expects a value"),
            (
                vec!["ldap", "--plain", "--mode", "rich"],
                "conflicting render mode flags",
            ),
            (
                vec!["ldap", "--color", "never", "--color=always"],
                "conflicting color mode flags",
            ),
            (
                vec!["ldap", "--ascii", "--unicode", "always"],
                "conflicting unicode mode flags",
            ),
        ] {
            assert!(
                scan_error(&tokens).contains(expected),
                "expected {expected:?} for tokens {tokens:?}"
            );
        }
    }

    #[test]
    fn invocation_help_hidden_completion_and_cli_argv_follow_visibility_and_flag_only_inputs_unit()
    {
        let rendered = invocation_help_view();
        assert!(!rendered.common_invocation_options.is_empty());
        assert!(
            rendered
                .common_invocation_options
                .iter()
                .any(|entry| entry.name.contains("--guide"))
        );
        assert!(
            rendered
                .common_invocation_options
                .iter()
                .any(|entry| entry.name.contains("--json"))
        );
        assert!(INVOCATION_HELP_SECTION.contains("--cache"));
        assert!(INVOCATION_HELP_SECTION.contains("interactive REPL"));
        assert!(INVOCATION_HELP_SECTION.contains("--guide"));
        assert!(INVOCATION_HELP_SECTION.contains("guide|json|table"));

        assert!(!should_show_invocation_help(&InvocationOptions::default()));
        assert!(should_show_invocation_help(&InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        }));

        let mut hidden = GuideView::from_text("Usage: osp [COMMAND]\n");
        extend_with_invocation_help(&mut hidden, HelpLevel::Normal);
        assert!(hidden.common_invocation_options.is_empty());

        let mut visible = GuideView::from_text("Usage: osp [COMMAND]\n");
        extend_with_invocation_help(&mut visible, HelpLevel::Verbose);
        assert!(!visible.common_invocation_options.is_empty());

        let hidden = hidden_invocation_completion_flags(&InvocationOptions::default());
        assert!(hidden.contains("--guide"));
        assert!(hidden.contains("--json"));
        assert!(hidden.contains("--plugin-provider"));
        assert!(hidden.contains("--debug"));

        let verbose = hidden_invocation_completion_flags(&InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        });
        assert!(!verbose.contains("--json"));
        assert!(!verbose.contains("--plugin-provider"));
        assert!(!verbose.contains("--debug"));

        let used_one_shots = hidden_invocation_completion_flags(&InvocationOptions {
            verbose: 1,
            format: Some(OutputFormat::Json),
            cache: true,
            plugin_provider: Some("ldap".to_string()),
            ..InvocationOptions::default()
        });
        assert!(used_one_shots.contains("--format"));
        assert!(used_one_shots.contains("--guide"));
        assert!(used_one_shots.contains("--json"));
        assert!(used_one_shots.contains("--table"));
        assert!(used_one_shots.contains("--cache"));
        assert!(used_one_shots.contains("--plugin-provider"));
        assert!(!used_one_shots.contains("--debug"));

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

        let empty = scan_cli_argv(&[]).expect("empty argv should scan");
        assert!(empty.argv.is_empty());
        assert_eq!(empty.invocation, InvocationOptions::default());

        let only_flags = scan_cli_argv(&[
            OsString::from("osp"),
            OsString::from("--json"),
            OsString::from("-vv"),
        ])
        .expect("cli argv scan should succeed");
        assert_eq!(only_flags.argv, vec![OsString::from("osp")]);
        assert_eq!(only_flags.invocation.format, Some(OutputFormat::Json));
        assert_eq!(only_flags.invocation.verbose, 2);
    }
}
