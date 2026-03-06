use anyhow::{Result, anyhow};
use osp_config::RuntimeConfig;
use osp_core::output_model::OutputResult;
use osp_core::row::Row;
use osp_dsl::{apply_pipeline, parse_pipeline};
use osp_ports::{LdapDirectory, parse_attributes};

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
    use osp_api::MockLdapClient;
    use osp_core::output_model::OutputResult;

    use super::{ParsedCommand, ServiceContext, execute_command, execute_line, parse_repl_command};

    fn output_rows(output: &OutputResult) -> &[osp_core::row::Row] {
        output.as_rows().expect("expected row output")
    }

    fn test_ctx() -> ServiceContext<MockLdapClient> {
        ServiceContext::new(
            Some("oistes".to_string()),
            MockLdapClient::default(),
            osp_config::RuntimeConfig::default(),
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
}
