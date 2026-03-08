use std::{
    path::{Path, PathBuf},
    process::Command,
};

use osp_dsl::parse::{
    lexer::{Span, StageSegment, tokenize_stage},
    pipeline::parse_pipeline,
};
use serde_json::Value;

#[test]
fn parser_stage_split_parity_with_python_reference() {
    let Some(env) = PythonReferenceEnv::discover() else {
        eprintln!("skipping python parity test: osprov-cli python environment not found");
        return;
    };

    let cases = [
        "ldap user oistes | P uid,cn | F uid=oistes",
        "ldap user 'foo|bar' | P uid",
        "ldap user \"foo|bar\" | P uid",
        "ldap user oistes || P uid",
        "ldap user oistes |  | P uid",
    ];

    for case in cases {
        let python_segments = env.python_parse_pipeline_segments(case);
        let parsed = parse_pipeline(case).expect("valid parser parity case should parse");
        let rust_segments = {
            let mut segments = Vec::with_capacity(parsed.stages.len() + 1);
            if !parsed.command.is_empty() {
                segments.push(parsed.command);
            }
            segments.extend(parsed.stages);
            segments
        };

        assert_eq!(
            rust_segments, python_segments,
            "pipeline segment mismatch for case: {case}"
        );
    }
}

#[test]
fn parser_tokenizer_operator_parity_with_python_reference() {
    let Some(env) = PythonReferenceEnv::discover() else {
        eprintln!("skipping python parity test: osprov-cli python environment not found");
        return;
    };

    let cases = [
        "uid=oistes",
        "vlan>=75",
        "status != active",
        "status == ==online",
        "==online",
        "!?interfaces",
        "name \"foo bar\"",
    ];

    for case in cases {
        let python_tokens = env.python_preprocess_filter_tokens(case);
        let stage = StageSegment {
            raw: case.to_string(),
            span: Span {
                start: 0,
                end: case.len(),
            },
        };
        let rust_tokens = tokenize_stage(&stage)
            .expect("rust tokenization should work")
            .into_iter()
            .map(|token| token.text)
            .collect::<Vec<_>>();

        assert_eq!(
            rust_tokens, python_tokens,
            "token mismatch for filter spec: {case}"
        );
    }
}

struct PythonReferenceEnv {
    python_bin: PathBuf,
    pythonpath: PathBuf,
}

impl PythonReferenceEnv {
    fn discover() -> Option<Self> {
        if let (Some(python_bin), Some(pythonpath)) = (
            std::env::var_os("OSPROV_PYTHON_BIN").map(PathBuf::from),
            std::env::var_os("OSPROV_PYTHONPATH").map(PathBuf::from),
        ) && python_bin.exists()
            && pythonpath.exists()
        {
            return Some(Self {
                python_bin,
                pythonpath,
            });
        }

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest.parent()?.parent()?;
        let default_python = repo_root.join("osprov-cli/.venv/bin/python");
        let default_pythonpath = repo_root.join("osprov-cli/src");

        if default_python.exists() && default_pythonpath.exists() {
            return Some(Self {
                python_bin: default_python,
                pythonpath: default_pythonpath,
            });
        }

        None
    }

    fn python_parse_pipeline_segments(&self, input: &str) -> Vec<String> {
        let script = r#"
import json
from osprov_cli.dsl.engine import parse_pipeline
import sys
pipeline = parse_pipeline(sys.argv[1])
print(json.dumps([stage.raw for stage in pipeline.stages]))
"#;

        let raw = self.run_python(script, &[input]);
        parse_json_array_of_strings(&raw)
    }

    fn python_preprocess_filter_tokens(&self, spec: &str) -> Vec<String> {
        let script = r#"
import json
import shlex
import sys
from osprov_cli.dsl.stages.filter import _preprocess_tokens
print(json.dumps(_preprocess_tokens(shlex.split(sys.argv[1]))))
"#;

        let raw = self.run_python(script, &[spec]);
        parse_json_array_of_strings(&raw)
    }

    fn run_python(&self, script: &str, args: &[&str]) -> String {
        let mut command = Command::new(&self.python_bin);
        command.arg("-c").arg(script);
        command.env("PYTHONPATH", &self.pythonpath);
        for arg in args {
            command.arg(arg);
        }
        let output = command.output().expect("python command should execute");

        assert!(
            output.status.success(),
            "python command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("stdout should be utf8")
    }
}

fn parse_json_array_of_strings(raw: &str) -> Vec<String> {
    let parsed: Value =
        serde_json::from_str(raw.trim()).expect("python output should be valid JSON");
    parsed
        .as_array()
        .expect("python output should be array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("array entry should be string")
                .to_string()
        })
        .collect()
}
