use super::emit_hnl::*;
use crate::map::map_spec;
use crate::spec::CliSpec;
use handl_language::Language;
use handl_plugin_sdkgen::{Emitter, SdkgenModule};
use std::sync::Arc;

fn lang() -> Arc<Language> {
    Arc::new(handl_kit_data_prelude::language_from_yaml().unwrap())
}

fn gh_module() -> SdkgenModule {
    let spec: CliSpec = serde_yaml::from_str(
        r#"
name: gh
commands:
  pr:
    commands:
      create:
        about: Create a pull request
        options:
          title: { flag: "--title", kind: string }
          draft: { flag: "--draft", kind: flag }
        positionals: [{ name: branch, kind: string }]
        rest: { name: extras, kind: string }
      list: {}
"#,
    )
    .unwrap();
    let mut m = map_spec(&spec, "host");
    m.source = Some("https://example/spec.yaml".to_string());
    m
}

/// The golden-shaped spec but with docs authored on an option, a
/// positional, and the `rest` tail — the doc-threading fixture.
fn doc_module() -> SdkgenModule {
    let spec: CliSpec = serde_yaml::from_str(
        r#"
name: gh
commands:
  pr:
    commands:
      create:
        about: Create a pull request
        options:
          title: { flag: "--title", kind: string, doc: "PR title" }
          draft: { flag: "--draft", kind: flag }
        positionals: [{ name: branch, kind: string, doc: "base branch" }]
        rest: { name: extras, kind: string, doc: "further argv words" }
      list: {}
"#,
    )
    .unwrap();
    map_spec(&spec, "host")
}

#[test]
fn port_declares_colon_style_exports() {
    let port = build_port(&gh_module(), "gh");
    assert!(port.contains("--- Generated port for gh.gh\n"), "{port}");
    assert!(port.contains("--- cli target: host\n"), "{port}");
    assert!(port.contains("--- source: https://example/spec.yaml\n"));
    assert!(port.contains("from \"result\" import { Result }\n"));
    assert!(port.contains("from \"cli\" import { Output, Spawn }\n"));
    assert!(port.contains("--- Create a pull request\n"));
    assert!(port.contains("--- cli: gh pr create\n"));
    assert!(
        port.contains("export GhPrCreate: (String, Obj, [String]) ~[1]~> Result<Output, Spawn>\n"),
        "{port}"
    );
    assert!(
        port.contains("export GhPrList: () ~[1]~> Result<Output, Spawn>\n"),
        "{port}"
    );
}

#[test]
fn adapter_binds_flags_and_calls_run() {
    let adapter = build_adapter(&gh_module(), "gh", "gh-cli-host");
    assert!(adapter.contains("--- Generated adapter for gh-cli-host/gh\n"));
    assert!(adapter.contains("from \"cli-c\" import { run, argv }\n"));
    assert!(
        adapter.contains("from \"../gh/gh\" import { GhPrCreate, GhPrList }\n"),
        "{adapter}"
    );
    // No docs authored → `doc: ""` on every entry (the lib-cli
    // contract: empty when unknown).
    assert!(
        adapter.contains(
            "gh_pr_create_flags := [{ name: \"draft\", flag: \"--draft\", kind: \"flag\", doc: \"\" }, { name: \"title\", flag: \"--title\", kind: \"string\", doc: \"\" }]"
        ),
        "{adapter}"
    );
    assert!(adapter.contains("gh_pr_list_flags := []\n"), "{adapter}");
    assert!(
        adapter.contains(
            "export gh_pr_create: GhPrCreate := (branch, opts, extras) ~> do {\n  run(argv(\"gh\", [\"pr\", \"create\"], [branch], gh_pr_create_flags, opts, extras))\n}\n"
        ),
        "{adapter}"
    );
    assert!(
        adapter.contains(
            "export gh_pr_list: GhPrList := () ~> do {\n  run(argv(\"gh\", [\"pr\", \"list\"], [], gh_pr_list_flags, null, []))\n}\n"
        ),
        "{adapter}"
    );
}

