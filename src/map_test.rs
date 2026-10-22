use super::map::map_spec;
use super::spec::*;

fn gh_spec() -> CliSpec {
    serde_yaml::from_str(
        r#"
name: gh
about: GitHub CLI
commands:
  pr:
    about: Manage pull requests
    commands:
      create:
        about: Create a pull request
        options:
          title: { flag: "--title", kind: string }
          draft: { flag: "--draft", kind: flag }
          label: { flag: "--label", kind: strings }
          count: { flag: "--count", kind: int }
        positionals: [{ name: branch, kind: string }, { name: n, kind: int }, { name: yes, kind: bool }]
        rest: { name: extras, kind: string }
      list:
        about: List pull requests
"#,
    )
    .unwrap()
}

#[test]
fn one_function_per_leaf_with_full_argv_shape() {
    let m = map_spec(&gh_spec(), "host");
    assert_eq!(m.library, "gh");
    assert_eq!(m.module, "gh");
    assert_eq!(m.backend_id, "cli");
    assert_eq!(m.client_package, "cli");
    assert_eq!(m.target, "host");
    assert_eq!(m.imports.len(), 1);
    assert_eq!(m.imports[0].package, "cli");

    assert_eq!(m.functions.len(), 2);
    let f = &m.functions[0];
    assert_eq!(f.name, "gh_pr_create");
    assert_eq!(f.port_name, "GhPrCreate");
    assert_eq!(f.c_decl, "gh pr create");
    assert_eq!(f.doc.as_deref(), Some("Create a pull request"));
    assert_eq!(f.return_port, "Output");
    assert_eq!(f.ret_descriptor, "cli");
    assert!(!f.errno_guard);
    assert!(f.variadic);

    // positionals, then `opts`, then `rest` last.
    let arg_names: Vec<&str> = f.port_args.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(arg_names, ["branch", "n", "yes", "opts", "extras"]);
    let arg_ports: Vec<&str> = f.port_args.iter().map(|a| a.port.as_str()).collect();
    assert_eq!(arg_ports, ["String", "Int", "Bool", "Obj", "[String]"]);

    // Options serialize sorted by spec name (BTreeMap order).
    assert_eq!(
        f.arg_descriptors,
        vec![
            "pos:branch",
            "pos:n",
            "pos:yes",
            "opt:int:count:--count",
            "opt:flag:draft:--draft",
            "opt:strings:label:--label",
            "opt:string:title:--title",
            "rest:extras",
        ]
    );

    let list = &m.functions[1];
    assert_eq!(list.name, "gh_pr_list");
    assert!(list.port_args.is_empty());
    assert!(!list.variadic);
}

#[test]
fn leafless_tool_is_a_root_leaf() {
    let spec: CliSpec =
        serde_yaml::from_str("name: my-tool\npositionals: [{ name: pat, kind: string }]\n")
            .unwrap();
    let m = map_spec(&spec, "host");
    // The module name is snake-cased; the library keeps the binary name.
    assert_eq!(m.module, "my_tool");
    assert_eq!(m.library, "my-tool");
    assert_eq!(m.functions.len(), 1);
    assert_eq!(m.functions[0].name, "my_tool");
    assert_eq!(m.functions[0].c_decl, "my-tool");
}

#[test]
fn command_names_snake_case_into_function_names() {
    let spec: CliSpec = serde_yaml::from_str(
        "name: tool\ncommands:\n  dry-run:\n    commands:\n      go-faster: {}\n",
    )
    .unwrap();
    let m = map_spec(&spec, "host");
    assert_eq!(m.functions[0].name, "tool_dry_run_go_faster");
    assert_eq!(m.functions[0].port_name, "ToolDryRunGoFaster");
    assert_eq!(m.functions[0].c_decl, "tool dry-run go-faster");
}

#[test]
fn branch_nodes_never_emit_functions() {
    let spec: CliSpec = serde_yaml::from_str(
        "name: t\ncommands:\n  grp:\n    options:\n      x: { flag: \"-x\" }\n    commands:\n      leaf: {}\n",
    )
    .unwrap();
    let m = map_spec(&spec, "host");
    assert_eq!(m.functions.len(), 1);
    assert_eq!(m.functions[0].name, "t_grp_leaf");
}

#[test]
fn empty_spec_emits_a_bare_leaf() {
    let m = map_spec(&CliSpec::default(), "t");
    assert_eq!(m.functions.len(), 1);
    assert_eq!(m.functions[0].name, "");
    assert!(m.functions[0].port_args.is_empty());
}

#[test]
fn rest_element_kind_maps_the_list_port() {
    let spec: CliSpec = serde_yaml::from_str("name: t\nrest: { name: ns, kind: int }\n").unwrap();
    let m = map_spec(&spec, "host");
    assert_eq!(m.functions[0].port_args[0].port, "[Int]");
}

#[test]
fn json_output_flags_tag_a_meta_descriptor() {
    for opt in [
        "json: { flag: \"--json\", kind: flag }",
        "format: { flag: \"--format\", kind: string }",
        "out: { flag: \"--output\", kind: string }",
        "o: { flag: \"-o\", kind: string }",
    ] {
        let spec: CliSpec = serde_yaml::from_str(&format!("name: t\noptions:\n  {opt}\n")).unwrap();
        let m = map_spec(&spec, "host");
        assert!(
            m.functions[0]
                .arg_descriptors
                .contains(&"meta:json-capable".to_string()),
            "{opt}"
        );
    }
    let spec: CliSpec =
        serde_yaml::from_str("name: t\noptions:\n  x: { flag: \"-x\", kind: flag }\n").unwrap();
    let m = map_spec(&spec, "host");
    assert!(
        !m.functions[0]
            .arg_descriptors
            .contains(&"meta:json-capable".to_string())
    );
}

#[test]
fn docs_thread_into_arg_descriptors() {
    let spec: CliSpec = serde_yaml::from_str(
        r#"
name: t
options:
  state: { flag: "--state", kind: string, doc: "filter by state: open, closed" }
  quiet: { flag: "--quiet", kind: flag, doc: "" }
  bare: { flag: "--bare", kind: flag, doc: "   " }
positionals: [{ name: path, kind: string, doc: "file to read" }]
rest: { name: extras, kind: string, doc: "further argv words" }
"#,
    )
    .unwrap();
    let m = map_spec(&spec, "host");
    assert_eq!(
        m.functions[0].arg_descriptors,
        vec![
            "pos:path:file to read",
            // Options emit in spec-key order (BTreeMap sort).
            // Empty/whitespace docs append nothing.
            "opt:flag:bare:--bare",
            "opt:flag:quiet:--quiet",
            // The doc may contain colons — it is the descriptor's
            // last component, taken whole by the emitter.
            "opt:string:state:--state:filter by state: open, closed",
            "rest:extras:further argv words",
        ]
    );
}
