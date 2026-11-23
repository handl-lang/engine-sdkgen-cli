//! CLI backend and emitters for the Handl codegen pipeline (ADR-0107,
//! PL-004) — SDKs for tools that only ship a binary.
//!
//! Pipeline, in module order:
//! - [`backend`] — `CliBackend`: `*.clispec.yaml` input or a bare tool
//!   name probed via `--help`, plus the `--config` overlay.
//! - [`spec`] — the authored spec format ([`CliSpec`]), validation,
//!   and `dump()` for the probe-then-hand-tune `emit_spec` workflow.
//! - [`probe`]/[`help`] — live help capture (fallback `--help` → `-h`
//!   → `help` → bare) and its pure parser with confidence diagnostics.
//! - [`config`] — `*.sdkgen.yaml` overrides ([`CliConfig`]).
//! - [`foreign`] — foreign spec ingestion: fig completion specs,
//!   cli-schema v1 documents, and usage (KDL) specs lowered into
//!   [`CliSpec`].
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

use handl_plugin_codegen::{
    AnyEmitter, AnySource, CodegenEmitterDef, CodegenRegistry, CodegenRequest, CodegenSourceDef,
    CodegenTarget, erase_emitter, erase_source,
};
use std::sync::Arc;

/// Register the `cli` codegen target on `reg` (ADR-0128 §0.1, PL-004):
/// source `cli`, emitter `hnl-cli`, and the target record
/// `(source: "cli", emitter: "hnl-cli")` — emitters resolve by name
/// through the registry, never by crate dependency.
pub fn register(reg: &mut CodegenRegistry) {
    reg.register_source(CodegenSourceDef {
        name: "cli",
        make: make_source,
    });
    reg.register_emitter(CodegenEmitterDef {
        name: "hnl-cli",
        make: make_emitter,
    });
    reg.register_target(CodegenTarget {
        name: "cli",
        source: "cli",
        emitter: "hnl-cli",
        source_filters: false,
        docs: None,
        spec_input: None,
    });
}

fn make_source(req: &CodegenRequest<'_>) -> Result<Box<dyn AnySource>, String> {
    let mut config = match req.config {
        Some(path) => CliConfig::load(path)?,
        None => CliConfig::default(),
    };
    // `--emit-spec <path|->` is an overlay applied post-config: the
    // flag wins over a config-file `emit_spec:` key.
    if let Some(path) = req.emit_spec {
        config.emit_spec = Some(path.to_string_lossy().into_owned());
    }
    Ok(erase_source(CliBackend::new(
        req.target.map(str::to_string),
        config,
    )))
}

fn make_emitter(req: &CodegenRequest<'_>) -> Result<Box<dyn AnyEmitter>, String> {
    Ok(erase_emitter(HnlCliEmitter::new(Arc::new(
        req.lang.clone(),
    ))))
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
