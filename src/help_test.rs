use super::help::*;

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
            variadic: false
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
            variadic: true
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
                variadic: false
            },
            ParsedPositional {
                name: "opt".into(),
                variadic: false
            },
        ]
    );
    // The last variadic placeholder becomes `rest`.
    assert_eq!(
        p.rest,
        Some(ParsedPositional {
            name: "tail".into(),
            variadic: true
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
