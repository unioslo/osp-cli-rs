use super::{
    BootstrapPhase, BootstrapScopeRule, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue,
    ResolvedConfig, ResolvedValue, SchemaEntry, SchemaValueType, Scope, SecretValue,
    adapt_value_for_schema, bootstrap_key_spec, is_alias_key, is_bootstrap_only_key, parse_env_key,
    parse_string_list, remaining_parts_are_bootstrap_profile_default, validate_bootstrap_value,
    validate_key_scope, value_type_name,
};
use std::collections::{BTreeMap, BTreeSet};

include!("tests/bootstrap_schema.rs");
include!("tests/layer_parsing.rs");
include!("tests/resolved_helpers.rs");
include!("tests/adaptation_env.rs");
include!("tests/resolve_options.rs");
