//! Lowering a [`CliSpec`] command tree into [`SdkgenModule`] IR.
//!
//! One [`Function`] per *leaf* command — a node with `commands` is a
//! branch (`gh pr`) and produces no function of its own. A tool with no
//! subcommands is a root leaf and emits a single `{tool}` function.
//!
//! ## The `arg_descriptors` contract
//!
//! Emitters reconstruct argv shape from descriptors alone:
//! - `pos:<name>` — a positional, in argv order.
//! - `opt:<kind>:<name>:<flag>` — a named option (`kind` ∈
//!   `flag|string|int|strings`, `flag` the literal spelling).
//! - `rest:<name>` — the variadic tail.
//!
//! `port_args` aligns: one arg per positional (String/Int/Bool), a
//! trailing `opts: Obj` iff the command has options, and `rest: [T]`
//! last when variadic. `return_port` is `Output` / descriptor `cli`.

use handl_plugin_sdkgen::{Arg, Function, ModuleImport, SdkgenModule};
use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::spec::{CliSpec, CommandNode};

/// The port type for a positional/`rest`-element kind.
fn positional_port(kind: &str) -> &'static str {
    match kind {
        "int" => "Int",
        "bool" => "Bool",
        _ => "String",
    }
}

/// Lower `spec` into a module for `target`.
pub fn map_spec(spec: &CliSpec, target: &str) -> SdkgenModule {
    let mut module = SdkgenModule {
        client_package: "cli".to_string(),
        backend_id: "cli".to_string(),
        target: target.to_string(),
        library: spec.name.clone(),
        module: spec.name.to_snake_case(),
        imports: vec![ModuleImport {
            package: "cli".to_string(),
            names: vec!["Output".to_string(), "Spawn".to_string()],
        }],
        ..Default::default()
    };
    let words = vec![spec.name.clone()];
    walk(&spec.root, &words, &mut module);
    module
}

/// Emit a `Function` for each leaf under `node`; `words` is the argv
/// path so far (`["gh", "pr", "create"]`).
fn walk(node: &CommandNode, words: &[String], module: &mut SdkgenModule) {
    if node.commands.is_empty() {
        module.functions.push(leaf_function(node, words));
        return;
    }
    for (name, sub) in &node.commands {
        let mut w = words.to_vec();
        w.push(name.clone());
        walk(sub, &w, module);
    }
}

fn leaf_function(node: &CommandNode, words: &[String]) -> Function {
    let base = words.join("_");
    let mut port_args = Vec::new();
    let mut descriptors = Vec::new();

    for pos in &node.positionals {
        port_args.push(Arg {
            name: pos.name.clone(),
            port: positional_port(&pos.kind).to_string(),
        });
        descriptors.push(format!("pos:{}", pos.name));
    }
    if !node.options.is_empty() {
        port_args.push(Arg {
            name: "opts".to_string(),
            port: "Obj".to_string(),
        });
        for (name, opt) in &node.options {
            descriptors.push(format!("opt:{}:{}:{}", opt.kind, name, opt.flag));
        }
    }
    if let Some(rest) = &node.rest {
        port_args.push(Arg {
            name: rest.name.clone(),
            port: format!("[{}]", positional_port(&rest.kind)),
        });
        descriptors.push(format!("rest:{}", rest.name));
    }

    Function {
        name: base.to_snake_case(),
        port_name: base.to_upper_camel_case(),
        port_args,
        arg_descriptors: descriptors,
        return_port: "Output".to_string(),
        ret_descriptor: "cli".to_string(),
        out_struct: None,
        variadic: node.rest.is_some(),
        errno_guard: false,
        // Provenance: the literal invocation, `gh pr create`.
        c_decl: words.join(" "),
        doc: node.about.clone(),
    }
}
