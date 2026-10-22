use super::foreign::*;
use super::spec::CliSpec;
use std::io::Write;
use std::path::Path;

fn write_file(name: &str, text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(text.as_bytes()).unwrap();
    (dir, p)
}

// ------------------------------------------------------------------
// detection
// ------------------------------------------------------------------

#[test]
fn detect_by_file_name() {
    let cases = [
        ("gh.fig.json", Format::Fig),
        ("gh.figspec", Format::Fig),
        ("gh.cli-schema.json", Format::CliSchema),
    ];
    for (name, fmt) in cases {
        // The name claims the dialect even before content is read.
        assert_eq!(
            ForeignSpec::detect(Path::new(name), "garbage"),
            Some(fmt),
            "{name}"
        );
    }
    // A `.fig.json` name anywhere in the path still claims fig.
    assert_eq!(
        ForeignSpec::detect(Path::new("/tmp/x/gh.fig.json"), "{}"),
        Some(Format::Fig)
    );
}

#[test]
fn detect_by_content_markers() {
    let p = Path::new("tool.json");
    assert_eq!(
        ForeignSpec::detect(p, r#"{"schemaVersion": 1, "name": "t"}"#),
        Some(Format::CliSchema)
    );
    assert_eq!(
        ForeignSpec::detect(p, r#"{"cli-schema-version": "1", "name": "t"}"#),
        Some(Format::CliSchema)
    );
    assert_eq!(
        ForeignSpec::detect(p, r#"{"name": "t", "subcommands": []}"#),
        Some(Format::Fig)
    );
    // Fig-shaped option rows alone are enough.
    assert_eq!(
        ForeignSpec::detect(
            p,
            r#"{"name": "t", "options": [{"name": ["--long", "-s"], "description": "x"}]}"#
        ),
        Some(Format::Fig)
    );
    // cli-schema markers win over fig-shaped content.
    assert_eq!(
        ForeignSpec::detect(p, r#"{"schemaVersion": 1, "subcommands": []}"#),
        Some(Format::CliSchema)
    );
}

#[test]
fn detect_rejects_non_specs() {
    let p = Path::new("tool.json");
    for src in [
        "",
        "not json at all",
        "42",
        "[1, 2]",
        "{}",
        r#"{"name": "t"}"#,
        r#"{"options": []}"#,
        r#"{"options": [{"description": "no name"}]}"#,
        r#"{"options": [{"name": {"nested": true}}]}"#,
        r#"{"subcommands": "not-an-array", "options": [{"name": "x"}]}"#,
    ] {
        let last_is_fig = src.contains("\"name\": \"x\"");
        // The last case: fig-shaped options rows DO mark fig even when
        // `subcommands` is malformed.
        let want = if last_is_fig { Some(Format::Fig) } else { None };
        assert_eq!(ForeignSpec::detect(p, src), want, "{src}");
    }
}

// ------------------------------------------------------------------
// fig parsing
// ------------------------------------------------------------------

#[test]
fn fig_maps_a_command_tree() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "gh",
          "description": "GitHub CLI",
          "options": [
            {"name": ["--help", "-h"], "description": "Show help"}
          ],
          "subcommands": [
            {
              "name": "pr",
              "description": "Manage PRs",
              "subcommands": [
                {
                  "name": "checkout",
                  "description": "Check out a PR",
                  "args": {"name": "pr", "description": "PR to check out"},
                  "options": [
                    {
                      "name": ["-b", "--branch"],
                      "description": "Branch name",
                      "args": {"name": "branch"}
                    },
                    {"name": "--detach", "exclusiveOn": ["--branch"]},
                    {"name": ["-v", "--verbose"], "isRequired": true, "isPersistent": true}
                  ]
                }
              ]
            }
          ]
        }"#,
    )
    .unwrap();
    assert_eq!(f.format, Format::Fig);
    let spec = &f.spec;
    assert_eq!(spec.name, "gh");
    assert_eq!(spec.about.as_deref(), Some("GitHub CLI"));
    // The root option prefers the long spelling; the short becomes an
    // alias annotation.
    let help = &spec.root.options["help"];
    assert_eq!(help.flag, "--help");
    assert_eq!(help.kind, "flag");
    assert_eq!(help.doc.as_deref(), Some("Show help [aliases: -h]"));

    let pr = &spec.root.commands["pr"];
    assert_eq!(pr.about.as_deref(), Some("Manage PRs"));
    let checkout = &pr.commands["checkout"];
    assert_eq!(checkout.positionals[0].name, "pr");
    let branch = &checkout.options["branch"];
    assert_eq!(branch.flag, "--branch");
    assert_eq!(branch.kind, "string");
    assert_eq!(
        branch.doc.as_deref(),
        Some("Branch name [aliases: -b] [value: branch]")
    );
    let detach = &checkout.options["detach"];
    assert_eq!(detach.kind, "flag");
    assert_eq!(detach.doc.as_deref(), Some("[exclusive with: --branch]"));
    let verbose = &checkout.options["verbose"];
    assert_eq!(
        verbose.doc.as_deref(),
        Some("[aliases: -v] [required] [persistent]")
    );
}

#[test]
fn fig_repeatable_and_multi_arg_options() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "t",
          "options": [
            {"name": "--tag", "isRepeatable": true, "args": {}},
            {"name": "--n", "isRepeatable": 3, "args": {"name": "n"}},
            {"name": "-v", "isRepeatable": true},
            {"name": "--once", "isRepeatable": false, "args": {}},
            {"name": "--two", "args": [{"name": "a"}, {"name": "b"}]}
          ]
        }"#,
    )
    .unwrap();
    assert_eq!(f.spec.root.options["tag"].kind, "strings");
    assert_eq!(f.spec.root.options["n"].kind, "strings");
    let v = &f.spec.root.options["v"];
    assert_eq!(v.kind, "flag");
    assert_eq!(v.doc.as_deref(), Some("[repeatable]"));
    assert_eq!(f.spec.root.options["once"].kind, "string");
    assert_eq!(f.spec.root.options["two"].kind, "string");
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("option `--two` takes 2 args")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn fig_args_become_positionals_and_rest() {
    // `args: null` is no args at all.
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{"name": "bare", "args": null, "options": [{"name": "--s", "args": "scalar"}]}"#,
    )
    .unwrap();
    assert!(f.spec.root.positionals.is_empty());
    // A scalar `args` on an option still means "takes a value".
    assert_eq!(f.spec.root.options["s"].kind, "string");

    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "cp",
          "args": [
            {"name": "Source File", "description": "the source"},
            {"name": "dest", "isOptional": true},
            {"name": "extras", "isVariadic": true, "default": "."},
            {"name": "dropped", "description": "after variadic"}
          ]
        }"#,
    )
    .unwrap();
    let root = &f.spec.root;
    assert_eq!(root.positionals.len(), 2);
    assert_eq!(root.positionals[0].name, "source_file");
    assert_eq!(root.positionals[0].doc.as_deref(), Some("the source"));
    assert_eq!(root.positionals[1].doc.as_deref(), Some("[optional]"));
    let rest = root.rest.as_ref().unwrap();
    assert_eq!(rest.name, "extras");
    assert_eq!(rest.doc.as_deref(), Some("[default: .]"));
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("arg after the variadic arg dropped")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn fig_subcommand_aliases_and_deep_nesting() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "npm",
          "subcommands": [
            {
              "name": ["install", "i", "add"],
              "description": "Install a package",
              "subcommands": [{"name": "deep", "subcommands": [{"name": "deeper"}]}]
            }
          ]
        }"#,
    )
    .unwrap();
    let install = &f.spec.root.commands["install"];
    assert_eq!(
        install.about.as_deref(),
        Some("Install a package [aliases: i, add]")
    );
    assert!(install.commands["deep"].commands.contains_key("deeper"));
}

