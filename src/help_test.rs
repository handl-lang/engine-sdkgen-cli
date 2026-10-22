use super::help::*;
use std::collections::BTreeMap;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/help/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn cobra_style_usage_flags_and_subcommands() {
    let p = parse_help(&fixture("cobra_gh.txt"), 3);
    // `gh pr create [flags]` — the `[flags]` marker is skipped.
    assert!(p.positionals.is_empty());
    assert_eq!(
        p.options,
        vec![
            ParsedOption {
                name: "title".into(),
                flag: "--title".into(),
                kind: "string".into(),
            },
            ParsedOption {
                name: "body".into(),
                flag: "--body".into(),
                kind: "string".into(),
            },
            ParsedOption {
                name: "draft".into(),
                flag: "--draft".into(),
                kind: "flag".into(),
            },
            ParsedOption {
                name: "label".into(),
                flag: "--label".into(),
                kind: "string".into(),
            },
            ParsedOption {
                name: "jobs".into(),
                flag: "--jobs".into(),
                kind: "int".into(),
            },
            // Global Flags: section is an options table too.
            ParsedOption {
                name: "repo".into(),
                flag: "--repo".into(),
                kind: "string".into(),
            },
            ParsedOption {
                name: "verbose".into(),
                flag: "--verbose".into(),
                kind: "flag".into(),
            },
        ]
    );
    assert!(p.subcommands.is_empty());
    // The `See 'gh help ...'` footer is ignored.
}

#[test]
fn clap_style_options_marker_and_command_table() {
    let p = parse_help(&fixture("cargo_clap.txt"), 1);
    // `[OPTIONS]`/`[COMMAND]` are markers — no positionals.
    assert!(p.positionals.is_empty());
    assert!(p.rest.is_none());
    // `-h, --help` is filtered out; `--verbose...` loses its dots.
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["version", "verbose", "color", "jobs"]);
    assert_eq!(p.options[1].flag, "--verbose");
    assert_eq!(p.options[2].kind, "string");
    assert_eq!(p.options[3].kind, "int");
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["build", "check", "run", "test"]);
    assert_eq!(p.subcommands[0].1, "Compile the current package");
}

#[test]
fn getopt_style_positional_rest_and_bracket_value() {
    let p = parse_help(&fixture("git_checkout.txt"), 2);
    assert_eq!(
        p.positionals,
        vec![ParsedPositional {
            name: "branch".into(),
            variadic: false,
            doc: None
        }]
    );
    // The `or:` continuation usage line is ignored — `<file>...` never
    // becomes a rest arg.
    assert!(p.rest.is_none());
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        ["b", "force", "track", "detach", "orphan", "dry_run"]
    );
    assert_eq!(p.options[0].flag, "-b");
    assert_eq!(p.options[0].kind, "string");
    assert_eq!(p.options[2].flag, "--track");
    assert_eq!(p.options[2].kind, "string");
    assert_eq!(p.options[5].flag, "--dry-run");
    assert_eq!(p.options[5].kind, "flag");
}

#[test]
fn argparse_style_choice_group_gives_subcommands() {
    let p = parse_help(&fixture("argparse.txt"), 1);
    // `[-h]` is an option placeholder, not a positional.
    assert!(p.positionals.is_empty());
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["clone", "pull", "push"]);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    // `--help` filtered; `depth N` is int; `--json` a flag.
    assert_eq!(names, ["depth", "json"]);
    assert_eq!(p.options[0].kind, "int");
    assert_eq!(p.options[1].kind, "flag");
}

#[test]
fn usage_only_fixture_gives_a_rest() {
    let p = parse_help(&fixture("minimal.txt"), 1);
    assert!(p.positionals.is_empty());
    assert_eq!(
        p.rest,
        Some(ParsedPositional {
            name: "text".into(),
            variadic: true,
            doc: None
        })
    );
    assert!(p.options.is_empty());
    assert!(p.subcommands.is_empty());
}

