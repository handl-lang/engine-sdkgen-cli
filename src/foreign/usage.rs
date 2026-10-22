//! The [usage] spec — KDL documents from the jdx/usage ecosystem
//! (`*.usage.kdl`, `*.kdl`; mise ships one) lowered into [`CliSpec`].
//!
//! The shape in the wild: top-level `name`/`bin`/`about`/`version`
//! metadata, then `flag "-f --force" help="…"`, `arg "<file>"`, and
//! `cmd "name" { … }` trees carrying `alias` children, `choices`,
//! `flagset`/`use` reuse, and `global=#true` flag inheritance.
//!
//! What does not map — `complete`/`mount`/`exec` runtime directives,
//! help-layout machinery (`help_template`, `display_order`, headings),
//! `output`/`exit_code`/`config`/`group`/`clause`, and any unknown
//! node or prop — is reported as one warning per scope.
//!
//! [usage]: https://usage.jdx.dev

use super::kdl::{self, KdlNode, KdlVal};
use super::{ident, insert_option, join_doc, push_positional};
use crate::spec::{CliSpec, CommandNode, OptionSpec, PositionalSpec};
use std::collections::{BTreeMap, BTreeSet};

/// Top-level nodes the metadata pass consumes — not warned about.
const ROOT_META: &[&str] = &[
    "name",
    "bin",
    "about",
    "about_md",
    "long_about",
    "version",
    "min_usage_version",
    "usage",
    "alias",
    "flagset",
];
/// `cmd` children consumed by [`usage_cmd`] itself (doc + aliases).
const CMD_META: &[&str] = &["alias", "long_help", "help_md"];
/// Flag/option props the mapping consumes.
const FLAG_PROPS: &[&str] = &[
    "help",
    "help_md",
    "long_help",
    "required",
    "default",
    "env",
    "var",
    "var_min",
    "var_max",
    "count",
    "global",
    "hide",
    "deprecated",
    "negate",
    "conflicts",
    "overrides",
    "requires",
    "required_if",
    "required_unless",
    "exclusive",
    "delimiter",
    "action",
    "value_optional",
    "validate",
    "validate_error",
];
/// Flag/option children the mapping consumes.
const FLAG_CHILDREN: &[&str] = &[
    "alias",
    "arg",
    "choices",
    "conflicts",
    "overrides",
    "requires",
    "note",
    "warning",
];
/// Positional `arg` props the mapping consumes.
const ARG_PROPS: &[&str] = &[
    "help",
    "help_md",
    "long_help",
    "required",
    "default",
    "env",
    "var",
    "var_min",
    "var_max",
    "hide",
    "deprecated",
    "sigil",
    "validate",
    "validate_error",
];
/// Positional `arg` children the mapping consumes.
const ARG_CHILDREN: &[&str] = &["choices", "note", "warning"];
/// `cmd` props the mapping consumes.
const CMD_PROPS: &[&str] = &[
    "help",
    "help_md",
    "long_help",
    "hide",
    "deprecated",
    "subcommand_required",
];

/// Whether `src` parses as KDL carrying usage markers — a top-level
/// `cmd`/`flag`/`option`/`arg`/`usage` node.
pub(crate) fn usage_shaped(src: &str) -> bool {
    let Ok(doc) = kdl::parse(src) else {
        return false;
    };
    doc.iter()
        .any(|n| matches!(n.name.as_str(), "cmd" | "flag" | "option" | "arg" | "usage"))
}

