//! Real-world `--help` violators (aCLImatise/cli-to-js corpus shapes):
//! go's `The commands are:`, dd's `operand=value` grammar, find's
//! expression tables, openssl's single-dash longs, tar's named mode
//! sections, java's alternate-form usage and indented `where options
//! include:`. Every fixture must parse without panicking and report
//! the confidence its grammar deserves.

use handl_plugin_codegen_cli::parse_help;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/help/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn go_help_parses_commands_table_cleanly() {
    let p = parse_help(&fixture("go-help.txt"), 1);
    assert!(p.has_usage);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let subs: Vec<&str> = p.subcommands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(subs.len(), 18, "{subs:?}");
    assert_eq!(
        p.subcommands[0],
        ("bug".to_string(), "start a bug report".to_string())
    );
    assert_eq!(p.confidence(), "high");
}

#[test]
fn gnu_dd_is_low_confidence_with_warnings_and_zero_options() {
    let p = parse_help(&fixture("gnu-dd.txt"), 1);
    assert!(p.has_usage);
    assert!(p.options.is_empty(), "{:?}", p.options);
    assert_eq!(p.rest.as_ref().unwrap().name, "operand");
    // The operand table and the headerless --help/--version rows warn.
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("table rows appeared outside a recognized section")),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("flag-like rows appeared outside an options section")),
        "{:?}",
        p.warnings
    );
    assert_eq!(p.confidence(), "low");
}

#[test]
fn gnu_find_recovers_usage_and_partial_options() {
    let p = parse_help(&fixture("gnu-find.txt"), 1);
    assert!(p.has_usage);
    assert_eq!(p.rest.as_ref().unwrap().name, "path");
    assert_eq!(p.positionals[0].name, "expression");
    // The `operators:`/`tests:`/`actions:` expression tables do not
    // fit the option grammar — they warn.
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unrecognized section")),
        "{:?}",
        p.warnings
    );
    // Single-dash longs in the `options (…)` sections do parse.
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"daystart"), "{names:?}");
    assert_eq!(p.confidence(), "medium");
}

#[test]
fn openssl_single_dash_options_parse_to_high_confidence() {
    let p = parse_help(&fixture("openssl-s-help.txt"), 1);
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"cipher"), "{names:?}");
    assert!(names.contains(&"verify_return_error"), "{names:?}");
    assert!(names.contains(&"4"), "{names:?}");
    assert_eq!(
        p.options.iter().find(|o| o.name == "cipher").unwrap().flag,
        "-cipher"
    );
    assert_eq!(p.confidence(), "high");
}

#[test]
fn tar_named_mode_sections_warn_and_stay_low_confidence() {
    let p = parse_help(&fixture("tar.txt"), 1);
    assert_eq!(p.rest.as_ref().unwrap().name, "file");
    assert!(p.options.is_empty(), "{:?}", p.options);
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unrecognized section `Main operation mode`")),
        "{:?}",
        p.warnings
    );
    assert!(
        p.warnings
            .iter()
            .any(|w| w.contains("unrecognized section `Operation modifiers`")),
        "{:?}",
        p.warnings
    );
    assert_eq!(p.confidence(), "low");
}

#[test]
fn java_alternate_usage_and_indented_header_parse() {
    let p = parse_help(&fixture("java.txt"), 1);
    assert!(p.has_usage);
    // The indented `where options include:` table parses — single-dash
    // longs and `-D<name>=<value>` shapes included.
    let names: Vec<&str> = p.options.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"version"), "{names:?}");
    assert!(names.contains(&"cp"), "{names:?}");
    assert!(names.contains(&"verbose"), "{names:?}");
    // No flag ever carries a `:` — `-verbose:[...]` is stripped.
    assert!(p.options.iter().all(|o| !o.flag.contains(':')));
    assert_eq!(p.confidence(), "high");
}

#[test]
fn top_level_or_line_and_unindented_usage_candidate() {
    // `or:` outside a usage continuation is skipped outright; a bare
    // `Usage:` followed by an unindented line keeps an empty body.
    let p = parse_help("or: alt form\n\nUsage:\nFlags:\n  -x   x\n", 1);
    assert!(p.has_usage);
    assert_eq!(p.options.len(), 1);
}

#[test]
fn kubectl_cronjob_parses_to_high_confidence() {
    let p = parse_help(&fixture("kubectl-create-cronjob.txt"), 3);
    assert_eq!(p.positionals.len(), 1);
    assert_eq!(p.positionals[0].name, "name");
    assert_eq!(p.options.len(), 11);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.confidence(), "high");
}

#[test]
fn git_prose_headers_yield_the_command_list() {
    let p = parse_help(&fixture("git-help.txt"), 1);
    assert_eq!(p.subcommands.len(), 24);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.confidence(), "high");
}

#[test]
fn aws_man_page_is_medium_confidence_with_options_and_services() {
    let p = parse_help(&fixture("aws-help.txt"), 1);
    assert!(p.has_usage);
    assert!(p.options.len() >= 15, "{}", p.options.len());
    assert!(!p.subcommands.is_empty());
    // The `Description` prose block still reports its table-shaped
    // rows — man pages stay medium, not silently high.
    assert_eq!(p.confidence(), "medium");
}
