use super::probe::*;
use std::collections::HashMap;
use std::sync::Mutex;

const ROOT_HELP: &str =
    "Usage: tool [OPTIONS] <dir>\n\nCommands:\n  add    Add things\n  list   List things\n";
const ADD_HELP: &str = "Usage: tool add [flags] <name>\n\nFlags:\n  -f, --force   Overwrite\n";
const LIST_HELP: &str = "Usage: tool list [flags]\n\nFlags:\n      --long   Long format\n";

/// A canned runner keyed on the argv tail.
fn canned(
    pages: &[(&[&str], Result<&str, &str>)],
) -> impl Fn(&str, &[String]) -> Result<String, String> + use<> {
    let table: HashMap<Vec<String>, Result<String, String>> = pages
        .iter()
        .map(|(args, r)| {
            (
                args.iter().map(|s| s.to_string()).collect::<Vec<String>>(),
                r.map(str::to_string).map_err(str::to_string),
            )
        })
        .collect();
    move |_tool, args| {
        table
            .get(&args.to_vec())
            .cloned()
            .unwrap_or_else(|| Err("unexpected argv".to_string()))
    }
}

#[test]
fn probes_root_then_each_subcommand() {
    let run = canned(&[
        (&["--help"], Ok(ROOT_HELP)),
        (&["add", "--help"], Ok(ADD_HELP)),
        (&["list", "--help"], Ok(LIST_HELP)),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    // Root positionals land on the root node.
    assert_eq!(r.root.positionals[0].name, "dir");
    let add = &r.commands["add"];
    assert_eq!(add.about.as_deref(), Some("Add things"));
    assert_eq!(add.positionals[0].name, "name");
    assert_eq!(add.options["force"].flag, "--force");
    assert!(r.commands["list"].options.contains_key("long"));
}

#[test]
fn failures_become_diagnostics_and_do_not_abort() {
    let run = canned(&[
        (&["--help"], Ok(ROOT_HELP)),
        (&["add", "--help"], Err("spawn exploded")),
        (&["list", "--help"], Ok(LIST_HELP)),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert_eq!(r.diagnostics.len(), 1);
    let d = &r.diagnostics[0];
    assert!(
        d.contains("tool add --help") && d.contains("spawn exploded"),
        "{d}"
    );
    // `add` still exists (as an about-only stub); `list` probed fine.
    assert!(r.commands.contains_key("add"));
    assert!(r.commands["list"].options.contains_key("long"));
}

#[test]
fn root_failure_reports_the_tool_name() {
    let run = canned(&[(&["--help"], Err("no such tool"))]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert_eq!(r.diagnostics.len(), 1);
    assert!(r.diagnostics[0].contains("probe `tool --help`"));
    assert!(r.commands.is_empty());
}

#[test]
fn max_depth_zero_records_children_without_probing() {
    let run = canned(&[(&["--help"], Ok(ROOT_HELP))]);
    let plan = ProbePlan {
        flag: "--help".into(),
        max_depth: 0,
    };
    let r = probe_tool("tool", &plan, &run);
    assert_eq!(r.commands.len(), 2);
    // Unprobed stubs: no options, but the table's about text survives.
    assert!(r.commands["add"].options.is_empty());
    assert_eq!(r.commands["add"].about.as_deref(), Some("Add things"));
}

#[test]
fn custom_flag_is_appended_to_every_argv() {
    let seen = Mutex::new(Vec::<Vec<String>>::new());
    let run = |_: &str, args: &[String]| {
        seen.lock().unwrap().push(args.to_vec());
        Ok("Usage: tool\n".to_string())
    };
    let plan = ProbePlan {
        flag: "-h".into(),
        max_depth: 1,
    };
    probe_tool("tool", &plan, &run);
    assert_eq!(*seen.lock().unwrap(), vec![vec!["-h".to_string()]]);
}

#[test]
fn duplicate_subcommand_rows_probe_once() {
    let dup = "Usage: tool\n\nCommands:\n  add   Add\n  add   Add again\n";
    let calls = Mutex::new(0usize);
    let run = |_: &str, args: &[String]| {
        *calls.lock().unwrap() += 1;
        Ok(if args == ["add".to_string(), "--help".to_string()] {
            ADD_HELP.to_string()
        } else {
            dup.to_string()
        })
    };
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    // Two `add` rows → one probed child (visited set); the first
    // row's about is kept (entry exists before re-visiting is skipped).
    assert_eq!(*calls.lock().unwrap(), 2);
    assert_eq!(r.commands["add"].about.as_deref(), Some("Add"));
}

#[test]
fn duplicate_usage_group_names_probe_once() {
    // `Usage: t {a|a}` records `a` twice (brace-group names push into
    // `subcommands` without dedup) — the second row hits the `visited`
    // guard and never re-probes.
    let root = "Usage: t {a|a}\n";
    let calls = Mutex::new(0usize);
    let run = |_: &str, args: &[String]| {
        *calls.lock().unwrap() += 1;
        Ok(if args == ["a".to_string(), "--help".to_string()] {
            "Usage: t a\n".to_string()
        } else {
            root.to_string()
        })
    };
    let r = probe_tool("t", &ProbePlan::default(), &run);
    // Root probe + one `a` probe; the duplicate row is skipped.
    assert_eq!(*calls.lock().unwrap(), 2);
    assert!(r.commands.contains_key("a"));
}

#[test]
fn about_from_the_table_survives_into_the_probed_node() {
    // The subcommand's own help carries no doc; the parent's table row
    // text fills `about`.
    let bare = "Usage: tool add <name>\n";
    let run = canned(&[
        (&["--help"], Ok(ROOT_HELP)),
        (&["add", "--help"], Ok(bare)),
        (&["list", "--help"], Ok(LIST_HELP)),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert_eq!(r.commands["add"].about.as_deref(), Some("Add things"));
}

#[test]
fn variadic_usage_maps_to_rest() {
    let help = "Usage: tool <src> <dst> [files]...\n";
    let run = canned(&[(&["--help"], Ok(help))]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    let names: Vec<&str> = r.root.positionals.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["src", "dst"]);
    assert_eq!(r.root.rest.as_ref().unwrap().name, "files");
}

#[test]
fn argv_beyond_the_canned_table_is_a_diagnostic() {
    // Only the root page is canned — the discovered `add`/`list`
    // probes miss the table and surface as diagnostics.
    let run = canned(&[(&["--help"], Ok(ROOT_HELP))]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert_eq!(r.diagnostics.len(), 2, "{:?}", r.diagnostics);
    assert!(r.diagnostics[0].contains("unexpected argv"));
}

#[test]
fn probe_plan_defaults() {
    let p = ProbePlan::default();
    assert_eq!(p.flag, "--help");
    assert_eq!(p.max_depth, 1);
    let r = CommandRunner::default();
    // with_timeout round-trips through the same struct.
    let _ = CommandRunner::with_timeout(std::time::Duration::from_millis(1));
    drop(r);
}

#[test]
fn falls_back_to_dash_h_when_help_unparses() {
    // git-style: `--help` and `help` produce nothing parseable, `-h` wins.
    let run = canned(&[
        (&["--help"], Ok("git: 'help' is not a git command")),
        (&["-h"], Ok("Usage: tool [-f] <n>\n\nFlags:\n  -f  force\n")),
        (&["help"], Ok("nope")),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(r.root.options.contains_key("f"));
    assert!(
        r.diagnostics.iter().any(|d| d.contains("`-h`")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn falls_back_to_help_subcommand_then_bare_invocation() {
    // openssl-style: only `tool help` parses.
    let run = canned(&[
        (&["--help"], Err("unknown option")),
        (&["-h"], Err("unknown option")),
        (&["help"], Ok("Usage: tool <cmd>\n\nCommands:\n  s  sub\n")),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(r.commands.contains_key("s"));
    assert!(
        r.diagnostics.iter().any(|d| d.contains("`help`")),
        "{:?}",
        r.diagnostics
    );

    // Only the bare invocation produces text.
    let run = canned(&[
        (&["--help"], Err("x")),
        (&["-h"], Err("x")),
        (&["help"], Err("x")),
        (&[], Ok("Usage: tool\n")),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(
        r.diagnostics.iter().any(|d| d.contains("bare invocation")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn all_candidates_failing_keeps_the_first_error() {
    let run = canned(&[
        (&["--help"], Err("spawn exploded")),
        (&["-h"], Err("other")),
        (&["help"], Err("other")),
        (&[], Err("other")),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert_eq!(r.diagnostics.len(), 1);
    assert!(
        r.diagnostics[0].contains("spawn exploded"),
        "{:?}",
        r.diagnostics
    );
    assert_eq!(r.confidence, "low");
}

#[test]
fn unusable_output_advances_to_the_next_candidate() {
    let run = canned(&[
        (&["--help"], Ok("just prose\n")),
        (&["-h"], Ok("Usage: tool\n")),
    ]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(
        r.diagnostics.iter().any(|d| d.contains("`-h`")),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn parse_warnings_and_confidence_propagate() {
    let warned = "Usage: tool\n\nFlags:\n  --    bad\n  -x    ok\n";
    let run = canned(&[(&["--help"], Ok(warned))]);
    let r = probe_tool("tool", &ProbePlan::default(), &run);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.contains("tool: unparsed flag-like row")),
        "{:?}",
        r.diagnostics
    );
    assert_eq!(r.confidence, "medium");
}

#[test]
fn configured_flag_wins_first_and_skips_diagnostics() {
    let run = canned(&[(&["help"], Ok("Usage: tool\n"))]);
    let plan = ProbePlan {
        flag: "help".into(),
        max_depth: 1,
    };
    let r = probe_tool("tool", &plan, &run);
    // The configured flag was the first candidate — no fallback note.
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}

#[test]
fn help_candidates_order_and_dedup() {
    assert_eq!(help_candidates("--help"), ["--help", "-h", "help", ""]);
    assert_eq!(help_candidates("-h"), ["-h", "--help", "help", ""]);
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/help/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

const LEAF_HELP: &str = "Usage: git x <n>\n\nFlags:\n  -f   force\n";

#[test]
fn promoted_concept_topics_that_fail_to_probe_are_dropped() {
    // A `git help -a`-shaped page: `annotate`/`blame` and the concept
    // topics `namespaces`/`protocol-v2` all arrive via loose-block
    // promotion, indistinguishable at parse time. The probe separates
    // them — `git namespaces --help` fails on every candidate because
    // the name is a `git help <topic>` page, not an argv word — so
    // promoted children that do not probe are removed; table- and
    // probe-proven children stay.
    let root = fixture("git-help-concepts.txt");
    let run = canned(&[
        (&["--help"], Ok(root.as_str())),
        (&["add", "--help"], Ok(LEAF_HELP)),
        (&["checkout", "--help"], Ok(LEAF_HELP)),
        (&["annotate", "--help"], Ok(LEAF_HELP)),
        (&["blame", "--help"], Ok(LEAF_HELP)),
    ]);
    let r = probe_tool("git", &ProbePlan::default(), &run);
    for kept in ["add", "checkout", "annotate", "blame"] {
        assert!(r.commands.contains_key(kept), "{kept} dropped");
    }
    for gone in ["namespaces", "protocol-v2"] {
        assert!(!r.commands.contains_key(gone), "{gone} survived");
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.contains(&format!("dropped `{gone}`"))),
            "{:?}",
            r.diagnostics
        );
    }
    // The only remaining diagnostics are the failed topic probes, the
    // drop notes, and the fake `… and options` section — `medium`
    // because the page has no usage line.
    assert_eq!(r.confidence, "medium");
}

#[test]
fn dropped_topic_failures_do_not_drag_confidence_down() {
    // Without the restore, each dropped topic's failed probe would
    // sink the whole walk to `low` — misreporting a clean page.
    let root = "Usage: t <cmd>\n\nweird head\n  topic1   guide one\n  topic2   guide two\n";
    let run = canned(&[(&["--help"], Ok(root))]);
    let r = probe_tool("t", &ProbePlan::default(), &run);
    assert!(r.commands.is_empty(), "{:?}", r.commands.keys());
    assert_eq!(r.confidence, "high", "{:?}", r.diagnostics);
    assert_eq!(
        r.diagnostics
            .iter()
            .filter(|d| d.contains("dropped `"))
            .count(),
        2,
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn unprobed_promoted_children_survive_at_max_depth() {
    // The pruning rule needs a *failed* probe — at the depth cap
    // children are recorded unprobed and promoted names stay stubs.
    let root = "Usage: t <cmd>\n\nweird head\n  aa   does a\n  bb   does b\n";
    let run = canned(&[(&["--help"], Ok(root))]);
    let plan = ProbePlan {
        flag: "--help".into(),
        max_depth: 0,
    };
    let r = probe_tool("t", &plan, &run);
    assert!(r.commands.contains_key("aa"));
    assert!(r.commands.contains_key("bb"));
}

#[test]
fn error_banner_pages_fall_through_to_the_help_candidate() {
    // aws-style: `--help` and `-h` answer a `[ERROR]` banner + usage
    // (usable-looking but bogus) — the probe must keep walking to
    // `help`, which yields the man-page command list.
    let run = canned(&[
        (
            &["--help"],
            Ok(
                "aws: [ERROR]: An error occurred (ParamValidation): the following arguments are required: command\n\nusage: aws [options] <command> <subcommand> [parameters]\n",
            ),
        ),
        (
            &["-h"],
            Ok(
                "aws: [ERROR]: An error occurred (ParamValidation): the following arguments are required: command\n",
            ),
        ),
        (
            &["help"],
            Ok(
                "Synopsis\n        aws [options] <command> [parameters]\n\nGlobal Options\n     --debug (boolean)   debug logging\n\nAvailable Services\n     * s3\n     * ec2\n",
            ),
        ),
        // `* s3`-style bullets promote — a promoted child whose probe
        // fails would be dropped, so the services need real pages.
        (&["s3", "--help"], Ok("Usage: aws s3 <cmd>\n")),
        (&["ec2", "--help"], Ok("Usage: aws ec2 <cmd>\n")),
    ]);
    let r = probe_tool("aws", &ProbePlan::default(), &run);
    assert!(r.commands.contains_key("s3"), "{:?}", r.commands);
    assert!(r.commands.contains_key("ec2"), "{:?}", r.commands);
    assert!(r.root.options.contains_key("debug"), "{:?}", r.root.options);
    assert_eq!(r.root.options["debug"].kind, "flag");
    assert!(
        r.diagnostics.iter().any(|d| d.contains("`help`")),
        "{:?}",
        r.diagnostics
    );
}
