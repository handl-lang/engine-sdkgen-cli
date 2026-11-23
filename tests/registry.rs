//! Registry wiring: the `cli` target resolves backend `cli` +
//! emitter `hnl-cli` by name, and `CodegenRegistry::run` drives a
//! spec file end-to-end into port/adapter files.

use handl_language::Language;
use handl_plugin_codegen::{CodegenRegistry, CodegenRequest, CodegenTargetError};
use std::io::Write;
use std::path::{Path, PathBuf};

fn lang() -> Language {
    handl_kit_data_prelude::language_from_yaml().unwrap()
}

fn registry() -> CodegenRegistry {
    let mut reg = CodegenRegistry::new();
    handl_plugin_codegen_cli::register(&mut reg);
    reg
}

fn request<'a>(spec: &'a Path, output: &'a Path, lang: &'a Language) -> CodegenRequest<'a> {
    CodegenRequest {
        spec,
        source_url: None,
        output,
        target: None,
        include: None,
        exclude: None,
        config: None,
        emit_spec: None,
        lang,
        cache_dir: None,
    }
}

#[test]
fn registers_backend_emitters_and_target() {
    let reg = registry();
    assert!(reg.target_names().contains(&"cli"));
    let t = reg.target("cli").unwrap();
    assert_eq!(t.source, "cli");
    assert_eq!(t.emitter, "hnl-cli");
    assert!(!t.source_filters);
    assert!(t.docs.is_none());
    assert!(t.spec_input.is_none());
}

#[test]
fn run_drives_spec_file_to_emitted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let spec_path = tmp.path().join("gh.clispec.yaml");
    let mut f = std::fs::File::create(&spec_path).unwrap();
    write!(
        f,
        "name: gh\ncommands:\n  pr:\n    commands:\n      list:\n        about: List PRs\n        options:\n          state: {{ flag: \"--state\", kind: string }}\n"
    )
    .unwrap();
    drop(f);
    let output = tmp.path().join("gh");
    let lang = lang();
    let req = request(&spec_path, &output, &lang);
    registry().run("cli", &req).unwrap();

    let port = std::fs::read_to_string(output.join("gh.hnl")).unwrap();
    assert!(port.contains("export GhPrList:"), "{port}");
    // Adapters nest inside the port dir (PL-004 `ArtifactRef` layout).
    let adapter = std::fs::read_to_string(output.join("gh-cli-host").join("gh.hnl")).unwrap();
    assert!(
        adapter.contains("run(argv(\"gh\", [\"pr\", \"list\"]"),
        "{adapter}"
    );
}

#[test]
fn run_reports_backend_errors_as_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("missing.yaml");
    let output = tmp.path().join("out");
    let lang = lang();
    let req = request(&spec, &output, &lang);
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Failed(e)) => assert!(e.contains("cli spec not found"), "{e}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn include_exclude_are_rejected_for_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("x.yaml");
    std::fs::write(&spec, "name: x\n").unwrap();
    let output = tmp.path().join("out");
    let lang = lang();
    let mut req = request(&spec, &output, &lang);
    req.include = Some("*");
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Usage(e)) => assert!(e.contains("--include"), "{e}"),
        other => panic!("expected Usage, got {other:?}"),
    }
}

#[test]
fn config_overlay_loads_through_make() {
    // `--config` is read inside the backend's `make`; a bad config path
    // surfaces as a Failed at make time.
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("gh.yaml");
    std::fs::write(&spec, "name: gh\ncommands: {}\n").unwrap();
    let output = tmp.path().join("out");
    let lang = lang();
    let mut req = request(&spec, &output, &lang);
    req.config = Some(Path::new("/no/such/config.yaml"));
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Failed(e)) => {
            assert!(e.contains("failed to read config"), "{e}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn target_default_is_host() {
    // `req.target` unset → module.target "host" → adapter pkg `-cli-host`.
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("t.yaml");
    std::fs::write(&spec, "name: t\n").unwrap();
    let output = tmp.path().join("t");
    let lang = lang();
    let req = request(&spec, &output, &lang);
    registry().run("cli", &req).unwrap();
    assert!(output.join("t-cli-host").join("t.hnl").exists());
}

#[test]
fn custom_target_slug_names_the_adapter_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("t.yaml");
    std::fs::write(&spec, "name: t\n").unwrap();
    let output = tmp.path().join("t");
    let lang = lang();
    let mut req = request(&spec, &output, &lang);
    req.target = Some("x86_64-unknown-linux");
    registry().run("cli", &req).unwrap();
    assert!(output.join("t-cli-linux-x86_64").join("t.hnl").exists());
}