#[test]
fn empty_and_sectionless_input_parse_to_nothing() {
    let p = parse_help("", 1);
    assert!(p.positionals.is_empty() && p.options.is_empty() && p.subcommands.is_empty());
    let p = parse_help("just some prose\nmore prose\n", 1);
    assert!(p.options.is_empty());
}

#[test]
fn usage_variants_all_parse() {
    // `<x>` mandatory, `[x]` optional, `x...` variadic, `<x>...`.
    let p = parse_help("usage: tool <in> [opt] rest... <tail>...\n", 1);
    assert_eq!(
        p.positionals,
        vec![
            ParsedPositional {
                name: "in".into(),
                variadic: false,
                doc: None
            },
            ParsedPositional {
                name: "opt".into(),
                variadic: false,
                doc: None
            },
        ]
    );
    // The last variadic placeholder becomes `rest`.
    assert_eq!(
        p.rest,
        Some(ParsedPositional {
            name: "tail".into(),
            variadic: true,
            doc: None
        })
    );
}

#[test]
fn duplicate_positional_names_dedup() {
    let p = parse_help("usage: prog FILE [FILE] out\n", 1);
    let names: Vec<&str> = p.positionals.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["file", "out"]);
}

#[test]
fn unnameable_and_marker_tokens_are_skipped() {
    let p = parse_help("usage: tool [<a|b>] [opts] --literal ...\n", 1);
    assert!(p.positionals.is_empty());
    assert!(p.rest.is_none()); // bare `...` has no name
}

#[test]
fn non_option_rows_in_tables_are_ignored() {
    let text = "Flags:\n  notaflag description\n  --real x\n\nProse paragraph\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 1);
    assert_eq!(p.options[0].name, "real");
}

#[test]
fn command_rows_skip_flags_and_bullets() {
    let text = "Commands:\n  add    Add it\n  -v     not a command\n  * star bullet\n  !bang  not a word\n  \n";
    let p = parse_help(text, 1);
    assert_eq!(
        p.subcommands,
        vec![("add".to_string(), "Add it".to_string())]
    );
}

#[test]
fn flagless_and_equals_value_forms() {
    // `--opt=VAL`, long-only, short-with-value, `[`-attached metavar.
    let text = "Options:\n  --opt=VAL\n  --plain\n  -q COUNT\n  --try[V]   optional value\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 4);
    assert_eq!(p.options[0].kind, "string");
    assert_eq!(p.options[1].kind, "flag");
    assert_eq!(p.options[2].flag, "-q");
    assert_eq!(p.options[2].kind, "int");
    assert_eq!(p.options[3].flag, "--try");
    assert_eq!(p.options[3].kind, "string");
}

#[test]
fn option_row_without_flag_or_name_is_none() {
    let p = parse_help("Flags:\n  --    lone dashes\n", 1);
    assert!(p.options.is_empty());
}

#[test]
fn unrecognized_headers_close_sections() {
    // `Examples:` is not an options table; its rows stay unparsed even
    // though they contain dashes.
    let text = "Flags:\n  --a    flag a\n\nExamples:\n  --not-a-flag\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 1);
}

#[test]
fn go_style_usage_and_commands_header() {
    let p = parse_help(&fixture("go-help.txt"), 1);
    // `Usage:` alone, a blank line, then an indented `go <command>` —
    // the multi-line usage block resolves and `command`/`arguments`
    // are markers, not positionals.
    assert!(p.has_usage);
    assert!(p.positionals.is_empty());
    assert!(p.rest.is_none());
    // `The commands are:` is a commands table.
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert!(subs.contains(&"build") && subs.contains(&"vet"), "{subs:?}");
    assert_eq!(p.subcommands[0].1, "start a bug report");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.confidence(), "high");
}