#[test]
fn adapter_flag_specs_carry_docs() {
    let adapter = build_adapter(&doc_module(), "gh", "gh-cli-host");
    assert!(
        adapter.contains(
            "{ name: \"draft\", flag: \"--draft\", kind: \"flag\", doc: \"\" }, { name: \"title\", flag: \"--title\", kind: \"string\", doc: \"PR title\" }"
        ),
        "{adapter}"
    );
    // Positional/rest names stay clean — the doc component never
    // leaks into the emitted arg names.
    assert!(
        adapter.contains("run(argv(\"gh\", [\"pr\", \"create\"], [branch]"),
        "{adapter}"
    );
    assert!(
        adapter.contains(", gh_pr_create_flags, opts, extras))"),
        "{adapter}"
    );
}

#[test]
fn doc_literals_take_the_first_line_and_escape_quotes_and_backslashes() {
    let spec: CliSpec = serde_yaml::from_str(
        r#"
name: t
options:
  q: { flag: "--q", kind: string, doc: "He said \"hi\"\nsecond line" }
  b: { flag: "--b", kind: string, doc: "a\\b path" }
  colon: { flag: "--colon", kind: string, doc: "filter by state: open, closed" }
"#,
    )
    .unwrap();
    let adapter = build_adapter(&map_spec(&spec, "host"), "t", "t-cli-host");
    // First line only; `"` escaped.
    assert!(adapter.contains("doc: \"He said \\\"hi\\\"\""), "{adapter}");
    // Backslash escaped so the literal still parses.
    assert!(adapter.contains("doc: \"a\\\\b path\""), "{adapter}");
    // Colons survive verbatim — the doc is the descriptor's last
    // component, taken whole.
    assert!(
        adapter.contains("doc: \"filter by state: open, closed\""),
        "{adapter}"
    );
    // The emitted adapter still canonicalizes — every doc literal
    // parses as handl source.
    let src = canonicalize(&lang(), adapter.clone());
    // Canonical fmt adds exactly one trailing newline.
    assert_eq!(
        src.trim_end(),
        adapter.trim_end(),
        "doc escaping broke canonical fmt"
    );
}

#[test]
fn port_appends_opts_and_args_doc_tables() {
    let port = build_port(&doc_module(), "gh");
    // The opts table: one row per doc'd option (draft has none), then
    // the args table for positional/rest docs, then the cli line.
    assert!(
        port.contains(
            "--- Create a pull request\n--- opts:\n---   title (string): PR title\n--- args:\n---   branch: base branch\n---   extras: further argv words\n--- cli: gh pr create\n"
        ),
        "{port}"
    );
    // The doc-less list command grows no tables — and it has no
    // `about`, so its comment is just the cli provenance line.
    assert!(
        port.contains("--- cli: gh pr list\nexport GhPrList"),
        "{port}"
    );
}

#[test]
fn port_omits_the_opts_table_when_no_option_has_a_doc() {
    // `gh_pr_create` has an `opts` arg but no option docs → no
    // `--- opts:` header at all.
    let port = build_port(&gh_module(), "gh");
    assert!(!port.contains("--- opts:"), "{port}");
    assert!(!port.contains("--- args:"), "{port}");
    assert!(port.contains("--- cli: gh pr create\n"), "{port}");
}

#[test]
fn emitted_source_canonicalizes() {
    let module = gh_module();
    let port = canonicalize(&lang(), build_port(&module, "gh"));
    // The formatter normalizes — `Result<Output, Spawn>` spacing.
    assert!(port.contains("export GhPrCreate"), "{port}");
    // Canonical output parses again.
    let again = canonicalize(&lang(), port.clone());
    assert_eq!(port, again);
}

#[test]
fn unparseable_source_falls_back_to_raw() {
    // A backslash has no lexer rule — the canonical fmt fails and the
    // raw text is returned instead.
    let raw = "a \\ b".to_string();
    assert_eq!(canonicalize(&lang(), raw.clone()), raw);
}

#[test]
fn target_slug_extracts_os_and_arch() {
    assert_eq!(target_slug("x86_64-unknown-linux"), "linux-x86_64");
    assert_eq!(target_slug("aarch64-apple-darwin"), "darwin-aarch64");
    assert_eq!(target_slug("host"), "host");
    assert_eq!(target_slug("wasm32-wasi"), "wasi-wasm32");
}

#[test]
fn keyword_arg_names_get_an_underscore() {
    assert_eq!(safe_ident("type"), "type_");
    assert_eq!(safe_ident("branch"), "branch");
}

