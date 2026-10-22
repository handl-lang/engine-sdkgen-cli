//! Foreign CLI spec ingestion — JSON spec formats from other
//! ecosystems lowered into the [`CliSpec`] IR.
//!
//! Two dialects are recognized:
//!
//! - **fig** — a serialized [withfig/autocomplete][fig] completion
//!   spec: the `subcommands` / `options` / `args` tree. `*.fig.json`
//!   and `*.figspec` names claim it outright; any other JSON file
//!   carrying a `subcommands` array or fig-shaped `options` rows
//!   (`{"name": ["--long", "-s"], …}`) is sniffed as fig.
//! - **cli-schema v1** — the [cli-schema] agent-facing document:
//!   `schemaVersion`, `commands` / `namespaces`, `parameters` with
//!   `role` / `type`. `*.cli-schema.json` names and the
//!   `schemaVersion` / `cli-schema-version` markers select it.
//!
//! Constructs with no IR analog — suggestion generators, intents,
//! environment requirements — are dropped with a warning naming them
//! (`fig git.checkout: unsupported subcommand fields dropped:
//! loadSpec`). Relations the IR cannot check — `exclusiveOn`,
//! `dependsOn`, aliases, `required` — degrade into doc annotations
//! (`[exclusive with: --interactive]`).
//!
//! [fig]: https://github.com/withfig/autocomplete
//! [cli-schema]: https://cli-schema.github.io

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

use crate::spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec};

/// A foreign spec dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Serialized fig completion spec (withfig/autocomplete).
    Fig,
    /// cli-schema v1 document.
    CliSchema,
}

