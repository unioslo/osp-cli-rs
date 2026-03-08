use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
#[test]
fn ldap_user_json_contract() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["--json", "ldap", "user", "oistes"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"uid\": \"oistes\""))
        .stdout(predicate::str::contains("\"netgroups\""));
}

#[cfg(unix)]
#[test]
fn ldap_user_defaults_to_global_user() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["-u", "oistes", "--json", "ldap", "user"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"uid\": \"oistes\""));
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
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"uid\": \"oistes\""))
        .stdout(predicate::str::contains("\"cn\""))
        .stdout(predicate::str::contains("\"homeDirectory\"").not());
}

#[cfg(unix)]
#[test]
fn ldap_netgroup_json_contract() {
    let fixture = LdapPluginFixture::new();

    let mut cmd = fixture.osp();
    cmd.args(["--json", "ldap", "netgroup", "ucore"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"cn\": \"ucore\""))
        .stdout(predicate::str::contains("\"members\""));
}

#[cfg(unix)]
#[test]
fn ldap_plugin_completes_subcommands_and_flags_contract() {
    let fixture = LdapPluginFixture::new();

    let mut subcommands = fixture.osp();
    subcommands.args(["repl", "debug-complete", "--line", "ldap "]);
    subcommands
        .assert()
        .success()
        .stdout(predicate::str::contains("\"label\": \"user\""))
        .stdout(predicate::str::contains("\"label\": \"netgroup\""));

    let mut long_flags = fixture.osp();
    long_flags.args(["repl", "debug-complete", "--line", "ldap user --a"]);
    long_flags
        .assert()
        .success()
        .stdout(predicate::str::contains("\"label\": \"--attributes\""));

    let mut short_flags = fixture.osp();
    short_flags.args(["repl", "debug-complete", "--line", "ldap user -"]);
    short_flags
        .assert()
        .success()
        .stdout(predicate::str::contains("\"label\": \"-a\""));
}

#[cfg(unix)]
struct LdapPluginFixture {
    plugin_dir: std::path::PathBuf,
    home_dir: std::path::PathBuf,
}

#[cfg(unix)]
impl LdapPluginFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();

        let mut plugin_dir = std::env::temp_dir();
        plugin_dir.push(format!("osp-cli-ldap-plugin-{nonce}"));
        std::fs::create_dir_all(&plugin_dir).expect("plugin dir should be created");

        let mut home_dir = std::env::temp_dir();
        home_dir.push(format!("osp-cli-ldap-home-{nonce}"));
        std::fs::create_dir_all(&home_dir).expect("home dir should be created");

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
impl Drop for LdapPluginFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.plugin_dir);
        let _ = std::fs::remove_dir_all(&self.home_dir);
    }
}

#[cfg(unix)]
fn ldap_plugin_script() -> &'static str {
    r#"#!/bin/sh
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
