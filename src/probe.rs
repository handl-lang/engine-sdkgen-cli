//! Live probing of a CLI tool's `--help` output into a [`CommandNode`]
//! tree.
//!
//! [`probe_tool`] runs `<tool> [path...] <flag>` for each candidate
//! help flag in turn — the configured flag, then `--help`, `-h`,
//! `help`, and a bare invocation — and keeps the first output that
//! parses (git wants `-h`, the openssl family wants `help`). The
//! result is parsed with [`crate::help`], its warnings land in
//! [`ProbeResult::diagnostics`], and the worst per-node confidence
//! becomes [`ProbeResult::confidence`]. Discovered subcommands recurse
//! up to `max_depth`. The runner is injectable (`&dyn Fn`) so tests
//! never spawn a process; [`CommandRunner`] is the real `std::process`
//! implementation used by the backend.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::help::{ParsedHelp, parse_help};
use crate::spec::{CommandNode, OptionSpec, PositionalSpec};

/// The runner signature [`probe_tool`] consumes: `(tool, argv_tail) →
/// captured help text`. `Err` is any probe-level failure — spawn
/// error or timeout; exit codes are ignored by [`CommandRunner`].
pub type RunHelp<'a> = dyn Fn(&str, &[String]) -> Result<String, String> + 'a;

/// Probe recursion parameters resolved from spec + config.
#[derive(Debug, Clone)]
pub struct ProbePlan {
    /// The help flag tried first on each probed argv (`--help`).
    pub flag: String,
    /// How many subcommand levels below the root to probe.
    pub max_depth: usize,
}

impl Default for ProbePlan {
    fn default() -> Self {
        Self {
            flag: "--help".to_string(),
            max_depth: 1,
        }
    }
}

/// What a probe produced: the discovered command tree plus
/// non-fatal diagnostics (spawn failures, timeouts, parse warnings).
#[derive(Debug)]
pub struct ProbeResult {
    /// Subcommands discovered at the root.
    pub commands: BTreeMap<String, CommandNode>,
    /// The root's own options/positionals (a tool without subcommands
    /// still parses to a leaf).
    pub root: CommandNode,
    /// Non-fatal failures and parse warnings, in probe order.
    pub diagnostics: Vec<String>,
    /// Worst per-node parse confidence — `high` | `medium` | `low`.
    pub confidence: String,
}

/// The help flags tried in order when `plan.flag` yields no parseable
/// output. `""` is the bare invocation — some tools print usage on a
/// no-arg run.
pub(crate) fn help_candidates(flag: &str) -> Vec<String> {
    let mut out = vec![flag.to_string()];
    for f in ["--help", "-h", "help", ""] {
        if !out.iter().any(|x| x == f) {
            out.push(f.to_string());
        }
    }
    out
}

/// The real runner — `<tool> args...` via `std::process::Command`
/// (never a shell), stdout preferred, stderr fallback (many tools print
/// help to stderr), exit code ignored.
pub struct CommandRunner {
    timeout: Duration,
}