#[test]
fn fig_dropped_constructs_warn() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": ["aws", "aws2"],
          "loadSpec": "aws-s3",
          "parserDirectives": {"flagsArePosixNoncompliant": true},
          "options": "oops",
          "args": "junk",
          "subcommands": "oops"
        }"#,
    )
    .unwrap();
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("extra root names dropped: aws2")),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings.iter().any(
            |w| w.contains("unsupported subcommand fields dropped: loadSpec, parserDirectives")
        ),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("`options` is not an array")),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("`subcommands` is not an array")),
        "{:?}",
        f.warnings
    );
    // A non-array `args` is a single arg — "junk" is not an object.
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("non-object arg dropped")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn fig_malformed_nodes_warn_and_skip() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "t",
          "options": [42, {"description": "no name"}],
          "args": [{"name": "a", "suggestions": ["x"], "generators": {"script": "ls"}}, "stray"],
          "subcommands": [7, {"description": "no name"}]
        }"#,
    )
    .unwrap();
    for needle in [
        "non-object option dropped",
        "option with no usable name dropped",
        "unsupported arg #1 fields dropped: generators, suggestions",
        "non-object arg dropped",
        "non-object subcommand dropped",
        "subcommand with no name dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing warning `{needle}`: {:?}",
            f.warnings
        );
    }
}

