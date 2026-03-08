use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::visit::{self, Visit};

// This test intentionally encodes logical component boundaries rather than
// today's crate layout, so the same policy can survive a later collapse into a
// single published foundation crate.
const WORKSPACE_CRATES: &[&str] = &[
    "osp_api",
    "osp_cli",
    "osp_completion",
    "osp_config",
    "osp_core",
    "osp_dsl",
    "osp_ports",
    "osp_repl",
    "osp_services",
    "osp_ui",
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Component {
    Api,
    Cli,
    Completion,
    Config,
    Core,
    Dsl,
    Ports,
    Repl,
    Services,
    Ui,
}

impl Component {
    fn crate_name(self) -> &'static str {
        match self {
            Self::Api => "osp-api",
            Self::Cli => "osp-cli",
            Self::Completion => "osp-completion",
            Self::Config => "osp-config",
            Self::Core => "osp-core",
            Self::Dsl => "osp-dsl",
            Self::Ports => "osp-ports",
            Self::Repl => "osp-repl",
            Self::Services => "osp-services",
            Self::Ui => "osp-ui",
        }
    }

    fn rust_module_name(self) -> &'static str {
        match self {
            Self::Api => "osp_api",
            Self::Cli => "osp_cli",
            Self::Completion => "osp_completion",
            Self::Config => "osp_config",
            Self::Core => "osp_core",
            Self::Dsl => "osp_dsl",
            Self::Ports => "osp_ports",
            Self::Repl => "osp_repl",
            Self::Services => "osp_services",
            Self::Ui => "osp_ui",
        }
    }

    fn runtime_allowed(self) -> BTreeSet<Component> {
        let values = match self {
            Self::Api => &[Self::Api, Self::Core, Self::Ports][..],
            Self::Cli => &[
                Self::Cli,
                Self::Completion,
                Self::Config,
                Self::Core,
                Self::Dsl,
                Self::Repl,
                Self::Ui,
            ][..],
            Self::Completion => &[Self::Completion][..],
            Self::Config => &[Self::Config][..],
            Self::Core => &[Self::Core][..],
            Self::Dsl => &[Self::Dsl, Self::Core][..],
            Self::Ports => &[Self::Ports, Self::Core][..],
            Self::Repl => &[Self::Repl, Self::Completion][..],
            Self::Services => &[
                Self::Services,
                Self::Config,
                Self::Core,
                Self::Dsl,
                Self::Ports,
            ][..],
            Self::Ui => &[Self::Ui, Self::Core][..],
        };
        values.iter().copied().collect()
    }

    fn dev_allowed(self) -> BTreeSet<Component> {
        let mut allowed = self.runtime_allowed();
        match self {
            Self::Cli => {
                allowed.extend([Self::Api, Self::Ports]);
            }
            Self::Services => {
                allowed.insert(Self::Api);
            }
            _ => {}
        }
        allowed
    }
}

#[test]
fn cargo_manifests_follow_runtime_and_dev_dependency_matrix() {
    for component in all_components() {
        let manifest_path = workspace_root()
            .join("crates")
            .join(component.crate_name())
            .join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));
        let parsed: toml::Value = toml::from_str(&manifest)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", manifest_path.display()));

        let runtime = internal_deps_from_table(parsed.get("dependencies"));
        let expected_runtime = component
            .runtime_allowed()
            .into_iter()
            .filter(|candidate| *candidate != component)
            .map(|candidate| candidate.crate_name().to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            runtime,
            expected_runtime,
            "runtime internal dependencies drifted for {}",
            component.crate_name()
        );

        let dev = internal_deps_from_table(parsed.get("dev-dependencies"));
        let allowed_dev = component
            .dev_allowed()
            .into_iter()
            .filter(|candidate| *candidate != component)
            .map(|candidate| candidate.crate_name().to_string())
            .collect::<BTreeSet<_>>();
        let disallowed_dev = dev
            .difference(&allowed_dev)
            .cloned()
            .collect::<BTreeSet<_>>();
        assert!(
            disallowed_dev.is_empty(),
            "dev internal dependencies drifted for {}: disallowed entries = {:?}",
            component.crate_name(),
            disallowed_dev
        );
    }
}

