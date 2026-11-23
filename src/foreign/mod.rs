//! Foreign CLI spec ingestion — spec formats from other ecosystems
//! lowered into the [`CliSpec`] IR.
//!
//! Three dialects are recognized, one file each (plus the shared
//! detection/lowering helpers here):
//!
//! - **fig** ([`fig`]) — a serialized [withfig/autocomplete][fig]
//!   completion spec: the `subcommands` / `options` / `args` tree.
//!   `*.fig.json` and `*.figspec` names claim it outright; any other
//!   JSON file carrying a `subcommands` array or fig-shaped `options`
//!   rows (`{"name": ["--long", "-s"], …}`) is sniffed as fig.
//! - **cli-schema v1** ([`cli_schema`]) — the [cli-schema]
//!   agent-facing document: `schemaVersion`, `commands` /
//!   `namespaces`, `parameters` with `role` / `type`.
//!   `*.cli-schema.json` names and the `schemaVersion` /
//!   `cli-schema-version` markers select it.
//! - **usage** ([`usage`], on the [`kdl`] reader) — the [usage] spec:
//!   KDL `flag "-f --force"` / `arg "<x>"` / `cmd "name" { … }` trees
//!   (mise ships one). `*.kdl` / `*.usage.kdl` names claim it; content
//!   is sniffed when a document parses as KDL and carries top-level
//!   `cmd`/`flag`/`arg`/`option`/`usage` nodes.
//!
//! Constructs with no IR analog but useful meaning — fig suggestion
//! `generators`, cli-schema `intent`/`environment` declarations —
//! degrade into bracketed doc annotations (`[generator: filepaths]`,
//! `[intent: destructive, scope: global]`, `[env: GITHUB_TOKEN
//! (required)]`), the same convention `exclusiveOn`/`dependsOn`/
//! aliases/`required` already use. Anything else unrecognized is
//! dropped with a warning naming it (`fig git.checkout: unsupported
//! subcommand fields dropped: loadSpec`).
//!
//! [fig]: https://github.com/withfig/autocomplete
//! [cli-schema]: https://cli-schema.github.io
//! [usage]: https://usage.jdx.dev

pub(crate) mod cli_schema;
pub(crate) mod fig;
pub(crate) mod kdl;
pub(crate) mod usage;

use serde_json::{Map, Value};
use std::path::Path;

use crate::spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec};

/// A foreign spec dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Serialized fig completion spec (withfig/autocomplete).
    Fig,
    /// cli-schema v1 document.
    CliSchema,
    /// usage spec (jdx/usage) — a KDL document.
    Usage,
}

impl Format {
    /// Diagnostic prefix for this dialect.
    pub fn label(self) -> &'static str {
        match self {
            Format::Fig => "fig",
            Format::CliSchema => "cli-schema",
            Format::Usage => "usage",
        }
    }
}

/// A foreign spec lowered into [`CliSpec`], plus warnings for the
/// constructs that did not map.
#[derive(Debug)]
pub struct ForeignSpec {
    /// Which dialect the input was written in.
    pub format: Format,
    /// The lowered spec — already validated.
    pub spec: CliSpec,
    /// One entry per group of dropped/renamed constructs.
    pub warnings: Vec<String>,
}

impl ForeignSpec {
    /// The dialect `path`/`src` should be parsed as, if any.
    ///
    /// File names claim first (`*.fig.json`, `*.figspec`,
    /// `*.cli-schema.json`, `*.kdl`/`*.usage.kdl`); otherwise content
    /// markers decide — `schemaVersion`/`cli-schema-version` selects
    /// cli-schema, a `subcommands` array or fig-shaped `options` rows
    /// select fig, and a document that parses as KDL with top-level
    /// `cmd`/`flag`/`arg`/`option`/`usage` nodes selects usage.
    pub fn detect(path: &Path, src: &str) -> Option<Format> {
        if is_fig_name(path) {
            return Some(Format::Fig);
        }
        if is_cli_schema_name(path) {
            return Some(Format::CliSchema);
        }
        if is_kdl_name(path) {
            return Some(Format::Usage);
        }
        if let Ok(v) = serde_json::from_str::<Value>(src)
            && let Some(obj) = v.as_object()
        {
            if obj.contains_key("cli-schema-version") || obj.contains_key("schemaVersion") {
                return Some(Format::CliSchema);
            }
            if fig_shaped(obj) {
                return Some(Format::Fig);
            }
        }
        if usage::usage_shaped(src) {
            return Some(Format::Usage);
        }
        None
    }

    /// Parse `src` (already detected as `format`) into a validated
    /// [`CliSpec`] plus drop warnings.
    pub fn parse(format: Format, src: &str) -> Result<Self, String> {
        let (spec, warnings) = match format {
            Format::Fig => fig::parse_fig(src)?,
            Format::CliSchema => cli_schema::parse_cli_schema(src)?,
            Format::Usage => usage::parse_usage(src)?,
        };
        spec.validate()?;
        Ok(Self {
            format,
            spec,
            warnings,
        })
    }