impl CommandRunner {
    /// A runner with the default 10s cap per invocation.
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(10),
        }
    }

    /// A runner with a custom cap — tests use short timeouts.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Run `tool args...` and return its help text. Output is read
    /// after the child exits — help text is far under the pipe buffer,
    /// so an incremental drain is unnecessary; an over-long child just
    /// hits the timeout.
    pub fn run(&self, tool: &str, args: &[String]) -> Result<String, String> {
        let mut child = Command::new(tool)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn `{tool}` failed: {e}"))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            // A failed poll is retried until the deadline rather than
            // failing the probe on a transient wait error.
            if let Ok(Some(_status)) = child.try_wait() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{tool}` timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_string(&mut out);
        }
        if out.trim().is_empty()
            && let Some(mut stderr) = child.stderr.take()
        {
            let _ = stderr.read_to_string(&mut out);
        }
        Ok(out)
    }
}

impl Default for CommandRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Probe `tool` into a command tree: root help, then each discovered
/// subcommand recursively to `plan.max_depth`. Failures land in
/// [`ProbeResult::diagnostics`] and never abort the walk.
pub fn probe_tool(tool: &str, plan: &ProbePlan, run: &RunHelp<'_>) -> ProbeResult {
    let mut result = ProbeResult {
        commands: BTreeMap::new(),
        root: CommandNode::default(),
        diagnostics: Vec::new(),
        confidence: "high".to_string(),
    };
    let mut visited = HashSet::new();
    probe_node(
        tool,
        &[],
        plan,
        0,
        &mut visited,
        run,
        &mut result.root,
        &mut result.diagnostics,
        &mut result.confidence,
    );
    result.commands = result.root.commands.clone();
    result
}

/// Confidence ordering — `low` is the worst, `high` the best.
fn worse(current: &mut String, next: &str) {
    fn rank(c: &str) -> u8 {
        match c {
            "low" => 0,
            "medium" => 1,
            _ => 2,
        }
    }
    if rank(next) < rank(current.as_str()) {
        *current = next.to_string();
    }
}

/// Fill `node` from `tool [path...] <flag>`, trying each candidate
/// help flag until one returns parseable text, then recurse into each
/// discovered subcommand while `depth < max_depth`. Returns `true`
/// when the node's own probe produced usable help — `false` on a
/// total probe failure, which the parent uses to prune children that
/// were only ever loose-block promotions (see below).
fn probe_node(
    tool: &str,
    path: &[String],
    plan: &ProbePlan,
    depth: usize,
    visited: &mut HashSet<String>,
    run: &RunHelp<'_>,
    node: &mut CommandNode,
    diagnostics: &mut Vec<String>,
    confidence: &mut String,
) -> bool {
    let dotted = path.join(".");
    let shown = if dotted.is_empty() {
        tool.to_string()
    } else {
        format!("{tool} {dotted}")
    };

    let mut parsed: Option<(String, ParsedHelp)> = None;
    let mut first_err: Option<String> = None;
    for flag in help_candidates(&plan.flag) {
        let mut args: Vec<String> = path.to_vec();
        if !flag.is_empty() {
            args.push(flag.clone());
        }
        match run(tool, &args) {
            Ok(text) => {
                let p = parse_help(&text, path.len() + 1);
                if p.is_usable() {
                    parsed = Some((flag, p));
                    break;
                }
                first_err.get_or_insert_with(|| "output did not parse".to_string());
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }

    let Some((used_flag, mut parsed)) = parsed else {
        diagnostics.push(format!(
            "probe `{shown} {flag}` failed: {e}",
            flag = plan.flag,
            e = first_err.unwrap_or_default()
        ));
        worse(confidence, "low");
        return false;
    };
    if used_flag != plan.flag {
        let used = if used_flag.is_empty() {
            "a bare invocation".to_string()
        } else {
            format!("`{used_flag}`")
        };
        diagnostics.push(format!(
            "{shown}: `{}` gave no parseable help — used {used}",
            plan.flag
        ));
    }
    for w in &parsed.warnings {
        diagnostics.push(format!("{shown}: {w}"));
    }
    worse(confidence, parsed.confidence());

    for pos in parsed.positionals {
        node.positionals.push(PositionalSpec {
            name: pos.name,
            kind: "string".to_string(),
            doc: pos.doc,
        });
    }
    if let Some(rest) = parsed.rest {
        node.rest = Some(PositionalSpec {
            name: rest.name,
            kind: "string".to_string(),
            doc: rest.doc,
        });
    }
    for opt in parsed.options {
        node.options.entry(opt.name).or_insert(OptionSpec {
            flag: opt.flag,
            kind: opt.kind,
            doc: None,
        });
    }
    // Names the loose-block promotion invented (see
    // [`ParsedHelp::promoted`]) — `git help -a` lists concept topics
    // (`attributes`, `namespaces`, `protocol-v2`) in sections that are
    // visually identical to its command lists, so table shape alone
    // cannot tell them apart. The probe can: `git attributes --help`
    // fails outright because the topic is a `git help <topic>` page,
    // not an argv word — a promoted child whose every help candidate
    // fails is removed from the tree. Table- or usage-discovered
    // children keep their about-only stub on failure: a real `Commands:`
    // row is evidence the argv word exists.
    let promoted = std::mem::take(&mut parsed.promoted);
    for (name, about) in parsed.subcommands {
        let entry = node.commands.entry(name.clone()).or_default();
        if entry.about.is_none() && !about.is_empty() {
            entry.about = Some(about);
        }
        if depth < plan.max_depth {
            let mut child_path = path.to_vec();
            child_path.push(name.clone());
            let dotted_child = child_path.join(".");
            // The visited set guards re-entry — a table that names a
            // command already probed (aliases, `help` loops) is skipped.
            if visited.insert(dotted_child) {
                let before = confidence.clone();
                let probed = probe_node(
                    tool,
                    &child_path,
                    plan,
                    depth + 1,
                    visited,
                    run,
                    entry,
                    diagnostics,
                    confidence,
                );
                if !probed && promoted.contains(name.as_str()) {
                    // A concept topic's failed probe must not drag the
                    // real tree's confidence down — restore the level
                    // the walk had before probing it.
                    *confidence = before;
                    node.commands.remove(name.as_str());
                    diagnostics.push(format!(
                        "{shown}: dropped `{name}` — promoted from a loose listing but did not probe (a concept topic, not a subcommand)"
                    ));
                }
            }
        }
    }
    true
}
