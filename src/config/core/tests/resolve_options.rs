#[test]
fn resolve_options_new_and_with_helpers_normalize_inputs_unit() {
    let options = super::ResolveOptions::new()
        .with_profile(" Dev ")
        .with_terminal(" REPL ")
        .with_profile_override(Some(" Prod ".to_string()))
        .with_terminal_override(Some(" CLI ".to_string()));

    assert_eq!(options.profile_override.as_deref(), Some("prod"));
    assert_eq!(options.terminal.as_deref(), Some("cli"));
}