#[test]
fn emit_writes_port_and_adapter_files() {
    let module = gh_module();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("gh");
    HnlCliEmitter::new(lang()).emit(&module, &out).unwrap();
    let port = std::fs::read_to_string(out.join("gh.hnl")).unwrap();
    let adapter = std::fs::read_to_string(tmp.path().join("gh-cli-host").join("gh.hnl")).unwrap();
    assert!(port.contains("export GhPrCreate:"), "{port}");
    assert!(adapter.contains("run(argv("), "{adapter}");
}

#[test]
fn emit_surfaces_dir_and_write_failures() {
    let module = gh_module();
    let tmp = tempfile::tempdir().unwrap();
    // output inside an existing *file* → create_dir_all fails.
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let e = HnlCliEmitter::new(lang())
        .emit(&module, &blocker.join("gh"))
        .unwrap_err();
    assert!(e.to_string().contains("failed to create output dir"), "{e}");

    // adapter dir collides with a file → create_dir_all fails.
    std::fs::write(tmp.path().join("gh-cli-host"), b"x").unwrap();
    let e = HnlCliEmitter::new(lang())
        .emit(&module, &tmp.path().join("gh"))
        .unwrap_err();
    assert!(
        e.to_string().contains("failed to create adapter dir"),
        "{e}"
    );
    std::fs::remove_file(tmp.path().join("gh-cli-host")).unwrap();

    // `{module}.hnl` pre-existing as a directory → write fails.
    let out = tmp.path().join("gh2");
    std::fs::create_dir_all(out.join("gh.hnl")).unwrap();
    let e = HnlCliEmitter::new(lang()).emit(&module, &out).unwrap_err();
    assert!(e.to_string().contains("failed to write"), "{e}");

    // adapter `{module}.hnl` as a directory → second write fails.
    let out2 = tmp.path().join("gh3");
    std::fs::create_dir_all(tmp.path().join("gh3-cli-host").join("gh.hnl")).unwrap();
    let e = HnlCliEmitter::new(lang()).emit(&module, &out2).unwrap_err();
    assert!(e.to_string().contains("failed to write"), "{e}");
}

#[test]
fn empty_module_still_emits_headers() {
    let mut module = SdkgenModule::default();
    module.module = "empty".to_string();
    module.target = "host".to_string();
    let port = build_port(&module, "lib");
    assert!(port.contains("--- Generated port for lib.empty\n"));
    let adapter = build_adapter(&module, "lib", "lib-cli-host");
    // No functions → no port import line (the `cli-c` import stays).
    assert!(!adapter.contains("from \"../"));
}

#[test]
fn port_stamps_confidence_and_diagnostics() {
    let mut module = gh_module();
    module.library = "gh".to_string();
    module.diagnostics = vec![
        "confidence: low".to_string(),
        "gh pr: unparsed flag-like row: `-- weird`".to_string(),
    ];
    let port = build_port(&module, "gh");
    assert!(port.contains("--- confidence: low\n"), "{port}");
    assert!(
        port.contains("low-confidence; author a .clispec.yaml or pass --config"),
        "{port}"
    );
    assert!(
        port.contains("--- diagnostic: gh pr: unparsed flag-like row: `-- weird`\n"),
        "{port}"
    );
    // The `confidence:` marker is consumed, not re-emitted as a diagnostic.
    assert!(!port.contains("--- diagnostic: confidence:"), "{port}");
}

#[test]
fn high_confidence_stamps_without_the_warning_note() {
    let mut module = gh_module();
    module.diagnostics = vec!["confidence: high".to_string()];
    let port = build_port(&module, "gh");
    assert!(port.contains("--- confidence: high\n"), "{port}");
    assert!(!port.contains("low-confidence"), "{port}");
}

#[test]
fn adapter_marks_json_capable_commands() {
    let mut module = gh_module();
    module.functions[0]
        .arg_descriptors
        .push("meta:json-capable".to_string());
    let adapter = build_adapter(&module, "gh", "gh-cli-host");
    // Exactly one marker — it lands on `gh_pr_create`, not `gh_pr_list`,
    // and it points at the free-function composition, not a helper.
    assert_eq!(adapter.matches("--- json-capable").count(), 1, "{adapter}");
    let mark = adapter.find("--- json-capable").unwrap();
    let create = adapter.find("export gh_pr_create").unwrap();
    assert!(mark < create, "{adapter}");
    assert!(
        adapter.contains("parse(r.value.stdout)"),
        "{adapter}"
    );
    assert!(adapter.contains("lib-json"), "{adapter}");
}
