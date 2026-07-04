//! Shared fuzzy-matching policies.
//!
//! Different product surfaces want different typo tolerance. This module keeps
//! those matcher choices in one place so completion, config recovery, and
//! search do not each invent their own fuzzy behavior.

use std::sync::OnceLock;

#[cfg(not(miri))]
use skim::CaseMatching;
#[cfg(not(miri))]
use skim::fuzzy_matcher::FuzzyMatcher as SkimFuzzyMatcher;
#[cfg(not(miri))]
use skim::fuzzy_matcher::arinae::ArinaeMatcher;

/// Lowercases text using Unicode case folding semantics.
///
/// This is stricter than ASCII-only lowercasing, so it is safe to use for
/// case-insensitive matching on user-facing text.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::fold_case;
///
/// assert_eq!(fold_case("LDAP"), "ldap");
/// assert_eq!(fold_case("ÅSE"), "åse");
/// ```
pub fn fold_case(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Small fuzzy-matcher contract used by completion and recovery helpers.
pub trait FuzzyMatcher {
    /// Returns a lower-is-better score when `query` fuzzily matches `candidate`.
    fn fuzzy_match(&self, candidate: &str, query: &str) -> Option<i64>;

    /// Returns the score plus the matched character indices in `candidate`.
    fn fuzzy_indices(&self, candidate: &str, query: &str) -> Option<(i64, Vec<usize>)>;
}

#[derive(Debug)]
struct AppFuzzyMatcher {
    #[cfg(not(miri))]
    inner: ArinaeMatcher,
    #[cfg(miri)]
    allow_word_skip: bool,
}

impl AppFuzzyMatcher {
    #[cfg(not(miri))]
    const fn new(inner: ArinaeMatcher) -> Self {
        Self { inner }
    }

    #[cfg(miri)]
    const fn new(allow_word_skip: bool) -> Self {
        Self { allow_word_skip }
    }
}

#[cfg(not(miri))]
impl FuzzyMatcher for AppFuzzyMatcher {
    fn fuzzy_match(&self, candidate: &str, query: &str) -> Option<i64> {
        self.inner.fuzzy_match(candidate, query)
    }

    fn fuzzy_indices(&self, candidate: &str, query: &str) -> Option<(i64, Vec<usize>)> {
        self.inner.fuzzy_indices(candidate, query)
    }
}

#[cfg(miri)]
impl FuzzyMatcher for AppFuzzyMatcher {
    fn fuzzy_match(&self, candidate: &str, query: &str) -> Option<i64> {
        subsequence_match(candidate, query, self.allow_word_skip).map(|(score, _)| score)
    }

    fn fuzzy_indices(&self, candidate: &str, query: &str) -> Option<(i64, Vec<usize>)> {
        subsequence_match(candidate, query, self.allow_word_skip)
    }
}

#[cfg(miri)]
fn subsequence_match(
    candidate: &str,
    query: &str,
    allow_word_skip: bool,
) -> Option<(i64, Vec<usize>)> {
    let candidate_lc = fold_case(candidate);
    let query_lc = fold_case(query);
    if query_lc.is_empty() {
        return Some((0, Vec::new()));
    }
    if candidate_lc == query_lc {
        let indices = candidate_lc.char_indices().map(|(idx, _)| idx).collect();
        return Some((0, indices));
    }

    let mut indices = Vec::new();
    let mut search_start = 0usize;
    for query_ch in query_lc.chars() {
        let haystack = &candidate_lc[search_start..];
        let relative = haystack.find(query_ch)?;
        let absolute = search_start + relative;
        indices.push(absolute);
        search_start = absolute + query_ch.len_utf8();
    }

    let gap_penalty = indices
        .windows(2)
        .map(|window| window[1].saturating_sub(window[0] + 1) as i64)
        .sum::<i64>();
    let prefix_bonus = indices.first().copied().unwrap_or_default() as i64;
    let word_penalty = if allow_word_skip {
        0
    } else {
        candidate_lc.split_whitespace().count().saturating_sub(1) as i64
    };
    Some((gap_penalty + prefix_bonus + word_penalty, indices))
}

/// Conservative fuzzy matcher for completion suggestions.
///
/// Completion should rescue near-misses like `lap -> ldap`, but it should not
/// spill short stubs like `ld` into unrelated commands. The normal build uses
/// skim's conservative matcher; Miri falls back to a narrower subsequence
/// matcher so the repo can still run under the interpreter.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::{FuzzyMatcher, completion_fuzzy_matcher};
///
/// assert!(completion_fuzzy_matcher()
///     .fuzzy_match("ldap", "lap")
///     .is_some());
/// ```
pub fn completion_fuzzy_matcher() -> &'static dyn FuzzyMatcher {
    static MATCHER: OnceLock<AppFuzzyMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| {
        #[cfg(not(miri))]
        {
            AppFuzzyMatcher::new(ArinaeMatcher::new(CaseMatching::Smart, false, false))
        }
        #[cfg(miri)]
        {
            AppFuzzyMatcher::new(false)
        }
    })
}

/// Typo-tolerant fuzzy matcher for config-key recovery suggestions.
///
/// Config lookup failures should help with misspellings like
/// `ui.formt -> ui.format`, but they should still stay narrower than broad
/// search-oriented matching. Callers are expected to pair this matcher with
/// explicit ranking such as same-namespace and last-segment preference.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::{FuzzyMatcher, config_fuzzy_matcher};
///
/// assert!(config_fuzzy_matcher()
///     .fuzzy_match("ui.format", "ui.formt")
///     .is_some());
/// ```
pub fn config_fuzzy_matcher() -> &'static dyn FuzzyMatcher {
    static MATCHER: OnceLock<AppFuzzyMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| {
        #[cfg(not(miri))]
        {
            AppFuzzyMatcher::new(ArinaeMatcher::new(CaseMatching::Smart, true, false))
        }
        #[cfg(miri)]
        {
            AppFuzzyMatcher::new(true)
        }
    })
}

/// Typo-tolerant fuzzy matcher for explicit DSL `%quick` searches.
///
/// `%quick` is the opt-in "be clever" path, so it intentionally accepts a
/// broader set of typo-like matches than shell completion does.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::{FuzzyMatcher, search_fuzzy_matcher};
///
/// assert!(search_fuzzy_matcher()
///     .fuzzy_match("doctor --mreg", "doctr mreg")
///     .is_some());
/// ```
pub fn search_fuzzy_matcher() -> &'static dyn FuzzyMatcher {
    static MATCHER: OnceLock<AppFuzzyMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| {
        #[cfg(not(miri))]
        {
            AppFuzzyMatcher::new(ArinaeMatcher::new(CaseMatching::Smart, true, false))
        }
        #[cfg(miri)]
        {
            AppFuzzyMatcher::new(true)
        }
    })
}
