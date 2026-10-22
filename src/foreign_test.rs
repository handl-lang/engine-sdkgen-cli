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

// ------------------------------------------------------------------
// usage (KDL) detection
// ------------------------------------------------------------------

#[test]
fn detect_usage_by_file_name() {
    for name in ["mise.usage.kdl", "tool.kdl"] {
        assert_eq!(
            ForeignSpec::detect(Path::new(name), "garbage {{{"),
            Some(Format::Usage),
            "{name}"
        );
    }
}

#[test]
fn detect_usage_by_content() {
    let p = Path::new("tool.txt");
    for src in [
        "flag \"-f --force\" help=\"Force\"",
        "cmd \"run\" { arg \"<x>\" }",
        "arg \"<file>\"",
        "option \"--opt <v>\"",
        "usage \"tool [OPTIONS]\"",
        "// a comment\nname mycli\nflag \"--x\"",
    ] {
        assert_eq!(ForeignSpec::detect(p, src), Some(Format::Usage), "{src}");
    }
    // KDL that carries no usage markers is not a spec; nor is KDL
    // that fails to parse, nor yaml-flavored text.
    for src in [
        "name mycli\nbin mycli",
        "name: mycli\ncommands:",
        "flag \"unterminated",
        "config { key value }",
    ] {
        assert_eq!(ForeignSpec::detect(p, src), None, "{src}");
    }
}

// ------------------------------------------------------------------
// usage (KDL) parsing — the grammar subset
// ------------------------------------------------------------------

use super::foreign::kdl::{self, KdlVal};

fn kdl(src: &str) -> Vec<kdl::KdlNode> {
    kdl::parse(src).unwrap()
}

#[test]
fn kdl_nodes_props_and_children() {
    let doc = kdl(r#"cmd "run" count=3 on=#true {
          flag "-v" ;
          arg "<x>"
        }"#);
    let run = &doc[0];
    assert_eq!(run.name, "cmd");
    assert_eq!(run.args, [KdlVal::Str("run".into())]);
    assert_eq!(run.prop("count"), Some(&KdlVal::Num("3".into())));
    assert_eq!(run.prop("on"), Some(&KdlVal::Bool(true)));
    assert_eq!(run.prop("missing"), None);
    assert_eq!(run.arg_str(0), Some("run"));
    assert_eq!(run.arg_str(3), None);
    let names: Vec<&str> = run.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["flag", "arg"]);
    let mut flags = run.children_named("flag");
    assert_eq!(flags.next().unwrap().arg_str(0), Some("-v"));
    assert!(flags.next().is_none());
    assert!(run.children_named("cmd").next().is_none());
}

#[test]
fn kdl_values() {
    let doc = kdl(
        "n 1 -2 +3 1.5 bare true false null #true #false #null #inf #-inf #nan a=b c = \"quoted\"",
    );
    let n = &doc[0];
    assert_eq!(
        n.args,
        [
            KdlVal::Num("1".into()),
            KdlVal::Num("-2".into()),
            KdlVal::Num("+3".into()),
            KdlVal::Num("1.5".into()),
            KdlVal::Str("bare".into()),
            KdlVal::Bool(true),
            KdlVal::Bool(false),
            KdlVal::Null,
            KdlVal::Bool(true),
            KdlVal::Bool(false),
            KdlVal::Null,
            KdlVal::Num("inf".into()),
            KdlVal::Num("-inf".into()),
            KdlVal::Num("nan".into()),
        ]
    );
    assert_eq!(n.prop("a"), Some(&KdlVal::Str("b".into())));
    assert_eq!(n.prop("c"), Some(&KdlVal::Str("quoted".into())));
}

