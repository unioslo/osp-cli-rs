//! Host-facing REPL configuration and outcome types.
//!
//! These types are the stable semantic surface around the editor engine:
//! callers describe prompts, history, completion, and restart behavior here,
//! while the neighboring editor modules keep the lower-level reedline
//! integration private.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::completion::CompletionTree;

use super::super::history_store::HistoryConfig;

pub(crate) const DEFAULT_HISTORY_MENU_ROWS: u16 = 5;

/// Static prompt text shown by the interactive editor.
///
/// The right-hand prompt is configured separately through
/// [`PromptRightRenderer`] because it is often dynamic.
#[derive(Debug, Clone)]
pub struct ReplPrompt {
    /// Left prompt text shown before the input buffer.
    pub left: String,
    /// Prompt indicator rendered after `left`.
    pub indicator: String,
}

/// Lazily renders the right-hand prompt for a REPL frame.
pub type PromptRightRenderer = Arc<dyn Fn() -> String + Send + Sync>;

/// Pre-processed editor input used for completion and highlighting.
///
/// REPL input can contain host-level flags or aliases that should not
/// participate in command completion. A [`LineProjector`] can blank those
/// spans while also hiding corresponding suggestions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct LineProjection {
    /// Projected line passed to completion and highlighting.
    pub line: String,
    /// Suggestion values that should be hidden for this projection.
    pub hidden_suggestions: BTreeSet<String>,
}

impl LineProjection {
    /// Returns a projection that leaves the line untouched.
    pub fn passthrough(line: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            hidden_suggestions: BTreeSet::new(),
        }
    }

    /// Marks suggestion values that should be suppressed for this projection.
    pub fn with_hidden_suggestions(mut self, hidden_suggestions: BTreeSet<String>) -> Self {
        self.hidden_suggestions = hidden_suggestions;
        self
    }
}

/// Projects a raw editor line into the view used by completion/highlighting.
pub type LineProjector = Arc<dyn Fn(&str) -> LineProjection + Send + Sync>;

/// Selects how aggressively the REPL should use the interactive line editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplInputMode {
    /// Use the interactive editor when terminal capabilities support it, else
    /// fall back to basic stdin line reading.
    Auto,
    /// Prefer the interactive editor even when the cursor-position capability
    /// probe would skip it.
    ///
    /// Non-terminal stdin or stdout still force the basic fallback.
    Interactive,
    /// Use plain stdin line reading instead of `reedline`.
    Basic,
}

/// Controls how a command-triggered REPL restart should be presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplReloadKind {
    /// Rebuild the REPL and continue without reprinting the intro surface.
    Default,
    /// Rebuild the REPL and re-render the intro/help chrome.
    WithIntro,
}

/// Outcome of executing one submitted REPL line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplLineResult {
    /// Print output and continue the current session.
    Continue(String),
    /// Replace the current input buffer instead of printing output.
    ReplaceInput(String),
    /// Exit the REPL with the given process status.
    Exit(i32),
    /// Rebuild the REPL runtime, optionally showing intro chrome again.
    Restart {
        /// Output to print before restarting.
        output: String,
        /// Restart presentation mode to use.
        reload: ReplReloadKind,
    },
}

/// Outcome of one `run_repl` session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplRunResult {
    /// Exit the editor loop and return a process status.
    Exit(i32),
    /// Restart the surrounding REPL host loop with refreshed state.
    Restart {
        /// Output to print before restarting.
        output: String,
        /// Restart presentation mode to use.
        reload: ReplReloadKind,
    },
}

/// Editor-host configuration for one REPL run.
///
/// This is the semantic boundary the app host should configure. The engine
/// implementation may change, but callers should still only describe prompt,
/// completion, history, and input-mode intent here.
#[non_exhaustive]
#[must_use]
pub struct ReplRunConfig {
    /// Left prompt and indicator strings.
    pub prompt: ReplPrompt,
    /// Legacy root words used when no structured completion tree is provided.
    pub completion_words: Vec<String>,
    /// Structured completion tree for commands, flags, and pipe verbs.
    pub completion_tree: Option<CompletionTree>,
    /// Visual configuration for completion menus and command highlighting.
    pub appearance: ReplAppearance,
    /// History backend configuration for the session.
    pub history_config: HistoryConfig,
    /// Chooses between interactive and basic input handling.
    pub input_mode: ReplInputMode,
    /// Optional renderer for the right-hand prompt.
    pub prompt_right: Option<PromptRightRenderer>,
    /// Optional projector used before completion/highlighting analysis.
    pub line_projector: Option<LineProjector>,
}

