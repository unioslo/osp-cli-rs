use skim::CaseMatching;
use skim::fuzzy_matcher::arinae::ArinaeMatcher;
use std::sync::OnceLock;

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

/// Conservative fuzzy matcher for completion suggestions.
///
/// Completion should rescue near-misses like `lap -> ldap`, but it should not
/// spill short stubs like `ld` into unrelated commands. Arinae with typo mode
/// disabled keeps that balance while still handling subsequence-style fuzzy
/// matches well.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::completion_fuzzy_matcher;
/// use skim::fuzzy_matcher::FuzzyMatcher;
///
/// assert!(completion_fuzzy_matcher()
///     .fuzzy_match("ldap", "lap")
///     .is_some());
/// ```
pub fn completion_fuzzy_matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| ArinaeMatcher::new(CaseMatching::Smart, false, false))
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
/// use osp_cli::core::fuzzy::config_fuzzy_matcher;
/// use skim::fuzzy_matcher::FuzzyMatcher;
///
/// assert!(config_fuzzy_matcher()
///     .fuzzy_match("ui.format", "ui.formt")
///     .is_some());
/// ```
pub fn config_fuzzy_matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| ArinaeMatcher::new(CaseMatching::Smart, true, false))
}

/// Typo-tolerant fuzzy matcher for explicit DSL `%quick` searches.
///
/// `%quick` is the opt-in "be clever" path, so it intentionally accepts a
/// broader set of typo-like matches than shell completion does.
///
/// # Examples
///
/// ```
/// use osp_cli::core::fuzzy::search_fuzzy_matcher;
/// use skim::fuzzy_matcher::FuzzyMatcher;
///
/// assert!(search_fuzzy_matcher()
///     .fuzzy_match("doctor --mreg", "doctr mreg")
///     .is_some());
/// ```
pub fn search_fuzzy_matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| ArinaeMatcher::new(CaseMatching::Smart, true, false))
}