#[test]
fn kdl_strings() {
    let doc = kdl(r#"n "a\nb\t\"q\"\\\/\s\u{41}""#);
    assert_eq!(doc[0].args[0], KdlVal::Str("a\nb\t\"q\"\\/ A".into()));
    // A `\` + newline folds inside a string; spaces after the
    // newline are content.
    let doc = kdl("n \"a\\\n  b\"");
    assert_eq!(doc[0].args[0], KdlVal::Str("a  b".into()));
}

#[test]
fn kdl_raw_and_multiline_strings() {
    let doc = kdl("n #\"a \\\" b\"#\nm ##\"x \"y\"\"##");
    assert_eq!(doc[0].args[0], KdlVal::Str("a \\\" b".into()));
    assert_eq!(doc[1].args[0], KdlVal::Str("x \"y\"".into()));
    // Multiline `"""` drops the opening newline and reads to `"""`.
    let doc = kdl("n \"\"\"\nline1\nline2\n\"\"\"");
    assert_eq!(doc[0].args[0], KdlVal::Str("line1\nline2\n".into()));
    // Raw multiline closes on `"""` + the opening hash count.
    let doc = kdl("a #\"\"\"\nx \"\"\" y\n\"\"\"#");
    assert_eq!(doc[0].args[0], KdlVal::Str("x \"\"\" y\n".into()));
}

#[test]
fn kdl_comments_and_elision() {
    let doc = kdl(
        "// line comment\na 1 /* block /* nested */ still */ b=2\n/- gone \"x\"\nc /- 3 4\n/-d 5\ne /- { f } g=1",
    );
    assert_eq!(doc[0].args, [KdlVal::Num("1".into())]);
    assert_eq!(doc[0].prop("b"), Some(&KdlVal::Num("2".into())));
    // `gone` is elided; `c` keeps only arg `4` (the `/- 3` entry is out).
    assert_eq!(doc[1].name, "c");
    assert_eq!(doc[1].args, [KdlVal::Num("4".into())]);
    // `/-d` elides node `d`; `e`'s `/- { … }` elides the child block.
    assert_eq!(doc[2].name, "e");
    assert!(doc[2].children.is_empty());
    assert_eq!(doc[2].prop("g"), Some(&KdlVal::Num("1".into())));
}

#[test]
fn kdl_continuations_terminators_and_types() {
    // `\` continues a node across lines; `;` ends one mid-line.
    let doc = kdl("a \\\n  \"x\" y=1; b 2\n(c)d (t)\"e\" (u)v=3");
    assert_eq!(doc[0].args, [KdlVal::Str("x".into())]);
    assert_eq!(doc[0].prop("y"), Some(&KdlVal::Num("1".into())));
    assert_eq!(doc[1].args, [KdlVal::Num("2".into())]);
    assert_eq!(doc[2].name, "d");
    assert_eq!(doc[2].args, [KdlVal::Str("e".into())]);
    assert_eq!(doc[2].prop("v"), Some(&KdlVal::Num("3".into())));
    // A quoted node name is legal.
    let doc = kdl("\"my node\" \"x\"");
    assert_eq!(doc[0].name, "my node");
}

#[test]
fn kdl_errors() {
    for (src, needle) in [
        ("}", "unmatched `}`"),
        ("a {", "unterminated `{` block"),
        ("a \"x", "unterminated string"),
        ("a \"\\q\"", "unknown escape `\\q`"),
        ("a \"x\\", "unterminated string"),
        ("a \"x\\ y\"", "expected a newline after `\\`"),
        ("a \"\\u0041\"", "expected `{` after `\\u`"),
        ("a \"\\u{110000}\"", "bad \\u{…} escape"),
        ("a \"\\u{41", "unterminated \\u{…} escape"),
        ("a #\"x", "unterminated raw string"),
        ("a #\"\"\"x", "unterminated raw string"),
        ("a #bogus", "unknown keyword `#bogus`"),
        ("a /* x", "unterminated `/*` comment"),
        ("a \\ ", "expected a newline after `\\`"),
        ("a (t", "unterminated `(type)` annotation"),
        ("a k=", "expected a value"),
        ("=x", "expected a node name"),
    ] {
        let e = kdl::parse(src).unwrap_err();
        assert!(e.contains(needle), "{src:?}: {e}");
    }
}

#[test]
fn kdl_value_helpers() {
    assert_eq!(KdlVal::Str("s".into()).as_str(), Some("s"));
    assert_eq!(KdlVal::Bool(true).as_str(), None);
    assert!(KdlVal::Bool(true).truthy());
    assert!(!KdlVal::Str("true".into()).truthy());
    assert_eq!(KdlVal::Str("s".into()).render(), Some("s".into()));
    assert_eq!(KdlVal::Num("3".into()).render(), Some("3".into()));
    assert_eq!(KdlVal::Bool(false).render(), Some("false".into()));
    assert_eq!(KdlVal::Null.render(), None);
}

// ------------------------------------------------------------------
// usage (KDL) mapping
// ------------------------------------------------------------------

#[test]
fn usage_maps_a_command_tree() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"name "My CLI"
        bin "mycli"
        alias "m"
        about "Process files"
        long_about "Longer docs."
        usage "mycli [OPTIONS] <COMMAND>"
        flag "-v --verbose" help="Verbose" global=#true
        flag "-u --user <user>" help="Run as user"
        arg "<dir>" help="Working dir"
        cmd "config" help="Manage config" {
          alias "cfg" "conf"
          cmd "add" help="Add an entry" {
            arg "<key>"
            flag "--force" help="Overwrite"
          }
        }"#,
    )
    .unwrap();
    assert_eq!(f.format, Format::Usage);
    let spec = &f.spec;
    assert_eq!(spec.name, "mycli");
    // `about` + `long_about` combine; root `alias` and the `usage`
    // synopsis annotate the module doc.
    assert_eq!(
        spec.about.as_deref(),
        Some("Process files\n\nLonger docs. [aliases: m] [usage: mycli [OPTIONS] <COMMAND>]")
    );
    let verbose = &spec.root.options["verbose"];
    assert_eq!(verbose.flag, "--verbose");
    assert_eq!(verbose.kind, "flag");
    assert_eq!(verbose.doc.as_deref(), Some("Verbose [aliases: -v]"));
    let user = &spec.root.options["user"];
    assert_eq!(user.kind, "string");
    assert_eq!(
        user.doc.as_deref(),
        Some("Run as user [aliases: -u] [value: user]")
    );
    assert_eq!(spec.root.positionals[0].name, "dir");
    let config = &spec.root.commands["config"];
    assert_eq!(
        config.about.as_deref(),
        Some("Manage config [aliases: cfg, conf]")
    );
    // The global `--verbose` lands on every command; `--user` does not.
    let add = &config.commands["add"];
    assert!(add.options.contains_key("verbose"));
    assert!(!add.options.contains_key("user"));
    assert_eq!(add.positionals[0].name, "key");
    assert_eq!(add.options["force"].doc.as_deref(), Some("Overwrite"));
}