impl ReplRunConfig {
    /// Creates the exact REPL runtime baseline for one run.
    ///
    /// The baseline starts with no completion words, no structured completion
    /// tree, default appearance overrides, [`ReplInputMode::Auto`], no
    /// right-hand prompt renderer, and no line projector.
    pub fn new(prompt: ReplPrompt, history_config: HistoryConfig) -> Self {
        Self {
            prompt,
            completion_words: Vec::new(),
            completion_tree: None,
            appearance: ReplAppearance::default(),
            history_config,
            input_mode: ReplInputMode::Auto,
            prompt_right: None,
            line_projector: None,
        }
    }

    /// Starts guided construction for a REPL run.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::repl::{HistoryConfig, ReplInputMode, ReplPrompt, ReplRunConfig};
    ///
    /// let config = ReplRunConfig::builder(
    ///     ReplPrompt::simple("osp> "),
    ///     HistoryConfig::builder().build(),
    /// )
    /// .with_completion_words(["help", "exit"])
    /// .with_input_mode(ReplInputMode::Basic)
    /// .build();
    ///
    /// assert_eq!(config.prompt.left, "osp> ");
    /// assert_eq!(config.input_mode, ReplInputMode::Basic);
    /// assert_eq!(
    ///     config.completion_words,
    ///     vec!["help".to_string(), "exit".to_string()]
    /// );
    /// ```
    pub fn builder(prompt: ReplPrompt, history_config: HistoryConfig) -> ReplRunConfigBuilder {
        ReplRunConfigBuilder::new(prompt, history_config)
    }
}

/// Builder for [`ReplRunConfig`].
#[must_use]
pub struct ReplRunConfigBuilder {
    config: ReplRunConfig,
}

impl ReplRunConfigBuilder {
    /// Starts a builder from the required prompt and history settings.
    pub fn new(prompt: ReplPrompt, history_config: HistoryConfig) -> Self {
        Self {
            config: ReplRunConfig::new(prompt, history_config),
        }
    }

    /// Replaces the legacy fallback completion words.
    ///
    /// If omitted, the config keeps an empty fallback word list.
    pub fn with_completion_words<I, S>(mut self, completion_words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.completion_words = completion_words.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the structured completion tree.
    ///
    /// If omitted, the REPL uses no structured completion tree.
    pub fn with_completion_tree(mut self, completion_tree: Option<CompletionTree>) -> Self {
        self.config.completion_tree = completion_tree;
        self
    }

    /// Replaces the REPL appearance overrides.
    ///
    /// If omitted, the config keeps [`ReplAppearance::default`].
    pub fn with_appearance(mut self, appearance: ReplAppearance) -> Self {
        self.config.appearance = appearance;
        self
    }

    /// Replaces the history configuration.
    ///
    /// If omitted, the builder keeps the history configuration passed to
    /// [`ReplRunConfigBuilder::new`].
    pub fn with_history_config(mut self, history_config: HistoryConfig) -> Self {
        self.config.history_config = history_config;
        self
    }

    /// Replaces the input-mode policy.
    ///
    /// If omitted, the config keeps [`ReplInputMode::Auto`].
    pub fn with_input_mode(mut self, input_mode: ReplInputMode) -> Self {
        self.config.input_mode = input_mode;
        self
    }

    /// Replaces the optional right-prompt renderer.
    ///
    /// If omitted, the REPL renders no right-hand prompt.
    pub fn with_prompt_right(mut self, prompt_right: Option<PromptRightRenderer>) -> Self {
        self.config.prompt_right = prompt_right;
        self
    }

    /// Replaces the optional completion/highlighting line projector.
    ///
    /// If omitted, the REPL analyzes the raw input line directly.
    pub fn with_line_projector(mut self, line_projector: Option<LineProjector>) -> Self {
        self.config.line_projector = line_projector;
        self
    }

    /// Builds the configured [`ReplRunConfig`].
    pub fn build(self) -> ReplRunConfig {
        self.config
    }
}

impl ReplPrompt {
    /// Builds a prompt with no indicator suffix.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::repl::ReplPrompt;
    ///
    /// let prompt = ReplPrompt::simple("osp> ");
    ///
    /// assert_eq!(prompt.left, "osp> ");
    /// assert!(prompt.indicator.is_empty());
    /// ```
    pub fn simple(left: impl Into<String>) -> Self {
        Self {
            left: left.into(),
            indicator: String::new(),
        }
    }
}

/// Style overrides for REPL-only completion and highlighting chrome.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct ReplAppearance {
    /// Style applied to non-selected completion text.
    pub completion_text_style: Option<String>,
    /// Background style applied to the completion menu.
    pub completion_background_style: Option<String>,
    /// Style applied to the selected completion entry.
    pub completion_highlight_style: Option<String>,
    /// Style applied to recognized command segments in the input line.
    pub command_highlight_style: Option<String>,
    /// Maximum number of visible rows in the history search menu.
    pub history_menu_rows: u16,
}

