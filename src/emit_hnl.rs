//! `hnl-cli` emitter: `<module>.hnl` port + adapter pair for CLI specs.
//!
//! The port declares each leaf command as
//! `export {Port}: ({args}) ~[1]~> Result<Output, Spawn>`; the adapter
//! (in the sibling `{lib}-cli-{slug}` package) binds it to
//! `cli-c`'s runner:
//!
//! ```text
//! export gh_pr_create: GhPrCreate := (branch, opts, extras) ~> do {
//!   run(argv("gh", ["pr", "create"], [branch], gh_pr_create_flags, opts, extras))
//! }
//! ```
//!
//! `cli-c` contract (the host-side package the adapter imports):
//! - `run: ([String]) ~[1]~> Result<Output, Spawn>`
//! - `argv: (String, [String], [String], [FlagSpec], Obj, [String])
//!   ~> [String]` — `(prog, subpath, positionals, flag specs, opts
//!   record-or-null, rest)`. `cli-c` stringifies scalars and expands
//!   `opts` fields per each spec's `kind`.

use handl_language::Language;
use handl_plugin_sdkgen::{Emitter, SdkgenError, SdkgenModule};
use std::path::Path;
use std::sync::Arc;

/// Emitter that writes CLI port/adapter `.hnl` files to disk.
///
/// The [`Language`] used to canonicalize emitted source is injected at
/// construction — same 0.8 seam as `sdkgen-file`'s [`FileEmitter`].
pub struct HnlCliEmitter {
    lang: Arc<Language>,
}

impl HnlCliEmitter {
    pub fn new(lang: Arc<Language>) -> Self {
        Self { lang }
    }
}

impl Emitter for HnlCliEmitter {
    fn emit(&self, module: &SdkgenModule, output: &Path) -> Result<(), SdkgenError> {
        std::fs::create_dir_all(output)
            .map_err(|e| SdkgenError(format!("failed to create output dir: {e}")))?;

        let lib = output
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("lib")
            .to_string();
        let module_name = &module.module;
        let adapter_pkg = format!("{lib}-cli-{}", target_slug(&module.target));
        let adapter_dir = output.parent().unwrap_or(output).join(&adapter_pkg);
        std::fs::create_dir_all(&adapter_dir)
            .map_err(|e| SdkgenError(format!("failed to create adapter dir: {e}")))?;

        let port_path = output.join(format!("{module_name}.hnl"));
        let adapter_path = adapter_dir.join(format!("{module_name}.hnl"));
        std::fs::write(
            &port_path,
            canonicalize(&self.lang, build_port(module, &lib)),
        )
        .map_err(|e| SdkgenError(format!("failed to write {port_path:?}: {e}")))?;
        std::fs::write(
            &adapter_path,
            canonicalize(&self.lang, build_adapter(module, &lib, &adapter_pkg)),
        )
        .map_err(|e| SdkgenError(format!("failed to write {adapter_path:?}: {e}")))?;
        Ok(())
    }
}

/// Run emitted source through the canonical formatter so generated
/// files satisfy `hnl fmt --check`; on a format error the raw text is
/// still written — an unformatted artifact beats none.
pub(crate) fn canonicalize(lang: &Language, src: String) -> String {
    match handl_plugin_ext_fmt::format_source_with_language(lang, &src) {
        Ok(formatted) => formatted,
        Err(e) => {
            eprintln!("sdkgen: emitted source failed canonical fmt ({e}); writing unformatted");
            src
        }
    }
}

/// `x86_64-unknown-linux` → `linux-x86_64`; single-word targets
/// (`host`, `browser`) stay verbatim. Mirrors `sdkgen-file`.
pub(crate) fn target_slug(target: &str) -> String {
    let parts: Vec<&str> = target.split('-').collect();
    if parts.len() == 1 {
        return target.to_string();
    }
    let arch = parts.first().copied().unwrap_or("unknown");
    let os = parts
        .get(2)
        .copied()
        .or(parts.get(1).copied())
        .unwrap_or("unknown");
    format!("{os}-{arch}")
}