#[test]
fn usage_flag_shapes() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "t"
        flag "--tag <t>" var=#true
        flag "--inc... <p>"
        flag "--x" var=#true
        flag "-v --verbose" count=#true
        flag "--user" { alias "-u" "-U"; arg "<user>" }
        flag "--range <start> <end>"
        flag "bare"
        flag 42
        option "--opt <v>" value_optional=#true"#,
    )
    .unwrap();
    let o = &f.spec.root.options;
    assert_eq!(o["tag"].kind, "strings");
    assert_eq!(o["inc"].kind, "strings");
    assert_eq!(o["x"].kind, "flag");
    assert_eq!(o["x"].doc.as_deref(), Some("[repeatable]"));
    assert_eq!(o["verbose"].doc.as_deref(), Some("[aliases: -v] [count]"));
    let user = &o["user"];
    assert_eq!(user.kind, "string");
    assert_eq!(user.doc.as_deref(), Some("[aliases: -u, -U] [value: user]"));
    assert_eq!(o["range"].kind, "string");
    assert_eq!(o["range"].doc.as_deref(), Some("[value: start]"));
    assert_eq!(o["opt"].doc.as_deref(), Some("[value: v] [optional value]"));
    for needle in [
        "flag `--range` takes 2 values — only the first is kept",
        "flag with no `-` spelling dropped",
        "non-string flag entry dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing `{needle}`: {:?}",
            f.warnings
        );
    }
}

#[test]
fn usage_flag_annotations() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "t"
        cmd "c" {
          flag "--file <f>" required=#true env="MY_FILE" default="f.txt" \
            delimiter="," var=#true var_min=1 var_max=3 action="help" \
            required_if="--mode" required_unless="--stdin" negate="--no-file" \
            exclusive=#true hide=#true deprecated="use --out" \
            validate="int(value) > 0" validate_error="must be positive" {
            conflicts "--a" "--b"
            requires "--c"
            overrides "--d"
            note "A note."
            warning "A warning."
          }
          flag "--old" deprecated=#true
          flag "--color <w>" { help_md "Pick a **color**"; long_help "Long form." }
        }"#,
    )
    .unwrap();
    let file = &f.spec.root.commands["c"].options["file"];
    assert_eq!(file.kind, "strings");
    assert_eq!(
        file.doc.as_deref(),
        Some(
            "[value: f] [required] [env: MY_FILE] [delimiter: ,] [default: f.txt] \
             [at least 1 values] [at most 3 values] [action: help] \
             [conflicts with: --a, --b] [overrides: --d] [requires: --c] \
             [required if: --mode] [required unless: --stdin] [negated by: --no-file] \
             [exclusive] [hidden] [deprecated: use --out] \
             [validation: int(value) > 0 (must be positive)] [note: A note.] [warning: A warning.]"
        )
    );
    let old = &f.spec.root.commands["c"].options["old"];
    assert_eq!(old.doc.as_deref(), Some("[deprecated]"));
    let color = &f.spec.root.commands["c"].options["color"];
    assert_eq!(
        color.doc.as_deref(),
        Some("Pick a **color**\n\nLong form. [value: w]")
    );
}