/// Parse a usage spec into a [`CliSpec`] plus drop warnings.
pub(crate) fn parse_usage(src: &str) -> Result<(CliSpec, Vec<String>), String> {
    let doc = kdl::parse(src).map_err(|e| format!("failed to parse usage spec: {e}"))?;
    let mut w = Vec::new();
    // `bin` is the argv[0] name; `name` is the friendly fallback.
    let tool = top_arg(&doc, "bin")
        .filter(|t| !t.is_empty())
        .or_else(|| top_arg(&doc, "name"));
    let Some(tool) = tool.filter(|t| !t.is_empty()) else {
        return Err("usage spec is missing `bin`/`name`".to_string());
    };
    let mut spec = CliSpec {
        name: tool.to_string(),
        ..Default::default()
    };
    let mut anns = Vec::new();
    let aliases: Vec<&str> = doc
        .iter()
        .filter(|n| n.name == "alias")
        .flat_map(|n| n.args.iter().filter_map(KdlVal::as_str))
        .collect();
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if let Some(synopsis) = top_arg(&doc, "usage") {
        anns.push(format!("usage: {synopsis}"));
    }
    let short = top_arg(&doc, "about").or_else(|| top_arg(&doc, "about_md"));
    spec.about = join_doc(
        doc_text(short, top_arg(&doc, "long_about")).as_deref(),
        &anns,
    );

    // `flagset "name" { … }` bodies are expanded by `use "name"`.
    let mut flagsets: BTreeMap<String, &KdlNode> = BTreeMap::new();
    for n in doc.iter().filter(|n| n.name == "flagset") {
        match n.arg_str(0) {
            Some(name) => {
                flagsets.insert(name.to_string(), n);
            }
            None => w.push(format!("usage {tool}: flagset with no name dropped")),
        }
    }
    usage_scope(
        &doc,
        &flagsets,
        &BTreeMap::new(),
        ROOT_META,
        tool,
        &mut spec.root,
        &mut w,
    );
    Ok((spec, w))
}

/// First string entry of the first top-level node named `name`.
fn top_arg<'a>(doc: &'a [KdlNode], name: &str) -> Option<&'a str> {
    doc.iter().find(|n| n.name == name)?.arg_str(0)
}

/// Short doc + long doc as `short\n\nlong` (either may be absent).
fn doc_text(short: Option<&str>, long: Option<&str>) -> Option<String> {
    match (short, long) {
        (Some(s), Some(l)) if !l.trim().is_empty() => Some(format!("{s}\n\n{l}")),
        (Some(s), _) => Some(s.to_string()),
        (None, Some(l)) => Some(l.to_string()),
        (None, None) => None,
    }
}

