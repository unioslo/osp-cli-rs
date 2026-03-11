use osp_cli::ports::LdapDirectory;
use osp_cli::ports::mock::MockLdapClient;

#[test]
fn ldap_filter_key_value_matches() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", Some("uid=oistes"), None)
        .expect("query should succeed");
    assert_eq!(rows.len(), 1);
}