#[test]
fn argparse_positional_section_annotates_group_and_docs() {
    let p = parse_help(&fixture("argparse.txt"), 1);
    // The `{clone,pull,push}` positional row describes the usage-group
    // subcommands — no unmatched-group warning, and the desc lands.
    let clone = p.subcommands.iter().find(|(n, _)| n == "clone").unwrap();
    assert_eq!(clone.1, "action to perform");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn positional_arguments_rows_doc_the_positionals() {
    let text = "usage: tool <src> <dst> [files]...\n\npositional arguments:\n  src     input file\n  dst     output file\n  files   extra files\n";
    let p = parse_help(text, 1);
    assert_eq!(p.positionals[0].doc.as_deref(), Some("input file"));
    assert_eq!(p.positionals[1].doc.as_deref(), Some("output file"));
    assert_eq!(p.rest.as_ref().unwrap().doc.as_deref(), Some("extra files"));
}

#[test]
fn arguments_and_args_headers_are_positional_sections() {
    for head in ["Arguments:", "Args:"] {
        let text = format!("usage: tool <in>\n\n{head}\n  in   the input\n");
        let p = parse_help(&text, 1);
        assert_eq!(p.positionals[0].doc.as_deref(), Some("the input"), "{head}");
    }
}

#[test]
fn multiline_usage_continuation_lines_parse_as_alternates() {
    let text = "Usage: prog [opts] <a>\n       prog [opts] <b>\n       (annotation, not usage)\n\nFlags:\n  -f   flag\n";
    let p = parse_help(text, 1);
    let names: Vec<&str> = p.positionals.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    // The parenthesized annotation is consumed but not parsed.
    assert!(!names.contains(&"annotation"));
}

#[test]
fn single_dash_long_options_parse() {
    let p = parse_help(&fixture("openssl-s-help.txt"), 1);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    // `Valid options are:` is an options table; `-cipher` is a long
    // flag spelled with one dash.
    assert!(names.contains(&"cipher"), "{names:?}");
    assert!(names.contains(&"verify"), "{names:?}");
    assert!(names.contains(&"nbio"), "{names:?}");
    let cipher = p.options.iter().find(|o| o.name == "cipher").unwrap();
    assert_eq!(cipher.flag, "-cipher");
    assert_eq!(cipher.kind, "string");
    assert_eq!(p.confidence(), "high");
}

#[test]
fn brace_group_unmatched_by_table_warns() {
    // `{a|b}` in usage but no commands table describing them.
    let p = parse_help("usage: prog {a|b}\n\nFlags:\n  -x   x\n", 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unmatched") && w.contains("`a`, `b`")),
        "{:?}",
        p.warnings
    );
    // A commands table row satisfies the match.
    let p = parse_help(
        "usage: prog {a|b}\n\nCommands:\n  a  does a\n  b  does b\n",
        1,
    );
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.subcommands[0].1, "does a");
}

#[test]
fn missing_usage_line_warns() {
    let p = parse_help("Flags:\n  -x   x\n", 1);
    assert!(p.warnings.iter().any(|w| w.contains("no usage line")));
}

#[test]
fn empty_options_section_warns() {
    let text = "Usage: t\n\nFlags:\n  notaflag nope\n\nCommands:\n  go  go\n";
    let p = parse_help(text, 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("options section `Flags` produced no option rows")),
        "{:?}",
        p.warnings
    );
    assert_eq!(p.subcommands.len(), 1);
}

#[test]
fn flag_like_rows_that_fail_parse_warn() {
    let text = "Usage: t\n\nFlags:\n  --    lone dashes\n  -x    ok\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unparsed flag-like row")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn multi_flag_rows_warn_about_dropped_spellings() {
    let text = "Usage: t\n\nOptions:\n  -D -O -P -debug   bundled\n  -v, --verbose   v\n";
    let p = parse_help(text, 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("several flag spellings")),
        "{:?}",
        p.warnings
    );
    // `-v, --verbose` is the canonical pair — no warning for it.
    assert_eq!(p.warnings.len(), 1);
    assert_eq!(p.options[0].name, "debug");
    assert_eq!(p.options[1].name, "verbose");
}