impl ReplAppearance {
    /// Starts guided construction for REPL-only appearance overrides.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::repl::ReplAppearance;
    ///
    /// let appearance = ReplAppearance::builder()
    ///     .with_history_menu_rows(8)
    ///     .with_command_highlight_style(Some("green".to_string()))
    ///     .build();
    ///
    /// assert_eq!(appearance.history_menu_rows, 8);
    /// assert_eq!(appearance.command_highlight_style.as_deref(), Some("green"));
    /// ```
    pub fn builder() -> ReplAppearanceBuilder {
        ReplAppearanceBuilder::new()
    }
}

impl Default for ReplAppearance {
    fn default() -> Self {
        Self {
            completion_text_style: None,
            completion_background_style: None,
            completion_highlight_style: None,
            command_highlight_style: None,
            history_menu_rows: DEFAULT_HISTORY_MENU_ROWS,
        }
    }
}

/// Builder for [`ReplAppearance`].
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ReplAppearanceBuilder {
    appearance: ReplAppearance,
}

impl ReplAppearanceBuilder {
    /// Starts a builder from the default REPL appearance baseline.
    pub fn new() -> Self {
        Self {
            appearance: ReplAppearance::default(),
        }
    }

    /// Replaces the style applied to non-selected completion text.
    ///
    /// If omitted, the REPL keeps the theme/default completion text style.
    pub fn with_completion_text_style(mut self, completion_text_style: Option<String>) -> Self {
        self.appearance.completion_text_style = completion_text_style;
        self
    }

    /// Replaces the menu background style.
    ///
    /// If omitted, the REPL keeps the theme/default completion background
    /// style.
    pub fn with_completion_background_style(
        mut self,
        completion_background_style: Option<String>,
    ) -> Self {
        self.appearance.completion_background_style = completion_background_style;
        self
    }

    /// Replaces the style applied to the selected completion entry.
    ///
    /// If omitted, the REPL keeps the theme/default completion highlight
    /// style.
    pub fn with_completion_highlight_style(
        mut self,
        completion_highlight_style: Option<String>,
    ) -> Self {
        self.appearance.completion_highlight_style = completion_highlight_style;
        self
    }

    /// Replaces the style applied to recognized command segments.
    ///
    /// If omitted, the REPL keeps the theme/default command-highlight style.
    pub fn with_command_highlight_style(mut self, command_highlight_style: Option<String>) -> Self {
        self.appearance.command_highlight_style = command_highlight_style;
        self
    }

    /// Replaces the maximum number of visible history-menu rows.
    ///
    /// If omitted, the builder keeps the default history-menu row count.
    pub fn with_history_menu_rows(mut self, history_menu_rows: u16) -> Self {
        self.appearance.history_menu_rows = history_menu_rows;
        self
    }

    /// Builds the configured [`ReplAppearance`].
    pub fn build(self) -> ReplAppearance {
        self.appearance
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReplAppearance, ReplInputMode, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunResult,
    };
    use crate::repl::HistoryConfig;

    #[test]
    fn run_config_builder_captures_host_surface_choices() {
        let appearance = ReplAppearance::builder()
            .with_history_menu_rows(8)
            .with_command_highlight_style(Some("green".to_string()))
            .build();
        let config = ReplRunConfig::builder(
            ReplPrompt::simple("osp> "),
            HistoryConfig::builder().build(),
        )
        .with_completion_words(["help", "exit"])
        .with_appearance(appearance.clone())
        .with_input_mode(ReplInputMode::Basic)
        .build();

        assert_eq!(config.prompt.left, "osp> ");
        assert_eq!(config.input_mode, ReplInputMode::Basic);
        assert_eq!(
            config.completion_words,
            vec!["help".to_string(), "exit".to_string()]
        );
        assert_eq!(config.appearance.history_menu_rows, 8);
        assert_eq!(
            config.appearance.command_highlight_style.as_deref(),
            Some("green")
        );
    }

    #[test]
    fn prompt_and_restart_outcomes_stay_plain_semantic_payloads() {
        let prompt = ReplPrompt::simple("osp");
        assert_eq!(prompt.left, "osp");
        assert!(prompt.indicator.is_empty());

        let restart = ReplRunResult::Restart {
            output: "reloading".to_string(),
            reload: ReplReloadKind::WithIntro,
        };
        assert!(matches!(
            restart,
            ReplRunResult::Restart {
                output,
                reload: ReplReloadKind::WithIntro
            } if output == "reloading"
        ));
    }
}
