#[cfg(unix)]
use crate::output_support::{first_json_row, parse_json_stdout};
#[cfg(unix)]
use crate::temp_support::TestTempDir;
use assert_cmd::Command;

#[cfg(unix)]
#[test]
fn ldap_user_json_contract() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["--json", "ldap", "user", "oistes"]);
    let output = cmd.assert().success().get_output().clone();
    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "ldap user");
    assert_eq!(row["uid"], "oistes");
    assert_eq!(row["cn"], "Mock LDAP User");
    assert_eq!(row["homeDirectory"], "/mock/home/oistes");
    assert_eq!(row["netgroups"], serde_json::json!(["ucore", "usit"]));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn ldap_netgroup_json_contract() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["--json", "ldap", "netgroup", "ucore"]);
    let output = cmd.assert().success().get_output().clone();
    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "ldap netgroup");
    assert_eq!(row["cn"], "ucore");
    assert_eq!(row["members"], serde_json::json!(["oistes", "trondham"]));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn ldap_plugin_completes_subcommands_and_flags_contract() {
    let fixture = LdapPluginFixture::new();

    let mut subcommands = fixture.osp();
    subcommands.args(["repl", "debug-complete", "--line", "ldap "]);
    let subcommands_output = subcommands.assert().success().get_output().clone();
    let subcommands_payload = parse_json_stdout(&subcommands_output.stdout);
    let subcommand_matches = subcommands_payload["matches"]
        .as_array()
        .expect("subcommand matches should render as an array");
    assert!(
        subcommand_matches
            .iter()
            .any(|item| item["label"] == "user")
    );
    assert!(
        subcommand_matches
            .iter()
            .any(|item| item["label"] == "netgroup")
    );

    let mut long_flags = fixture.osp();
    long_flags.args(["repl", "debug-complete", "--line", "ldap user --a"]);
    let long_flags_output = long_flags.assert().success().get_output().clone();
    let long_flags_payload = parse_json_stdout(&long_flags_output.stdout);
    let long_flag_matches = long_flags_payload["matches"]
        .as_array()
        .expect("long-flag matches should render as an array");
    assert!(
        long_flag_matches
            .iter()
            .any(|item| item["id"] == "--attributes")
    );

    for stub in ["-", "--"] {
        let mut flags = fixture.osp();
        flags.args([
            "repl",
            "debug-complete",
            "--line",
            &format!("ldap user {stub}"),
        ]);
        let output = flags.assert().success().get_output().clone();
        let payload = parse_json_stdout(&output.stdout);
        let labels = payload["matches"]
            .as_array()
            .expect("flag matches should render as an array")
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["--attributes, -a", "--filter"]);
    }
}

#[cfg(unix)]
struct LdapPluginFixture {
    plugin_dir: TestTempDir,
    home_dir: TestTempDir,
}

#[cfg(unix)]
impl LdapPluginFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let plugin_dir = crate::temp_support::make_temp_dir("osp-cli-ldap-plugin");
        let home_dir = crate::temp_support::make_temp_dir("osp-cli-ldap-home");

        let plugin_path = plugin_dir.join("osp-ldap");
        std::fs::write(&plugin_path, ldap_plugin_script())
            .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("plugin metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");

        Self {
            plugin_dir,
            home_dir,
        }
    }

    fn osp(&self) -> Command {
        let mut cmd = crate::test_env::command(assert_cmd::cargo::cargo_bin!("osp"));
        cmd.envs(crate::test_env::isolated_env(&self.home_dir))
            .env("OSP_PLUGIN_PATH", &self.plugin_dir)
            .env("PATH", "/usr/bin:/bin");
        cmd
    }
}

#[cfg(unix)]
fn ldap_plugin_script() -> &'static str {
    r#"#!/usr/bin/env bash
PATH=/usr/bin:/bin:$PATH
set -euo pipefail

if [ "${1:-}" = "--osp-describe" ]; then
  cat <<'JSON'
{"about":"LDAP plugin","args":[],"flags":{},"subcommands":[{"name":"user","about":"Lookup LDAP users","args":[{"name":"uid","about":"User id","multi":false,"value_type":null,"suggestions":[]}],"flags":{"--filter":{"about":"LDAP filter","flag_only":false,"multi":false,"value_type":null,"suggestions":[]},"--attributes":{"about":"Comma-separated attributes","flag_only":false,"multi":false,"value_type":null,"suggestions":[{"value":"uid","meta":"User id","display":null,"sort":null},{"value":"cn","meta":"Common name","display":null,"sort":null}]},"-a":{"alias_of":"--attributes","about":"Comma-separated attributes","flag_only":false,"multi":false,"value_type":null,"suggestions":[{"value":"uid","meta":"User id","display":null,"sort":null},{"value":"cn","meta":"Common name","display":null,"sort":null}]}},"subcommands":[]},{"name":"netgroup","about":"Lookup LDAP netgroups","args":[{"name":"name","about":"Netgroup name","multi":false,"value_type":null,"suggestions":[]}],"flags":{},"subcommands":[]}],"output":"json"}
JSON
  exit 0
fi

selected_command="${OSP_COMMAND:-${1:-}}"
if [ "${1:-}" = "$selected_command" ]; then
  shift || true
fi
cmd="${1:-}"
shift || true

case "$cmd" in
  user)
    uid="${1:?user id required}"
    cat <<JSON
{"protocol_version":1,"ok":true,"data":[{"uid":"$uid","cn":"Mock LDAP User","homeDirectory":"/mock/home/$uid","netgroups":["ucore","usit"]}],"error":null,"meta":{"format_hint":"table","columns":["uid","cn"]}}
JSON
    ;;

  netgroup)
    name="${1:-ucore}"
    cat <<JSON
{"protocol_version":1,"ok":true,"data":[{"cn":"$name","members":["oistes","trondham"]}],"error":null,"meta":{"format_hint":"table","columns":["cn","members"]}}
JSON
    ;;

  *)
    cat <<JSON
{"protocol_version":1,"ok":false,"data":{},"error":{"code":"UNKNOWN_COMMAND","message":"unknown command: $cmd","details":{}},"meta":{}}
JSON
    ;;
esac
"#
}
