#[cfg(unix)]
#[test]
fn help_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-help-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (presentation, color, unicode) in cases {
        let output = help_output(&config_path, presentation, color, unicode);
        let plain = strip_ansi(&output);

        assert!(
            plain.contains("Usage"),
            "missing usage for {presentation}/{color}/{unicode}: {plain:?}"
        );

        if color == "always" {
            assert!(
                output.contains("\x1b[32mUsage"),
                "missing title color for {presentation}/{color}/{unicode}: {output:?}"
            );
        } else {
            assert!(
                !output.contains("\x1b["),
                "unexpected ANSI for {presentation}/{color}/{unicode}: {output:?}"
            );
        }

        match presentation {
            "expressive" => {
                if color == "always" && unicode == "always" {
                    assert!(
                        output.contains("\x1b[31m─ "),
                        "missing unicode border color for {presentation}: {output:?}"
                    );
                } else if color == "always" && unicode == "never" {
                    assert!(
                        output.contains("\x1b[31m- "),
                        "missing ascii border color for {presentation}: {output:?}"
                    );
                }

                if unicode == "always" {
                    assert!(
                        plain.contains("─ Usage"),
                        "missing unicode chrome for {presentation}/{color}: {plain:?}"
                    );
                } else {
                    assert!(
                        plain.contains("- Usage"),
                        "missing ascii chrome for {presentation}/{color}: {plain:?}"
                    );
                }
            }
            "compact" | "austere" => {
                assert!(
                    plain.contains("Usage: osp [OPTIONS] [COMMAND]"),
                    "{presentation} help should stay clap-like for {color}/{unicode}: {plain:?}"
                );
                assert!(
                    !plain.contains("─ Usage") && !plain.contains("- Usage"),
                    "{presentation} help should not render ruled section chrome for {color}/{unicode}: {plain:?}"
                );
                if color == "always" {
                    assert!(
                        output.contains("\x1b[33mplugins\x1b[0m"),
                        "missing command key color for {presentation}/{unicode}: {output:?}"
                    );
                } else {
                    assert!(
                        !output.contains("\x1b["),
                        "unexpected ANSI for {presentation}/{color}/{unicode}: {output:?}"
                    );
                }
            }
            _ => unreachable!("unknown presentation: {presentation}"),
        }
    }

}

#[cfg(unix)]
#[test]
fn table_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-table-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (presentation, color, unicode) in cases {
        let output = table_output(&config_path, presentation, color, unicode);
        let plain = strip_ansi(&output);

        assert!(
            plain.contains("id") && plain.contains("name"),
            "missing table headers for {presentation}/{color}/{unicode}: {plain:?}"
        );

        if color == "always" {
            assert!(
                output.contains("\x1b[34mid\x1b[0m"),
                "missing header color for {presentation}/{color}/{unicode}: {output:?}"
            );
        } else {
            assert!(
                !output.contains("\x1b["),
                "unexpected ANSI for {presentation}/{color}/{unicode}: {output:?}"
            );
        }

        if unicode == "always" {
            let expected_corner = if presentation == "expressive" {
                '╭'
            } else {
                '┏'
            };
            assert!(
                plain.starts_with(expected_corner),
                "unexpected unicode table chrome for {presentation}/{color}: {plain:?}"
            );
        } else {
            assert!(
                plain.starts_with('+'),
                "unexpected ascii table chrome for {presentation}/{color}: {plain:?}"
            );
        }
    }

}

