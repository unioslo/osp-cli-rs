use anyhow::{Result, anyhow};
use crate::osp_config::RuntimeConfig;
use crate::osp_core::output_model::OutputResult;
use crate::osp_core::row::Row;
use crate::osp_dsl::{apply_pipeline, parse_pipeline};
use crate::osp_ports::{LdapDirectory, parse_attributes};

pub struct ServiceContext<L: LdapDirectory> {
    pub user: Option<String>,
    pub ldap: L,
    pub config: RuntimeConfig,
}

impl<L: LdapDirectory> ServiceContext<L> {
    pub fn new(user: Option<String>, ldap: L, config: RuntimeConfig) -> Self {
        Self { user, ldap, config }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    LdapUser {
        uid: Option<String>,
        filter: Option<String>,
        attributes: Option<String>,
    },
    LdapNetgroup {
        name: Option<String>,
        filter: Option<String>,
        attributes: Option<String>,
    },
}

pub fn execute_line<L: LdapDirectory>(ctx: &ServiceContext<L>, line: &str) -> Result<OutputResult> {
    let parsed_pipeline = parse_pipeline(line)?;
    if parsed_pipeline.command.is_empty() {
        return Ok(OutputResult::from_rows(Vec::new()));
    }

    let tokens = shell_words::split(&parsed_pipeline.command)
        .map_err(|err| anyhow!("failed to parse command: {err}"))?;
    let command = parse_repl_command(&tokens)?;
    apply_pipeline(execute_command(ctx, &command)?, &parsed_pipeline.stages)
}

pub fn parse_repl_command(tokens: &[String]) -> Result<ParsedCommand> {
    if tokens.is_empty() {
        return Err(anyhow!("empty command"));
    }
    if tokens[0] != "ldap" {
        return Err(anyhow!("unsupported command: {}", tokens[0]));
    }
    if tokens.len() < 2 {
        return Err(anyhow!("missing ldap subcommand"));
    }

    match tokens[1].as_str() {
        "user" => parse_ldap_user_tokens(tokens),
        "netgroup" => parse_ldap_netgroup_tokens(tokens),
        other => Err(anyhow!("unsupported ldap subcommand: {other}")),
    }
}

fn parse_ldap_user_tokens(tokens: &[String]) -> Result<ParsedCommand> {
    let mut uid: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut attributes: Option<String> = None;

    let mut i = 2usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--filter" => {
                i += 1;
                let value = tokens
                    .get(i)
                    .ok_or_else(|| anyhow!("--filter requires a value"))?;
                filter = Some(value.clone());
            }
            "--attributes" | "-a" => {
                i += 1;
                let value = tokens
                    .get(i)
                    .ok_or_else(|| anyhow!("--attributes requires a value"))?;
                attributes = Some(value.clone());
            }
            token if token.starts_with('-') => return Err(anyhow!("unknown option: {token}")),
            value => {
                if uid.is_some() {
                    return Err(anyhow!("ldap user accepts one uid positional argument"));
                }
                uid = Some(value.to_string());
            }
        }
        i += 1;
    }

    Ok(ParsedCommand::LdapUser {
        uid,
        filter,
        attributes,
    })
}

fn parse_ldap_netgroup_tokens(tokens: &[String]) -> Result<ParsedCommand> {
    let mut name: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut attributes: Option<String> = None;

    let mut i = 2usize;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--filter" => {
                i += 1;
                let value = tokens
                    .get(i)
                    .ok_or_else(|| anyhow!("--filter requires a value"))?;
                filter = Some(value.clone());
            }
            "--attributes" | "-a" => {
                i += 1;
                let value = tokens
                    .get(i)
                    .ok_or_else(|| anyhow!("--attributes requires a value"))?;
                attributes = Some(value.clone());
            }
            token if token.starts_with('-') => return Err(anyhow!("unknown option: {token}")),
            value => {
                if name.is_some() {
                    return Err(anyhow!(
                        "ldap netgroup accepts one name positional argument"
                    ));
                }
                name = Some(value.to_string());
            }
        }
        i += 1;
    }

    Ok(ParsedCommand::LdapNetgroup {
        name,
        filter,
        attributes,
    })
}

