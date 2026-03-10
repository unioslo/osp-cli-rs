use skim::CaseMatching;
use skim::fuzzy_matcher::arinae::ArinaeMatcher;
use std::sync::OnceLock;

/// Lowercases text using Unicode case folding semantics.
pub fn fold_case(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Conservative fuzzy matcher for completion suggestions.
///
/// Completion should rescue near-misses like `lap -> ldap`, but it should not
/// spill short stubs like `ld` into unrelated commands. Arinae with typo mode
/// disabled keeps that balance while still handling subsequence-style fuzzy
/// matches well.
pub fn completion_fuzzy_matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| ArinaeMatcher::new(CaseMatching::Smart, false, false))
}

/// Typo-tolerant fuzzy matcher for explicit DSL `%quick` searches.
///
/// `%quick` is the opt-in "be clever" path, so it intentionally accepts a
/// broader set of typo-like matches than shell completion does.
pub fn search_fuzzy_matcher() -> &'static ArinaeMatcher {
    static MATCHER: OnceLock<ArinaeMatcher> = OnceLock::new();
    MATCHER.get_or_init(|| ArinaeMatcher::new(CaseMatching::Smart, true, false))
}