#[test]
fn fig_dedupes_flags_and_names() {
    let f = ForeignSpec::parse(
        Format::Fig,
        r#"{
          "name": "t",
          "options": [
            {"name": "-x", "description": "short"},
            {"name": "--x", "description": "long — same ident, distinct flag"},
            {"name": "--x", "description": "exact dup — dropped"},
            {"name": "---"},
            {"name": "--x!"}
          ],
          "args": [{"name": "a"}, {"name": "a"}, {}]
        }"#,
    )
    .unwrap();
    // Same flag spelling: the second `--x` drops.
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("duplicate flag `--x` dropped")),
        "{:?}",
        f.warnings
    );
    // `-x` and `--x` collide on ident `x` → `x_2`.
    assert_eq!(f.spec.root.options["x"].flag, "-x");
    assert_eq!(f.spec.root.options["x_2"].flag, "--x");
    // `---` strips to nothing → `opt`.
    assert_eq!(f.spec.root.options["opt"].flag, "---");
    // `--x!` idents to `x_` → trailing underscore trimmed → `x_3`.
    assert_eq!(f.spec.root.options["x_3"].flag, "--x!");
    // Duplicate positional renamed; a nameless arg numbers itself.
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("duplicate positional `a` renamed `a_2`")),
        "{:?}",
        f.warnings
    );
    assert_eq!(f.spec.root.positionals[2].name, "arg3");
}

#[test]
fn fig_errors() {
    assert_eq!(
        ForeignSpec::parse(Format::Fig, "not json").unwrap_err(),
        "failed to parse fig spec: expected ident at line 1 column 2"
    );
    assert_eq!(
        ForeignSpec::parse(Format::Fig, "[]").unwrap_err(),
        "fig spec root must be a JSON object"
    );
    assert_eq!(
        ForeignSpec::parse(Format::Fig, "{}").unwrap_err(),
        "fig spec is missing `name`"
    );
    assert_eq!(
        ForeignSpec::parse(Format::Fig, r#"{"name": ""}"#).unwrap_err(),
        "fig spec is missing `name`"
    );
    // A flag spelling that fails IR validation surfaces as an error.
    let e = ForeignSpec::parse(
        Format::Fig,
        r#"{"name": "t", "options": [{"name": "verbose"}]}"#,
    )
    .unwrap_err();
    assert!(e.contains("must start with `-`"), "{e}");
}

// ------------------------------------------------------------------
// cli-schema parsing
// ------------------------------------------------------------------

#[test]
fn cli_schema_maps_a_command_tree() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "deploy",
          "version": "2.0.0",
          "description": "Deploy tool",
          "globalOptions": [
            {"role": "flag", "name": "verbose", "shortName": "v", "type": "boolean", "required": false, "summary": "Verbose"}
          ],
          "commands": [
            {
              "name": "apply",
              "path": ["env"],
              "summary": "Apply config",
              "notes": "Applies the given file.",
              "parameters": [
                {"role": "positional", "name": "environment", "type": "string", "required": true},
                {"role": "flag", "name": "file", "shortName": "f", "type": "string", "required": true, "summary": "Config"},
                {"role": "flag", "name": "count", "type": "integer", "required": false},
                {"role": "positional", "name": "rest-args", "type": "string", "variadic": true}
              ]
            }
          ]
        }"#,
    )
    .unwrap();
    let spec = &f.spec;
    assert_eq!(spec.name, "deploy");
    assert_eq!(spec.about.as_deref(), Some("Deploy tool"));
    // Global options land on the root node and propagate to leaves.
    assert_eq!(spec.root.options["verbose"].flag, "--verbose");
    assert_eq!(
        spec.root.options["verbose"].doc.as_deref(),
        Some("Verbose [alias: -v]")
    );
    let apply = &spec.root.commands["env"].commands["apply"];
    assert_eq!(
        apply.about.as_deref(),
        Some("Apply config\n\nApplies the given file.")
    );
    assert!(apply.options.contains_key("verbose"));
    assert_eq!(apply.options["file"].kind, "string");
    assert_eq!(
        apply.options["file"].doc.as_deref(),
        Some("Config [required] [alias: -f]")
    );
    assert_eq!(apply.options["count"].kind, "int");
    assert_eq!(apply.positionals[0].name, "environment");
    assert_eq!(apply.rest.as_ref().unwrap().name, "rest_args");
}

