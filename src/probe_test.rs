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
