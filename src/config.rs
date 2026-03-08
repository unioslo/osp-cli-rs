//! Configuration loading, resolution, schema, and persistence.

pub mod schema {
    pub use crate::osp_config::{
        ActiveProfileSource, BootstrapConfigExplain, BootstrapKeySpec, BootstrapPhase,
        BootstrapScopeRule, BootstrapValueRule, ConfigLayer, ConfigSchema, ExplainInterpolation,
        ExplainInterpolationStep, LayerEntry, ResolveOptions, ResolvedValue, SchemaEntry,
        SchemaValueType,
    };
}

pub mod load {
    pub use crate::osp_config::{
        ChainedLoader, ConfigLoader, EnvSecretsLoader, EnvVarLoader, LoadedLayers, LoaderPipeline,
        SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
    };
}

pub mod resolve {
    pub use crate::osp_config::{
        ConfigExplain, ConfigResolver, ExplainCandidate, ExplainLayer, ResolvedConfig,
    };
}

pub mod runtime {
    pub use crate::osp_config::{
        DEFAULT_DEBUG_LEVEL, DEFAULT_LOG_FILE_ENABLED, DEFAULT_LOG_FILE_LEVEL,
        DEFAULT_PROFILE_NAME, DEFAULT_REPL_HISTORY_DEDUPE, DEFAULT_REPL_HISTORY_ENABLED,
        DEFAULT_REPL_HISTORY_MAX_ENTRIES, DEFAULT_REPL_HISTORY_PROFILE_SCOPED, DEFAULT_REPL_INTRO,
        DEFAULT_SESSION_CACHE_MAX_RESULTS, DEFAULT_UI_CHROME_FRAME, DEFAULT_UI_COLUMN_WEIGHT,
        DEFAULT_UI_GRID_PADDING, DEFAULT_UI_HELP_LAYOUT, DEFAULT_UI_INDENT, DEFAULT_UI_MARGIN,
        DEFAULT_UI_MEDIUM_LIST_MAX, DEFAULT_UI_MESSAGES_LAYOUT,
        DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH, DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO,
        DEFAULT_UI_PRESENTATION, DEFAULT_UI_SHORT_LIST_MAX, DEFAULT_UI_TABLE_BORDER,
        DEFAULT_UI_TABLE_OVERFLOW, DEFAULT_UI_WIDTH, RuntimeConfig, RuntimeConfigPaths,
        RuntimeDefaults, RuntimeLoadOptions,
    };
}

pub mod store {
    pub use crate::osp_config::{
        TomlEditResult, secret_file_mode, set_scoped_value_in_toml, unset_scoped_value_in_toml,
    };
}

pub use crate::osp_config::{
    ConfigError, ConfigSource, ConfigValue, Scope, SecretValue, bootstrap_key_spec,
    build_runtime_pipeline, default_cache_root_dir, default_config_root_dir,
    default_state_root_dir, is_alias_key, is_bootstrap_only_key, validate_bootstrap_value,
    validate_key_scope,
};
