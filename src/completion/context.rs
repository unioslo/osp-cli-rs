use crate::completion::model::{CommandLine, CompletionContext, CompletionNode, CompletionTree};

pub(crate) struct ResolvedNodes<'a> {
    pub(crate) context_node: &'a CompletionNode,
    pub(crate) flag_scope_node: &'a CompletionNode,
}

pub(crate) struct TreeResolver<'a> {
    tree: &'a CompletionTree,
}

impl<'a> TreeResolver<'a> {
    pub(crate) fn new(tree: &'a CompletionTree) -> Self {
        Self { tree }
    }

    pub(crate) fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" || token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            matched += 1;
            if child.value_key || child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }

    pub(crate) fn resolved_nodes(&self, context: &CompletionContext) -> ResolvedNodes<'a> {
        ResolvedNodes {
            context_node: self
                .resolve_exact(&context.matched_path)
                .unwrap_or(&self.tree.root),
            flag_scope_node: self
                .resolve_exact(&context.flag_scope_path)
                .unwrap_or(&self.tree.root),
        }
    }

    pub(crate) fn resolve_flag_scope_path(&self, matched_path: &[String]) -> Vec<String> {
        let floor = if matched_path.is_empty() { 0 } else { 1 };
        for i in (floor..=matched_path.len()).rev() {
            let prefix = &matched_path[..i];
            let Some(node) = self.resolve_exact(prefix) else {
                continue;
            };
            if !node.flags.is_empty() {
                return prefix.to_vec();
            }
        }
        if matched_path.is_empty() {
            Vec::new()
        } else {
            matched_path.to_vec()
        }
    }

    pub(crate) fn resolve_context(&self, path: &[String]) -> (&'a CompletionNode, Vec<String>) {
        let mut node = &self.tree.root;
        let mut matched = Vec::new();

        for segment in path {
            let Some(next) = node.children.get(segment) else {
                break;
            };
            node = next;
            matched.push(segment.clone());
            if node.value_leaf {
                break;
            }
        }

        (node, matched)
    }

    pub(crate) fn resolve_exact(&self, path: &[String]) -> Option<&'a CompletionNode> {
        let (node, matched) = self.resolve_context(path);
        (matched.len() == path.len()).then_some(node)
    }
}

pub(crate) struct ProviderSelection<'a> {
    provider: Option<&'a str>,
    normalized_os: Option<String>,
}

impl<'a> ProviderSelection<'a> {
    pub(crate) fn from_command(cmd: &'a CommandLine) -> Self {
        let provider = cmd
            .flag_values("--provider")
            .and_then(|values| values.first())
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
            .or_else(|| cmd.has_flag("--nrec").then_some("nrec"))
            .or_else(|| cmd.has_flag("--vmware").then_some("vmware"));
        let normalized_os = cmd
            .flag_values("--os")
            .and_then(|values| values.first())
            .map(|value| normalize_token(value));

        Self {
            provider,
            normalized_os,
        }
    }

    pub(crate) fn name(&self) -> Option<&'a str> {
        self.provider
    }

    pub(crate) fn normalized_os(&self) -> Option<&str> {
        self.normalized_os.as_deref()
    }
}

fn normalize_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}