#[test]
fn cli_schema_namespaces_and_inherited_options() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "cli-schema-version": "1",
          "name": "t",
          "namespaces": [
            {
              "segment": "auth",
              "summary": "Auth commands",
              "options": [{"role": "flag", "name": "profile", "type": "string"}],
              "defaultCommand": {
                "kind": "namespace",
                "summary": "Show auth status",
                "parameters": [{"role": "flag", "name": "verbose", "type": "boolean"}]
              },
              "commands": [
                {"name": "login", "summary": "Log in"},
                {"name": "whoami", "path": ["extra"], "aliases": ["me"]}
              ],
              "namespaces": [{"segment": "token", "commands": [{"name": "revoke"}]}]
            }
          ]
        }"#,
    )
    .unwrap();
    let auth = &f.spec.root.commands["auth"];
    assert_eq!(auth.about.as_deref(), Some("Auth commands"));
    // Namespace options apply inside the namespace, defaultCommand
    // params are the namespace node's own.
    assert!(auth.options.contains_key("profile"));
    assert!(auth.options.contains_key("verbose"));
    let login = &auth.commands["login"];
    assert_eq!(login.about.as_deref(), Some("Log in"));
    assert!(login.options.contains_key("profile"));
    // `path` nests a command under extra segments inside the namespace.
    let whoami = &auth.commands["extra"].commands["whoami"];
    assert_eq!(whoami.about.as_deref(), Some("[aliases: me]"));
    assert!(whoami.options.contains_key("profile"));
    // Nested namespaces recurse.
    assert!(auth.commands["token"].commands.contains_key("revoke"));
    assert!(
        auth.commands["token"].commands["revoke"]
            .options
            .contains_key("profile")
    );
}

#[test]
fn cli_schema_root_default_and_command_overrides() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "globalOptions": [{"role": "flag", "name": "verbose", "type": "boolean", "summary": "global"}],
          "rootDefault": {
            "kind": "root",
            "parameters": [
              {"role": "flag", "name": "watch", "type": "boolean"},
              {"role": "positional", "name": "target", "type": "string"}
            ]
          },
          "commands": [
            {
              "name": "run",
              "parameters": [{"role": "flag", "name": "verbose", "type": "string", "summary": "own wins"}]
            }
          ]
        }"#,
    )
    .unwrap();
    let root = &f.spec.root;
    assert_eq!(root.options["watch"].flag, "--watch");
    assert_eq!(root.positionals[0].name, "target");
    // A command's own `verbose` beats the global one.
    let run = &root.commands["run"];
    assert_eq!(run.options["verbose"].kind, "string");
    assert_eq!(run.options["verbose"].doc.as_deref(), Some("own wins"));
}