/// Props of `n` the mapping does not consume → one warning naming
/// the dropped keys.
fn warn_props(w: &mut Vec<String>, path: &str, what: &str, n: &KdlNode, known: &[&str]) {
    let bad: BTreeSet<&str> = n
        .props
        .iter()
        .map(|(k, _)| k.as_str())
        .filter(|k| !known.contains(k))
        .collect();
    if !bad.is_empty() {
        w.push(format!(
            "usage {path}: {what}: unsupported props dropped: {}",
            bad.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
}

/// Children of `n` the mapping does not consume → one warning naming
/// the dropped node kinds. (Not used for `cmd` — its children are a
/// scope and warn through `usage_scope`'s unknown-node list.)
fn warn_children(w: &mut Vec<String>, path: &str, what: &str, n: &KdlNode, known: &[&str]) {
    let bad: BTreeSet<&str> = n
        .children
        .iter()
        .map(|c| c.name.as_str())
        .filter(|c| !known.contains(c))
        .collect();
    if !bad.is_empty() {
        w.push(format!(
            "usage {path}: {what}: unsupported child nodes dropped: {}",
            bad.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
}

/// One scope — a command node's children or the document root.
/// `consumed` names nodes handled by the caller (root metadata, cmd
/// doc/aliases) that should not warn.
fn usage_scope(
    nodes: &[KdlNode],
    flagsets: &BTreeMap<String, &KdlNode>,
    inherited: &BTreeMap<String, OptionSpec>,
    consumed: &[&str],
    path: &str,
    node: &mut CommandNode,
    w: &mut Vec<String>,
) {
    // Pass 1: flags/options/args/`use` expansions — `global=#true`
    // flags are collected for the subtree.
    let mut own_globals: BTreeMap<String, OptionSpec> = BTreeMap::new();
    let mut unknown = BTreeSet::new();
    for n in nodes {
        match n.name.as_str() {
            "flag" | "option" => {
                if let Some((key, spec, global)) = usage_flag(n, path, w) {
                    if global {
                        own_globals.insert(key.clone(), spec.clone());
                    }
                    insert_option(node, &key, spec, "usage", path, w);
                }
            }
            "arg" => usage_arg(n, path, node, w),
            "use" => usage_use(n, flagsets, path, node, &mut own_globals, w),
            "cmd" => {}
            other if consumed.contains(&other) => {}
            other => {
                unknown.insert(other);
            }
        }
    }
    if !unknown.is_empty() {
        w.push(format!(
            "usage {path}: unsupported nodes dropped: {}",
            unknown.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    // Descendants inherit scope globals; a same-key flag of their own
    // scope wins (`entry().or_insert` below).
    let mut merged = inherited.clone();
    for (k, v) in own_globals {
        merged.insert(k, v);
    }
    // Pass 2: commands recurse with the merged global set.
    for c in nodes.iter().filter(|n| n.name == "cmd") {
        usage_cmd(c, flagsets, &merged, path, node, w);
    }
}

/// `use "name"` — splice a top-level `flagset` body into this scope.
fn usage_use(
    n: &KdlNode,
    flagsets: &BTreeMap<String, &KdlNode>,
    path: &str,
    node: &mut CommandNode,
    own_globals: &mut BTreeMap<String, OptionSpec>,
    w: &mut Vec<String>,
) {
    let Some(name) = n.arg_str(0) else {
        w.push(format!("usage {path}: `use` with no flagset name dropped"));
        return;
    };
    let Some(fs) = flagsets.get(name) else {
        w.push(format!(
            "usage {path}: `use` names unknown flagset `{name}`"
        ));
        return;
    };
    for c in &fs.children {
        match c.name.as_str() {
            "flag" | "option" => {
                if let Some((key, spec, global)) = usage_flag(c, path, w) {
                    if global {
                        own_globals.insert(key.clone(), spec.clone());
                    }
                    insert_option(node, &key, spec, "usage", path, w);
                }
            }
            "arg" => usage_arg(c, path, node, w),
            other => w.push(format!(
                "usage {path}: flagset `{name}` node `{other}` dropped"
            )),
        }
    }
}

/// Strip a `<…>`/`[…]` wrapper and a trailing `…`/`...` off a def
/// token — `(<name>, optional, variadic)`.
fn def_token(tok: &str) -> (String, bool, bool) {
    let variadic = tok.ends_with("...") || tok.ends_with('…');
    let tok = tok.trim_end_matches("...").trim_end_matches('…');
    let (inner, optional) = if let Some(i) = tok.strip_prefix('<').and_then(|t| t.strip_suffix('>'))
    {
        (i, false)
    } else if let Some(i) = tok.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        (i, true)
    } else {
        (tok, false)
    };
    (inner.to_string(), optional, variadic)
}

/// `flag "-f --force <x>" …` → (`key`, spec, `global`).
fn usage_flag(n: &KdlNode, path: &str, w: &mut Vec<String>) -> Option<(String, OptionSpec, bool)> {
    let mut spellings: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    let mut var = n.prop("var").is_some_and(KdlVal::truthy);
    let mut optional_value = n.prop("value_optional").is_some_and(KdlVal::truthy);
    for a in &n.args {
        let Some(def) = a.as_str() else {
            w.push(format!("usage {path}: non-string flag entry dropped"));
            continue;
        };
        for tok in def.split_whitespace() {
            if tok.starts_with('-') {
                let (_, _, dots) = def_token(tok);
                var |= dots;
                spellings.push(
                    tok.trim_end_matches("...")
                        .trim_end_matches('…')
                        .to_string(),
                );
            } else {
                let (name, optional, dots) = def_token(tok);
                if name.is_empty() {
                    w.push(format!(
                        "usage {path}: unrecognized flag def token `{tok}` dropped"
                    ));
                    continue;
                }
                var |= dots;
                optional_value |= optional;
                values.push(name);
            }
        }
    }
    // `flag "--user" { arg "<user>" }` — a child arg declares the value.
    for a in n.children_named("arg") {
        if a.prop("var").is_some_and(KdlVal::truthy) {
            var = true;
        }
        for s in a.args.iter().filter_map(KdlVal::as_str) {
            for tok in s.split_whitespace() {
                let (name, optional, dots) = def_token(tok);
                var |= dots;
                optional_value |= optional;
                if !name.is_empty() {
                    values.push(name);
                }
            }
        }
    }

    // Prefer the `--long` spelling for argv emission; the rest alias.
    let flag = spellings
        .iter()
        .find(|s| s.starts_with("--"))
        .or_else(|| spellings.first())
        .cloned();
    let what = match &flag {
        Some(f) => format!("flag `{f}`"),
        None => "flag".to_string(),
    };
    warn_props(w, path, &what, n, FLAG_PROPS);
    warn_children(w, path, &what, n, FLAG_CHILDREN);
    let Some(flag) = flag else {
        w.push(format!("usage {path}: flag with no `-` spelling dropped"));
        return None;
    };
    if values.len() > 1 {
        w.push(format!(
            "usage {path}: flag `{flag}` takes {} values — only the first is kept",
            values.len()
        ));
    }

    let takes = !values.is_empty();
    let count = n.prop("count").is_some_and(KdlVal::truthy);
    let kind = match (takes, var) {
        (true, true) => "strings",
        (true, false) => "string",
        (false, _) => "flag",
    };

    let mut anns = Vec::new();
    let mut aliases: Vec<String> = spellings
        .iter()
        .filter(|s| s.as_str() != flag)
        .cloned()
        .collect();
    for a in n.children_named("alias") {
        aliases.extend(a.args.iter().filter_map(|v| v.render()));
    }
    aliases.dedup();
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if let Some(v) = values.first() {
        anns.push(format!("value: {v}"));
    }
    if optional_value {
        anns.push("optional value".to_string());
    }
    if n.prop("required").is_some_and(KdlVal::truthy) {
        anns.push("required".to_string());
    }
    for (key, label) in [("env", "env"), ("delimiter", "delimiter")] {
        if let Some(v) = n.prop(key).and_then(KdlVal::render) {
            anns.push(format!("{label}: {v}"));
        }
    }
    if let Some(d) = n.prop("default").and_then(KdlVal::render) {
        anns.push(format!("default: {d}"));
    }
    if var && !takes {
        anns.push("repeatable".to_string());
    }
    if count {
        anns.push("count".to_string());
    }
    if let Some(v) = n.prop("var_min").and_then(KdlVal::render) {
        anns.push(format!("at least {v} values"));
    }
    if let Some(v) = n.prop("var_max").and_then(KdlVal::render) {
        anns.push(format!("at most {v} values"));
    }
    if let Some(a) = n.prop("action").and_then(KdlVal::render) {
        anns.push(format!("action: {a}"));
    }
    for (key, label) in [
        ("conflicts", "conflicts with"),
        ("overrides", "overrides"),
        ("requires", "requires"),
        ("required_if", "required if"),
        ("required_unless", "required unless"),
        ("negate", "negated by"),
    ] {
        let mut members: Vec<String> = n.prop(key).and_then(KdlVal::render).into_iter().collect();
        for c in n.children_named(key) {
            members.extend(c.args.iter().filter_map(|v| v.render()));
        }
        if !members.is_empty() {
            anns.push(format!("{label}: {}", members.join(", ")));
        }
    }
    if n.prop("exclusive").is_some_and(KdlVal::truthy) {
        anns.push("exclusive".to_string());
    }
    let choices: Vec<&str> = n
        .children_named("choices")
        .flat_map(|c| c.args.iter().filter_map(KdlVal::as_str))
        .collect();
    if !choices.is_empty() {
        anns.push(format!("one of: {}", choices.join(", ")));
    }
    if n.prop("hide").is_some_and(KdlVal::truthy) {
        anns.push("hidden".to_string());
    }
    if let Some(d) = deprecated_ann(n.prop("deprecated")) {
        anns.push(d);
    }
    if let Some(expr) = n.prop("validate").and_then(KdlVal::render) {
        match n.prop("validate_error").and_then(KdlVal::render) {
            Some(msg) => anns.push(format!("validation: {expr} ({msg})")),
            None => anns.push(format!("validation: {expr}")),
        }
    }
    for (child, label) in [("note", "note"), ("warning", "warning")] {
        for c in n.children_named(child) {
            if let Some(t) = c.arg_str(0) {
                anns.push(format!("{label}: {t}"));
            }
        }
    }

    let short = prop_str(n, "help")
        .or_else(|| prop_str(n, "help_md"))
        .or_else(|| n.children_named("help_md").find_map(|c| c.arg_str(0)));
    let doc = join_doc(
        doc_text(
            short,
            prop_str(n, "long_help")
                .or_else(|| n.children_named("long_help").find_map(|c| c.arg_str(0))),
        )
        .as_deref(),
        &anns,
    );
    let global = n.prop("global").is_some_and(KdlVal::truthy);
    Some((
        ident(&flag),
        OptionSpec {
            flag,
            kind: kind.to_string(),
            doc,
        },
        global,
    ))
}

/// `arg "<file>" …` — each value token is a positional; `...`/`var`
/// makes the last one the `rest` tail.
fn usage_arg(n: &KdlNode, path: &str, node: &mut CommandNode, w: &mut Vec<String>) {
    let label = n.arg_str(0).unwrap_or("arg").to_string();
    warn_props(w, path, &format!("arg `{label}`"), n, ARG_PROPS);
    warn_children(w, path, &format!("arg `{label}`"), n, ARG_CHILDREN);
    if n.prop("sigil").is_some() {
        w.push(format!("usage {path}: sigil arg `{label}` dropped"));
        return;
    }
    let var = n.prop("var").is_some_and(KdlVal::truthy);
    let mut tokens = Vec::new();
    for a in n.args.iter().filter_map(KdlVal::as_str) {
        for tok in a.split_whitespace() {
            tokens.push(tok.to_string());
        }
    }
    if tokens.is_empty() {
        w.push(format!("usage {path}: arg with no name dropped"));
        return;
    }
    let last = tokens.len() - 1;
    let choices: Vec<&str> = n
        .children_named("choices")
        .flat_map(|c| c.args.iter().filter_map(KdlVal::as_str))
        .collect();
    let short = prop_str(n, "help")
        .or_else(|| prop_str(n, "help_md"))
        .or_else(|| n.children_named("help_md").find_map(|c| c.arg_str(0)));
    let text = doc_text(
        short,
        prop_str(n, "long_help")
            .or_else(|| n.children_named("long_help").find_map(|c| c.arg_str(0))),
    );
    for (i, tok) in tokens.iter().enumerate() {
        let (name, optional, dots) = def_token(tok);
        let name = if name.is_empty() {
            format!("arg{}", i + 1)
        } else {
            ident(&name)
        };
        let mut anns = Vec::new();
        if optional
            || n.prop("required")
                .is_some_and(|v| matches!(v, KdlVal::Bool(false)))
        {
            anns.push("optional".to_string());
        }
        if let Some(d) = n.prop("default").and_then(KdlVal::render) {
            anns.push(format!("default: {d}"));
        }
        if let Some(e) = n.prop("env").and_then(KdlVal::render) {
            anns.push(format!("env: {e}"));
        }
        if let Some(v) = n.prop("var_min").and_then(KdlVal::render) {
            anns.push(format!("at least {v} values"));
        }
        if let Some(v) = n.prop("var_max").and_then(KdlVal::render) {
            anns.push(format!("at most {v} values"));
        }
        if !choices.is_empty() {
            anns.push(format!("one of: {}", choices.join(", ")));
        }
        if n.prop("hide").is_some_and(KdlVal::truthy) {
            anns.push("hidden".to_string());
        }
        if let Some(d) = deprecated_ann(n.prop("deprecated")) {
            anns.push(d);
        }
        if let Some(expr) = n.prop("validate").and_then(KdlVal::render) {
            match n.prop("validate_error").and_then(KdlVal::render) {
                Some(msg) => anns.push(format!("validation: {expr} ({msg})")),
                None => anns.push(format!("validation: {expr}")),
            }
        }
        for (child, lbl) in [("note", "note"), ("warning", "warning")] {
            for c in n.children_named(child) {
                if let Some(t) = c.arg_str(0) {
                    anns.push(format!("{lbl}: {t}"));
                }
            }
        }
        let pos = PositionalSpec {
            name,
            kind: "string".to_string(),
            doc: join_doc(text.as_deref(), &anns),
        };
        if i == last && (dots || var) {
            if node.rest.is_some() {
                w.push(format!(
                    "usage {path}: second variadic arg `{}` dropped",
                    pos.name
                ));
            } else {
                node.rest = Some(pos);
            }
        } else {
            if dots {
                w.push(format!(
                    "usage {path}: variadic arg `{}` is not last — mapped as a positional",
                    pos.name
                ));
            }
            push_positional(node, pos, "usage", path, w);
        }
    }
}

/// `cmd "name" … { children }` → a command node.
fn usage_cmd(
    n: &KdlNode,
    flagsets: &BTreeMap<String, &KdlNode>,
    globals: &BTreeMap<String, OptionSpec>,
    path: &str,
    node: &mut CommandNode,
    w: &mut Vec<String>,
) {
    let Some(name) = n.arg_str(0).filter(|s| !s.is_empty()) else {
        w.push(format!("usage {path}: cmd with no name dropped"));
        return;
    };
    if n.args.len() > 1 {
        let extra: Vec<String> = n.args[1..].iter().filter_map(|v| v.render()).collect();
        if !extra.is_empty() {
            w.push(format!(
                "usage {path}: cmd `{name}` extra args dropped: {}",
                extra.join(", ")
            ));
        }
    }
    warn_props(w, path, &format!("cmd `{name}`"), n, CMD_PROPS);
    let cpath = format!("{path}.{name}");
    let child = node.commands.entry(name.to_string()).or_default();
    let mut anns = Vec::new();
    let aliases: Vec<String> = n
        .children_named("alias")
        .flat_map(|a| a.args.iter().filter_map(|v| v.render()))
        .collect();
    if !aliases.is_empty() {
        anns.push(format!("aliases: {}", aliases.join(", ")));
    }
    if n.prop("hide").is_some_and(KdlVal::truthy) {
        anns.push("hidden".to_string());
    }
    if n.prop("subcommand_required").is_some_and(KdlVal::truthy) {
        anns.push("subcommand required".to_string());
    }
    if let Some(d) = deprecated_ann(n.prop("deprecated")) {
        anns.push(d);
    }
    let short = prop_str(n, "help")
        .or_else(|| prop_str(n, "help_md"))
        .or_else(|| n.children_named("help_md").find_map(|c| c.arg_str(0)));
    if let Some(about) = join_doc(
        doc_text(
            short,
            prop_str(n, "long_help")
                .or_else(|| n.children_named("long_help").find_map(|c| c.arg_str(0))),
        )
        .as_deref(),
        &anns,
    ) {
        child.about = Some(about);
    }
    usage_scope(&n.children, flagsets, globals, CMD_META, &cpath, child, w);
    // Global flags land last — the command's own spelling wins ties.
    for (k, v) in globals {
        child.options.entry(k.clone()).or_insert_with(|| v.clone());
    }
}

/// A string prop value (numbers/bools do not carry docs).
fn prop_str<'a>(n: &'a KdlNode, key: &str) -> Option<&'a str> {
    n.prop(key).and_then(KdlVal::as_str)
}

/// `deprecated="…"`/`deprecated=#true` → a doc annotation.
fn deprecated_ann(v: Option<&KdlVal>) -> Option<String> {
    match v {
        Some(KdlVal::Str(m)) => Some(format!("deprecated: {m}")),
        Some(KdlVal::Bool(false)) | Some(KdlVal::Null) | None => None,
        Some(_) => Some("deprecated".to_string()),
    }
}
