use super::spec::*;
use std::io::Write;

fn write_spec(text: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(text.as_bytes()).unwrap();
    f
}

#[test]
fn loads_a_full_spec() {
    let f = write_spec(
        r#"
name: gh
about: GitHub CLI
probe: { flag: "--help", max_depth: 2 }
commands:
  pr:
    about: Manage PRs
    commands:
      create:
        about: Create a pull request
        options:
          title: { flag: "--title", kind: string, doc: "PR title" }
          draft: { flag: "--draft", kind: flag }
          label: { flag: "--label", kind: strings }
        positionals: [{ name: branch, kind: string, doc: "base branch" }]
        rest: { name: extras, kind: string }
"#,
    );
    let spec = CliSpec::load(f.path()).unwrap();
    assert_eq!(spec.name, "gh");
    assert_eq!(spec.about.as_deref(), Some("GitHub CLI"));
    assert_eq!(spec.probe.as_ref().unwrap().max_depth, Some(2));
    let create = &spec.root.commands["pr"].commands["create"];
    assert_eq!(create.options["title"].kind, "string");
    assert_eq!(create.options["draft"].kind, "flag");
    assert_eq!(create.positionals[0].name, "branch");
    assert_eq!(create.rest.as_ref().unwrap().name, "extras");
}

#[test]
fn root_level_args_make_a_tool_without_subcommands() {
    let f = write_spec(
        r#"
name: grep
options:
  ignore_case: { flag: "-i", kind: flag }
positionals: [{ name: pattern, kind: string }]
rest: { name: files, kind: string }
"#,
    );
    let spec = CliSpec::load(f.path()).unwrap();
    assert!(spec.root.commands.is_empty());
    assert_eq!(spec.root.options["ignore_case"].flag, "-i");
    assert_eq!(spec.root.rest.as_ref().unwrap().kind, "string");
}

#[test]
fn missing_name_is_an_error() {
    let f = write_spec("commands: {}\n");
    assert_eq!(
        CliSpec::load(f.path()).unwrap_err(),
        "spec is missing `name`"
    );
}

#[test]
fn unreadable_and_unparseable_specs_error() {
    let missing = std::path::Path::new("/no/such/spec.yaml");
    assert!(
        CliSpec::load(missing)
            .unwrap_err()
            .contains("failed to read spec")
    );
    let f = write_spec("name: [not, a, string\n");
    assert!(
        CliSpec::load(f.path())
            .unwrap_err()
            .contains("failed to parse spec")
    );
}

#[test]
fn unknown_option_kind_is_rejected() {
    let f = write_spec("name: t\noptions:\n  x: { flag: \"--x\", kind: floot }\n");
    let e = CliSpec::load(f.path()).unwrap_err();
    assert!(e.contains("unknown kind `floot`"), "{e}");
    assert!(e.contains("flag, string, int, strings"), "{e}");
}

#[test]
fn flags_must_start_with_a_dash() {
    let f = write_spec("name: t\noptions:\n  x: { flag: \"title\" }\n");
    let e = CliSpec::load(f.path()).unwrap_err();
    assert!(e.contains("must start with `-`"), "{e}");
}

#[test]
fn duplicate_flag_spellings_are_rejected() {
    let f = write_spec("name: t\noptions:\n  a: { flag: \"--x\" }\n  b: { flag: \"--x\" }\n");
    assert!(
        CliSpec::load(f.path())
            .unwrap_err()
            .contains("duplicate flag spelling `--x`")
    );
}

#[test]
fn duplicate_positional_names_are_rejected() {
    let f = write_spec("name: t\npositionals: [{ name: x }, { name: x }]\n");
    assert!(
        CliSpec::load(f.path())
            .unwrap_err()
            .contains("duplicate positional `x`")
    );
}

#[test]
fn bad_names_and_kinds_are_rejected() {
    for body in [
        // option name containing ':' would corrupt descriptors
        "options:\n  \"a:b\": { flag: \"--x\" }\n",
        // empty positional name
        "positionals: [{ name: \"\" }]\n",
        // unknown positional kind
        "positionals: [{ name: x, kind: float }]\n",
        // bad rest kind
        "rest: { name: tail, kind: float }\n",
    ] {
        let f = write_spec(&format!("name: t\n{body}"));
        assert!(CliSpec::load(f.path()).is_err(), "{body}");
    }
    // positional name with ':'
    let f = write_spec("name: t\npositionals: [{ name: \"a:b\" }]\n");
    assert!(CliSpec::load(f.path()).is_err());
    // flag containing ':'
    let f = write_spec("name: t\noptions:\n  x: { flag: \"--a:b\" }\n");
    assert!(CliSpec::load(f.path()).is_err());
}

#[test]
fn nested_errors_carry_the_dotted_path() {
    let f = write_spec(
        r#"
name: t
commands:
  a:
    commands:
      b:
        options:
          c: { flag: "oops" }
"#,
    );
    let e = CliSpec::load(f.path()).unwrap_err();
    assert!(e.starts_with("t.a.b:"), "{e}");
}

#[test]
fn empty_subcommand_name_is_rejected() {
    let f = write_spec("name: t\ncommands:\n  \"\": {}\n");
    assert!(CliSpec::load(f.path()).is_err());
}
