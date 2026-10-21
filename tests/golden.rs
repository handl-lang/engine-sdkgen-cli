//! Golden-file tests: a spec file through the `cli` backend produces
//! `SdkgenModule` IR; the `hnl-cli` emitter renders it. The emitted
//! files are compared byte-for-byte against `tests/fixtures/golden/` —
//! regenerate with `REGEN_GOLDEN=1`.

use handl_language::Language;
use handl_plugin_sdkgen::{Emitter, SdkgenBackend};
use handl_plugin_sdkgen_cli::{CliBackend, CliConfig, HnlCliEmitter};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

const SPEC: &str = r#"
name: gh
about: GitHub CLI
commands:
  pr:
    about: Manage pull requests
    commands:
      create:
        about: Create a pull request
        options:
          title:  { flag: "--title", kind: string, doc: "PR title" }
          draft:  { flag: "--draft", kind: flag }
          label:  { flag: "--label", kind: strings }
        positionals: [{ name: branch, kind: string, doc: "base branch" }]
        rest: { name: extras, kind: string }
      list:
        about: List pull requests
"#;

fn lang() -> Arc<Language> {
    Arc::new(handl_kit_data_prelude::language_from_yaml().unwrap())
}

fn spec_path(dir: &Path) -> std::path::PathBuf {
    let p = dir.join("gh.clispec.yaml");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(SPEC.as_bytes()).unwrap();
    p
}

fn compare(name: &str, actual: &str) {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(name);
    if std::env::var("REGEN_GOLDEN").is_ok() {
        std::fs::write(&golden, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("missing golden {name}: {e} — run with REGEN_GOLDEN=1"));
    assert_eq!(actual, expected, "golden {name} drifted");
}

#[test]
fn hnl_cli_port_and_adapter_match_golden() {
    let tmp = tempfile::tempdir().unwrap();
    let spec = spec_path(tmp.path());
    let module = CliBackend::new(None, CliConfig::default())
        .parse_header(&spec)
        .unwrap();

    let out = tmp.path().join("gh");
    HnlCliEmitter::new(lang()).emit(&module, &out).unwrap();
    let port = std::fs::read_to_string(out.join("gh.hnl")).unwrap();
    let adapter = std::fs::read_to_string(tmp.path().join("gh-cli-host").join("gh.hnl")).unwrap();
    compare("gh_port.hnl", &port);
    compare("gh_adapter.hnl", &adapter);
    // Both files canonicalize — `hnl fmt --check` passes on output.
    for text in [&port, &adapter] {
        assert_eq!(
            handl_plugin_ext_fmt::format_source_with_language(&lang(), text).as_deref(),
            Ok(text.as_str()),
            "emitted file is not canonical"
        );
    }
}
