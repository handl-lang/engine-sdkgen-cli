//! The `cli` [`SdkgenBackend`]: `*.clispec.yaml` files load directly;
//! anything else is a tool name (or path) probed via `--help`.
//!
//! A spec file carrying a `probe:` block *also* probes `name` and
//! overlays the authored `commands` on the discovered tree — authored
//! nodes win field-by-field, so a spec can declare only the tree shape
//! and let probe fill in options.

use handl_plugin_sdkgen::{SdkgenBackend, SdkgenError, SdkgenModule};
use std::path::Path;

use crate::config::CliConfig;
use crate::map::map_spec;
use crate::probe::{CommandRunner, ProbePlan, probe_tool};
use crate::spec::{CliSpec, CommandNode, ProbeSpec};

/// CLI backend: spec-file or live-probe input, [`CliConfig`] overlay.
pub struct CliBackend {
    target: Option<String>,
    config: CliConfig,
}

impl CliBackend {
    pub fn new(target: Option<String>, config: CliConfig) -> Self {
        Self { target, config }
    }
}

/// Whether `path` names a `*.yaml`/`*.yml` spec (existing or not).
fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml")
    )
}

impl SdkgenBackend for CliBackend {
    fn parse_header(&self, path: &Path) -> Result<SdkgenModule, SdkgenError> {
        let mut diagnostics = Vec::new();

        // Resolve the input into a spec plus the probe target string —
        // for a spec file the probed tool is the spec's own `name`.
        let (mut spec, probe_target) = if path.is_file() && is_yaml(path) {
            let spec = CliSpec::load(path).map_err(SdkgenError)?;
            let target = spec.name.clone();
            (spec, target)
        } else if is_yaml(path) {
            return Err(SdkgenError(format!(
                "cli spec not found: {}",
                path.display()
            )));
        } else {
            let arg = path.to_string_lossy().into_owned();
            if path.exists() {
                // An existing non-yaml file is a directory or device —
                // not a spec and not a runnable tool.
                if !path.is_file() {
                    return Err(SdkgenError(format!(
                        "not a spec or tool: {}",
                        path.display()
                    )));
                }
            } else if arg.contains('/') || arg.contains('\\') {
                return Err(SdkgenError(format!("tool or spec not found: {arg}")));
            }
            // Bare tool name (or executable path): probe it. The spec
            // name is the basename — `probe` is implicit.
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&arg)
                .to_string();
            (
                CliSpec {
                    name,
                    probe: Some(ProbeSpec::default()),
                    ..Default::default()
                },
                arg,
            )
        };

        // Probe when the spec asks for it (explicit `probe:` or the
        // implicit plan for a bare tool input).
        if spec.probe.is_some() {
            let probe = spec.probe.clone().unwrap_or_default();
            let plan = ProbePlan {
                flag: probe.flag.clone().unwrap_or_else(|| "--help".to_string()),
                max_depth: self.config.max_depth.or(probe.max_depth).unwrap_or(1),
            };
            let runner = CommandRunner::new();
            let result = probe_tool(&probe_target, &plan, &|t, a| runner.run(t, a));
            // `confidence:` is consumed by emitters as a header
            // annotation; the per-node warnings/failures follow it.
            diagnostics.push(format!("confidence: {}", result.confidence));
            diagnostics.extend(result.diagnostics);
            // Authored commands overlay the probed tree field-by-field.
            let authored = std::mem::take(&mut spec.root.commands);
            spec.root.commands = result.root.commands;
            merge_commands(&mut spec.root.commands, authored);
            if spec.root.options.is_empty() {
                spec.root.options = result.root.options;
            }
            if spec.root.positionals.is_empty() {
                spec.root.positionals = result.root.positionals;
            }
            if spec.root.rest.is_none() {
                spec.root.rest = result.root.rest;
            }
        }

        diagnostics.extend(self.config.apply(&mut spec));
        spec.validate().map_err(SdkgenError)?;

        // `emit_spec` — the "probe then hand-tune" workflow: dump the
        // resolved tree as canonical `.clispec.yaml` (`-` → stdout).
        if let Some(dest) = &self.config.emit_spec {
            let yaml = spec.dump();
            if dest == "-" {
                print!("{yaml}");
            } else {
                std::fs::write(dest, yaml).map_err(|e| {
                    SdkgenError(format!("failed to write emitted spec `{dest}`: {e}"))
                })?;
            }
        }

        let target = self.target.clone().unwrap_or_else(|| "host".to_string());
        let mut module = map_spec(&spec, &target);
        module.diagnostics = diagnostics;
        Ok(module)
    }
}

/// Overlay `authored` on `probed`: matching paths merge field-by-field
/// (authored wins), authored-only commands are inserted.
fn merge_commands(
    probed: &mut std::collections::BTreeMap<String, CommandNode>,
    authored: std::collections::BTreeMap<String, CommandNode>,
) {
    for (name, node) in authored {
        match probed.get_mut(&name) {
            Some(existing) => merge_node(existing, node),
            None => {
                probed.insert(name, node);
            }
        }
    }
}

fn merge_node(probed: &mut CommandNode, authored: CommandNode) {
    if authored.about.is_some() {
        probed.about = authored.about;
    }
    for (name, opt) in authored.options {
        probed.options.insert(name, opt);
    }
    if !authored.positionals.is_empty() {
        probed.positionals = authored.positionals;
    }
    if authored.rest.is_some() {
        probed.rest = authored.rest;
    }
    merge_commands(&mut probed.commands, authored.commands);
}
