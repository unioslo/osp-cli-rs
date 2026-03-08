use std::collections::HashMap;

use anyhow::Result;
use crate::osp_core::row::Row;
use crate::osp_ports::{LdapDirectory, apply_filter_and_projection};
use serde_json::json;

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
            json!({
                "uid": "oistes",
                "cn": "Øistein Søvik",
                "uidNumber": "361000",
                "gidNumber": "346297",
                "homeDirectory": "/uio/kant/usit-gsd-u1/oistes",
                "loginShell": "/local/gnu/bin/bash",
                "objectClass": ["uioMembership", "top", "account", "posixAccount"],
                "eduPersonAffiliation": ["employee", "member", "staff"],
                "uioAffiliation": "ANSATT@373034",
                "uioPrimaryAffiliation": "ANSATT@373034",
                "netgroups": ["ucore", "usit", "it-uio-azure-users"],
                "filegroups": ["oistes", "ucore", "usit"]
            })
            .as_object()
            .cloned()
            .expect("static user fixture must be object"),
        );

        let mut netgroups = HashMap::new();
        netgroups.insert(
            "ucore".to_string(),
            json!({
                "cn": "ucore",
                "description": "Kjernen av Unix-grupp på USIT",
                "objectClass": ["top", "nisNetgroup"],
                "members": [
                    "andreasd",
                    "arildlj",
                    "kjetilk",
                    "oistes",
                    "trondham",
                    "werner"
                ]
            })
            .as_object()
            .cloned()
            .expect("static netgroup fixture must be object"),
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
    let re = regex::Regex::new(&format!("^{escaped}$"))
        .expect("escaped wildcard patterns must compile as regexes");
    re.is_match(value)
}

#[cfg(test)]
mod tests {
    use crate::osp_ports::LdapDirectory;

    use super::MockLdapClient;

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