#[test]
fn unknown_headers_carrying_rows_warn() {
    let text =
        "Usage: t\n\nExamples:\n  foo bar   an example\n  baz qux   another\n\nFlags:\n  -x   x\n";
    let p = parse_help(text, 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unrecognized section `Examples` carried 2 table rows")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn headerless_stray_rows_warn() {
    let text = "Usage: t [x]...\n\n  bs=BYTES    block size\n  if=FILE     input\n\n  --help      show help\n";
    let p = parse_help(text, 1);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("2 table rows appeared outside a recognized section")),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("1 flag-like rows appeared outside an options section")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn confidence_levels() {
    // Options/subcommands absent → low even with a clean parse.
    assert_eq!(parse_help("Usage: echo [TEXT]...\n", 1).confidence(), "low");
    // Clean parse with options → high.
    assert_eq!(
        parse_help("Usage: t\n\nFlags:\n  -x   x\n", 1).confidence(),
        "high"
    );
    // Usable but warned → medium.
    assert_eq!(
        parse_help("Usage: t\n\nFlags:\n  --    bad\n  -x   ok\n", 1).confidence(),
        "medium"
    );
    // Unusable → !is_usable.
    assert!(!parse_help("just prose\n", 1).is_usable());
    assert!(parse_help("Usage: t\n", 1).is_usable());
}

#[test]
fn indented_colon_headers_recognize_sections() {
    // java-style: ` where options include:` is indented one space.
    let text = "Usage: java [options] <mainclass> [args...]\n\n where options include:\n\n    -version      print version\n    -cp <path>    class path\n";
    let p = parse_help(text, 1);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["version", "cp"], "{names:?}");
    assert_eq!(p.options[0].flag, "-version");
    assert_eq!(p.options[1].kind, "string");
}

#[test]
fn or_lines_usage_without_body_and_bad_positional_rows() {
    // `or:` outside a usage block is ignored outright.
    let p = parse_help("or: not usage\nUsage: t\n\nFlags:\n  -x   x\n", 1);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);

    // `Usage:` alone followed by an unindented line has no body.
    let p = parse_help("Usage:\nFlags:\n  -x   x\n", 1);
    assert!(p.has_usage);
    assert_eq!(p.options.len(), 1);

    // Positional-section rows that are not metavars are skipped;
    // a bare name still yields a doc-free row.
    let text = "usage: t <in>\n\npositional arguments:\n  in     the input\n  -v     not a metavar\n  {}\n  []\n  NODE\n";
    let p = parse_help(text, 1);
    assert_eq!(p.positionals[0].doc.as_deref(), Some("the input"));
}

#[test]
fn usage_opt_group_metavar_and_empty_brace_parts() {
    // `[-D debug opts]` — the bracket group swallows every token to `]`.
    let p = parse_help("usage: t [-D debug opts] <in>\n", 1);
    let names: Vec<&str> = p.positionals.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["in"], "{names:?}");
    // `{a||b}` — an empty choice part is skipped, not made a subcommand.
    let p = parse_help("usage: t {a||b}\n\nCommands:\n  a  a\n  b  b\n", 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["a", "b"]);
}

#[test]
fn parenthesized_section_headers_strip_the_annotation() {
    // `(always true)`-style qualifiers are cut before suffix matching.
    let text = "Usage: t\n\noptional arguments (always true):\n  --x   x\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 1, "{:?}", p.warnings);
    assert_eq!(p.options[0].flag, "--x");

    // A parenthesized commands table header also lands.
    let text = "Usage: t <cmd>\n\nAvailable commands (built-in):\n  run   run it\n";
    let p = parse_help(text, 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["run"], "{:?}", p.warnings);
}

#[test]
fn unknown_section_prose_rows_are_ignored_quietly() {
    // A `Section::None` row that is neither flag-like nor a table row is
    // skipped without counting toward `stray_rows`/`unknown`.
    let text =
        "Usage: t\n\nExamples:\n  run the thing and see\n  plain prose\n\nFlags:\n  -x   x\n";
    let p = parse_help(text, 1);
    assert!(
        p.warnings.is_empty(),
        "prose should not count as table rows: {:?}",
        p.warnings
    );
}