#[test]
fn missing_yaml_spec_is_an_error_not_a_probe() {
    // `x.yaml` that doesn't exist fails cleanly rather than probing a
    // nonsense tool name.
    let tmp = tempfile::tempdir().unwrap();
    let spec = PathBuf::from("definitely-missing-spec.yaml");
    let output = tmp.path().join("out");
    let lang = lang();
    let req = request(&spec, &output, &lang);
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Failed(e)) => assert!(e.contains("cli spec not found"), "{e}"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn separator_path_that_does_not_exist_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("no").join("such").join("tool");
    let output = tmp.path().join("out");
    let lang = lang();
    let req = request(&spec, &output, &lang);
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Failed(e)) => {
            assert!(e.contains("tool or spec not found"), "{e}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn directory_input_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let output = tmp.path().join("out");
    let lang = lang();
    let req = request(tmp.path(), &output, &lang);
    match registry().run("cli", &req) {
        Err(CodegenTargetError::Failed(e)) => {
            assert!(e.contains("not a spec or tool"), "{e}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn emit_spec_flows_through_request_config() {
    // `--config tool.sdkgen.yaml` with `emit_spec:` dumps the resolved
    // tree — the "probe then hand-tune" entry point.
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("t.yaml");
    std::fs::write(&spec, "name: t\ncommands:\n  a: {}\n").unwrap();
    let emitted = tmp.path().join("t.clispec.yaml");
    let cfg = tmp.path().join("t.sdkgen.yaml");
    std::fs::write(&cfg, format!("emit_spec: {}\n", emitted.display())).unwrap();
    let output = tmp.path().join("out");
    let lang = lang();
    let mut req = request(&spec, &output, &lang);
    req.config = Some(&cfg);
    registry().run("cli", &req).unwrap();
    let text = std::fs::read_to_string(&emitted).unwrap();
    assert!(text.contains("name: t"), "{text}");
    assert!(text.contains("a:"), "{text}");
}

#[test]
fn emit_spec_flag_flows_through_request_and_wins_over_config() {
    // `req.emit_spec` is the `--emit-spec` argv flag: it dumps the
    // resolved tree with no config file at all, and beats the
    // config-file `emit_spec:` key when both are set.
    let tmp = tempfile::tempdir().unwrap();
    let spec = tmp.path().join("t.yaml");
    std::fs::write(&spec, "name: t\ncommands:\n  a: {}\n").unwrap();
    let emitted = tmp.path().join("t.clispec.yaml");
    let output = tmp.path().join("out");
    let lang = lang();
    let mut req = request(&spec, &output, &lang);
    req.emit_spec = Some(emitted.as_path());
    registry().run("cli", &req).unwrap();
    let text = std::fs::read_to_string(&emitted).unwrap();
    assert!(text.contains("name: t"), "{text}");

    let cfg = tmp.path().join("t.sdkgen.yaml");
    let cfg_dest = tmp.path().join("cfg.clispec.yaml");
    std::fs::write(&cfg, format!("emit_spec: {}\n", cfg_dest.display())).unwrap();
    let flag_dest = tmp.path().join("flag.clispec.yaml");
    let mut req = request(&spec, &output, &lang);
    req.config = Some(cfg.as_path());
    req.emit_spec = Some(flag_dest.as_path());
    registry().run("cli", &req).unwrap();
    assert!(flag_dest.exists());
    assert!(
        !cfg_dest.exists(),
        "config emit_spec should lose to the flag"
    );
}
