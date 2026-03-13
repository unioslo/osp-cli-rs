#![allow(missing_docs)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn runtime_source_avoids_panic_helpers_unit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_runtime_rust_files(&root.join("src"), &mut files);
    files.sort();

    let mut violations = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        violations.extend(find_forbidden_uses(&file, &source));
    }

    assert!(
        violations.is_empty(),
        "runtime source contains panic helpers:\n{}",
        violations.join("\n")
    );
}

fn collect_runtime_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(root)
        .unwrap_or_else(|err| panic!("failed to read directory {}: {err}", root.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                root.display()
            )
        });
        let path = entry.path();
        if path.is_dir() {
            if path.file_name() == Some(OsStr::new("tests")) {
                continue;
            }
            collect_runtime_rust_files(&path, out);
            continue;
        }
        if path.extension() != Some(OsStr::new("rs")) {
            continue;
        }
        if path.file_name() == Some(OsStr::new("tests.rs")) {
            continue;
        }
        out.push(path);
    }
}

fn find_forbidden_uses(path: &Path, source: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let mut pending_cfg_test = false;
    let mut skipped_depth = 0usize;

    for (line_number, raw_line) in source.lines().enumerate() {
        let line_number = line_number + 1;
        let trimmed = raw_line.trim_start();

        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }

        let line = strip_strings_and_line_comments(raw_line);
        let trimmed = line.trim_start();

        if skipped_depth > 0 {
            skipped_depth = advance_depth(skipped_depth, &line);
            continue;
        }

        if pending_cfg_test {
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }
            let opens = line.chars().filter(|&ch| ch == '{').count();
            let closes = line.chars().filter(|&ch| ch == '}').count();
            if opens > 0 {
                skipped_depth = opens.saturating_sub(closes);
            }
            pending_cfg_test = false;
            continue;
        }

        if trimmed.starts_with("//") {
            continue;
        }

        if contains_forbidden_helper(&line) {
            violations.push(format!(
                "{}:{line_number}: {}",
                path.display(),
                raw_line.trim()
            ));
        }
    }

    violations
}

fn strip_strings_and_line_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }

        if ch == '/' && matches!(chars.peek(), Some('/')) {
            break;
        }

        out.push(ch);
    }

    out
}

fn advance_depth(depth: usize, line: &str) -> usize {
    let opens = line.chars().filter(|&ch| ch == '{').count();
    let closes = line.chars().filter(|&ch| ch == '}').count();
    depth.saturating_add(opens).saturating_sub(closes)
}

fn contains_forbidden_helper(line: &str) -> bool {
    line.contains(".expect(")
        || line.contains(".expect_err(")
        || line.contains(".unwrap(")
        || line.contains(".unwrap_err(")
        || line.contains("panic!")
        || line.contains("unreachable!")
}