#[test]
fn equals_tokens_without_flag_lhs_or_empty_value() {
    // `KEY=VAL` is a metavar, not a flag; `--x=` carries an empty value.
    let text = "Usage: t\n\nOptions:\n  -v KEY=VAL   var\n  --x=         empty\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 2);
    assert_eq!(p.options[0].flag, "-v");
    assert_eq!(p.options[0].kind, "string");
    assert_eq!(p.options[1].flag, "--x");
}

#[test]
fn cobra_colon_command_rows_strip_the_colon() {
    // gh spells command-table rows `agent-task:   Work with ...` — the
    // colon is punctuation; keeping it re-probes `gh agent-task:` which
    // cobra answers with an error page re-listing the ROOT table.
    let text = "Usage: gh <command>\n\nAvailable Commands:\n  agent-task:   Work with agent tasks\n  pr:   Manage pull requests\n";
    let p = parse_help(text, 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["agent-task", "pr"], "{:?}", p.warnings);
}

#[test]
fn error_output_is_not_help_text() {
    // `gh nosuch --help` exits 0 with `unknown command` + the root
    // table on stderr — usable-looking but bogus. Parsing it must not
    // yield subcommands/options, so the help-flag fallback can move on.
    let text = "unknown command \"nosuch\" for \"gh\"\n\nUsage:  gh <command>\n\nAvailable commands:\n  pr\n  run\n";
    let p = parse_help(text, 1);
    assert!(!p.is_usable());
    assert!(p.options.is_empty() && p.subcommands.is_empty());
    assert!(p.warnings[0].contains("unknown command"));

    let text2 = "Error: something failed\n\nUsage: t\n\nOptions:\n  -x   x\n";
    assert!(!parse_help(text2, 1).is_usable());
    let text3 = "error: bad input\n\nUsage: t\n\nOptions:\n  -x   x\n";
    assert!(!parse_help(text3, 1).is_usable());
}

#[test]
fn kubectl_grouped_usage_and_blank_separated_options() {
    // Real `kubectl create cronjob --help`: the usage line mixes
    // `NAME` with `--image=image`, a quoted `--schedule='0/5 * * * ?'`,
    // and a `-- [COMMAND] [args...]` tail — only `name` is a
    // positional. The `Options:` table blank-separates every row from
    // its description paragraph; all rows still parse, and
    // `=false`-style defaults read as `flag` kind.
    let p = parse_help(&fixture("kubectl-create-cronjob.txt"), 3);
    assert_eq!(
        p.positionals,
        vec![ParsedPositional {
            name: "name".into(),
            variadic: false,
            doc: None
        }],
        "{:?}",
        p.positionals
    );
    assert!(p.rest.is_none());
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["image"], "string");
    assert_eq!(kinds["schedule"], "string");
    assert_eq!(kinds["restart"], "string");
    assert_eq!(kinds["dry_run"], "string");
    assert_eq!(kinds["output"], "string");
    assert_eq!(kinds["allow_missing_template_keys"], "flag");
    assert_eq!(kinds["save_config"], "flag");
    assert_eq!(kinds["show_managed_fields"], "flag");
    assert_eq!(p.options.len(), 11, "{:?}", p.options);
    assert_eq!(
        p.options.iter().find(|o| o.name == "output").unwrap().flag,
        "--output"
    );
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.confidence(), "high");
}