    /// Read and ingest `path` if it is a recognized foreign spec —
    /// `Ok(None)` when the file is unreadable, not UTF-8 JSON, or
    /// carries no foreign markers (e.g. an executable to probe).
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let Ok(src) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        let Some(format) = Self::detect(path, &src) else {
            return Ok(None);
        };
        Self::parse(format, &src).map(Some)
    }
}

/// `*.fig.json` / `*.figspec` file names claim the fig dialect.
fn is_fig_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".fig.json") || n.ends_with(".figspec"))
}

/// `*.cli-schema.json` — the cli-schema sidecar convention.
fn is_cli_schema_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".cli-schema.json"))
}

/// `*.kdl` / `*.usage.kdl` — the usage-spec convention (mise's spec
/// is `mise.usage.kdl`).
fn is_kdl_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".kdl"))
}

/// Whether `path`'s name alone marks a foreign spec — used to error
/// on missing spec files instead of probing them as tools.
pub(crate) fn claims_path(path: &Path) -> bool {
    is_fig_name(path) || is_cli_schema_name(path) || is_kdl_name(path)
}

/// A JSON object shaped like a fig spec: a `subcommands` array, or a
/// non-empty `options` array whose rows all carry `name` spellings.
fn fig_shaped(obj: &Map<String, Value>) -> bool {
    if obj.get("subcommands").is_some_and(|s| s.is_array()) {
        return true;
    }
    obj.get("options")
        .and_then(Value::as_array)
        .is_some_and(|opts| {
            !opts.is_empty()
                && opts
                    .iter()
                    .all(|o| o.get("name").is_some_and(|n| n.is_string() || n.is_array()))
        })
}

// ------------------------------------------------------------------
// Shared mapping helpers — the lowering utilities every dialect's
// mapper (`fig`, `cli_schema`, `usage`) is built on.
// ------------------------------------------------------------------

/// `string | string[]` → owned list of the string members.
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

/// Keys present in `obj` that the mapping does not consume — `x-*`
/// vendor extensions are intentionally free.
fn unsupported<'a>(obj: &'a Map<String, Value>, known: &[&str]) -> Vec<&'a str> {
    obj.keys()
        .map(String::as_str)
        .filter(|k| !known.contains(k) && !k.starts_with("x-"))
        .collect()
}

fn warn_unsupported(
    w: &mut Vec<String>,
    fmt: &str,
    path: &str,
    what: &str,
    obj: &Map<String, Value>,
    known: &[&str],
) {
    let dropped = unsupported(obj, known);
    if !dropped.is_empty() {
        w.push(format!(
            "{fmt} {path}: unsupported {what} fields dropped: {}",
            dropped.join(", ")
        ));
    }
}

/// Doc text: the description plus bracketed annotations for the
/// relations the IR cannot express (`[aliases: -m]`, `[required]`).
fn join_doc(description: Option<&str>, annotations: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    parts.extend(annotations.iter().map(|a| format!("[{a}]")));
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Make a spec map key / positional name out of a raw flag or arg
/// label: `--dry-run` → `dry_run`, `Pull Request` → `pull_request`.
fn ident(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim_matches('-').chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// A positional name unique within `node` — suffixes `_2`, `_3`, …
fn unique_pos_name(node: &CommandNode, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    loop {
        let taken = node.positionals.iter().any(|p| p.name == name)
            || node.rest.as_ref().is_some_and(|r| r.name == name);
        if !taken {
            return name;
        }
        name = format!("{base}_{n}");
        n += 1;
    }
}

/// Push `pos`, renaming on a collision so `validate` stays happy.
fn push_positional(
    node: &mut CommandNode,
    mut pos: PositionalSpec,
    fmt: &str,
    path: &str,
    w: &mut Vec<String>,
) {
    let base = if pos.name.is_empty() {
        "arg".to_string()
    } else {
        pos.name.clone()
    };
    let unique = unique_pos_name(node, &base);
    if unique != base {
        w.push(format!(
            "{fmt} {path}: duplicate positional `{base}` renamed `{unique}`"
        ));
    }
    pos.name = unique;
    node.positionals.push(pos);
}

/// Insert `opt` under `key`, deduplicating the key and refusing a
/// flag spelling the node already has (the first spec wins).
fn insert_option(
    node: &mut CommandNode,
    key: &str,
    opt: OptionSpec,
    fmt: &str,
    path: &str,
    w: &mut Vec<String>,
) {
    if node.options.values().any(|o| o.flag == opt.flag) {
        w.push(format!(
            "{fmt} {path}: duplicate flag `{}` dropped",
            opt.flag
        ));
        return;
    }
    let base = if key.is_empty() { "opt" } else { key };
    let mut name = base.to_string();
    let mut n = 2;
    while node.options.contains_key(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    node.options.insert(name, opt);
}
