use std::sync::Mutex;

use super::{
    ClipboardError, ClipboardService,
    backend::{
        ClipboardCommand, OSC52_MAX_BYTES_DEFAULT, base64_encode, base64_encoded_len,
        copy_via_command, osc52_enabled, osc52_max_bytes, platform_backends,
    },
};

fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

fn acquire_env_lock() -> std::sync::MutexGuard<'static, ()> {
    env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_path_for_test(value: Option<&str>) {
    let key = "PATH";
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

fn set_env_for_test(key: &str, value: Option<&str>) {
    match value {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

#[test]
fn base64_encoder_matches_known_values() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
}

#[test]
fn base64_length_and_env_helpers_behave_predictably() {
    let _guard = acquire_env_lock();
    assert_eq!(base64_encoded_len(0), 0);
    assert_eq!(base64_encoded_len(1), 4);
    assert_eq!(base64_encoded_len(4), 8);

    let osc52_original = std::env::var("OSC52").ok();
    let max_original = std::env::var("OSC52_MAX_BYTES").ok();

    set_env_for_test("OSC52", Some("off"));
    assert!(!osc52_enabled());
    set_env_for_test("OSC52", Some("yes"));
    assert!(osc52_enabled());

    set_env_for_test("OSC52_MAX_BYTES", Some("4096"));
    assert_eq!(osc52_max_bytes(), 4096);
    set_env_for_test("OSC52_MAX_BYTES", Some("0"));
    assert_eq!(osc52_max_bytes(), OSC52_MAX_BYTES_DEFAULT);

    set_env_for_test("OSC52", osc52_original.as_deref());
    set_env_for_test("OSC52_MAX_BYTES", max_original.as_deref());
}

#[test]
fn clipboard_error_display_covers_backend_spawn_and_status_cases() {
    assert_eq!(
        ClipboardError::NoBackendAvailable {
            attempts: vec!["osc52".to_string(), "xclip".to_string()],
        }
        .to_string(),
        "no clipboard backend available (tried: osc52, xclip)"
    );
    assert_eq!(
        ClipboardError::SpawnFailed {
            command: "xclip".to_string(),
            reason: "missing".to_string(),
        }
        .to_string(),
        "failed to start clipboard command `xclip`: missing"
    );
    assert_eq!(
        ClipboardError::CommandFailed {
            command: "xclip".to_string(),
            status: 7,
            stderr: "no display".to_string(),
        }
        .to_string(),
        "clipboard command `xclip` failed with status 7: no display"
    );
    assert_eq!(
        ClipboardError::Io("broken pipe".to_string()).to_string(),
        "clipboard I/O error: broken pipe"
    );
}

#[test]
fn command_backend_reports_success_and_failure() {
    let _guard = acquire_env_lock();
    copy_via_command(
        ClipboardCommand {
            command: "/bin/sh",
            args: &["-c", "cat >/dev/null"],
        },
        "hello",
    )
    .expect("shell sink should succeed");

    let err = copy_via_command(
        ClipboardCommand {
            command: "/bin/sh",
            args: &["-c", "echo nope >&2; exit 7"],
        },
        "hello",
    )
    .expect_err("non-zero clipboard command should fail");

    assert!(matches!(
        err,
        ClipboardError::CommandFailed {
            status: 7,
            ref stderr,
            ..
        } if stderr.contains("nope")
    ));
}

#[test]
fn platform_backends_prefers_wayland_when_present() {
    let _guard = acquire_env_lock();
    let original = std::env::var("WAYLAND_DISPLAY").ok();
    set_env_for_test("WAYLAND_DISPLAY", Some("wayland-0"));
    let backends = platform_backends();
    set_env_for_test("WAYLAND_DISPLAY", original.as_deref());

    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        assert!(!backends.is_empty());
    } else {
        assert_eq!(backends[0].command, "wl-copy");
    }
}

#[test]
fn copy_without_osc52_reports_no_backend_when_path_is_empty() {
    let _guard = acquire_env_lock();
    let key = "PATH";
    let original = std::env::var(key).ok();
    set_path_for_test(Some(""));

    let service = ClipboardService::new().with_osc52(false);
    let result = service.copy_text("hello");

    if let Some(value) = original {
        set_path_for_test(Some(&value));
    } else {
        set_path_for_test(None);
    }

    match result {
        Err(ClipboardError::NoBackendAvailable { attempts }) => {
            assert!(!attempts.is_empty());
        }
        Err(ClipboardError::SpawnFailed { .. }) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn command_backend_reports_spawn_failure_for_missing_binary() {
    let err = copy_via_command(
        ClipboardCommand {
            command: "/definitely/missing/clipboard-bin",
            args: &[],
        },
        "hello",
    )
    .expect_err("missing binary should fail to spawn");
    assert!(matches!(err, ClipboardError::SpawnFailed { .. }));
}

#[test]
fn platform_backends_include_x11_fallbacks_without_wayland() {
    let _guard = acquire_env_lock();
    let original = std::env::var("WAYLAND_DISPLAY").ok();
    set_env_for_test("WAYLAND_DISPLAY", None);
    let backends = platform_backends();
    set_env_for_test("WAYLAND_DISPLAY", original.as_deref());

    if !(cfg!(target_os = "windows") || cfg!(target_os = "macos")) {
        let names = backends
            .iter()
            .map(|backend| backend.command)
            .collect::<Vec<_>>();
        assert!(names.contains(&"xclip"));
        assert!(names.contains(&"xsel"));
    }
}

#[test]
fn command_failure_without_stderr_uses_short_display() {
    let err = ClipboardError::CommandFailed {
        command: "xclip".to_string(),
        status: 9,
        stderr: String::new(),
    };
    assert_eq!(
        err.to_string(),
        "clipboard command `xclip` failed with status 9"
    );
}

#[test]
fn osc52_helpers_respect_env_toggles_and_defaults() {
    let _guard = acquire_env_lock();
    let original_enabled = std::env::var("OSC52").ok();
    let original_max = std::env::var("OSC52_MAX_BYTES").ok();

    set_env_for_test("OSC52", Some("off"));
    assert!(!osc52_enabled());
    set_env_for_test("OSC52", Some("FALSE"));
    assert!(!osc52_enabled());
    set_env_for_test("OSC52", None);
    assert!(osc52_enabled());

    set_env_for_test("OSC52_MAX_BYTES", Some("2048"));
    assert_eq!(osc52_max_bytes(), 2048);
    set_env_for_test("OSC52_MAX_BYTES", Some("0"));
    assert_eq!(osc52_max_bytes(), OSC52_MAX_BYTES_DEFAULT);
    set_env_for_test("OSC52_MAX_BYTES", Some("wat"));
    assert_eq!(osc52_max_bytes(), OSC52_MAX_BYTES_DEFAULT);

    set_env_for_test("OSC52", original_enabled.as_deref());
    set_env_for_test("OSC52_MAX_BYTES", original_max.as_deref());
}

#[test]
fn clipboard_service_builders_toggle_osc52_preference() {
    let default = ClipboardService::new();
    assert!(default.prefer_osc52);

    let disabled = ClipboardService::new().with_osc52(false);
    assert!(!disabled.prefer_osc52);
}

#[test]
fn copy_via_osc52_writer_is_callable_unit() {
    let _guard = acquire_env_lock();
    ClipboardService::new()
        .copy_via_osc52("ping")
        .expect("osc52 writer should succeed on stdout");
}