#[test]
fn kubectl_get_alternation_groups_do_not_leak_positionals() {
    // Real `kubectl get --help`: `[(-o|--output=)json|yaml|…]` and
    // `(TYPE[.VERSION][.GROUP] [NAME | -l label] | TYPE/NAME …)` are
    // grouped alternations — skipping them whole is what stops `f`,
    // `l`, `label`, `type_version_group_name` leaking as positionals.
    let p = parse_help(&fixture("kubectl-get.txt"), 2);
    assert!(p.positionals.is_empty(), "{:?}", p.positionals);
    assert!(p.rest.is_none());
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["output"], "string");
    assert_eq!(kinds["selector"], "string");
    assert_eq!(kinds["field_selector"], "string");
    assert_eq!(kinds["filename"], "string");
    assert_eq!(kinds["chunk_size"], "int");
    assert_eq!(kinds["watch"], "flag");
    assert_eq!(kinds["all_namespaces"], "flag");
    assert_eq!(kinds["no_headers"], "flag");
    assert_eq!(kinds["server_print"], "flag");
    assert_eq!(p.options.len(), 23, "{:?}", p.options);
    assert!(
        p.warnings.iter().any(|w| w.contains("alternation groups")),
        "{:?}",
        p.warnings
    );
    assert_eq!(p.confidence(), "medium");
}