pub fn execute_command<L: LdapDirectory>(
    ctx: &ServiceContext<L>,
    command: &ParsedCommand,
) -> Result<Vec<Row>> {
    match command {
        ParsedCommand::LdapUser {
            uid,
            filter,
            attributes,
        } => {
            let effective_uid = uid
                .clone()
                .or_else(|| ctx.user.clone())
                .ok_or_else(|| anyhow!("ldap user requires <uid> or -u/--user"))?;
            let attrs = parse_attributes(attributes.as_deref())?;
            ctx.ldap
                .user(&effective_uid, filter.as_deref(), attrs.as_deref())
        }
        ParsedCommand::LdapNetgroup {
            name,
            filter,
            attributes,
        } => {
            let effective_name = name
                .clone()
                .ok_or_else(|| anyhow!("ldap netgroup requires <name>"))?;
            let attrs = parse_attributes(attributes.as_deref())?;
            ctx.ldap
                .netgroup(&effective_name, filter.as_deref(), attrs.as_deref())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::osp_api::MockLdapClient;
    use crate::osp_core::output_model::OutputResult;

    use super::{ParsedCommand, ServiceContext, execute_command, execute_line, parse_repl_command};

    fn output_rows(output: &OutputResult) -> &[crate::osp_core::row::Row] {
        output.as_rows().expect("expected row output")
    }

    fn test_ctx() -> ServiceContext<MockLdapClient> {
        ServiceContext::new(
            Some("oistes".to_string()),
            MockLdapClient::default(),
            crate::osp_config::RuntimeConfig::default(),
        )
    }

    #[test]
    fn parses_repl_user_command_with_options() {
        let cmd = parse_repl_command(&[
            "ldap".to_string(),
            "user".to_string(),
            "oistes".to_string(),
            "--filter".to_string(),
            "uid=oistes".to_string(),
            "--attributes".to_string(),
            "uid,cn".to_string(),
        ])
        .expect("command should parse");

        assert_eq!(
            cmd,
            ParsedCommand::LdapUser {
                uid: Some("oistes".to_string()),
                filter: Some("uid=oistes".to_string()),
                attributes: Some("uid,cn".to_string())
            }
        );
    }

    #[test]
    fn ldap_user_defaults_to_global_user() {
        let ctx = test_ctx();
        let rows = execute_command(
            &ctx,
            &ParsedCommand::LdapUser {
                uid: None,
                filter: None,
                attributes: None,
            },
        )
        .expect("ldap user should default to global user");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("uid").and_then(|v| v.as_str()), Some("oistes"));
    }

    #[test]
    fn execute_line_supports_pipeline() {
        let ctx = test_ctx();
        let rows = execute_line(&ctx, "ldap user oistes | P uid,cn")
            .expect("pipeline command should execute");
        assert_eq!(output_rows(&rows).len(), 1);
        assert!(output_rows(&rows)[0].contains_key("uid"));
        assert!(output_rows(&rows)[0].contains_key("cn"));
    }

    #[test]
    fn parse_repl_command_rejects_empty_and_unknown_commands() {
        let empty = parse_repl_command(&[]).expect_err("empty command should fail");
        assert!(empty.to_string().contains("empty command"));

        let unsupported = parse_repl_command(&["mreg".to_string()])
            .expect_err("unsupported root command should fail");
        assert!(unsupported.to_string().contains("unsupported command"));

        let missing_subcommand = parse_repl_command(&["ldap".to_string()])
            .expect_err("missing ldap subcommand should fail");
        assert!(
            missing_subcommand
                .to_string()
                .contains("missing ldap subcommand")
        );
    }

    #[test]
    fn parse_repl_command_supports_netgroup_and_short_attribute_flag() {
        let cmd = parse_repl_command(&[
            "ldap".to_string(),
            "netgroup".to_string(),
            "ops".to_string(),
            "-a".to_string(),
            "cn,description".to_string(),
            "--filter".to_string(),
            "ops".to_string(),
        ])
        .expect("netgroup command should parse");

        assert_eq!(
            cmd,
            ParsedCommand::LdapNetgroup {
                name: Some("ops".to_string()),
                filter: Some("ops".to_string()),
                attributes: Some("cn,description".to_string()),
            }
        );
    }

    #[test]
    fn parse_repl_command_rejects_unknown_options_and_extra_positionals() {
        let unknown =
            parse_repl_command(&["ldap".to_string(), "user".to_string(), "--wat".to_string()])
                .expect_err("unknown flag should fail");
        assert!(unknown.to_string().contains("unknown option"));

        let extra = parse_repl_command(&[
            "ldap".to_string(),
            "netgroup".to_string(),
            "ops".to_string(),
            "extra".to_string(),
        ])
        .expect_err("extra positional should fail");
        assert!(
            extra
                .to_string()
                .contains("ldap netgroup accepts one name positional argument")
        );
    }

    #[test]
    fn execute_command_requires_explicit_subject_when_defaults_are_missing() {
        let ctx = ServiceContext::new(
            None,
            MockLdapClient::default(),
            crate::osp_config::RuntimeConfig::default(),
        );
        let err = execute_command(
            &ctx,
            &ParsedCommand::LdapUser {
                uid: None,
                filter: None,
                attributes: None,
            },
        )
        .expect_err("ldap user should require uid when global user is missing");
        assert!(
            err.to_string()
                .contains("ldap user requires <uid> or -u/--user")
        );

        let err = execute_command(
            &ctx,
            &ParsedCommand::LdapNetgroup {
                name: None,
                filter: None,
                attributes: None,
            },
        )
        .expect_err("ldap netgroup should require a name");
        assert!(err.to_string().contains("ldap netgroup requires <name>"));
    }

    #[test]
    fn execute_line_handles_blank_and_shell_parse_errors() {
        let ctx = test_ctx();

        let blank = execute_line(&ctx, "   ").expect("blank line should be a no-op");
        assert!(output_rows(&blank).is_empty());

        let err = execute_line(&ctx, "ldap user \"unterminated")
            .expect_err("invalid shell quoting should fail");
        assert!(err.to_string().contains("unterminated"));
    }
}