#[test]
fn non_test_source_imports_follow_logical_layer_matrix() {
    let root = workspace_root().join("crates");
    let mut failures = Vec::new();

    for component in all_components() {
        let src_root = root.join(component.crate_name()).join("src");
        let mut files = Vec::new();
        collect_rust_files(&src_root, &mut files);

        for file in files {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
            let syntax = syn::parse_file(&source)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));
            let imports = non_test_workspace_imports(&syntax);
            let allowed = component
                .runtime_allowed()
                .into_iter()
                .map(Component::rust_module_name)
                .collect::<BTreeSet<_>>();

            let disallowed = imports
                .difference(&allowed)
                .copied()
                .collect::<BTreeSet<_>>();
            if !disallowed.is_empty() {
                failures.push(format!(
                    "{} imports disallowed workspace modules: {}",
                    relative_to_workspace(&file),
                    disallowed.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "non-test source imports drifted from the logical layer matrix:\n{}",
        failures.join("\n")
    );
}

#[test]
fn foundation_prototype_imports_follow_logical_layer_matrix() {
    let root = workspace_root().join("foundation").join("src");
    let mut failures = Vec::new();

    for component in all_components() {
        let src_root = root.join(component.rust_module_name());
        let mut files = Vec::new();
        collect_rust_files(&src_root, &mut files);

        for file in files {
            let source = fs::read_to_string(&file)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
            let syntax = syn::parse_file(&source)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));
            let imports = non_test_workspace_imports(&syntax);
            let allowed = component
                .runtime_allowed()
                .into_iter()
                .map(Component::rust_module_name)
                .collect::<BTreeSet<_>>();

            let disallowed = imports
                .difference(&allowed)
                .copied()
                .collect::<BTreeSet<_>>();
            if !disallowed.is_empty() {
                failures.push(format!(
                    "{} imports disallowed foundation modules: {}",
                    relative_to_workspace(&file),
                    disallowed.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "foundation prototype imports drifted from the logical layer matrix:\n{}",
        failures.join("\n")
    );
}

#[test]
fn foundation_public_facade_stays_curated() {
    let lib_path = workspace_root()
        .join("foundation")
        .join("src")
        .join("lib.rs");
    let source = fs::read_to_string(&lib_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", lib_path.display()));
    let syntax = syn::parse_file(&source)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", lib_path.display()));

    let public_modules = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(item_mod) if matches!(item_mod.vis, syn::Visibility::Public(_)) => {
                Some(item_mod.ident.to_string())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    let expected_modules = [
        "app",
        "runtime",
        "config",
        "core",
        "dsl",
        "ports",
        "api",
        "services",
        "ui",
        "completion",
        "repl",
        "cli",
        "plugin",
        "prelude",
        "osp_core",
        "osp_config",
        "osp_dsl",
        "osp_ports",
        "osp_api",
        "osp_services",
        "osp_ui",
        "osp_completion",
        "osp_repl",
        "osp_cli",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();

    assert_eq!(
        public_modules, expected_modules,
        "foundation/src/lib.rs top-level public modules drifted"
    );

    assert!(
        !source.contains("classify_exit_code")
            && !source.contains("render_report_message")
            && !source.contains("pub use crate::app::{App, AppBuilder, Cli"),
        "foundation public facade leaked internal diagnostics or raw state exports"
    );
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("osp-cli crate dir should have a parent")
        .parent()
        .expect("crates dir should have a parent")
        .to_path_buf()
}

fn all_components() -> [Component; 10] {
    [
        Component::Api,
        Component::Cli,
        Component::Completion,
        Component::Config,
        Component::Core,
        Component::Dsl,
        Component::Ports,
        Component::Repl,
        Component::Services,
        Component::Ui,
    ]
}

fn internal_deps_from_table(section: Option<&toml::Value>) -> BTreeSet<String> {
    section
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|table| table.keys())
        .filter(|name| name.starts_with("osp-"))
        .cloned()
        .collect()
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => panic!("failed to read {}: {err}", root.display()),
    };
    for entry in entries {
        let entry = entry.expect("directory entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn relative_to_workspace(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .expect("path should be inside workspace")
        .display()
        .to_string()
}

fn non_test_workspace_imports(file: &syn::File) -> BTreeSet<&'static str> {
    let mut visitor = WorkspaceImportVisitor::default();
    visitor.visit_file(file);
    visitor.hits
}

#[derive(Default)]
struct WorkspaceImportVisitor {
    hits: BTreeSet<&'static str>,
}

impl WorkspaceImportVisitor {
    fn record_path(&mut self, path: &syn::Path) {
        let mut segments = path.segments.iter();
        let Some(first) = segments.next() else {
            return;
        };
        let candidate = if first.ident == "crate" {
            segments.next().map(|segment| &segment.ident)
        } else {
            Some(&first.ident)
        };
        let Some(candidate) = candidate else {
            return;
        };
        if let Some(hit) = WORKSPACE_CRATES
            .iter()
            .copied()
            .find(|hit| candidate == *hit)
        {
            self.hits.insert(hit);
        }
    }
}

impl<'ast> Visit<'ast> for WorkspaceImportVisitor {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if is_test_only_attrs(&node.attrs) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if is_test_only_attrs(&node.attrs) {
            return;
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_use_path(&mut self, node: &'ast syn::UsePath) {
        if let Some(hit) = WORKSPACE_CRATES
            .iter()
            .copied()
            .find(|candidate| node.ident == *candidate)
        {
            self.hits.insert(hit);
        }
        visit::visit_use_path(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.record_path(node);
        visit::visit_path(self, node);
    }
}

fn is_test_only_attrs(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path.is_ident("test") {
            return true;
        }
        if path.is_ident("cfg") {
            return attr.meta.to_token_stream().to_string().contains("test");
        }
        false
    })
}