#[test]
fn kubectl_set_resources_paren_groups_are_skipped() {
    // Real `kubectl set resources --help`: `(-f FILENAME | TYPE NAME)`
    // plus a never-closed `([--limits=… & --requests=…]` group — the
    // whole tail is alternation-shaped and yields no positionals.
    let p = parse_help(&fixture("kubectl-set-resources.txt"), 3);
    assert!(p.positionals.is_empty(), "{:?}", p.positionals);
    assert!(p.rest.is_none());
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["limits"], "string");
    assert_eq!(kinds["requests"], "string");
    assert_eq!(kinds["containers"], "string");
    assert_eq!(kinds["filename"], "string");
    assert_eq!(kinds["local"], "flag");
    assert_eq!(kinds["all"], "flag");
    assert!(
        p.warnings.iter().any(|w| w.contains("alternation groups")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn kubectl_bool_default_metavars_are_flags() {
    // Real `kubectl version --help`: `--client=false:` is a boolean
    // flag with a printed default, not a `string` option.
    let p = parse_help(&fixture("kubectl-version.txt"), 2);
    assert_eq!(p.options.len(), 2, "{:?}", p.options);
    let client = p.options.iter().find(|o| o.name == "client").unwrap();
    assert_eq!(client.flag, "--client");
    assert_eq!(client.kind, "flag");
    assert_eq!(
        p.options.iter().find(|o| o.name == "output").unwrap().kind,
        "string"
    );
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn git_prose_command_sections_promote_to_subcommands() {
    // Real `git --help`: `start a working area (see also: …)` and
    // friends are prose headers — their indented `name   desc` rows
    // are command-shaped, so each block becomes a command list.
    let p = parse_help(&fixture("git-help.txt"), 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs.len(), 24, "{subs:?}");
    for want in [
        "clone", "init", "add", "commit", "fetch", "pull", "push", "tag",
    ] {
        assert!(subs.contains(&want), "{want} missing from {subs:?}");
    }
    assert_eq!(
        p.subcommands.iter().find(|(n, _)| n == "clone").unwrap().1,
        "Clone a repository into a new directory"
    );
    assert!(p.positionals.is_empty());
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.confidence(), "high");
    assert!(p.is_usable());
}

#[test]
fn git_help_a_recognized_and_prose_sections_merge() {
    // `git help -a` mixes `… Commands` headers (recognized) with
    // `Ancillary Commands / Manipulators`-style prose headers
    // (promoted) and bare-name rows under `External commands`.
    let p = parse_help(&fixture("git-help-a.txt"), 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    for want in [
        "add",
        "checkout",
        "config",
        "reflog",
        "annotate",
        "fsck",
        "credential-gcloud.sh",
        "filter-repo",
    ] {
        assert!(subs.contains(&want), "{want} missing from {subs:?}");
    }
    assert_eq!(subs.len(), 27, "{subs:?}");
    assert_eq!(
        p.subcommands.iter().find(|(n, _)| n == "reflog").unwrap().1,
        "Manage reflog information"
    );
    // No usage line — `help -a` is a table listing, not a help page.
    assert!(!p.has_usage);
    assert_eq!(p.confidence(), "medium");
}

#[test]
fn aws_man_page_parses_synopsis_options_and_service_bullets() {
    // Real `aws help`: groff man output — SGR escapes, `SYNOPSIS`
    // usage, `Global Options` as `--flag (type)` rows blank-separated
    // from their paragraphs, `* <svc>` bullets under `Available
    // Services`.
    let p = parse_help(&fixture("aws-help.txt"), 1);
    assert!(p.has_usage);
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["debug"], "flag");
    assert_eq!(kinds["no_verify_ssl"], "flag");
    assert_eq!(kinds["endpoint_url"], "string");
    assert_eq!(kinds["region"], "string");
    assert_eq!(kinds["cli_read_timeout"], "int");
    assert_eq!(kinds["cli_connect_timeout"], "int");
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["accessanalyzer", "acm", "acm-pca", "amplify"] {
        assert!(subs.contains(&want), "{want} missing from {subs:?}");
    }
    // `* json`-style value bullets under `--output` stay inside the
    // options section — they never become subcommands.
    assert!(!subs.contains(&"json"), "{subs:?}");
    assert_eq!(p.confidence(), "medium");
}

#[test]
fn aws_error_page_is_rejected_as_error_output() {
    // `aws --help` answers a ParamValidation error banner + usage —
    // usable-looking but bogus; the probe must fall through to `help`.
    let text = "aws: [ERROR]: An error occurred (ParamValidation): the following arguments are required: command\n\nusage: aws [options] <command> <subcommand> [parameters]\n";
    let p = parse_help(text, 1);
    assert!(!p.is_usable());
    assert!(p.warnings[0].contains("[ERROR]"));
}

#[test]
fn synopsis_header_leads_a_usage_block() {
    let text = "Name\n     t -\n\nSynopsis\n        t [options] <file> [parameters]\n\nOptions\n  --x  x\n";
    let p = parse_help(text, 1);
    assert!(p.has_usage);
    // `<file>` is a positional; `[options]`/`[parameters]` are markers.
    assert_eq!(p.positionals.len(), 1);
    assert_eq!(p.positionals[0].name, "file");
    assert_eq!(p.options.len(), 1);
}

#[test]
fn ansi_escapes_are_stripped_before_parsing() {
    let text = "\u{1b}[1mOptions\u{1b}[0m\n  \u{1b}[1m--debug \u{1b}[22m(boolean)\n\nCommands:\n  run  go\n";
    let p = parse_help(text, 1);
    assert_eq!(p.options.len(), 1, "{:?}", p.warnings);
    assert_eq!(p.options[0].flag, "--debug");
    assert_eq!(p.options[0].kind, "flag");
    assert_eq!(p.subcommands.len(), 1);
}

#[test]
fn usage_quoted_option_values_stay_single_tokens() {
    // `--schedule='0/5 * * * ?'` is one option token — the quoted `*`
    // and `?` words must not leak as `''` positionals.
    let p = parse_help(
        "usage: t NAME --image=x --schedule='0/5 * * * ?' -- [args...]\n",
        1,
    );
    assert_eq!(
        p.positionals,
        vec![ParsedPositional {
            name: "name".into(),
            variadic: false,
            doc: None
        }]
    );
    assert!(p.rest.is_none());
}

#[test]
fn usage_paren_group_and_assignments_and_punct_tokens() {
    // `(-f F | TYPE NAME)` alternation skipped wholesale; `KEY=VAL`
    // names its left side; `*`/`?`/`)` cannot name a positional.
    let p = parse_help("usage: t (-f F | -l L) KEY_1=VAL_1 * ? ] [\n", 1);
    assert_eq!(
        p.positionals,
        vec![ParsedPositional {
            name: "key_1".into(),
            variadic: false,
            doc: None
        }],
        "{:?}",
        p.positionals
    );
    assert!(
        p.warnings.iter().any(|w| w.contains("alternation groups")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn options_section_survives_blank_separated_rows() {
    // kubectl/man-page tables put a blank plus a description
    // paragraph between rows — the section stays open while the next
    // non-blank line is indented.
    let text = "Usage: t\n\nOptions:\n    --a=x:\n\tFirst flag.\n\n    --b=y:\n\tSecond flag.\n\nNext:\n  z\n";
    let p = parse_help(text, 1);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["a", "b"], "{names:?}");
}

#[test]
fn prose_sections_of_command_rows_promote_only_when_majority() {
    // Two command-shaped rows out of two → a command list.
    let p = parse_help(
        "usage: t\n\nsome prose head\n  aa   does a\n  bb   does b\n",
        1,
    );
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["aa", "bb"], "{subs:?}");

    // A lone command-shaped row among prose-ish rows stays a stray.
    let p = parse_help(
        "usage: t\n\nExamples:\n  t aa   like a command\n  foo bar   an example\n  t bb   another one\n",
        1,
    );
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unrecognized section `Examples` carried 3 table rows")),
        "{:?}",
        p.warnings
    );
    assert!(p.subcommands.is_empty(), "{:?}", p.subcommands);
}

#[test]
fn bullet_rows_promote_and_single_row_blocks_do_not() {
    // `* name` bullets dominate → subcommands (aws service shape).
    let text =
        "usage: t <cmd>\n\nThings available\n  * alpha\n  * beta\n\nSee Also\n  * t help topics\n";
    let p = parse_help(text, 1);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs, ["alpha", "beta"], "{subs:?}");
    // The one-row `See Also` block is below the promotion threshold.
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("1 table rows appeared outside a recognized section")),
        "{:?}",
        p.warnings
    );
}

