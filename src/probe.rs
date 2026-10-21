//! Live probing of a CLI tool's `--help` output into a [`CommandNode`]
//! tree.
//!
//! [`probe_tool`] runs `<tool> [path...] --help`, parses the result with
//! [`crate::help`], and recurses into discovered subcommands up to
//! `max_depth`. The runner is injectable (`&dyn Fn`) so tests never
//! spawn a process; [`CommandRunner`] is the real `std::process`
//! implementation used by the backend.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::help::parse_help;
use crate::spec::{CommandNode, OptionSpec, PositionalSpec};

/// The runner signature [`probe_tool`] consumes: `(tool, argv_tail) →
/// captured help text`. `Err` is any probe-level failure — spawn
/// error or timeout; exit codes are ignored by [`CommandRunner`].
pub type RunHelp<'a> = dyn Fn(&str, &[String]) -> Result<String, String> + 'a;

/// Probe recursion parameters resolved from spec + config.
#[derive(Debug, Clone)]
pub struct ProbePlan {
    /// The help flag appended to each probed argv (`--help`).
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
/// non-fatal diagnostics (spawn failures, timeouts).
#[derive(Debug, Default)]
pub struct ProbeResult {
    /// Subcommands discovered at the root.
    pub commands: BTreeMap<String, CommandNode>,
    /// The root's own options/positionals (a tool without subcommands
    /// still parses to a leaf).
    pub root: CommandNode,
    /// Non-fatal failures, in probe order.
    pub diagnostics: Vec<String>,
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
    let mut result = ProbeResult::default();
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
    );
    result.commands = result.root.commands.clone();
    result
}

/// Fill `node` from `tool [path...] <flag>`, then recurse into each
/// discovered subcommand while `depth < max_depth`.
fn probe_node(
    tool: &str,
    path: &[String],
    plan: &ProbePlan,
    depth: usize,
    visited: &mut HashSet<String>,
    run: &RunHelp<'_>,
    node: &mut CommandNode,
    diagnostics: &mut Vec<String>,
) {
    let args: Vec<String> = path.iter().cloned().chain([plan.flag.clone()]).collect();
    let dotted = path.join(".");
    let text = match run(tool, &args) {
        Ok(t) => t,
        Err(e) => {
            let shown = if dotted.is_empty() {
                tool.to_string()
            } else {
                format!("{tool} {dotted}")
            };
            diagnostics.push(format!(
                "probe `{shown} {flag}` failed: {e}",
                flag = plan.flag
            ));
            return;
        }
    };
    let parsed = parse_help(&text, path.len() + 1);
    for pos in parsed.positionals {
        node.positionals.push(PositionalSpec {
            name: pos.name,
            kind: "string".to_string(),
            doc: None,
        });
    }
    if let Some(rest) = parsed.rest {
        node.rest = Some(PositionalSpec {
            name: rest.name,
            kind: "string".to_string(),
            doc: None,
        });
    }
    for opt in parsed.options {
        node.options.entry(opt.name).or_insert(OptionSpec {
            flag: opt.flag,
            kind: opt.kind,
            doc: None,
        });
    }
    for (name, about) in parsed.subcommands {
        let entry = node
            .commands
            .entry(name.clone())
            .or_insert_with(CommandNode::default);
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
                probe_node(
                    tool,
                    &child_path,
                    plan,
                    depth + 1,
                    visited,
                    run,
                    entry,
                    diagnostics,
                );
            }
        }
    }
}
