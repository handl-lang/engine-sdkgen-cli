//! CLI backend and emitters for the Handl SDK generator (ADR-0107) —
//! SDKs for tools that only ship a binary.
//!
//! Pipeline, in module order:
//! - [`backend`] — `CliBackend`: `*.clispec.yaml` input or a bare tool
//!   name probed via `--help`, plus the `--config` overlay.
//! - [`spec`] — the authored spec format ([`CliSpec`]), validation,
//!   and `dump()` for the probe-then-hand-tune `emit_spec` workflow.
//! - [`probe`]/[`help`] — live help capture (fallback `--help` → `-h`
//!   → `help` → bare) and its pure parser with confidence diagnostics.
//! - [`config`] — `*.sdkgen.yaml` overrides ([`CliConfig`]).
//! - [`foreign`] — foreign spec ingestion: fig completion specs and
//!   cli-schema v1 documents lowered into [`CliSpec`].
//! - [`map`] — `CliSpec` → `SdkgenModule` (one `Function` per leaf
//!   command; see the `arg_descriptors` contract there).
//! - [`emit_hnl`] — `hnl-cli` emitter: `.hnl` port/adapter pair against
//!   the `cli`/`cli-c` packages.

mod backend;
mod config;
mod emit_hnl;
mod foreign;
mod help;
mod map;
mod probe;
mod spec;

pub use backend::CliBackend;
pub use config::CliConfig;
pub use emit_hnl::HnlCliEmitter;
pub use foreign::{ForeignSpec, Format};
pub use help::{ParsedHelp, ParsedOption, ParsedPositional, parse_help};
pub use probe::{CommandRunner, ProbePlan, ProbeResult, RunHelp, probe_tool};
pub use spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec, ProbeSpec};

use handl_plugin_sdkgen::{
    SdkgenBackend, SdkgenBackendDef, SdkgenEmitterDef, SdkgenRegistry, SdkgenRequest, SdkgenTarget,
};
use std::sync::Arc;

/// Register the `cli` sdkgen target on `reg` (ADR-0128 §0.1): backend
/// `cli`, emitter `hnl-cli`, and the target record `(backend: "cli",
/// emitter: "hnl-cli")` — emitters resolve by name through the
/// registry, never by crate dependency.
pub fn register(reg: &mut SdkgenRegistry) {
    reg.register_backend(SdkgenBackendDef {
        name: "cli",
        make: make_backend,
    });
    reg.register_emitter(SdkgenEmitterDef {
        name: "hnl-cli",
        make: |req| Ok(Box::new(HnlCliEmitter::new(Arc::new(req.lang.clone())))),
    });
    reg.register_target(SdkgenTarget {
        name: "cli",
        backend: "cli",
        emitter: "hnl-cli",
        source_filters: false,
        docs: None,
        spec_input: None,
    });
}

fn make_backend(req: &SdkgenRequest<'_>) -> Result<Box<dyn SdkgenBackend>, String> {
    let config = match req.config {
        Some(path) => CliConfig::load(path)?,
        None => CliConfig::default(),
    };
    Ok(Box::new(CliBackend::new(
        req.target.map(str::to_string),
        config,
    )))
}

#[cfg(test)]
mod config_test;
#[cfg(test)]
mod emit_hnl_test;
#[cfg(test)]
mod foreign_test;
#[cfg(test)]
mod help_test;
#[cfg(test)]
mod map_test;
#[cfg(test)]
mod probe_test;
#[cfg(test)]
mod spec_test;