#[test]
fn cli_schema_parameter_roles_and_types() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "commands": [{
            "name": "c",
            "parameters": [
              {"role": "dryRun", "name": "dry-run", "type": "boolean"},
              {"role": "confirmationSkip", "name": "yes", "type": "boolean"},
              {"role": "flag", "name": "num", "type": "number"},
              {"role": "flag", "name": "many", "type": "array", "elementType": "string"},
              {"role": "flag", "name": "choice", "type": "enum", "enumValues": ["a", "b"]},
              {"role": "flag", "name": "rep", "type": "string", "repeatable": true, "separator": ","},
              {"role": "flag", "name": "v", "type": "boolean", "repeatable": true},
              {"role": "flag", "name": "untyped"},
              {"role": "flag", "name": "weird", "type": "mystery"},
              {"role": "positional", "name": "b", "type": "boolean"},
              {"role": "positional", "name": "n", "type": "number"},
              {"role": "bogus", "name": "skipped"},
              {"role": "flag"}
            ]
          }]
        }"#,
    )
    .unwrap();
    let c = &f.spec.root.commands["c"];
    assert_eq!(c.options["dry_run"].doc.as_deref(), Some("[role: dry-run]"));
    assert_eq!(
        c.options["yes"].doc.as_deref(),
        Some("[role: confirmationSkip]")
    );
    assert_eq!(c.options["num"].kind, "int");
    assert_eq!(c.options["many"].kind, "strings");
    assert_eq!(c.options["many"].doc.as_deref(), Some("[elements: string]"));
    assert_eq!(c.options["choice"].doc.as_deref(), Some("[one of: a, b]"));
    assert_eq!(c.options["rep"].kind, "strings");
    assert_eq!(c.options["rep"].doc.as_deref(), Some("[separator: ,]"));
    let v = &c.options["v"];
    assert_eq!(v.kind, "flag");
    assert_eq!(v.doc.as_deref(), Some("[repeatable]"));
    assert_eq!(c.options["untyped"].kind, "flag");
    assert_eq!(c.options["weird"].kind, "string");
    assert_eq!(c.positionals[0].kind, "bool");
    assert_eq!(c.positionals[1].kind, "int");
    assert!(!c.options.contains_key("skipped"));
    for needle in [
        "flag `untyped` has no `type` — mapped as a boolean flag",
        "parameter `weird` has unknown type `mystery` — mapped as `string`",
        "parameter `skipped` has unknown role `bogus` — dropped",
        "parameter with no `name` dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing warning `{needle}`: {:?}",
            f.warnings
        );
    }
}

#[test]
fn cli_schema_annotations_and_dropped_fields() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "environment": {"variables": []},
          "requiresAuth": true,
          "commands": [
            {
              "name": "nuke",
              "hidden": true,
              "deprecated": {"replacement": "destroy"},
              "intent": {"kind": "write"},
              "output": {"formats": ["json"]},
              "streaming": true,
              "usage": "t nuke <x>",
              "parameters": [
                {
                  "role": "flag",
                  "name": "level",
                  "type": "string",
                  "summary": "How hard",
                  "defaultValue": "1",
                  "aliases": ["intensity"],
                  "validations": [{"kind": "range", "min": 0}, {"kind": "regex"}],
                  "x-vendor": true
                },
                {"role": "flag", "name": "old", "type": "boolean", "deprecated": true},
                {"role": "flag", "name": "older", "type": "boolean", "deprecated": {}}
              ]
            },
            {"name": "gone", "deprecated": {"message": "removed"}},
            {"deprecated": true},
            "stray"
          ],
          "x-custom": 1
        }"#,
    )
    .unwrap();
    // Agent-facing root fields the IR cannot hold are warned about —
    // `x-` extensions pass silently.
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("unsupported root fields dropped: environment, requiresAuth")),
        "{:?}",
        f.warnings
    );
    assert!(!f.warnings.iter().any(|w| w.contains("x-custom")));
    let nuke = &f.spec.root.commands["nuke"];
    assert_eq!(
        nuke.about.as_deref(),
        Some("[hidden] [deprecated: use `destroy`]")
    );
    assert!(
        f.warnings.iter().any(|w| w.contains(
            "unsupported command `nuke` fields dropped: intent, output, streaming, usage"
        )),
        "{:?}",
        f.warnings
    );
    let level = &nuke.options["level"];
    assert_eq!(
        level.doc.as_deref(),
        Some("How hard [aliases: --intensity] [default: 1] [constraints: range, regex]")
    );
    assert_eq!(nuke.options["old"].doc.as_deref(), Some("[deprecated]"));
    assert_eq!(nuke.options["older"].doc.as_deref(), Some("[deprecated]"));
    assert_eq!(
        f.spec.root.commands["gone"].about.as_deref(),
        Some("[deprecated: removed]")
    );
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("command with no name dropped"))
    );
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("non-object command dropped"))
    );
}