#[test]
fn sentence_lines_are_not_section_headers() {
    // `Use "kubectl options" for a list ….` is a sentence, not an
    // options header — it must not spawn an empty options section.
    let text = "Usage: t\n\nOptions:\n  --a   a\n\nUse \"t options\" for a list of global command-line options (applies to all commands).\n";
    let p = parse_help(text, 1);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.options.len(), 1);
}

#[test]
fn bool_and_numeric_metavars_map_kinds() {
    let text = "Options:\n  --flag=false:   on/off\n  --sw (boolean)    switch\n  --n=42:           count\n  --depth <n>       deep\n";
    let p = parse_help(text, 1);
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["flag"], "flag");
    assert_eq!(kinds["sw"], "flag");
    assert_eq!(kinds["n"], "int");
    assert_eq!(kinds["depth"], "int");
}

#[test]
fn runs_of_blank_lines_inside_a_section_are_tolerated() {
    // Multiple blanks in a row between rows keep the section alive the
    // same way one does.
    let text =
        "Usage: t\n\nOptions:\n  --a=x:\n\tdesc a\n\n\n    --b=y:\n\tdesc b\n\n\nTail:\n  z\n";
    let p = parse_help(text, 1);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["a", "b"], "{names:?}");
}

#[test]
fn kubectl_global_options_page_parses() {
    // `kubectl options` heads its table `The following options can be
    // passed to any command:` — the header suffix rule lands it in
    // Options and every row parses (boolean defaults → flag).
    let p = parse_help(&fixture("kubectl-options.txt"), 2);
    assert!(p.options.len() >= 25, "{}", p.options.len());
    let kinds: BTreeMap<&str, &str> = p
        .options
        .iter()
        .map(|o| (o.name.as_str(), o.kind.as_str()))
        .collect();
    assert_eq!(kinds["namespace"], "string");
    assert_eq!(kinds["server"], "string");
    assert_eq!(kinds["as"], "string");
    assert_eq!(kinds["warnings_as_errors"], "flag");
    // The standalone options page carries no usage line — that is the
    // only warning.
    assert_eq!(p.warnings, vec!["no usage line found".to_string()]);
}
