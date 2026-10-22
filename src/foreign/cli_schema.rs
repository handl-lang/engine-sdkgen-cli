//! The [cli-schema] v1 agent-facing document (`*.cli-schema.json`)
//! lowered into [`CliSpec`]: `schemaVersion`, `commands` /
//! `namespaces`, `parameters` with `role` / `type`.
//!
//! Constructs with no IR analog but useful meaning — `intent` /
//! `environment` declarations — degrade into bracketed doc
//! annotations (`[intent: destructive, scope: global]`, `[env:
//! GITHUB_TOKEN (required)]`); anything else unrecognized is dropped
//! with a warning naming it.
//!
//! [cli-schema]: https://cli-schema.github.io

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::{ident, insert_option, join_doc, push_positional, strings, truthy, warn_unsupported};
use crate::spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec};

/// `version`/`schemaVersion` count as consumed (read, just unused by
/// the IR); `environment`/`intent` degrade to doc annotations rather
/// than IR fields; everything else unlisted — `authCommands`,
/// `reservedMetaCommands`, `output` — is reported as dropped.
const CS_ROOT_KEYS: &[&str] = &[
    "schemaVersion",
    "cli-schema-version",
    "name",
    "version",
    "description",
    "environment",
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
    "intent",
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

pub(crate) fn parse_cli_schema(src: &str) -> Result<(CliSpec, Vec<String>), String> {
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
    // The root `environment` object declares env vars and config
    // files the program needs — doc annotations on the module, not IR.
    let env_anns = cs_environment_anns(obj.get("environment"));
    if !env_anns.is_empty() {
        spec.about = join_doc(spec.about.as_deref(), &env_anns);
    }

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

/// A command's `intent` object — the side-effect profile
/// (`destructive`, `idempotent`, `scope`, `requiresConfirmation`,
/// `requiresAuth`) — flattens to one `intent: …` doc annotation. True
/// booleans become bare words (`requiresConfirmation` → `requires
/// confirmation`), non-empty strings render `key: value`.
fn cs_intent_ann(v: Option<&Value>) -> Option<String> {
    let o = v?.as_object()?;
    let mut parts = Vec::new();
    for (k, val) in o {
        let label = match k.as_str() {
            "requiresConfirmation" => "requires confirmation",
            "requiresAuth" => "requires auth",
            other => other,
        };
        match val {
            Value::Bool(true) => parts.push(label.to_string()),
            Value::String(s) if !s.is_empty() => parts.push(format!("{label}: {s}")),
            _ => {}
        }
    }
    (!parts.is_empty()).then(|| format!("intent: {}", parts.join(", ")))
}

/// The root `environment` object — `variables` (env vars) and
/// `configFiles` — flattens to `env: A, B (required)` / `config:
/// path` annotations on the module doc.
fn cs_environment_anns(v: Option<&Value>) -> Vec<String> {
    let mut anns = Vec::new();
    let Some(o) = v.and_then(Value::as_object) else {
        return anns;
    };
    let vars: Vec<String> = o
        .get("variables")
        .and_then(Value::as_array)
        .map(|vs| {
            vs.iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str)?.to_string();
                    Some(if truthy(v.get("required")) {
                        format!("{name} (required)")
                    } else {
                        name
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if !vars.is_empty() {
        anns.push(format!("env: {}", vars.join(", ")));
    }
    let files: Vec<String> = o
        .get("configFiles")
        .and_then(Value::as_array)
        .map(|fs| {
            fs.iter()
                .filter_map(|f| {
                    let path = f.get("path").and_then(Value::as_str)?.to_string();
                    Some(if truthy(f.get("required")) {
                        format!("{path} (required)")
                    } else {
                        path
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if !files.is_empty() {
        anns.push(format!("config: {}", files.join(", ")));
    }
    anns
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
    if let Some(i) = cs_intent_ann(c.get("intent")) {
        anns.push(i);
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
