use super::config::CliConfig;
use super::spec::*;
use std::collections::BTreeMap;
use std::io::Write;

fn spec() -> CliSpec {
    let mut create = CommandNode::default();
    create.options.insert(
        "reviewers".to_string(),
        OptionSpec {
            flag: "--reviewer".to_string(),
            kind: "string".to_string(),
            doc: None,
        },
    );
    create.positionals.push(PositionalSpec {
        name: "count".to_string(),
        kind: "string".to_string(),
        doc: None,
    });
    create.rest = Some(PositionalSpec {
        name: "extras".to_string(),
        kind: "string".to_string(),
        doc: None,
    });
    let mut pr = CommandNode::default();
    pr.commands.insert("create".to_string(), create);
    pr.commands
        .insert("delete".to_string(), CommandNode::default());
    let mut spec = CliSpec {
        name: "gh".to_string(),
        ..Default::default()
    };
    spec.root.commands.insert("pr".to_string(), pr);
    spec
}

fn config(yaml: &str) -> CliConfig {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    CliConfig::load(f.path()).unwrap()
}

#[test]
fn loads_all_overlay_fields() {
    let c = config(
        r#"
rename: { "pr.create": "make" }
drop: ["pr.delete"]
types: { "pr.create.reviewers": strings }
max_depth: 2
"#,
    );
    assert_eq!(c.rename["pr.create"], "make");
    assert_eq!(c.drop, ["pr.delete"]);
    assert_eq!(c.types["pr.create.reviewers"], "strings");
    assert_eq!(c.max_depth, Some(2));
}

#[test]
fn load_errors_on_missing_and_bad_files() {
    assert!(
        CliConfig::load(std::path::Path::new("/no/such.yaml"))
            .unwrap_err()
            .contains("failed to read config")
    );
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(b"drop: {not: a list\n").unwrap();
    assert!(
        CliConfig::load(f.path())
            .unwrap_err()
            .contains("failed to parse config")
    );
}

#[test]
fn drop_removes_a_command_subtree() {
    let mut s = spec();
    let c = config("drop: [\"pr.delete\"]\n");
    let diags = c.apply(&mut s);
    assert!(diags.is_empty());
    assert!(!s.root.commands["pr"].commands.contains_key("delete"));
    assert!(s.root.commands["pr"].commands.contains_key("create"));
}

#[test]
fn drop_reports_a_miss() {
    let mut s = spec();
    let c = config("drop: [\"pr.nope\", \"gone.deeper.x\"]\n");
    let diags = c.apply(&mut s);
    assert_eq!(diags.len(), 2);
    assert!(diags.iter().all(|d| d.contains("matched no command")));
}

#[test]
fn rename_moves_the_leaf() {
    let mut s = spec();
    let c = config("rename: { \"pr.create\": make }\n");
    let diags = c.apply(&mut s);
    assert!(diags.is_empty());
    let pr = &s.root.commands["pr"];
    assert!(pr.commands.contains_key("make"));
    assert!(!pr.commands.contains_key("create"));
    // The moved node keeps its options.
    assert!(pr.commands["make"].options.contains_key("reviewers"));
}

#[test]
fn rename_reports_a_miss() {
    let mut s = spec();
    let c = config("rename: { \"pr.gone\": x, \"nope.deeper\": y }\n");
    let diags = c.apply(&mut s);
    assert_eq!(diags.len(), 2);
}

#[test]
fn types_override_option_positional_and_rest_kinds() {
    let mut s = spec();
    let c = config(
        r#"
types:
  pr.create.reviewers: strings
  pr.create.count: int
  pr.create.extras: int
"#,
    );
    let diags = c.apply(&mut s);
    assert!(diags.is_empty(), "{diags:?}");
    let create = &s.root.commands["pr"].commands["create"];
    assert_eq!(create.options["reviewers"].kind, "strings");
    assert_eq!(create.positionals[0].kind, "int");
    assert_eq!(create.rest.as_ref().unwrap().kind, "int");
}

#[test]
fn types_report_misses_and_bad_kinds() {
    let mut s = spec();
    let c = config(
        r#"
types:
  pr.create.reviewers: float
  pr.create.count: float
  pr.create.extras: float
  pr.create.missing: string
  gone.x.y: string
"#,
    );
    let diags = c.apply(&mut s);
    assert_eq!(diags.len(), 5, "{diags:?}");
    // `types` is a sorted map — diagnostics come out in key order.
    assert!(diags[0].contains("matched no command"), "{diags:?}");
    assert!(diags[1].contains("not a positional kind"), "{diags:?}");
    assert!(diags[2].contains("not a positional kind"), "{diags:?}");
    assert!(
        diags[3].contains("matched no option or positional"),
        "{diags:?}"
    );
    assert!(diags[4].contains("not an option kind"), "{diags:?}");
}

#[test]
fn apply_runs_drop_and_types_before_rename() {
    // Keys reference original names — `pr.create` in `types` still
    // applies even though `rename` later moves it to `make`.
    let mut s = spec();
    let c = CliConfig {
        drop: vec!["pr.delete".to_string()],
        types: BTreeMap::from([("pr.create.reviewers".to_string(), "strings".to_string())]),
        rename: BTreeMap::from([("pr.create".to_string(), "make".to_string())]),
        ..Default::default()
    };
    let diags = c.apply(&mut s);
    assert!(diags.is_empty());
    let pr = &s.root.commands["pr"];
    assert_eq!(pr.commands["make"].options["reviewers"].kind, "strings");
    assert!(!pr.commands.contains_key("delete"));
}

#[test]
fn emit_spec_loads_from_the_config_file() {
    let c = config("emit_spec: out.clispec.yaml\n");
    assert_eq!(c.emit_spec.as_deref(), Some("out.clispec.yaml"));
}
