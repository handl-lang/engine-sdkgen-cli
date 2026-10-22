//! `*.sdkgen.yaml` configuration overlay for the `cli` backend.
//!
//! ```yaml
//! rename: { "pr.create": "make" }       # rename a command (dotted path)
//! drop: ["pr.delete"]                   # remove a command subtree
//! types: { "pr.create.reviewers": strings }  # override an option/positional kind
//! max_depth: 2                          # probe recursion cap
//! emit_spec: gh.clispec.yaml            # dump the resolved tree ("-" for stdout)
//! ```
//!
//! The overlay is applied after spec load or probing. All dotted paths
//! refer to the *original* command names — `drop`/`types` run before
//! `rename`, so a renamed command's new name is not a valid `types` key.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

use crate::spec::{CliSpec, CommandNode, OPTION_KINDS, POSITIONAL_KINDS};

/// Per-tool SDK generation configuration.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    /// `dotted.command.path` → new leaf name.
    pub rename: BTreeMap<String, String>,
    /// `dotted.command.path` subtrees to remove entirely.
    #[serde(default)]
    pub drop: Vec<String>,
    /// `dotted.command.path.arg` → new kind for an option or positional.
    pub types: BTreeMap<String, String>,
    /// Probe recursion cap — overrides the spec's `probe.max_depth`.
    pub max_depth: Option<usize>,
    /// Write the resolved command tree (post-probe, post-overlay) as a
    /// canonical `.clispec.yaml` to this path — the "probe then
    /// hand-tune" workflow. `"-"` writes to stdout. The `--emit-spec`
    /// CLI flag sets this post-load and wins when both are present.
    pub emit_spec: Option<String>,
}

impl CliConfig {
    /// Load a `*.sdkgen.yaml` file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config {}: {e}", path.display()))?;
        serde_yaml::from_str(&s).map_err(|e| format!("failed to parse config: {e}"))
    }

    /// Apply the overlay to `spec` — `drop`, then `types`, then
    /// `rename`. Returns diagnostics for paths that matched nothing.
    pub fn apply(&self, spec: &mut CliSpec) -> Vec<String> {
        let mut diags = Vec::new();
        for path in &self.drop {
            let removed = match parent_commands(&mut spec.root, path) {
                Some((map, leaf)) => map.remove(leaf.as_str()).is_some(),
                None => false,
            };
            if !removed {
                diags.push(format!("config: drop `{path}` matched no command"));
            }
        }
        for (path, kind) in &self.types {
            if let Err(e) = apply_type(&mut spec.root, path, kind) {
                diags.push(e);
            }
        }
        for (path, new_name) in &self.rename {
            match parent_commands(&mut spec.root, path) {
                Some((map, leaf)) => match map.remove(leaf.as_str()) {
                    Some(node) => {
                        map.insert(new_name.clone(), node);
                    }
                    None => diags.push(format!("config: rename `{path}` matched no command")),
                },
                None => diags.push(format!("config: rename `{path}` matched no command")),
            }
        }
        diags
    }
}

/// Split a dotted `a.b.leaf` into (the commands map containing `leaf`,
/// `leaf`) — `None` when a middle segment names no subcommand.
fn parent_commands<'a>(
    root: &'a mut CommandNode,
    path: &str,
) -> Option<(&'a mut BTreeMap<String, CommandNode>, String)> {
    let mut segs: Vec<&str> = path.split('.').collect();
    let leaf = segs.pop()?.to_string();
    let mut node = root;
    for seg in segs {
        node = node.commands.get_mut(seg)?;
    }
    Some((&mut node.commands, leaf))
}

/// Set the kind of the option, positional, or rest arg named by the
/// last segment of `path`.
fn apply_type(root: &mut CommandNode, path: &str, kind: &str) -> Result<(), String> {
    let mut segs: Vec<&str> = path.split('.').collect();
    let arg = segs.pop().unwrap_or_default();
    let mut node = &mut *root;
    for seg in segs {
        node = match node.commands.get_mut(seg) {
            Some(n) => n,
            None => return Err(format!("config: types `{path}` matched no command")),
        };
    }
    if let Some(opt) = node.options.get_mut(arg) {
        if !OPTION_KINDS.contains(&kind) {
            return Err(format!(
                "config: types `{path}` — `{kind}` is not an option kind"
            ));
        }
        opt.kind = kind.to_string();
        return Ok(());
    }
    if let Some(pos) = node.positionals.iter_mut().find(|p| p.name == arg) {
        if !POSITIONAL_KINDS.contains(&kind) {
            return Err(format!(
                "config: types `{path}` — `{kind}` is not a positional kind"
            ));
        }
        pos.kind = kind.to_string();
        return Ok(());
    }
    if let Some(rest) = &mut node.rest
        && rest.name == arg
    {
        if !POSITIONAL_KINDS.contains(&kind) {
            return Err(format!(
                "config: types `{path}` — `{kind}` is not a positional kind"
            ));
        }
        rest.kind = kind.to_string();
        return Ok(());
    }
    Err(format!(
        "config: types `{path}` matched no option or positional"
    ))
}