#[test]
fn cli_schema_malformed_containers_warn() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "globalOptions": {"role": "flag"},
          "rootDefault": "oops",
          "commands": "oops",
          "namespaces": [
            "stray",
            {
              "summary": "no segment"
            },
            {
              "segment": "ns",
              "options": [{"role": "positional", "name": "bad"}, 42],
              "defaultCommand": "oops",
              "commands": ["stray-cmd"],
              "namespaces": ["stray-ns"]
            },
            {
              "segment": "ns2",
              "commands": "oops",
              "namespaces": "oops"
            },
            {"segment": "lone"},
            {"segment": "deep", "commands": [{"name": "x", "parameters": [{"role": "positional", "name": "a", "variadic": true}, {"role": "positional", "name": "b", "variadic": true}, {"role": "positional", "name": "!!!"}]}]}
          ]
        }"#,
    )
    .unwrap();
    for needle in [
        "parameter list is not an array — dropped",
        "`rootDefault` is not an object — dropped",
        "`commands` is not an array — dropped",
        "non-object namespace dropped",
        "namespace with no `segment` dropped",
        "positional `bad` is not valid here — dropped",
        "non-object parameter dropped",
        "`defaultCommand` is not an object — dropped",
        "cli-schema t.ns: non-object command dropped",
        "cli-schema t.ns: non-object namespace dropped",
        "cli-schema t.ns2: `commands` is not an array — dropped",
        "cli-schema t.ns2: `namespaces` is not an array — dropped",
        "second variadic positional `b` dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing warning `{needle}`: {:?}",
            f.warnings
        );
    }
    // A positional that idents to nothing falls back to `arg`.
    assert_eq!(
        f.spec.root.commands["deep"].commands["x"].positionals[0].name,
        "arg"
    );
    // A namespace with no children still lands as a command node.
    assert!(f.spec.root.commands.contains_key("lone"));
    // A non-array `namespaces` at the root warns too.
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{"schemaVersion": 1, "name": "t", "namespaces": "oops"}"#,
    )
    .unwrap();
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("`namespaces` is not an array — dropped")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn cli_schema_notes_defaultcommand_and_hidden_params() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "namespaces": [{
            "segment": "ns",
            "defaultCommand": {"summary": "From the handler"},
            "commands": [{"name": "c", "notes": "Only extended prose."}]
          }],
          "commands": [{
            "name": "c2",
            "parameters": [{"role": "flag", "name": "quiet", "type": "boolean", "hidden": true}]
          }]
        }"#,
    )
    .unwrap();
    let ns = &f.spec.root.commands["ns"];
    // With no namespace summary, `defaultCommand.summary` is the about.
    assert_eq!(ns.about.as_deref(), Some("From the handler"));
    assert_eq!(
        ns.commands["c"].about.as_deref(),
        Some("Only extended prose.")
    );
    assert_eq!(
        f.spec.root.commands["c2"].options["quiet"].doc.as_deref(),
        Some("[hidden]")
    );
}

#[test]
fn cli_schema_command_path_edges() {
    let f = ForeignSpec::parse(
        Format::CliSchema,
        r#"{
          "schemaVersion": 1,
          "name": "t",
          "commands": [
            {"name": "deep", "path": ["a", "", "b"]},
            {"name": "flat", "path": "not-an-array"}
          ]
        }"#,
    )
    .unwrap();
    assert!(
        f.spec.root.commands["a"].commands["b"]
            .commands
            .contains_key("deep")
    );
    assert!(f.spec.root.commands.contains_key("flat"));
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("empty `path` segment ignored"))
    );
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("`path` is not an array — ignored"))
    );
}

