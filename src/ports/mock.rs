//! Test and example doubles for the service-layer ports.
//!
//! These helpers exist to make the public port surfaces easy to demonstrate
//! and exercise without bootstrapping the full host stack.

use std::collections::HashMap;

use anyhow::Result;

use crate::core::row::Row;

use super::{LdapDirectory, apply_filter_and_projection};

/// In-memory LDAP test double used by examples, unit tests, and service tests.
///
/// The fixture data intentionally stays small and deterministic so callers can
/// exercise filtering, wildcard lookup, and projection behavior without
/// talking to a real directory. The default fixture currently contains one
/// `oistes` user entry and one `ucore` netgroup entry.
///
/// # Examples
///
/// ```
/// use osp_cli::ports::LdapDirectory;
/// use osp_cli::ports::mock::MockLdapClient;
///
/// let ldap = MockLdapClient::default();
/// let rows = ldap.user("oistes", Some("uid=oistes"), None).unwrap();
///
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows[0].get("uid").and_then(|value| value.as_str()), Some("oistes"));
/// ```
#[derive(Debug, Clone)]
pub struct MockLdapClient {
    users: HashMap<String, Row>,
    netgroups: HashMap<String, Row>,
}

impl Default for MockLdapClient {
    fn default() -> Self {
        let mut users = HashMap::new();
        users.insert(
            "oistes".to_string(),
            crate::row! {
                "uid" => "oistes",
                "cn" => "Øistein Søvik",
                "uidNumber" => "361000",
                "gidNumber" => "346297",
                "homeDirectory" => "/uio/kant/usit-gsd-u1/oistes",
                "loginShell" => "/local/gnu/bin/bash",
                "objectClass" => ["uioMembership", "top", "account", "posixAccount"],
                "eduPersonAffiliation" => ["employee", "member", "staff"],
                "uioAffiliation" => "ANSATT@373034",
                "uioPrimaryAffiliation" => "ANSATT@373034",
                "netgroups" => ["ucore", "usit", "it-uio-azure-users"],
                "filegroups" => ["oistes", "ucore", "usit"]
            },
        );

        let mut netgroups = HashMap::new();
        netgroups.insert(
            "ucore".to_string(),
            crate::row! {
                "cn" => "ucore",
                "description" => "Kjernen av Unix-grupp på USIT",
                "objectClass" => ["top", "nisNetgroup"],
                "members" => [
                    "andreasd",
                    "arildlj",
                    "kjetilk",
                    "oistes",
                    "trondham",
                    "werner"
                ]
            },
        );

        Self { users, netgroups }
    }
}

impl LdapDirectory for MockLdapClient {
    fn user(
        &self,
        uid: &str,
        filter: Option<&str>,
        attributes: Option<&[String]>,
    ) -> Result<Vec<Row>> {
        let raw_rows = if uid.contains('*') {
            self.users
                .iter()
                .filter(|(key, _)| wildcard_match(uid, key))
                .map(|(_, row)| row.clone())
                .collect::<Vec<Row>>()
        } else {
            self.users
                .get(uid)
                .cloned()
                .map(|row| vec![row])
                .unwrap_or_default()
        };

        Ok(apply_filter_and_projection(raw_rows, filter, attributes))
    }

    fn netgroup(
        &self,
        name: &str,
        filter: Option<&str>,
        attributes: Option<&[String]>,
    ) -> Result<Vec<Row>> {
        let raw_rows = if name.contains('*') {
            self.netgroups
                .iter()
                .filter(|(key, _)| wildcard_match(name, key))
                .map(|(_, row)| row.clone())
                .collect::<Vec<Row>>()
        } else {
            self.netgroups
                .get(name)
                .cloned()
                .map(|row| vec![row])
                .unwrap_or_default()
        };

        Ok(apply_filter_and_projection(raw_rows, filter, attributes))
    }
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let escaped = regex::escape(pattern).replace("\\*", ".*");
    match regex::Regex::new(&format!("^{escaped}$")) {
        Ok(re) => re.is_match(value),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::MockLdapClient;
    use crate::ports::LdapDirectory;

    #[test]
    fn user_filter_uid_equals_returns_match() {
        let ldap = MockLdapClient::default();
        let rows = ldap
            .user("oistes", Some("uid=oistes"), None)
            .expect("query should succeed");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn wildcard_queries_match_users_and_netgroups() {
        let ldap = MockLdapClient::default();

        let users = ldap.user("oi*", None, None).expect("query should succeed");
        assert_eq!(users.len(), 1);
        assert_eq!(
            users[0].get("uid").and_then(|value| value.as_str()),
            Some("oistes")
        );

        let netgroups = ldap
            .netgroup("u*", None, Some(&["cn".to_string()]))
            .expect("query should succeed");
        assert_eq!(netgroups.len(), 1);
        assert_eq!(
            netgroups[0].get("cn").and_then(|value| value.as_str()),
            Some("ucore")
        );
        assert_eq!(netgroups[0].len(), 1);
    }

    #[test]
    fn missing_entries_return_empty_results() {
        let ldap = MockLdapClient::default();

        let users = ldap
            .user("does-not-exist", Some("uid=does-not-exist"), None)
            .expect("query should succeed");
        assert!(users.is_empty());

        let netgroups = ldap
            .netgroup("nope*", None, None)
            .expect("query should succeed");
        assert!(netgroups.is_empty());
    }

    #[test]
    fn exact_netgroup_queries_return_single_match() {
        let ldap = MockLdapClient::default();

        let netgroups = ldap
            .netgroup("ucore", None, Some(&["cn".to_string()]))
            .expect("query should succeed");

        assert_eq!(netgroups.len(), 1);
        assert_eq!(
            netgroups[0].get("cn").and_then(|value| value.as_str()),
            Some("ucore")
        );
    }
}
