//! `CliBackend::parse_header` probe paths: bare tool names, executable
//! paths, spec files carrying `probe:`, and the `max_depth` config cap.

use handl_plugin_sdkgen::SdkgenBackend;
use handl_plugin_sdkgen_cli::{CliBackend, CliConfig};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// An executable fixture: root help lists two subcommands plus a
/// positional/option/rest; child help is one leaf's table.
fn fixture_tool(dir: &Path) -> PathBuf {
    let path = dir.join("faketool");
    std::fs::write(
        &path,
        "#!/bin/sh\n\
         if [ -z \"$2\" ]; then\n\
         printf 'Usage: faketool [flags] <file> [more]...\\n\\nFlags:\\n      --root-opt  R\\n\\nCommands:\\n  shared   Shared thing\\n  extra    Extra thing\\n'\n\
         else\n\
         printf 'Usage: faketool %s <name>\\n\\nFlags:\\n      --child-opt  C\\n' \"$1\"\n\
         fi\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // exec of a just-written file can race ETXTBSY — settle briefly.
    std::thread::sleep(std::time::Duration::from_millis(20));
    path
}

fn fn_names(m: &handl_plugin_sdkgen::SdkgenModule) -> Vec<&str> {
    m.functions.iter().map(|f| f.name.as_str()).collect()
}

#[test]
fn existing_executable_path_is_probed() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path());
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&tool)
        .unwrap();
    assert_eq!(m.diagnostics, ["confidence: high"], "{:?}", m.diagnostics);
    assert_eq!(m.library, "faketool");
    // Both children probed at depth 1 → two leaf functions.
    let mut names = fn_names(&m);
    names.sort_unstable();
    assert_eq!(names, ["faketool_extra", "faketool_shared"]);
    let shared = m
        .functions
        .iter()
        .find(|f| f.name == "faketool_shared")
        .unwrap();
    assert_eq!(
        shared.arg_descriptors,
        vec!["pos:name", "opt:flag:child_opt:--child-opt:C"]
    );
    // `c_decl` shows the tool name (basename), not the probe path.
    assert_eq!(shared.c_decl, "faketool shared");
}

#[test]
fn bare_tool_name_probes_path_and_survives_spawn_failure() {
    // Not on PATH → the probe error lands in diagnostics and the tool
    // still maps to a bare root-leaf function.
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(Path::new("no-real-tool-xyz"))
        .unwrap();
    assert_eq!(m.diagnostics.len(), 2, "{:?}", m.diagnostics);
    assert_eq!(m.diagnostics[0], "confidence: low");
    assert!(m.diagnostics[1].contains("spawn"), "{:?}", m.diagnostics);
    assert_eq!(fn_names(&m), ["no_real_tool_xyz"]);
    assert_eq!(m.functions[0].c_decl, "no-real-tool-xyz");
}

#[test]
fn spec_with_probe_merges_authored_over_probed() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path());
    let spec_path = tmp.path().join("tool.clispec.yaml");
    std::fs::write(
        &spec_path,
        format!(
            "name: {tool}\n\
             probe: {{ flag: \"--help\", max_depth: 1 }}\n\
             commands:\n\
             \x20 shared:\n\
             \x20   about: authored about\n\
             \x20   options: {{ forced: {{ flag: \"--forced\", kind: int }} }}\n\
             \x20   positionals: [{{ name: apos, kind: int }}]\n\
             \x20   rest: {{ name: arest, kind: int }}\n\
             \x20   commands: {{ deep: {{}} }}\n\
             \x20 declared: {{ about: declared only }}\n",
            tool = tool.display()
        ),
    )
    .unwrap();

    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&spec_path)
        .unwrap();
    assert_eq!(m.diagnostics, ["confidence: high"], "{:?}", m.diagnostics);
    let mut names = fn_names(&m);
    names.sort_unstable();
    // `shared` keeps its authored `deep` child → not a leaf; `declared`
    // is authored-only; `extra` came from the probe.
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.iter().any(|n| n.ends_with("_shared_deep")));
    assert!(names.iter().any(|n| n.ends_with("_declared")));
    // The probed `extra` leaf keeps its probed shape.
    let extra = m
        .functions
        .iter()
        .find(|f| f.name.ends_with("_extra"))
        .unwrap();
    assert_eq!(
        extra.arg_descriptors,
        vec!["pos:name", "opt:flag:child_opt:--child-opt:C"]
    );
}

#[test]
fn config_max_depth_caps_the_probe() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path());
    let cfg = CliConfig {
        max_depth: Some(0),
        ..Default::default()
    };
    let m = CliBackend::new(None, cfg).parse_header(&tool).unwrap();
    // Children recorded but never probed → bare leaf functions.
    let mut names = fn_names(&m);
    names.sort_unstable();
    assert_eq!(names, ["faketool_extra", "faketool_shared"]);
    assert!(m.functions[0].port_args.is_empty());
}

#[test]
fn spec_probe_flag_and_config_drive_the_plan() {
    // A spec file whose `probe.flag` points nowhere sensible still
    // parses: the runner fails, the diagnostic lands, authored
    // commands remain.
    let tmp = tempfile::tempdir().unwrap();
    let spec_path = tmp.path().join("t.yaml");
    std::fs::write(
        &spec_path,
        "name: no-real-tool-xyz\nprobe: { flag: \"help\" }\ncommands:\n  only: {}\n",
    )
    .unwrap();
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&spec_path)
        .unwrap();
    assert_eq!(m.diagnostics.len(), 2);
    assert_eq!(fn_names(&m), ["no_real_tool_xyz_only"]);
}

#[test]
fn emit_spec_writes_the_resolved_tree_and_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path());
    let out = tmp.path().join("faketool.clispec.yaml");
    let cfg = CliConfig {
        emit_spec: Some(out.to_string_lossy().into_owned()),
        ..Default::default()
    };
    CliBackend::new(None, cfg).parse_header(&tool).unwrap();

    let spec = handl_plugin_sdkgen_cli::CliSpec::load(&out).unwrap();
    assert_eq!(spec.name, "faketool");
    // `probe:` is never emitted — the dumped tree is the resolved one.
    assert!(spec.probe.is_none());
    let shared = &spec.root.commands["shared"];
    assert_eq!(shared.about.as_deref(), Some("Shared thing"));
    assert_eq!(shared.options["child_opt"].flag, "--child-opt");
    assert_eq!(shared.positionals[0].name, "name");
    // Re-loading gives back exactly the tree that was written.
    let yaml = spec.dump();
    let again: handl_plugin_sdkgen_cli::CliSpec = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(again.root, spec.root);
}

#[test]
fn emit_spec_dash_prints_and_bad_dest_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path());
    // `-` writes the spec to stdout — the run still succeeds.
    let cfg = CliConfig {
        emit_spec: Some("-".to_string()),
        ..Default::default()
    };
    CliBackend::new(None, cfg).parse_header(&tool).unwrap();

    // A directory as the destination fails the write.
    let cfg = CliConfig {
        emit_spec: Some(tmp.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    let e = CliBackend::new(None, cfg).parse_header(&tool).unwrap_err();
    assert!(
        e.to_string().contains("failed to write emitted spec"),
        "{e}"
    );
}
