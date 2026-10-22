//! Foreign spec ingestion through `CliBackend::parse_header` —
//! `*.fig.json`, `*.figspec`, cli-schema documents, and usage-spec
//! KDL files resolve to the same `SdkgenModule` IR a `.clispec.yaml`
//! produces; unrecognized files still fall back to the tool probe.

use handl_plugin_sdkgen::SdkgenBackend;
use handl_plugin_sdkgen_cli::{CliBackend, CliConfig, CliSpec};
use std::io::Write;
use std::path::Path;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/specs")
        .join(name)
}

fn fn_names(m: &handl_plugin_sdkgen::SdkgenModule) -> Vec<&str> {
    m.functions.iter().map(|f| f.name.as_str()).collect()
}

fn write_spec(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(text.as_bytes()).unwrap();
    p
}

#[test]
fn fig_spec_loads_through_the_backend() {
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&fixture("gh.fig.json"))
        .unwrap();
    assert_eq!(m.library, "gh");
    let mut names = fn_names(&m);
    names.sort_unstable();
    assert_eq!(
        names,
        ["gh_pr_checkout", "gh_pr_list", "gh_repo_clone"],
        "{names:?}"
    );
    let checkout = m
        .functions
        .iter()
        .find(|f| f.name == "gh_pr_checkout")
        .unwrap();
    assert_eq!(
        checkout.arg_descriptors,
        vec![
            "pos:pr",
            "opt:string:branch:--branch",
            "opt:flag:detach:--detach",
            "opt:flag:recurse_submodules:--recurse-submodules",
        ]
    );
    let list = m.functions.iter().find(|f| f.name == "gh_pr_list").unwrap();
    assert_eq!(
        list.doc.as_deref(),
        Some("List pull requests [aliases: ls]")
    );
    assert!(
        list.arg_descriptors
            .contains(&"opt:strings:label:--label".to_string()),
        "{:?}",
        list.arg_descriptors
    );
    let clone = m
        .functions
        .iter()
        .find(|f| f.name == "gh_repo_clone")
        .unwrap();
    assert_eq!(
        clone.arg_descriptors,
        vec!["pos:repository", "pos:directory", "rest:extra_args"]
    );
    // The `generators` field on the variadic arg did not map.
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d.contains("unsupported arg #3 fields dropped: generators")),
        "{:?}",
        m.diagnostics
    );
}

#[test]
fn cli_schema_document_loads_through_the_backend() {
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&fixture("cli-schema-v1.json"))
        .unwrap();
    assert_eq!(m.library, "deploy");
    let mut names = fn_names(&m);
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "deploy_env_apply",
            "deploy_env_delete",
            "deploy_fleet_status"
        ],
        "{names:?}"
    );
    let apply = m
        .functions
        .iter()
        .find(|f| f.name == "deploy_env_apply")
        .unwrap();
    // Own params + namespace `options` + globalOptions all reach the
    // leaf.
    for needle in [
        "pos:environment",
        "opt:string:file:--file",
        "opt:strings:tag:--tag",
        "opt:flag:dry_run:--dry-run",
        "opt:string:region:--region",
        "opt:flag:verbose:--verbose",
        "rest:extras",
    ] {
        assert!(
            apply.arg_descriptors.contains(&needle.to_string()),
            "missing {needle}: {:?}",
            apply.arg_descriptors
        );
    }
    let delete = m
        .functions
        .iter()
        .find(|f| f.name == "deploy_env_delete")
        .unwrap();
    assert_eq!(
        delete.doc.as_deref(),
        Some("Delete an environment [deprecated: use `env destroy` instead]")
    );
    let status = m
        .functions
        .iter()
        .find(|f| f.name == "deploy_fleet_status")
        .unwrap();
    assert!(
        status
            .arg_descriptors
            .contains(&"opt:flag:verbose:--verbose".to_string()),
        "{:?}",
        status.arg_descriptors
    );
}

#[test]
fn figspec_extension_and_content_sniffing() {
    let tmp = tempfile::tempdir().unwrap();
    // `.figspec` name claims fig even for a bare spec.
    let p = write_spec(
        tmp.path(),
        "tool.figspec",
        r#"{"name": "tl", "options": [{"name": "--force"}]}"#,
    );
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap();
    assert_eq!(m.library, "tl");
    assert_eq!(m.functions[0].arg_descriptors, ["opt:flag:force:--force"]);

    // A plain `.json` file with fig-shaped content is sniffed as fig.
    let p = write_spec(
        tmp.path(),
        "tool.json",
        r#"{"name": "sniffed", "subcommands": [{"name": "go"}]}"#,
    );
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap();
    assert_eq!(fn_names(&m), ["sniffed_go"]);
}

#[test]
fn missing_and_malformed_foreign_specs_error() {
    let tmp = tempfile::tempdir().unwrap();
    // A claimed spec name that does not exist is an error, not a probe.
    for name in ["no.fig.json", "no.figspec", "no.cli-schema.json"] {
        let e = CliBackend::new(None, CliConfig::default())
            .parse_header(&tmp.path().join(name))
            .unwrap_err();
        assert!(e.to_string().contains("cli spec not found"), "{name}: {e}");
    }
    // A directory wearing a spec name errors the same way.
    let dir = tmp.path().join("d.fig.json");
    std::fs::create_dir(&dir).unwrap();
    let e = CliBackend::new(None, CliConfig::default())
        .parse_header(&dir)
        .unwrap_err();
    assert!(e.to_string().contains("cli spec not found"), "{e}");
    // A `.fig.json` file with broken JSON reports the fig parse error.
    let p = write_spec(tmp.path(), "bad.fig.json", "{oops");
    let e = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap_err();
    assert!(e.to_string().contains("failed to parse fig spec"), "{e}");
}