#[test]
fn usage_choices_and_flagset_use() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "t"
        flagset "common" {
          flag "--profile <p>" help="Profile"
          arg "[extra]"
          complete run="x"
        }
        cmd "a" {
          use "common"
          flag "--env <e>" { choices "dev" "prod" }
          arg "<word>" { choices "one" "two" }
        }
        cmd "b" { use "missing" }
        cmd "c" { use }
        flagset"#,
    )
    .unwrap();
    let a = &f.spec.root.commands["a"];
    assert_eq!(a.options["profile"].flag, "--profile");
    assert_eq!(a.positionals[0].name, "extra");
    assert_eq!(a.positionals[0].doc.as_deref(), Some("[optional]"));
    assert_eq!(
        a.options["env"].doc.as_deref(),
        Some("[value: e] [one of: dev, prod]")
    );
    assert_eq!(a.positionals[1].name, "word");
    assert_eq!(a.positionals[1].doc.as_deref(), Some("[one of: one, two]"));
    for needle in [
        "`use` names unknown flagset `missing`",
        "`use` with no flagset name dropped",
        "flagset `common` node `complete` dropped",
        "flagset with no name dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing `{needle}`: {:?}",
            f.warnings
        );
    }
}

#[test]
fn usage_args_become_positionals_and_rest() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "cp"
        arg "<source-file>" help="the source"
        arg "[dest]" default="." env="DEST" hide=#true
        arg "<start> <end>" var_min=2 var_max=2
        arg "<extras>..." deprecated=#true
        arg "[files]" var=#true
        arg "<ignored>..."
        arg "+sigil" sigil="+"
        arg
        arg "<dup>"
        arg "<dup>""#,
    )
    .unwrap();
    let root = &f.spec.root;
    let names: Vec<&str> = root.positionals.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["source_file", "dest", "start", "end", "dup", "dup_2"]
    );
    assert_eq!(root.positionals[0].doc.as_deref(), Some("the source"));
    assert_eq!(
        root.positionals[1].doc.as_deref(),
        Some("[optional] [default: .] [env: DEST] [hidden]")
    );
    assert_eq!(
        root.positionals[2].doc.as_deref(),
        Some("[at least 2 values] [at most 2 values]")
    );
    // `var=#true` puts the *last* token on the rest tail — `files`
    // arrives after `extras` claimed it, so it drops.
    let rest = root.rest.as_ref().unwrap();
    assert_eq!(rest.name, "extras");
    assert_eq!(rest.doc.as_deref(), Some("[deprecated]"));
    for needle in [
        "second variadic arg `files` dropped",
        "second variadic arg `ignored` dropped",
        "sigil arg `+sigil` dropped",
        "arg with no name dropped",
        "duplicate positional `dup` renamed `dup_2`",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing `{needle}`: {:?}",
            f.warnings
        );
    }
}

#[test]
fn usage_cmd_metadata() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "t"
        cmd "a" "extra" hide=#true subcommand_required=#true deprecated=#true {
          long_help "Long a."
          help_md "Md a."
          example "t a"
          mount run="t m"
          exec "x"
          flag "--g" global=#true
          cmd "deep" { flag "--h" global=#true; cmd "deeper" {} }
        }
        cmd "b" display_order=10 help_heading="H"
        cmd "same" { flag "--one" }
        cmd "same" { flag "--two" }
        cmd"#,
    )
    .unwrap();
    let a = &f.spec.root.commands["a"];
    assert_eq!(
        a.about.as_deref(),
        Some("Md a.\n\nLong a. [hidden] [subcommand required] [deprecated]")
    );
    for needle in [
        "cmd `a` extra args dropped: extra",
        "usage t.a: unsupported nodes dropped: example, exec, mount",
        "usage t: cmd `b`: unsupported props dropped: display_order, help_heading",
        "cmd with no name dropped",
    ] {
        assert!(
            f.warnings.iter().any(|w| w.contains(needle)),
            "missing `{needle}`: {:?}",
            f.warnings
        );
    }
    // A global declared on `a` reaches `deep` and `deeper`; `deep`'s
    // own global reaches only its subtree.
    let deep = &a.commands["deep"];
    assert!(deep.options.contains_key("g"));
    assert!(deep.options.contains_key("h"));
    let deeper = &deep.commands["deeper"];
    assert!(deeper.options.contains_key("g"));
    assert!(deeper.options.contains_key("h"));
    // `g` is global on `a` — `b` is outside `a`'s subtree, so neither
    // `g` nor `h` reaches it.
    assert!(!f.spec.root.commands["b"].options.contains_key("h"));
    assert!(!f.spec.root.commands["b"].options.contains_key("g"));
    // Same-named `cmd`s merge into one node.
    let same = &f.spec.root.commands["same"];
    assert!(same.options.contains_key("one") && same.options.contains_key("two"));
}