pub(crate) fn build_port(module: &SdkgenModule, lib: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("--- Generated port for {lib}.{}\n", module.module));
    out.push_str(&format!("--- cli target: {}\n", module.target));
    if let Some(source) = &module.source {
        out.push_str(&format!("--- source: {source}\n"));
    }
    out.push('\n');
    out.push_str("from \"result\" import { Result }\n");
    // Backend-declared imports — `cli` carries `Output`/`Spawn`.
    for imp in &module.imports {
        out.push_str(&format!(
            "from \"{}\" import {{ {} }}\n",
            imp.package,
            imp.names.join(", ")
        ));
    }
    out.push('\n');

    for f in &module.functions {
        if let Some(d) = &f.doc {
            for line in d.lines() {
                out.push_str(&format!("--- {}\n", line.trim()));
            }
        }
        out.push_str(&format!("--- cli: {}\n", f.c_decl));
        let arg_list = f
            .port_args
            .iter()
            .map(|a| a.port.clone())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "export {}: ({}) ~[1]~> Result<{}, Spawn>\n\n",
            f.port_name, arg_list, f.return_port
        ));
    }
    out
}

pub(crate) fn build_adapter(module: &SdkgenModule, lib: &str, adapter_pkg: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "--- Generated adapter for {adapter_pkg}/{}\n",
        module.module
    ));
    if let Some(source) = &module.source {
        out.push_str(&format!("--- source: {source}\n"));
    }
    out.push('\n');
    out.push_str("from \"cli-c\" import { run, argv }\n");
    let ports: Vec<String> = module
        .functions
        .iter()
        .map(|f| f.port_name.clone())
        .collect();
    if !ports.is_empty() {
        out.push_str(&format!(
            "from \"../{lib}/{}\" import {{ {} }}\n",
            module.module,
            ports.join(", ")
        ));
    }
    out.push('\n');

    for f in &module.functions {
        // `opt:kind:name:flag` descriptors → the per-command FlagSpec
        // table `cli-c.argv` expands `opts` against.
        let specs: Vec<String> = f
            .arg_descriptors
            .iter()
            .filter_map(|d| d.strip_prefix("opt:"))
            .filter_map(|rest| {
                let (kind, rest) = rest.split_once(':')?;
                let (name, flag) = rest.rsplit_once(':')?;
                Some(format!(
                    "{{ name: \"{name}\", flag: \"{flag}\", kind: \"{kind}\" }}"
                ))
            })
            .collect();
        let flags_binding = format!("{}_flags", f.name);
        out.push_str(&format!("{flags_binding} := [{}]\n\n", specs.join(", ")));

        // argv(prog, subpath, positionals, flag_specs, opts, rest) —
        // subpath is the c_decl minus the program word.
        let subpath: Vec<String> = f
            .c_decl
            .split_whitespace()
            .skip(1)
            .map(|s| format!("\"{s}\""))
            .collect();
        let prog = f.c_decl.split_whitespace().next().unwrap_or(&f.name);

        let positional_names: Vec<String> = f
            .arg_descriptors
            .iter()
            .filter_map(|d| d.strip_prefix("pos:"))
            .map(safe_ident)
            .collect();
        let positionals = format!("[{}]", positional_names.join(", "));
        let rest_arg = f
            .arg_descriptors
            .iter()
            .find_map(|d| d.strip_prefix("rest:"))
            .map(safe_ident)
            .unwrap_or_else(|| "[]".to_string());
        let opts_arg = if f.port_args.iter().any(|a| a.name == "opts") {
            "opts".to_string()
        } else {
            "null".to_string()
        };

        let arg_names = f
            .port_args
            .iter()
            .map(|a| safe_ident(&a.name))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "export {}: {} := ({}) ~> do {{\n",
            f.name, f.port_name, arg_names
        ));
        out.push_str(&format!(
            "  run(argv(\"{prog}\", [{}], {positionals}, {flags_binding}, {opts_arg}, {rest_arg}))\n",
            subpath.join(", ")
        ));
        out.push_str("}\n\n");
    }
    out
}

/// Canonical Handl keywords (mirrors `sdkgen-file`/`lexer_rules.yaml`):
/// spec argument names that collide (`data`, `type`, `for`, …) get a
/// `_` suffix so `(params) ~> do` still parses.
const HANDL_KEYWORDS: &[&str] = &[
    "absurd", "app", "arr", "as", "axiom", "data", "do", "else", "export", "first", "for", "from",
    "grade", "grades", "handle", "if", "id2", "impl", "import", "in", "infer", "law", "left",
    "let", "match", "op", "rec", "return", "rewrite", "right", "second", "sort", "then", "trait",
    "type", "where", "while", "with",
];

pub(crate) fn safe_ident(name: &str) -> String {
    if HANDL_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}