#[test]
fn unrecognized_files_fall_back_to_probing() {
    let tmp = tempfile::tempdir().unwrap();
    // A `.json` file with no foreign markers is not a spec — it is
    // treated like any other existing file and probed (which fails to
    // spawn, surfacing probe diagnostics rather than spec errors).
    let p = write_spec(tmp.path(), "data.json", r#"{"unrelated": 1}"#);
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap();
    assert_eq!(m.diagnostics[0], "confidence: low");
    assert!(fn_names(&m)[0].starts_with("data"), "{:?}", fn_names(&m));
}

#[test]
fn foreign_spec_emit_spec_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("gh.clispec.yaml");
    let cfg = CliConfig {
        emit_spec: Some(out.to_string_lossy().into_owned()),
        ..Default::default()
    };
    CliBackend::new(None, cfg)
        .parse_header(&fixture("gh.fig.json"))
        .unwrap();
    // The emitted canonical spec re-loads to the same tree.
    let spec = CliSpec::load(&out).unwrap();
    assert_eq!(spec.name, "gh");
    assert!(spec.root.commands["pr"].commands.contains_key("list"));
    assert_eq!(
        spec.root.commands["repo"].commands["clone"]
            .rest
            .as_ref()
            .unwrap()
            .name,
        "extra_args"
    );
}

#[test]
fn usage_kdl_spec_loads_through_the_backend() {
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&fixture("example.kdl"))
        .unwrap();
    assert_eq!(m.library, "mycli");
    let mut names = fn_names(&m);
    names.sort_unstable();
    assert_eq!(
        names,
        ["mycli_config_add", "mycli_config_remove", "mycli_deploy"],
        "{names:?}"
    );
    let deploy = m
        .functions
        .iter()
        .find(|f| f.name == "mycli_deploy")
        .unwrap();
    // `use "output"` splices the flagset in; the two `global=#true`
    // root flags reach every leaf.
    assert_eq!(
        deploy.arg_descriptors,
        vec![
            "pos:service",
            "opt:string:color:--color",
            "opt:flag:dry_run:--dry-run",
            "opt:string:env:--env",
            "opt:string:output:--output",
            "opt:flag:quiet:--quiet",
            "opt:flag:verbose:--verbose",
            "rest:files",
            "meta:json-capable",
        ]
    );
    let env_doc = deploy.arg_descriptors.iter().find(|d| d.contains("--env"));
    assert!(env_doc.is_some());
    assert_eq!(deploy.doc.as_deref(), Some("Deploy a service [aliases: d]"));
    let add = m
        .functions
        .iter()
        .find(|f| f.name == "mycli_config_add")
        .unwrap();
    assert_eq!(
        add.arg_descriptors,
        vec![
            "pos:key",
            "pos:value",
            "opt:flag:force:--force",
            "opt:flag:quiet:--quiet",
            "opt:flag:verbose:--verbose",
        ]
    );
    let remove = m
        .functions
        .iter()
        .find(|f| f.name == "mycli_config_remove")
        .unwrap();
    assert_eq!(
        remove.doc.as_deref(),
        Some("Remove a config entry [deprecated: use `config rm`]")
    );
    // `min_usage_version`, `version`, and the flagset's own
    // `complete`-style machinery are consumed or warned about.
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
}

#[test]
fn usage_kdl_name_claims_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["no.kdl", "no.usage.kdl"] {
        let e = CliBackend::new(None, CliConfig::default())
            .parse_header(&tmp.path().join(name))
            .unwrap_err();
        assert!(e.to_string().contains("cli spec not found"), "{name}: {e}");
    }
    let p = write_spec(tmp.path(), "bad.kdl", "bin {oops");
    let e = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap_err();
    assert!(e.to_string().contains("failed to parse usage spec"), "{e}");
}

#[test]
fn usage_kdl_content_sniffing() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_spec(
        tmp.path(),
        "tool.txt",
        "bin \"sniffed\"\ncmd \"go\" { arg \"<x>\" }",
    );
    let m = CliBackend::new(None, CliConfig::default())
        .parse_header(&p)
        .unwrap();
    assert_eq!(fn_names(&m), ["sniffed_go"]);
}

#[test]
fn usage_kdl_emit_spec_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("mycli.clispec.yaml");
    let cfg = CliConfig {
        emit_spec: Some(out.to_string_lossy().into_owned()),
        ..Default::default()
    };
    CliBackend::new(None, cfg)
        .parse_header(&fixture("example.kdl"))
        .unwrap();
    let spec = CliSpec::load(&out).unwrap();
    assert_eq!(spec.name, "mycli");
    let deploy = &spec.root.commands["deploy"];
    assert_eq!(deploy.rest.as_ref().unwrap().name, "files");
    assert!(deploy.options.contains_key("output"));
}