#[cfg(unix)]
#[test]
fn message_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-message-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (index, (presentation, color, unicode)) in cases.into_iter().enumerate() {
        let xdg_config_home = home.join(format!("xdg-{index}"));
        std::fs::create_dir_all(&xdg_config_home).expect("xdg config home should be created");

        let success_info = success_info_output(&config_path, presentation, color, unicode);
        let success_plain = strip_ansi(&success_info);
        assert!(
            success_plain.contains("active theme set to: plain"),
            "missing success body for {presentation}/{color}/{unicode}: {success_plain:?}"
        );
        assert!(
            success_plain.contains("theme change is for the current process"),
            "missing info body for {presentation}/{color}/{unicode}: {success_plain:?}"
        );

        let warning_success =
            warning_success_output(&config_path, &xdg_config_home, presentation, color, unicode);
        let warning_plain = strip_ansi(&warning_success);
        assert!(
            warning_plain.contains("writing a sensitive key to config store; prefer --secrets"),
            "missing warning body for {presentation}/{color}/{unicode}: {warning_plain:?}"
        );
        assert!(
            warning_plain.contains("set value for ui.prompt.secrets"),
            "missing success body for {presentation}/{color}/{unicode}: {warning_plain:?}"
        );

        let error = error_output(&config_path, presentation, color, unicode);
        let error_plain = strip_ansi(&error);
        assert!(
            error_plain.contains("config key not found: missing.key"),
            "missing error body for {presentation}/{color}/{unicode}: {error_plain:?}"
        );

        if color == "always" {
            match presentation {
                "austere" => {
                    assert!(
                        success_info.contains("\x1b[92msuccess\x1b[0m"),
                        "missing austere success color for {unicode}: {success_info:?}"
                    );
                    assert!(
                        success_info.contains("\x1b[34minfo\x1b[0m"),
                        "missing austere info color for {unicode}: {success_info:?}"
                    );
                    assert!(
                        warning_success.contains("\x1b[33mwarning\x1b[0m"),
                        "missing austere warning color for {unicode}: {warning_success:?}"
                    );
                    assert!(
                        error.contains("\x1b[31merror\x1b[0m"),
                        "missing austere error color for {unicode}: {error:?}"
                    );
                }
                "compact" => {
                    assert!(
                        success_info.contains("\x1b[92m") && success_info.contains("Success:"),
                        "missing compact success color for {unicode}: {success_info:?}"
                    );
                    assert!(
                        success_info.contains("\x1b[34m") && success_info.contains("Info:"),
                        "missing compact info color for {unicode}: {success_info:?}"
                    );
                    assert!(
                        warning_success.contains("\x1b[33m") && warning_success.contains("Warnings:"),
                        "missing compact warning color for {unicode}: {warning_success:?}"
                    );
                    assert!(
                        error.contains("\x1b[31m") && error.contains("Errors:"),
                        "missing compact error color for {unicode}: {error:?}"
                    );
                }
                _ => {
                    assert!(
                        success_info.contains("\x1b[92m") && success_info.contains("Success"),
                        "missing full success color for {presentation}/{unicode}: {success_info:?}"
                    );
                    assert!(
                        success_info.contains("\x1b[34m") && success_info.contains("Info"),
                        "missing full info color for {presentation}/{unicode}: {success_info:?}"
                    );
                    assert!(
                        warning_success.contains("\x1b[33m") && warning_success.contains("Warnings"),
                        "missing full warning color for {presentation}/{unicode}: {warning_success:?}"
                    );
                    assert!(
                        error.contains("\x1b[31m") && error.contains("Errors"),
                        "missing full error color for {presentation}/{unicode}: {error:?}"
                    );
                }
            }
        } else {
            assert!(!success_info.contains("\x1b["));
            assert!(!warning_success.contains("\x1b["));
            assert!(!error.contains("\x1b["));
        }

        match presentation {
            "austere" => {
                assert!(
                    success_plain.contains("  success: active theme set to: plain"),
                    "austere success should be minimal for {color}/{unicode}: {success_plain:?}"
                );
                assert!(
                    warning_plain.contains("  warning: writing a sensitive key"),
                    "austere warning should be minimal for {color}/{unicode}: {warning_plain:?}"
                );
                assert!(
                    error_plain.contains("  error: config key not found: missing.key"),
                    "austere error should be minimal for {color}/{unicode}: {error_plain:?}"
                );
            }
            "compact" => {
                assert!(
                    success_plain.contains("Success:\n  active theme set to: plain"),
                    "compact success should use titled paragraphs for {color}/{unicode}: {success_plain:?}"
                );
                assert!(
                    warning_plain.contains("Warnings:\n  writing a sensitive key"),
                    "compact warning should use titled paragraphs for {color}/{unicode}: {warning_plain:?}"
                );
                assert!(
                    error_plain.contains("Errors:\n  config key not found: missing.key"),
                    "compact error should use titled paragraphs for {color}/{unicode}: {error_plain:?}"
                );
                let rule = if unicode == "always" { '─' } else { '-' };
                assert!(
                    !success_plain.contains(rule),
                    "compact success should not use ruled chrome for {color}/{unicode}: {success_plain:?}"
                );
            }
            _ => {
                let rule = if unicode == "always" { '─' } else { '-' };
                assert!(
                    success_plain.contains(rule) && success_plain.contains("Success"),
                    "full success should keep ruled chrome for {presentation}/{color}/{unicode}: {success_plain:?}"
                );
                assert!(
                    warning_plain.contains(rule) && warning_plain.contains("Warnings"),
                    "full warning should keep ruled chrome for {presentation}/{color}/{unicode}: {warning_plain:?}"
                );
                assert!(
                    error_plain.contains(rule) && error_plain.contains("Errors"),
                    "full error should keep ruled chrome for {presentation}/{color}/{unicode}: {error_plain:?}"
                );
            }
        }
    }

}