impl Format {
    /// Diagnostic prefix for this dialect.
    pub fn label(self) -> &'static str {
        match self {
            Format::Fig => "fig",
            Format::CliSchema => "cli-schema",
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
    /// `*.cli-schema.json`); otherwise JSON content markers decide —
    /// `schemaVersion`/`cli-schema-version` selects cli-schema, a
    /// `subcommands` array or fig-shaped `options` rows select fig.
    pub fn detect(path: &Path, src: &str) -> Option<Format> {
        if is_fig_name(path) {
            return Some(Format::Fig);
        }
        if is_cli_schema_name(path) {
            return Some(Format::CliSchema);
        }
        let v: Value = serde_json::from_str(src).ok()?;
        let obj = v.as_object()?;
        if obj.contains_key("cli-schema-version") || obj.contains_key("schemaVersion") {
            return Some(Format::CliSchema);
        }
        if fig_shaped(obj) {
            return Some(Format::Fig);
        }
        None
    }

    /// Parse `src` (already detected as `format`) into a validated
    /// [`CliSpec`] plus drop warnings.
    pub fn parse(format: Format, src: &str) -> Result<Self, String> {
        let (spec, warnings) = match format {
            Format::Fig => parse_fig(src)?,
            Format::CliSchema => parse_cli_schema(src)?,
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

/// Whether `path`'s name alone marks a foreign spec — used to error
/// on missing spec files instead of probing them as tools.
pub(crate) fn claims_path(path: &Path) -> bool {
    is_fig_name(path) || is_cli_schema_name(path)
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
// Shared mapping helpers
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

// ------------------------------------------------------------------
// fig
// ------------------------------------------------------------------

/// Subcommand-level keys the fig mapping consumes — everything else
/// (`loadSpec`, `generateSpec`, `parserDirectives`, display machinery)
/// is reported as dropped.
const FIG_NODE_KEYS: &[&str] = &["name", "description", "subcommands", "options", "args"];
const FIG_OPTION_KEYS: &[&str] = &[
    "name",
    "description",
    "args",
    "isRequired",
    "isRepeatable",
    "isPersistent",
    "exclusiveOn",
    "dependsOn",
];
const FIG_ARG_KEYS: &[&str] = &["name", "description", "isVariadic", "isOptional", "default"];

fn parse_fig(src: &str) -> Result<(CliSpec, Vec<String>), String> {
    let v: Value =
        serde_json::from_str(src).map_err(|e| format!("failed to parse fig spec: {e}"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| "fig spec root must be a JSON object".to_string())?;
    let mut names = strings(obj.get("name")).into_iter();
    let Some(name) = names.next().filter(|n| !n.is_empty()) else {
        return Err("fig spec is missing `name`".to_string());
    };
    let mut w = Vec::new();
    let extra: Vec<String> = names.filter(|n| !n.is_empty()).collect();
    if !extra.is_empty() {
        w.push(format!(
            "fig {name}: extra root names dropped: {}",
            extra.join(", ")
        ));
    }
    let mut spec = CliSpec {
        name: name.clone(),
        ..Default::default()
    };
    fig_node(obj, &[], &name, &mut spec.root, &mut w);
    // The root node's `about` is the module doc — `CliSpec::about` and
    // a flattened `root.about` would collide on dump, so move it up.
    spec.about = spec.root.about.take();
    Ok((spec, w))
}

fn fig_node(
    obj: &Map<String, Value>,
    aliases: &[String],
    path: &str,
    node: &mut CommandNode,
    w: &mut Vec<String>,
) {
    warn_unsupported(w, "fig", path, "subcommand", obj, FIG_NODE_KEYS);
    let mut anns = Vec::new();
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if let Some(doc) = join_doc(obj.get("description").and_then(Value::as_str), &anns) {
        node.about = Some(doc);
    }
    if let Some(opts) = obj.get("options") {
        match opts.as_array() {
            Some(list) => {
                for o in list {
                    match o.as_object() {
                        Some(o) => fig_option(o, path, node, w),
                        None => w.push(format!("fig {path}: non-object option dropped")),
                    }
                }
            }
            None => w.push(format!("fig {path}: `options` is not an array — dropped")),
        }
    }
    if let Some(args) = obj.get("args") {
        fig_args(args, path, node, w);
    }
    if let Some(subs) = obj.get("subcommands") {
        match subs.as_array() {
            Some(list) => {
                for s in list {
                    match s.as_object() {
                        Some(s) => fig_subcommand(s, path, node, w),
                        None => w.push(format!("fig {path}: non-object subcommand dropped")),
                    }
                }
            }
            None => w.push(format!(
                "fig {path}: `subcommands` is not an array — dropped"
            )),
        }
    }
}

fn fig_subcommand(s: &Map<String, Value>, path: &str, node: &mut CommandNode, w: &mut Vec<String>) {
    let names = strings(s.get("name"));
    let mut it = names.into_iter();
    let Some(name) = it.next().filter(|n| !n.is_empty()) else {
        w.push(format!("fig {path}: subcommand with no name dropped"));
        return;
    };
    let aliases: Vec<String> = it.filter(|a| !a.is_empty()).collect();
    let child = node.commands.entry(name.clone()).or_default();
    fig_node(s, &aliases, &format!("{path}.{name}"), child, w);
}

/// An option row's `args` field: whether the flag takes a value, and
/// the arg objects it declares (fig allows several; the IR keeps one).
fn fig_option_args(args: Option<&Value>) -> (bool, Vec<&Map<String, Value>>) {
    match args {
        Some(Value::Object(o)) => (true, vec![o]),
        Some(Value::Array(a)) => (
            !a.is_empty(),
            a.iter().filter_map(Value::as_object).collect(),
        ),
        Some(Value::Null) | None => (false, Vec::new()),
        Some(_) => (true, Vec::new()),
    }
}

/// `isRepeatable: true | <n>` — `true` or a count above 1.
fn fig_repeatable(v: Option<&Value>) -> bool {
    truthy(v) || v.and_then(Value::as_u64).is_some_and(|n| n > 1)
}

fn fig_option(o: &Map<String, Value>, path: &str, node: &mut CommandNode, w: &mut Vec<String>) {
    let names = strings(o.get("name"));
    // Prefer the long spelling for argv emission; the rest are aliases.
    let flag = names
        .iter()
        .find(|n| n.starts_with("--"))
        .or_else(|| names.first());
    let Some(flag) = flag else {
        w.push(format!("fig {path}: option with no usable name dropped"));
        return;
    };
    warn_unsupported(
        w,
        "fig",
        path,
        &format!("option `{flag}`"),
        o,
        FIG_OPTION_KEYS,
    );

    let (takes_arg, arg_objs) = fig_option_args(o.get("args"));
    if arg_objs.len() > 1 {
        w.push(format!(
            "fig {path}: option `{flag}` takes {} args — only the first is kept",
            arg_objs.len()
        ));
    }
    let repeatable = fig_repeatable(o.get("isRepeatable"));
    let kind = match (takes_arg, repeatable) {
        (true, true) => "strings",
        (true, false) => "string",
        (false, _) => "flag",
    };

    let mut anns = Vec::new();
    let aliases: Vec<&str> = names
        .iter()
        .filter(|n| n.as_str() != flag)
        .map(String::as_str)
        .collect();
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if truthy(o.get("isRequired")) {
        anns.push("required".to_string());
    }
    if truthy(o.get("isPersistent")) {
        anns.push("persistent".to_string());
    }
    if repeatable && !takes_arg {
        anns.push("repeatable".to_string());
    }
    if let Some(first) = arg_objs.first() {
        if let Some(n) = first
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            anns.push(format!("value: {n}"));
        }
    }
    for (key, label) in [
        ("exclusiveOn", "exclusive with"),
        ("dependsOn", "depends on"),
    ] {
        let rel = strings(o.get(key));
        if !rel.is_empty() {
            anns.push(format!("{label}: {}", rel.join(", ")));
        }
    }

    let spec = OptionSpec {
        flag: flag.clone(),
        kind: kind.to_string(),
        doc: join_doc(o.get("description").and_then(Value::as_str), &anns),
    };
    insert_option(node, &ident(flag), spec, "fig", path, w);
}

/// Subcommand `args` — ordered positionals; an `isVariadic` entry
/// becomes the `rest` tail and anything after it is dropped.
fn fig_args(args: &Value, path: &str, node: &mut CommandNode, w: &mut Vec<String>) {
    let list: Vec<&Value> = match args {
        Value::Array(a) => a.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    };
    let mut variadic_seen = node.rest.is_some();
    for (i, a) in list.iter().enumerate() {
        let Some(a) = a.as_object() else {
            w.push(format!("fig {path}: non-object arg dropped"));
            continue;
        };
        if variadic_seen {
            w.push(format!("fig {path}: arg after the variadic arg dropped"));
            continue;
        }
        warn_unsupported(w, "fig", path, &format!("arg #{}", i + 1), a, FIG_ARG_KEYS);
        let mut anns = Vec::new();
        if truthy(a.get("isOptional")) {
            anns.push("optional".to_string());
        }
        if let Some(d) = a.get("default").and_then(Value::as_str) {
            anns.push(format!("default: {d}"));
        }
        let name = a
            .get("name")
            .and_then(Value::as_str)
            .map(ident)
            .unwrap_or_default();
        let pos = PositionalSpec {
            name: if name.is_empty() {
                format!("arg{}", i + 1)
            } else {
                name
            },
            kind: "string".to_string(),
            doc: join_doc(a.get("description").and_then(Value::as_str), &anns),
        };
        if truthy(a.get("isVariadic")) {
            variadic_seen = true;
            node.rest = Some(pos);
        } else {
            push_positional(node, pos, "fig", path, w);
        }
    }
}

// ------------------------------------------------------------------
// cli-schema v1
// ------------------------------------------------------------------

/// `version`/`schemaVersion` count as consumed (read, just unused by
/// the IR); everything else unlisted — `environment`, `intent`,
/// `authCommands` — is reported as dropped.
const CS_ROOT_KEYS: &[&str] = &[
    "schemaVersion",
    "cli-schema-version",
    "name",
    "version",
    "description",
    "globalOptions",
    "rootDefault",
    "commands",
    "namespaces",
];
const CS_COMMAND_KEYS: &[&str] = &[
    "name",
    "path",
    "summary",
    "notes",
    "parameters",
    "aliases",
    "hidden",
    "deprecated",
];
const CS_NAMESPACE_KEYS: &[&str] = &[
    "segment",
    "summary",
    "notes",
    "options",
    "defaultCommand",
    "commands",
    "namespaces",
];
const CS_PARAM_KEYS: &[&str] = &[
    "role",
    "name",
    "type",
    "required",
    "shortName",
    "summary",
    "defaultValue",
    "repeatable",
    "separator",
    "aliases",
    "enumValues",
    "elementType",
    "hidden",
    "variadic",
    "deprecated",
    "validations",
];
const CS_HANDLER_KEYS: &[&str] = &["kind", "summary", "notes", "parameters"];

fn parse_cli_schema(src: &str) -> Result<(CliSpec, Vec<String>), String> {
    let v: Value = serde_json::from_str(src)
        .map_err(|e| format!("failed to parse cli-schema document: {e}"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| "cli-schema document root must be a JSON object".to_string())?;
    if let Some(version) = obj
        .get("schemaVersion")
        .or_else(|| obj.get("cli-schema-version"))
    {
        let ok = version.as_u64() == Some(1) || version.as_str() == Some("1");
        if !ok {
            return Err(format!("unsupported cli-schema version {version}"));
        }
    }
    let Some(name) = obj
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    else {
        return Err("cli-schema document is missing `name`".to_string());
    };
    let mut w = Vec::new();
    warn_unsupported(&mut w, "cli-schema", name, "root", obj, CS_ROOT_KEYS);
    let mut spec = CliSpec {
        name: name.to_string(),
        about: obj
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };

    // `globalOptions` are flags every command accepts — propagate them
    // down the tree (a command's own spelling wins) and into the root
    // node, which is the bare-`tool` leaf.
    let mut inherited = BTreeMap::new();
    if let Some(g) = obj.get("globalOptions") {
        let mut holder = CommandNode::default();
        cs_params(&mut holder, g, name, false, &mut w);
        for (k, v) in holder.options {
            spec.root
                .options
                .entry(k.clone())
                .or_insert_with(|| v.clone());
            inherited.insert(k, v);
        }
    }
    // `rootDefault` is the handler for a bare invocation — its
    // parameters are the root node's own.
    if let Some(rd) = obj.get("rootDefault") {
        match rd.as_object() {
            Some(rd) => {
                warn_unsupported(
                    &mut w,
                    "cli-schema",
                    name,
                    "rootDefault",
                    rd,
                    CS_HANDLER_KEYS,
                );
                if spec.about.is_none() {
                    spec.about = summary_notes(rd);
                }
                if let Some(ps) = rd.get("parameters") {
                    cs_params(&mut spec.root, ps, name, true, &mut w);
                }
            }
            None => w.push(format!(
                "cli-schema {name}: `rootDefault` is not an object — dropped"
            )),
        }
    }
    if let Some(nss) = obj.get("namespaces") {
        match nss.as_array() {
            Some(list) => {
                for ns in list {
                    match ns.as_object() {
                        Some(ns) => cs_namespace(&mut spec.root, ns, &inherited, name, &mut w),
                        None => w.push(format!("cli-schema {name}: non-object namespace dropped")),
                    }
                }
            }
            None => w.push(format!(
                "cli-schema {name}: `namespaces` is not an array — dropped"
            )),
        }
    }
    if let Some(cmds) = obj.get("commands") {
        match cmds.as_array() {
            Some(list) => {
                for c in list {
                    match c.as_object() {
                        Some(c) => cs_command(&mut spec.root, c, &inherited, name, &mut w),
                        None => w.push(format!("cli-schema {name}: non-object command dropped")),
                    }
                }
            }
            None => w.push(format!(
                "cli-schema {name}: `commands` is not an array — dropped"
            )),
        }
    }
    Ok((spec, w))
}

/// `summary` (one line) + `notes` (extended) combined as doc text.
fn summary_notes(obj: &Map<String, Value>) -> Option<String> {
    let summary = obj.get("summary").and_then(Value::as_str);
    let notes = obj.get("notes").and_then(Value::as_str);
    match (summary, notes) {
        (Some(s), Some(n)) if !n.is_empty() => Some(format!("{s}\n\n{n}")),
        (Some(s), _) => Some(s.to_string()),
        (None, Some(n)) => Some(n.to_string()),
        (None, None) => None,
    }
}

/// `deprecated: bool | { message, replacement }` → doc annotation.
fn deprecated_ann(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Bool(true)) => Some("deprecated".to_string()),
        Some(Value::Object(o)) => {
            if let Some(m) = o.get("message").and_then(Value::as_str) {
                Some(format!("deprecated: {m}"))
            } else if let Some(r) = o.get("replacement").and_then(Value::as_str) {
                Some(format!("deprecated: use `{r}`"))
            } else {
                Some("deprecated".to_string())
            }
        }
        _ => None,
    }
}

/// A mapped parameter — an option (spec name + spec) or a positional
/// (with the `variadic` flag selecting the `rest` tail).
enum CsParam {
    Option(String, OptionSpec),
    Positional(PositionalSpec, bool),
}

/// Map a `parameters`/`globalOptions`/`options` array into `node`.
/// `positionals` is false where the schema forbids them
/// (`globalOptions`, namespace `options`).
fn cs_params(
    node: &mut CommandNode,
    params: &Value,
    path: &str,
    positionals: bool,
    w: &mut Vec<String>,
) {
    let Some(list) = params.as_array() else {
        w.push(format!(
            "cli-schema {path}: parameter list is not an array — dropped"
        ));
        return;
    };
    for p in list {
        let Some(p) = p.as_object() else {
            w.push(format!("cli-schema {path}: non-object parameter dropped"));
            continue;
        };
        match cs_param(p, path, w) {
            Some(CsParam::Option(key, opt)) => {
                insert_option(node, &key, opt, "cli-schema", path, w)
            }
            Some(CsParam::Positional(pos, variadic)) => {
                if !positionals {
                    w.push(format!(
                        "cli-schema {path}: positional `{}` is not valid here — dropped",
                        pos.name
                    ));
                } else if variadic {
                    if node.rest.is_some() {
                        w.push(format!(
                            "cli-schema {path}: second variadic positional `{}` dropped",
                            pos.name
                        ));
                    } else {
                        node.rest = Some(pos);
                    }
                } else {
                    push_positional(node, pos, "cli-schema", path, w);
                }
            }
            None => {}
        }
    }
}

fn cs_param(p: &Map<String, Value>, path: &str, w: &mut Vec<String>) -> Option<CsParam> {
    let Some(name) = p
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    else {
        w.push(format!(
            "cli-schema {path}: parameter with no `name` dropped"
        ));
        return None;
    };
    warn_unsupported(
        w,
        "cli-schema",
        path,
        &format!("parameter `{name}`"),
        p,
        CS_PARAM_KEYS,
    );
    let role = p.get("role").and_then(Value::as_str).unwrap_or("flag");
    let ty = p.get("type").and_then(Value::as_str);
    let repeatable = truthy(p.get("repeatable"));

    let mut anns = Vec::new();
    if truthy(p.get("required")) {
        anns.push("required".to_string());
    }
    if let Some(s) = p
        .get("shortName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        anns.push(format!("alias: -{s}"));
    }
    let aliases = strings(p.get("aliases"));
    if !aliases.is_empty() {
        anns.push(format!(
            "aliases: {}",
            aliases
                .iter()
                .map(|a| format!("--{a}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(d) = p.get("defaultValue").and_then(Value::as_str) {
        anns.push(format!("default: {d}"));
    }
    let values = strings(p.get("enumValues"));
    if !values.is_empty() {
        anns.push(format!("one of: {}", values.join(", ")));
    }
    if let Some(s) = p.get("separator").and_then(Value::as_str) {
        anns.push(format!("separator: {s}"));
    }
    if let Some(t) = p.get("elementType").and_then(Value::as_str) {
        anns.push(format!("elements: {t}"));
    }
    if truthy(p.get("hidden")) {
        anns.push("hidden".to_string());
    }
    if let Some(d) = deprecated_ann(p.get("deprecated")) {
        anns.push(d);
    }
    let constraints: Vec<String> = p
        .get("validations")
        .and_then(Value::as_array)
        .map(|vs| {
            vs.iter()
                .filter_map(|v| v.get("kind").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !constraints.is_empty() {
        anns.push(format!("constraints: {}", constraints.join(", ")));
    }
    // Repeatable booleans (`-vvv`) can't repeat in the IR — annotate.
    if repeatable && matches!(role, "flag" | "confirmationSkip" | "dryRun") && ty == Some("boolean")
    {
        anns.push("repeatable".to_string());
    }
    match role {
        "flag" => {}
        "positional" => {}
        "confirmationSkip" | "dryRun" => anns.push(format!(
            "role: {}",
            if role == "dryRun" { "dry-run" } else { role }
        )),
        _ => {}
    }
    let doc = join_doc(p.get("summary").and_then(Value::as_str), &anns);

    match role {
        "flag" | "confirmationSkip" | "dryRun" => {
            let kind = match ty {
                Some("boolean") => "flag",
                Some("integer") | Some("number") => "int",
                Some("array") => "strings",
                Some("string") | Some("enum") => "string",
                Some(other) => {
                    w.push(format!(
                        "cli-schema {path}: parameter `{name}` has unknown type `{other}` — mapped as `string`"
                    ));
                    "string"
                }
                None => {
                    w.push(format!(
                        "cli-schema {path}: flag `{name}` has no `type` — mapped as a boolean flag"
                    ));
                    "flag"
                }
            };
            let kind = if repeatable && kind != "flag" {
                "strings"
            } else {
                kind
            };
            Some(CsParam::Option(
                ident(name),
                OptionSpec {
                    flag: format!("--{name}"),
                    kind: kind.to_string(),
                    doc,
                },
            ))
        }
        "positional" => {
            let kind = match ty {
                Some("integer") | Some("number") => "int",
                Some("boolean") => "bool",
                _ => "string",
            };
            Some(CsParam::Positional(
                PositionalSpec {
                    name: ident(name),
                    kind: kind.to_string(),
                    doc,
                },
                truthy(p.get("variadic")),
            ))
        }
        other => {
            w.push(format!(
                "cli-schema {path}: parameter `{name}` has unknown role `{other}` — dropped"
            ));
            None
        }
    }
}

fn cs_command(
    parent: &mut CommandNode,
    c: &Map<String, Value>,
    inherited: &BTreeMap<String, OptionSpec>,
    path: &str,
    w: &mut Vec<String>,
) {
    let Some(name) = c
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    else {
        w.push(format!("cli-schema {path}: command with no name dropped"));
        return;
    };
    warn_unsupported(
        w,
        "cli-schema",
        path,
        &format!("command `{name}`"),
        c,
        CS_COMMAND_KEYS,
    );

    // `path` segments are namespace words prefixing the command name —
    // `{"name": "delete", "path": ["repo"]}` is `repo delete`.
    let mut cpath = path.to_string();
    let mut container = &mut *parent;
    match c.get("path") {
        Some(Value::Array(segs)) => {
            for s in segs {
                match s.as_str().filter(|s| !s.is_empty()) {
                    Some(seg) => {
                        cpath = format!("{cpath}.{seg}");
                        container = container.commands.entry(seg.to_string()).or_default();
                    }
                    None => w.push(format!("cli-schema {cpath}: empty `path` segment ignored")),
                }
            }
        }
        Some(_) => w.push(format!(
            "cli-schema {cpath}: command `{name}` `path` is not an array — ignored"
        )),
        None => {}
    }
    let node = container.commands.entry(name.to_string()).or_default();
    let cpath = format!("{cpath}.{name}");

    let mut anns = Vec::new();
    let aliases = strings(c.get("aliases"));
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if truthy(c.get("hidden")) {
        anns.push("hidden".to_string());
    }
    if let Some(d) = deprecated_ann(c.get("deprecated")) {
        anns.push(d);
    }
    if let Some(about) = join_doc(summary_notes(c).as_deref(), &anns) {
        node.about = Some(about);
    }
    if let Some(ps) = c.get("parameters") {
        cs_params(node, ps, &cpath, true, w);
    }
    // Global flags land last — a command's own spelling wins ties.
    for (k, v) in inherited {
        node.options.entry(k.clone()).or_insert_with(|| v.clone());
    }
}

fn cs_namespace(
    parent: &mut CommandNode,
    ns: &Map<String, Value>,
    inherited: &BTreeMap<String, OptionSpec>,
    path: &str,
    w: &mut Vec<String>,
) {
    let Some(seg) = ns
        .get("segment")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        w.push(format!(
            "cli-schema {path}: namespace with no `segment` dropped"
        ));
        return;
    };
    warn_unsupported(
        w,
        "cli-schema",
        path,
        &format!("namespace `{seg}`"),
        ns,
        CS_NAMESPACE_KEYS,
    );
    let cpath = format!("{path}.{seg}");
    let node = parent.commands.entry(seg.to_string()).or_default();
    if let Some(about) = join_doc(summary_notes(ns).as_deref(), &[]) {
        if node.about.is_none() {
            node.about = Some(about);
        }
    }
    // Flags from enclosing namespaces land on this node too.
    for (k, v) in inherited {
        node.options.entry(k.clone()).or_insert_with(|| v.clone());
    }
    // Namespace `options` are flags available on everything inside —
    // they join the inherited set for the subtree.
    let mut inherited = inherited.clone();
    if let Some(opts) = ns.get("options") {
        let mut holder = CommandNode::default();
        cs_params(&mut holder, opts, &cpath, false, w);
        for (k, v) in holder.options {
            node.options.entry(k.clone()).or_insert_with(|| v.clone());
            inherited.insert(k, v);
        }
    }
    // `defaultCommand` is the handler for the bare namespace — its
    // parameters are the namespace node's own.
    match ns.get("defaultCommand") {
        Some(Value::Object(dc)) => {
            warn_unsupported(
                w,
                "cli-schema",
                &cpath,
                "defaultCommand",
                dc,
                CS_HANDLER_KEYS,
            );
            if node.about.is_none() {
                node.about = summary_notes(dc);
            }
            if let Some(ps) = dc.get("parameters") {
                cs_params(node, ps, &cpath, true, w);
            }
        }
        Some(_) => w.push(format!(
            "cli-schema {cpath}: `defaultCommand` is not an object — dropped"
        )),
        None => {}
    }
    if let Some(cmds) = ns.get("commands") {
        match cmds.as_array() {
            Some(list) => {
                for c in list {
                    match c.as_object() {
                        Some(c) => cs_command(node, c, &inherited, &cpath, w),
                        None => w.push(format!("cli-schema {cpath}: non-object command dropped")),
                    }
                }
            }
            None => w.push(format!(
                "cli-schema {cpath}: `commands` is not an array — dropped"
            )),
        }
    }
    if let Some(nss) = ns.get("namespaces") {
        match nss.as_array() {
            Some(list) => {
                for n in list {
                    match n.as_object() {
                        Some(n) => cs_namespace(node, n, &inherited, &cpath, w),
                        None => w.push(format!("cli-schema {cpath}: non-object namespace dropped")),
                    }
                }
            }
            None => w.push(format!(
                "cli-schema {cpath}: `namespaces` is not an array — dropped"
            )),
        }
    }
}