#[test]
fn usage_dropped_root_constructs_warn() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "t"
        output "json"
        example "t --help"
        mount run="t commands"
        complete run="x"
        exec "t"
        junk"#,
    )
    .unwrap();
    assert!(
        f.warnings.iter().any(|w| w.contains(
            "usage t: unsupported nodes dropped: complete, example, exec, junk, mount, output"
        )),
        "{:?}",
        f.warnings
    );
}

#[test]
fn usage_errors() {
    assert_eq!(
        ForeignSpec::parse(Format::Usage, "flag \"--x\"").unwrap_err(),
        "usage spec is missing `bin`/`name`"
    );
    assert!(
        ForeignSpec::parse(Format::Usage, "a {")
            .unwrap_err()
            .starts_with("failed to parse usage spec: line 1")
    );
    // `name` is the fallback when `bin` is absent.
    let f = ForeignSpec::parse(Format::Usage, "name \"Friendly\"").unwrap();
    assert_eq!(f.spec.name, "Friendly");
}

#[test]
fn usage_spec_round_trips_through_dump() {
    let f = ForeignSpec::parse(
        Format::Usage,
        r#"bin "gh" about "GH CLI"
        cmd "pr" { cmd "checkout" { arg "<n>"; flag "-b --branch <b>" } }"#,
    )
    .unwrap();
    let yaml = f.spec.dump();
    let (_d, p) = write_file("u.clispec.yaml", &yaml);
    let back = CliSpec::load(&p).unwrap();
    assert_eq!(back, f.spec);
}

// ------------------------------------------------------------------
// kdl lexer edges
// ------------------------------------------------------------------

#[test]
fn kdl_raw_strings_and_escapes() {
    use super::foreign::kdl;
    let nodes = kdl::parse("flag \"-f --force\" { arg r\"<val>\"; arg r##\"<v2>\"## }").unwrap();
    assert_eq!(nodes[0].name, "flag");
    let n = kdl::parse("flag \"a\\rb\" \"c\\bd\" \"e\\ff\"").unwrap();
    assert_eq!(n[0].args[0].as_str(), Some("a\rb"));
    assert_eq!(n[0].args[1].as_str(), Some("c\x08d"));
    assert_eq!(n[0].args[2].as_str(), Some("e\x0cf"));
}

#[test]
fn kdl_multiline_string_and_error() {
    use super::foreign::kdl;
    let ok = kdl::parse("cmd \"x\" { about \"\"\"two\nlines\"\"\" }").unwrap();
    assert_eq!(ok[0].children[0].name, "about");
    let err = kdl::parse("cmd \"x\" { about \"\"\"never ends").unwrap_err();
    assert!(err.contains("unterminated multiline"), "{err}");
}

#[test]
fn kdl_block_comment_and_ty_annot() {
    use super::foreign::kdl;
    let nodes = kdl::parse("/* lead */ cmd /* mid */ \"x\" { flag \"-a\" v=(int)\"5\" }").unwrap();
    assert_eq!(nodes[0].name, "cmd");
    let err = kdl::parse("flag \"-a\" v=(unclosed").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn kdl_entry_edges() {
    use super::foreign::kdl;
    let nodes = kdl::parse("cmd \"x\" a=1 b=true c=word d=\"str\"").unwrap();
    assert!(nodes[0].prop("a").is_some());
    assert!(nodes[0].prop("b").is_some());
    assert!(nodes[0].prop("c").is_some());
    assert!(nodes[0].prop("d").is_some());
}

#[test]
fn format_label_covers_usage() {
    assert_eq!(Format::Usage.label(), "usage");
    assert_eq!(Format::Fig.label(), "fig");
    assert_eq!(Format::CliSchema.label(), "cli-schema");
}
