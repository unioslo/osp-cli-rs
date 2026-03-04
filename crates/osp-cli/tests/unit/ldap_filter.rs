use osp_api::MockLdapClient;
use osp_ports::LdapDirectory;

#[test]
fn ldap_filter_key_value_matches() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", Some("uid=oistes"), None)
        .expect("query should succeed");
    assert_eq!(rows.len(), 1);
}
