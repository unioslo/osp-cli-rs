#[cfg(unix)]
use crate::temp_support::TestTempDir;
use assert_cmd::Command;
use serde_json::Value;

fn parse_json_stdout(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}

fn first_json_row<'a>(payload: &'a Value, context: &str) -> &'a Value {
    payload
        .as_array()
        .unwrap_or_else(|| panic!("{context} should render a JSON array"))
        .first()
        .unwrap_or_else(|| panic!("{context} should render at least one row"))
}

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
fn ldap_user_defaults_to_global_user() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["-u", "oistes", "--json", "ldap", "user"]);
    let output = cmd.assert().success().get_output().clone();
    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "ldap user default subject");
    assert_eq!(row["uid"], "oistes");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn ldap_user_supports_attributes_and_filter() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args([
        "-u",
        "oistes",
        "--json",
        "ldap",
        "user",
        "oistes",
        "--filter",
        "uid=oistes",
        "--attributes",
        "uid,cn",
    ]);
    let output = cmd.assert().success().get_output().clone();
    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "ldap user attribute projection");
    assert_eq!(row["uid"], "oistes");
    assert_eq!(row["cn"], "Mock LDAP User");
    let object = row
        .as_object()
        .expect("ldap projected row should render as an object");
    assert!(!object.contains_key("homeDirectory"));
    assert!(!object.contains_key("netgroups"));
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
            .any(|item| item["label"] == "--attributes")
    );

    let mut short_flags = fixture.osp();
    short_flags.args(["repl", "debug-complete", "--line", "ldap user -"]);
    let short_flags_output = short_flags.assert().success().get_output().clone();
    let short_flags_payload = parse_json_stdout(&short_flags_output.stdout);
    let short_flag_matches = short_flags_payload["matches"]
        .as_array()
        .expect("short-flag matches should render as an array");
    assert!(short_flag_matches.iter().any(|item| item["label"] == "-a"));
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
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
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

if [ "${1:-}" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"ldap","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"ldap","about":"LDAP plugin","args":[],"flags":{},"subcommands":[{"name":"user","about":"Lookup LDAP users","args":[{"name":"uid","about":"User id","multi":false,"value_type":null,"suggestions":[]}],"flags":{"--filter":{"about":"LDAP filter","flag_only":false,"multi":false,"value_type":null,"suggestions":[]},"--attributes":{"about":"Comma-separated attributes","flag_only":false,"multi":false,"value_type":null,"suggestions":[{"value":"uid","meta":"User id","display":null,"sort":null},{"value":"cn","meta":"Common name","display":null,"sort":null}]},"-a":{"about":"Comma-separated attributes","flag_only":false,"multi":false,"value_type":null,"suggestions":[{"value":"uid","meta":"User id","display":null,"sort":null},{"value":"cn","meta":"Common name","display":null,"sort":null}]}},"subcommands":[]},{"name":"netgroup","about":"Lookup LDAP netgroups","args":[{"name":"name","about":"Netgroup name","multi":false,"value_type":null,"suggestions":[]}],"flags":{},"subcommands":[]}]}]}
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
    uid=""
    filter=""
    attrs=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --filter)
          filter="${2:-}"
          shift 2
          ;;
        --attributes|-a)
          attrs="${2:-}"
          shift 2
          ;;
        *)
          if [ -z "$uid" ]; then
            uid="$1"
          fi
          shift
          ;;
      esac
    done

    if [ -z "$uid" ]; then
      uid="oistes"
    fi

    if [[ "$filter" == uid=* ]]; then
      wanted="${filter#uid=}"
      if [ "$wanted" != "$uid" ]; then
        cat <<JSON
{"protocol_version":1,"ok":true,"data":[],"error":null,"meta":{"format_hint":"table","columns":["uid","cn"]}}
JSON
        exit 0
      fi
    fi

    if [ "$attrs" = "uid,cn" ] || [ "$attrs" = "cn,uid" ]; then
      cat <<JSON
{"protocol_version":1,"ok":true,"data":[{"uid":"$uid","cn":"Mock LDAP User"}],"error":null,"meta":{"format_hint":"table","columns":["uid","cn"]}}
JSON
    else
      cat <<JSON
{"protocol_version":1,"ok":true,"data":[{"uid":"$uid","cn":"Mock LDAP User","homeDirectory":"/mock/home/$uid","netgroups":["ucore","usit"]}],"error":null,"meta":{"format_hint":"table","columns":["uid","cn"]}}
JSON
    fi
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
