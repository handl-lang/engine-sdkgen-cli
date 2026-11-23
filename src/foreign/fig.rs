//! The [fig] completion spec — serialized [withfig/autocomplete][fig]
//! documents (`*.fig.json`, `*.figspec`) lowered into [`CliSpec`]:
//! the `subcommands` / `options` / `args` tree.
//!
//! Suggestion `generators` cannot run in the IR and degrade to
//! `[generator: filepaths]`-style doc annotations; anything else
//! unrecognized is dropped with a warning naming it (`fig
//! git.checkout: unsupported subcommand fields dropped: loadSpec`).
//!
//! [fig]: https://github.com/withfig/autocomplete

use serde_json::{Map, Value};

use super::{ident, insert_option, join_doc, push_positional, strings, truthy, warn_unsupported};
use crate::spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec};

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
const FIG_ARG_KEYS: &[&str] = &[
    "name",
    "description",
    "isVariadic",
    "isOptional",
    "default",
    "generators",
];

pub(crate) fn parse_fig(src: &str) -> Result<(CliSpec, Vec<String>), String> {
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
    if let Some(first) = arg_objs.first()
        && let Some(n) = first
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    {
        anns.push(format!("value: {n}"));
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
    if let Some(first) = arg_objs.first()
        && let Some(a) = fig_generators_ann(first.get("generators"))
    {
        anns.push(a);
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
        if let Some(g) = fig_generators_ann(a.get("generators")) {
            anns.push(g);
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

/// A fig `generators` entry — object or array of suggestion sources
/// (`{"template": "filepaths"}`, `{"script": "ls"}`, `{"custom":
/// "fn"}`). The IR cannot run completion generators, so they degrade
/// to a `generator: <what>` / `generators: <a>, <b>` doc annotation.
fn fig_generators_ann(v: Option<&Value>) -> Option<String> {
    let list: Vec<&Value> = match v? {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    if list.is_empty() {
        return None;
    }
    let descs: Vec<String> = list.iter().map(|g| fig_generator_desc(g)).collect();
    let key = if descs.len() == 1 {
        "generator"
    } else {
        "generators"
    };
    Some(format!("{key}: {}", descs.join(", ")))
}

/// One generator's one-line descriptor: its `template` name(s),
/// `script <cmd>`, `custom <fn>` — or a bare `custom` when the shape
/// is unrecognized.
fn fig_generator_desc(g: &Value) -> String {
    let Some(o) = g.as_object() else {
        return g.as_str().unwrap_or("custom").to_string();
    };
    if let Some(t) = o.get("template") {
        let names = strings(Some(t));
        if names.is_empty() {
            "template".to_string()
        } else {
            names.join("+")
        }
    } else if let Some(s) = o.get("script") {
        let cmd = strings(Some(s)).join(" ");
        if cmd.is_empty() {
            "script".to_string()
        } else {
            format!("script `{cmd}`")
        }
    } else if let Some(c) = o.get("custom").and_then(Value::as_str) {
        format!("custom {c}")
    } else {
        "custom".to_string()
    }
}