#[test]
fn cli_schema_errors() {
    assert_eq!(
        ForeignSpec::parse(Format::CliSchema, "nope").unwrap_err(),
        "failed to parse cli-schema document: expected ident at line 1 column 2"
    );
    assert_eq!(
        ForeignSpec::parse(Format::CliSchema, "[]").unwrap_err(),
        "cli-schema document root must be a JSON object"
    );
    assert_eq!(
        ForeignSpec::parse(Format::CliSchema, r#"{"schemaVersion": 1}"#).unwrap_err(),
        "cli-schema document is missing `name`"
    );
    assert_eq!(
        ForeignSpec::parse(Format::CliSchema, r#"{"schemaVersion": 2, "name": "t"}"#).unwrap_err(),
        "unsupported cli-schema version 2"
    );
    // A missing version marker is tolerated by the parser (detection
    // normally requires it).
    assert_eq!(
        ForeignSpec::parse(Format::CliSchema, r#"{"name": "t"}"#)
            .unwrap()
            .spec
            .name,
        "t"
    );
}

// ------------------------------------------------------------------
// load / round-trip
// ------------------------------------------------------------------

#[test]
fn load_reads_foreign_specs_and_skips_everything_else() {
    // A fig spec on disk ingests.
    let (_d, p) = write_file("gh.fig.json", r#"{"name": "gh", "subcommands": []}"#);
    let f = ForeignSpec::load(&p).unwrap().unwrap();
    assert_eq!(f.format, Format::Fig);
    assert_eq!(f.spec.name, "gh");

    // Content-sniffed cli-schema under a plain `.json` name.
    let (_d, p) = write_file(
        "tool.json",
        r#"{"schemaVersion": 1, "name": "t", "version": "0"}"#,
    );
    let f = ForeignSpec::load(&p).unwrap().unwrap();
    assert_eq!(f.format, Format::CliSchema);

    // Not a spec at all → None (the caller will probe it instead).
    let (_d, p) = write_file("data.json", r#"{"unrelated": true}"#);
    assert!(ForeignSpec::load(&p).unwrap().is_none());

    // Unreadable/non-UTF-8 → None.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bin");
    std::fs::write(&p, [0xff, 0xfe, 0xfd]).unwrap();
    assert!(ForeignSpec::load(&p).unwrap().is_none());
    // Missing file → None.
    assert!(
        ForeignSpec::load(Path::new("/no/such/file.fig.json"))
            .unwrap()
            .is_none()
    );
    // A claimed name with bad content is a hard error.
    let (_d, p) = write_file("bad.figspec", "{oops");
    assert!(
        ForeignSpec::load(&p)
            .unwrap_err()
            .contains("failed to parse fig spec")
    );
}

#[test]
fn foreign_specs_dump_and_reload() {
    for (fmt, src) in [
        (
            Format::Fig,
            r#"{"name": "gh", "description": "GitHub CLI",
                "subcommands": [{"name": "pr", "options": [{"name": ["-s", "--state"], "args": {"name": "state"}}], "args": {"name": "n"}}]}"#,
        ),
        (
            Format::CliSchema,
            r#"{"schemaVersion": 1, "name": "deploy", "version": "1",
                "commands": [{"name": "run", "parameters": [{"role": "positional", "name": "x", "type": "integer"}, {"role": "flag", "name": "f", "type": "string"}]}]}"#,
        ),
    ] {
        let spec = ForeignSpec::parse(fmt, src).unwrap().spec;
        let yaml = spec.dump();
        let (_d, p) = write_file("round.clispec.yaml", &yaml);
        let back = CliSpec::load(&p).unwrap();
        assert_eq!(back, spec, "{fmt:?} spec did not round-trip");
    }
}

#[test]
fn format_labels() {
    assert_eq!(Format::Fig.label(), "fig");
    assert_eq!(Format::CliSchema.label(), "cli-schema");
}
