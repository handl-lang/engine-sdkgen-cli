//! Pure parser for `--help` text — usage lines, option/positional/
//! subcommand tables — plus parse-confidence diagnostics. No process
//! spawning lives here; [`crate::probe`] feeds captured output to
//! [`parse_help`].
//!
//! Recognized shapes (cobra/clap/getopt/argparse conventions):
//! - `Usage: gh pr create [<flags>] <branch> [<revs>...]` — `<x>`/`[x]`
//!   positionals, `x...`/`[x]...` variadic → `rest`, `{a,b}`/`{a|b}`
//!   choice groups → subcommands, `[OPTIONS]`/`[COMMAND]` markers
//!   skipped. `Usage:` alone takes the next indented line (cobra/go),
//!   and indented continuation lines are alternate forms (`or:`-led
//!   alternates are ignored). `SYNOPSIS`-headed man pages feed the
//!   same path. Parenthesized alternation groups
//!   (`(TYPE NAME | -f FILE)`) are modeled, not skipped: an
//!   all-positional alternation emits alternative[0]'s positionals
//!   (`(TYPE NAME | TYPE/NAME)` → `type`, `name`), a flag-bearing one
//!   flattens to a named `rest` tail with a warning, and quoted option
//!   values (`--schedule='0/5 * * * ?'`) stay single tokens.
//! - `Flags:`/`Options:`/`Global Flags:`/`Valid options are:` tables —
//!   rows like `-t, --title <string>`, `--draft`, `--opt=VAL`, and
//!   single-dash longs (`-cipher`, `-version`). kubectl's
//!   `--flag=<default>:` rows and man pages' `--flag (boolean)` both
//!   parse; boolean-ish metavars (`=false`, `(boolean)`) mean `flag`
//!   kind, not `string`. Repeatable value-taking options — `...` or
//!   `[]`/`[=VAL]` markers in the spec, a plural `VALS` metavar, or
//!   "multiple times"/"(repeatable)"/"comma-separated" phrasing in the
//!   row — infer `strings` (repeat-emits `flag <v>` per element); a
//!   repeatable no-arg switch (`-v...`) stays `flag`. Blank lines do
//!   not close a table while the
//!   following row stays indented — kubectl and groff separate every
//!   row with a blank plus a description paragraph.
//! - `Commands:`/`Available Commands:`/`The commands are:`/
//!   `SUBCOMMANDS:` tables — rows like `pr   Manage pull requests`.
//!   Stretches under *unrecognized* headers (git's `start a working
//!   area (see also: …)` prose) whose rows are command-shaped —
//!   `name   desc` or `* name` bullets (aws's `Available Services`) —
//!   are promoted to subcommands when they dominate the block.
//! - `positional arguments:`/`Arguments:`/`Args:` tables — row
//!   descriptions annotate the usage-line positionals (and `{a|b}`
//!   rows describe usage-group subcommands).
//!
//! Anything recognized as table-shaped but unparsed lands in
//! [`ParsedHelp::warnings`] — the aCLImatise/cli-to-js lesson is that
//! `--help` scraping must report how confident it is, not silently
//! emit a half-empty SDK.

use heck::ToSnakeCase;
use std::collections::{BTreeMap, HashSet};

/// A positional placeholder parsed from a usage line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPositional {
    /// Snake-cased argument name.
    pub name: String,
    /// Whether it was a variadic (`x...`) placeholder.
    pub variadic: bool,
    /// Description from a `positional arguments:` table row, if any.
    pub doc: Option<String>,
}

/// An option row parsed from a flags/options table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOption {
    /// Spec-map key — the long flag snake-cased, or the short letter.
    pub name: String,
    /// The flag spelling handed back to the tool.
    pub flag: String,
    /// `flag` | `string` | `int` | `strings` (repeatable).
    pub kind: String,
}

/// What [`parse_help`] extracted from one help text.
#[derive(Debug, Default)]
pub struct ParsedHelp {
    /// Non-variadic positionals, in usage order.
    pub positionals: Vec<ParsedPositional>,
    /// The variadic tail — last `x...` placeholder wins.
    pub rest: Option<ParsedPositional>,
    /// Parsed option rows (pre-dedup; callers key them by `name`).
    pub options: Vec<ParsedOption>,
    /// `(name, about)` rows — from tables and usage `{a,b}` groups.
    pub subcommands: Vec<(String, String)>,
    /// Subcommand names that entered `subcommands` only via loose-block
    /// promotion (an unrecognized header or headerless stretch), never
    /// via a recognized commands table or a usage `{a|b}` group. `git
    /// help -a`'s concept-topic sections look exactly like command
    /// lists, and the probe uses this set to prune rows that fail to
    /// probe outright — `git attributes --help` errors because
    /// `attributes` is a `git help <topic>` page, not an argv word.
    pub promoted: HashSet<String>,
    /// Whether a `Usage:` line was seen at all.
    pub has_usage: bool,
    /// Non-fatal parse diagnostics — table-shaped text that did not
    /// parse into the IR.
    pub warnings: Vec<String>,
}

impl ParsedHelp {
    /// Whether the text yielded anything an SDK can use — the probe's
    /// help-flag fallback keeps trying until this is true.
    pub fn is_usable(&self) -> bool {
        self.has_usage || !self.options.is_empty() || !self.subcommands.is_empty()
    }

    /// `high`: clean parse · `medium`: usable but with warnings ·
    /// `low`: only usage/positionals recovered — an authored
    /// `.clispec.yaml` is warranted.
    pub fn confidence(&self) -> &'static str {
        if self.options.is_empty() && self.subcommands.is_empty() {
            "low"
        } else if self.warnings.is_empty() {
            "high"
        } else {
            "medium"
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Options,
    Commands,
    Positionals,
}

/// A row in a `positional arguments:` table.
enum PositionalRow {
    /// `{a|b|c}` — describes the usage-group subcommands.
    Group(Vec<String>, String),
    /// `FILE   input file` — describes a usage-line positional.
    Named(String, String),
}

/// Parse one `tool --help` output. `prog_words` is how many leading
/// usage tokens are the program itself (`["gh","pr","create"]` → 3).
pub fn parse_help(text: &str, prog_words: usize) -> ParsedHelp {
    let mut out = ParsedHelp::default();
    // groff/man-page help (aws) carries SGR escapes — bold section
    // heads, underlined words — that would poison every token they
    // touch; strip them up front.
    let text = strip_ansi(text);
    // A failed invocation can still print a valid-looking help page —
    // cobra answers `gh nosuch --help` with `unknown command` + the
    // ROOT command table (exit 0!), and aws answers `aws --help` with
    // an `[ERROR]` banner + usage. Parsing it re-parents the root
    // table under a bogus path, so error-shaped output is unusable.
    if let Some(first) = text.lines().map(str::trim).find(|l| !l.is_empty())
        && (first.starts_with("unknown command")
            || first.starts_with("Error:")
            || first.starts_with("error:")
            || first.contains("[ERROR]")
            || first.contains("[error]")
            || first.contains("the following arguments are required"))
    {
        out.warnings
            .push(format!("error output, not help text: `{first}`"));
        return out;
    }
    let mut section = Section::None;
    let mut header = String::new();
    let mut opt_rows = 0usize;
    let mut section_rows = 0usize;
    let mut stray_rows = 0usize;
    let mut stray_flags = 0usize;
    let mut brace_names: Vec<String> = Vec::new();
    let mut table_subs: HashSet<String> = HashSet::new();
    let mut pos_docs: BTreeMap<String, String> = BTreeMap::new();
    let mut sub_docs: BTreeMap<String, String> = BTreeMap::new();
    // Rows seen in an unrecognized stretch — headerless, under a prose
    // header (git's `start a working area`), or under an unknown
    // `Foo:` header — are buffered so the block can be promoted to a
    // command list when its rows turn out command-shaped.
    // `loose_noncommand` marks documentation-section headers (git's
    // concept guides / manual topics / interfaces lists) whose
    // command-shaped rows are `help <topic>` names, not subcommands.
    let mut loose_named: Option<String> = None;
    let mut loose_noncommand = false;
    let mut loose_rows = 0usize;
    let mut loose_flags = 0usize;
    let mut loose_cmds: Vec<(String, String)> = Vec::new();
    let mut skipped_groups = 0usize;
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // A blank closes a block only when the next non-blank line
            // is unindented (or the text ends): kubectl and man pages
            // blank-separate every row from its description, so an
            // indented continuation keeps the section alive. Consume
            // any run of blanks while peeking.
            let mut closes = true;
            while let Some(&next) = lines.peek() {
                if next.trim().is_empty() {
                    lines.next();
                    continue;
                }
                closes = next == next.trim_start();
                break;
            }
            if !closes {
                continue;
            }
            if section != Section::None && section_rows > 0 {
                close_section(section, &header, opt_rows, &mut out.warnings);
                section = Section::None;
            }
            flush_loose(
                &mut loose_named,
                &mut loose_noncommand,
                &mut loose_rows,
                &mut loose_flags,
                &mut loose_cmds,
                &mut stray_rows,
                &mut stray_flags,
                &mut out,
                &mut table_subs,
            );
            continue;
        }
        if is_or_line(trimmed) {
            continue;
        }
        // `Usage:`/`usage:` and the man-page `SYNOPSIS` head both lead
        // a usage block; a bare header takes the next indented line.
        if (trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("usage:"))
            || trimmed
                .trim_end_matches(':')
                .eq_ignore_ascii_case("synopsis")
        {
            close_section(section, &header, opt_rows, &mut out.warnings);
            flush_loose(
                &mut loose_named,
                &mut loose_noncommand,
                &mut loose_rows,
                &mut loose_flags,
                &mut loose_cmds,
                &mut stray_rows,
                &mut stray_flags,
                &mut out,
                &mut table_subs,
            );
            section = Section::None;
            out.has_usage = true;
            let mut body = if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("usage:") {
                trimmed[6..].trim().to_string()
            } else {
                String::new()
            };
            // Cobra/go put `Usage:` alone with the command on a later,
            // indented line (go inserts a blank line first).
            if body.is_empty() {
                while let Some(&next) = lines.peek() {
                    let t = next.trim();
                    if t.is_empty() {
                        lines.next();
                        continue;
                    }
                    if next == next.trim_start() {
                        break;
                    }
                    lines.next();
                    body = t.to_string();
                    break;
                }
            }
            let prog_tok = body.split_whitespace().next().unwrap_or("").to_string();
            parse_usage(
                &body,
                prog_words,
                &mut out,
                &mut brace_names,
                &mut skipped_groups,
            );
            // Alternate forms and annotations (`or:` lines, parenthesized
            // notes, other invocation modes) continue indented until a
            // blank or unindented line. Only lines repeating the program
            // token are parsed; the rest are skipped.
            while let Some(&next) = lines.peek() {
                let t = next.trim();
                if t.is_empty() || next == next.trim_start() || t.starts_with('-') {
                    break;
                }
                lines.next();
                if is_or_line(t) {
                    continue;
                }
                if !prog_tok.is_empty() && t.split_whitespace().next() == Some(prog_tok.as_str()) {
                    parse_usage(
                        t,
                        prog_words,
                        &mut out,
                        &mut brace_names,
                        &mut skipped_groups,
                    );
                }
            }
            continue;
        }
        let indented = line != line.trim_start();
        // Unindented lines are headers or prose; minimally-indented
        // lines ending in `:` with no column split are headers too
        // (java's ` where options include:`). Table rows are always
        // deeper-indented. A line ending in `.` is a sentence, not a
        // header (kubectl's `Use "kubectl options" for ….` footer).
        let sentence = trimmed.ends_with('.');
        if !sentence
            && (!indented
                || (trimmed.ends_with(':')
                    && !trimmed.starts_with('-')
                    && split_columns(trimmed).is_none()))
        {
            close_section(section, &header, opt_rows, &mut out.warnings);
            flush_loose(
                &mut loose_named,
                &mut loose_noncommand,
                &mut loose_rows,
                &mut loose_flags,
                &mut loose_cmds,
                &mut stray_rows,
                &mut stray_flags,
                &mut out,
                &mut table_subs,
            );
            section = section_header(trimmed);
            header = trimmed.trim_end_matches(':').trim().to_string();
            opt_rows = 0;
            section_rows = 0;
            loose_noncommand = false;
            if section == Section::None {
                if non_command_header(&normalized_head(trimmed)) {
                    // Documentation/topic sections — rows name `help
                    // <topic>` pages; keep the header for the warning
                    // but keep the rows from promoting.
                    loose_named = Some(header.clone());
                    loose_noncommand = true;
                } else if trimmed.ends_with(':') {
                    loose_named = Some(header.clone());
                }
            }
            continue;
        }
        if section != Section::None {
            section_rows += 1;
        }
        match section {
            Section::Options => match parse_option_row(trimmed) {
                Some((opt, dropped)) => {
                    if opt.flag != "-h" && opt.flag != "--help" {
                        if dropped > 0 {
                            out.warnings.push(format!(
                                "row `{trimmed}` lists several flag spellings; only `{}` captured",
                                opt.flag
                            ));
                        }
                        out.options.push(opt);
                        opt_rows += 1;
                    }
                }
                None => {
                    if trimmed.starts_with('-') {
                        out.warnings
                            .push(format!("unparsed flag-like row: `{trimmed}`"));
                    }
                }
            },
            Section::Commands => {
                if let Some((name, desc)) = parse_command_row(trimmed) {
                    // A recognized table row outranks loose promotion —
                    // a name both sources produced is treated as
                    // table-discovered, never pruned by the probe.
                    out.promoted.remove(&name);
                    push_subcommand(&mut out, &mut table_subs, name, desc);
                }
            }
            Section::Positionals => match parse_positional_row(trimmed) {
                Some(PositionalRow::Group(names, desc)) => {
                    for n in names {
                        sub_docs.entry(n).or_insert(desc.clone());
                    }
                }
                Some(PositionalRow::Named(name, desc)) => {
                    pos_docs.entry(name).or_insert(desc);
                }
                None => {}
            },
            Section::None => {
                // Buffer table-shaped rows so the block can be
                // recognized as a command list once it closes.
                if trimmed.starts_with('-') {
                    loose_rows += 1;
                    loose_flags += 1;
                } else if let Some(row) = loose_command_row(trimmed) {
                    loose_rows += 1;
                    loose_cmds.push(row);
                } else if split_columns(trimmed).is_some() {
                    loose_rows += 1;
                }
            }
        }
    }
    close_section(section, &header, opt_rows, &mut out.warnings);
    flush_loose(
        &mut loose_named,
        &mut loose_noncommand,
        &mut loose_rows,
        &mut loose_flags,
        &mut loose_cmds,
        &mut stray_rows,
        &mut stray_flags,
        &mut out,
        &mut table_subs,
    );

    if !out.has_usage {
        out.warnings.push("no usage line found".to_string());
    }
    if stray_rows > 0 {
        out.warnings.push(format!(
            "{stray_rows} table rows appeared outside a recognized section"
        ));
    }
    if stray_flags > 0 {
        out.warnings.push(format!(
            "{stray_flags} flag-like rows appeared outside an options section"
        ));
    }
    // Table descriptions annotate the usage-derived args — usage lines
    // come first, but applying docs in a post-pass keeps either order
    // working.
    for pos in &mut out.positionals {
        pos.doc = pos_docs.get(&pos.name).cloned();
    }
    if let Some(rest) = &mut out.rest {
        rest.doc = pos_docs.get(&rest.name).cloned();
    }
    for (name, about) in &mut out.subcommands {
        if about.is_empty()
            && let Some(d) = sub_docs.get(name.as_str())
        {
            *about = d.clone();
        }
    }
    let mut unmatched = Vec::new();
    for name in &brace_names {
        if !table_subs.contains(name.as_str()) && !sub_docs.contains_key(name.as_str()) {
            unmatched.push(name.as_str());
        }
    }
    if !unmatched.is_empty() {
        out.warnings.push(format!(
            "usage group names `{}` unmatched by any commands table",
            unmatched.join("`, `")
        ));
    }
    if skipped_groups > 0 {
        out.warnings.push(format!(
            "usage line carried {skipped_groups} parenthesized alternation groups that were skipped"
        ));
    }
    out
}

/// Emit the warning due when leaving a recognized section: an options
/// table that yielded no rows.
fn close_section(section: Section, header: &str, opt_rows: usize, warnings: &mut Vec<String>) {
    if section == Section::Options && opt_rows == 0 {
        warnings.push(format!(
            "options section `{header}` produced no option rows"
        ));
    }
}

/// Merge a table/subcommand row into `out.subcommands` — a name
/// already listed by a usage `{a|b}` group gets its description
/// filled, not a duplicate row.
fn push_subcommand(
    out: &mut ParsedHelp,
    table_subs: &mut HashSet<String>,
    name: String,
    desc: String,
) {
    table_subs.insert(name.clone());
    match out.subcommands.iter_mut().find(|(n, _)| *n == name) {
        Some((_, about)) => {
            if about.is_empty() {
                *about = desc;
            }
        }
        None => out.subcommands.push((name, desc)),
    }
}

/// Close an unrecognized stretch: if its table rows are dominated by
/// command-shaped rows (`name   desc` or `* name` bullets — git's
/// prose-headed groups, aws's `* service` list, `git help -a`'s
/// `… Commands / Interrogators` sections), promote them to
/// subcommands — unless the stretch's header is a known documentation
/// list (`noncommand`), where command-shaped rows are `help <topic>`
/// page names; otherwise keep the historical diagnostics —
/// `unrecognized section` under a colon header, stray counts
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn flush_loose(
    named: &mut Option<String>,
    noncommand: &mut bool,
    rows: &mut usize,
    flags: &mut usize,
    cmds: &mut Vec<(String, String)>,
    stray_rows: &mut usize,
    stray_flags: &mut usize,
    out: &mut ParsedHelp,
    table_subs: &mut HashSet<String>,
) {
    if !*noncommand && cmds.len() >= 2 && cmds.len() * 2 >= *rows {
        for (name, desc) in cmds.drain(..) {
            // A name a commands table or usage `{a|b}` group already
            // recorded keeps its stronger provenance — only rows the
            // loose block invented are marked `promoted`.
            if !out.subcommands.iter().any(|(n, _)| *n == name) {
                out.promoted.insert(name.clone());
            }
            push_subcommand(out, table_subs, name, desc);
        }
    } else if let Some(h) = named.take() {
        if *rows > 0 {
            out.warnings.push(format!(
                "unrecognized section `{h}` carried {rows} table rows"
            ));
        }
    } else {
        *stray_rows += *rows - *flags;
        *stray_flags += *flags;
    }
    *rows = 0;
    *flags = 0;
    cmds.clear();
    *named = None;
    *noncommand = false;
}

/// A command-list row in an unrecognized stretch: either a `* name
/// [desc]` bullet (aws's service list) or a `name   desc` column row
/// whose name is a single lowercase-ish command word (git's prose-
/// headed groups). Uppercase words, `KEY=VAL` operands, and multi-word
/// left columns are prose/table rows, not commands.
fn loose_command_row(trimmed: &str) -> Option<(String, String)> {
    let (name, desc) = if let Some(rest) = trimmed.strip_prefix('*') {
        let rest = rest.trim_start();
        let mut it = rest.splitn(2, char::is_whitespace);
        (
            it.next().unwrap_or("").to_string(),
            it.next().unwrap_or("").trim().to_string(),
        )
    } else {
        let (left, right) = split_columns(trimmed)?;
        (left.to_string(), right.to_string())
    };
    // A command word is one lowercase-ish token (`clone`,
    // `sparse-checkout`, `credential-gcloud.sh`) — an uppercase letter
    // or `=`/`()`/`'` marks prose (`The  AWS …`) and operand rows
    // (`bs=BYTES`), and a multi-word left column is a sentence.
    if name.split_whitespace().count() != 1 {
        return None;
    }
    let ok = name.chars().next().is_some_and(|c| c.is_alphanumeric())
        && name
            .chars()
            .all(|c| !c.is_uppercase() && !"=(){}<>|,'\"".contains(c));
    ok.then(|| (name.trim_end_matches(':').to_string(), desc))
}

/// `or:` / `or ` alternate-usage lines (git, java) are ignored.
fn is_or_line(trimmed: &str) -> bool {
    trimmed == "or" || trimmed.starts_with("or:") || trimmed.starts_with("or ")
}

/// The header text normalized for matching — colon and `(…)`
/// qualifiers cut, lowercased, and trailing copula phrases (`are`,
/// `is`, `as follows`, `include`) stripped: `The commands are:` →
/// `the commands`, `The Git concept guides are:` → `the git concept
/// guides`.
fn normalized_head(trimmed: &str) -> String {
    let mut head = trimmed.trim_end_matches(':').to_lowercase();
    if let Some(i) = head.find('(') {
        head = head[..i].trim_end().to_string();
    }
    loop {
        let mut stripped = false;
        for suffix in [
            " as follows",
            " follows",
            " include",
            " can be passed to any command",
            " are",
            " is",
        ] {
            if let Some(h) = head.strip_suffix(suffix) {
                head = h.trim_end().to_string();
                stripped = true;
            }
        }
        if !stripped {
            break;
        }
    }
    head
}

/// An unrecognized header that heads a *documentation* list — `git
/// help -a`'s concept topics (`User-facing repository, command and
/// file interfaces`, `Manual topics`), `git help -g`'s `The Git
/// concept guides are:` — whose rows name `git help <topic>` pages,
/// not argv subcommands. Suppressing promotion here keeps those rows
/// out of `subcommands` without a doomed probe per topic.
fn non_command_header(head: &str) -> bool {
    ["concept", "topic", "guide", "manual", "interface"]
        .iter()
        .any(|w| head.contains(w))
}

/// Map a header line to a section kind — suffix-matched, so
/// `Global Flags`, `Valid options are`, `The commands are`, and
/// parenthesized headers (`normal options (always true)`) all land.
fn section_header(trimmed: &str) -> Section {
    let head = normalized_head(trimmed);
    if head.ends_with("flags")
        || head.ends_with("options")
        || head.ends_with("optionals")
        || head == "optional arguments"
        || head == "named arguments"
    {
        Section::Options
    } else if head == "positional arguments" || head == "arguments" || head == "args" {
        Section::Positionals
    } else if head.ends_with("commands") || head.ends_with("subcommands") {
        Section::Commands
    } else {
        Section::None
    }
}

/// Usage tokens that are markers, not positionals — `[OPTIONS]`,
/// `[COMMAND]`, `SUBCOMMAND`-style placeholders covered by tables.
const USAGE_MARKERS: &[&str] = &[
    "options",
    "option",
    "flags",
    "flag",
    "opts",
    "command",
    "commands",
    "cmd",
    "subcommand",
    "subcommands",
    "parameters",
    "params",
    "arguments",
    "args",
];

/// Strip ANSI CSI escapes (`\e[1m`, `\e[24m`) — groff-rendered man
/// pages (aws's `help`) bold/underline section heads and keywords, and
/// the escape bytes would otherwise poison every token they touch.
fn strip_ansi(text: &str) -> String {
    if !text.contains('\u{1b}') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            // CSI sequence: parameter/intermediate bytes until a
            // final byte in 0x40..=0x7e.
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Whitespace-split a usage body, keeping `'…'`/`"…"` quoted regions
/// whole — kubectl's `--schedule='0/5 * * * ?'` would otherwise leak
/// its `*`/`?` words as bogus positionals.
fn usage_tokens(body: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut quote = '\0';
    for c in body.chars() {
        if quote != '\0' {
            cur.push(c);
            if c == quote {
                quote = '\0';
            }
        } else if c == '\'' || c == '"' {
            cur.push(c);
            quote = c;
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// Parse a usage body like `gh pr create [<flags>] <branch> [revs...]`,
/// skipping the first `prog_words` program tokens. `{a,b}`/`{a|b}`
/// groups are subcommand choice lists; their names are also recorded
/// in `brace_names` so unmatched groups can be diagnosed.
/// `skipped_groups` counts parenthesized groups that turn out not to
/// be flattenable alternations (`([--limits=X & --requests=Y]`,
/// `[(-o|--output=)json|yaml]`) — reported once in `warnings`.
/// Real alternations (`(TYPE NAME | -f FILE)`) are buffered whole and
/// modeled by [`finish_alternation`].
fn parse_usage(
    body: &str,
    prog_words: usize,
    out: &mut ParsedHelp,
    brace_names: &mut Vec<String>,
    skipped_groups: &mut usize,
) {
    let mut seen = std::collections::HashSet::new();
    // A `[` group that opens on an option token (`[-D debugopts]`)
    // swallows its metavar — skip until the `]` closes.
    let mut opt_group = false;
    // A `(` group is buffered token-by-token until its `)` — the
    // alternation it carries is modeled when the group closes.
    let mut group_depth = 0usize;
    let mut group: Vec<String> = Vec::new();
    for tok in usage_tokens(body).iter().skip(prog_words) {
        let opens = tok.matches('(').count();
        let closes = tok.matches(')').count();
        if group_depth > 0 || opens > 0 || closes > 0 {
            group.push(tok.clone());
            group_depth = (group_depth + opens).saturating_sub(closes);
            if group_depth == 0 {
                finish_alternation(&group, out, &mut seen, skipped_groups);
                group.clear();
            }
            continue;
        }
        if opt_group {
            if tok.contains(']') {
                opt_group = false;
            }
            continue;
        }
        // `{a,b}` / `{a|b}` groups are choice lists — for CLIs those are
        // the subcommand set (argparse puts `{clone,pull}` in usage).
        let brace = tok.trim_start_matches('[').starts_with('{');
        if brace {
            let inner = tok.trim_matches(|c| "[]{}<>".contains(c));
            for part in inner.split([',', '|']) {
                // Subcommand names stay literal — they are argv words
                // (`c_decl`); snake-casing happens at map time.
                let name = part.trim();
                if !name.is_empty() {
                    brace_names.push(name.to_string());
                    out.subcommands.push((name.to_string(), String::new()));
                }
            }
            continue;
        }
        if tok.starts_with('-') {
            continue;
        }
        let variadic = tok.contains("...");
        let inner = tok.trim_matches(|c| "[]{}<>.".contains(c));
        // A metavar of literal choices (`<a|b>`) cannot be named —
        // skip rather than emit a bogus positional. `[-h]`/`[--opt]`
        // are option placeholders, not positionals; an unclosed `[`
        // means the flag's metavar follows (`[-D debugopts]`).
        if inner.is_empty() || inner.contains('|') || inner.contains(',') {
            continue;
        }
        if inner.starts_with('-') {
            if tok.matches('[').count() > tok.matches(']').count() {
                opt_group = true;
            }
            continue;
        }
        // `NAME=VALUE` operands name only their left side — the
        // `=value` is assignment syntax (kubectl `KEY_1=VAL_1` pairs,
        // dd `bs=BYTES`), not part of the argument name.
        let inner = inner.split('=').next().unwrap_or(inner);
        let lower = inner.to_lowercase();
        if USAGE_MARKERS.contains(&lower.as_str()) {
            continue;
        }
        let name = lower.to_snake_case();
        // Punctuation-only tokens (`*`, `?`, leftover brackets) cannot
        // name an argument — they used to become `''` positionals.
        if !name.chars().any(|c| c.is_alphanumeric()) {
            continue;
        }
        let pos = ParsedPositional {
            name,
            variadic,
            doc: None,
        };
        if variadic {
            out.rest = Some(pos);
        } else if seen.insert(pos.name.clone()) {
            out.positionals.push(pos);
        }
    }
    // A group left open by a truncated usage line (`kubectl set
    // resources`'s `([--limits=… & --requests=…]` tail) is analyzed at
    // end-of-line the same way a closed one is.
    if !group.is_empty() {
        finish_alternation(&group, out, &mut seen, skipped_groups);
    }
}

/// Emit one parsed positional: a `variadic` token lands on `rest` —
/// or, when `rest` is already taken, falls back to an ordinary
/// positional (two `x...` tails cannot both hold the argv tail;
/// keeping the name as a plain positional is the least-wrong shape).
fn emit_positional(out: &mut ParsedHelp, seen: &mut HashSet<String>, mut pos: ParsedPositional) {
    if pos.variadic && out.rest.is_none() {
        out.rest = Some(pos);
    } else {
        pos.variadic = false;
        if seen.insert(pos.name.clone()) {
            out.positionals.push(pos);
        }
    }
}

/// A token inside an alternation is flag-like once its bracketing
/// decoration is stripped — `-f`, `[-f`, `-l`, `--all)]`.
fn alt_flag_like(tok: &str) -> bool {
    tok.trim_matches(|c| "[](){}<>\"'".contains(c))
        .starts_with('-')
}

/// The argument name a metavar-ish token inside an alternation yields
/// — `NAME` → `name`, `TYPE[.VERSION][.GROUP]/NAME` →
/// `type_version_group_name`, `KEY=VAL` → `key`. Unnameable tokens
/// (option spellings, `a|b` choice metas, punctuation, marker words)
/// return `None`.
fn positional_name(tok: &str) -> Option<String> {
    let inner = tok.trim_matches(|c| "[](){}<>.".contains(c));
    if inner.is_empty() || inner.contains('|') || inner.contains(',') || inner.starts_with('-') {
        return None;
    }
    let inner = inner.split('=').next().unwrap_or(inner);
    let lower = inner.to_lowercase();
    if USAGE_MARKERS.contains(&lower.as_str()) {
        return None;
    }
    let name = lower.to_snake_case();
    (name.chars().any(|c| c.is_alphanumeric())).then_some(name)
}

/// Split a buffered `(`-group's tokens into `|`-separated
/// alternatives. The group's own outer parens are stripped from the
/// first/last token; a `|` nested inside `[…]`/`(…)` (`[NAME | -l
/// label]`) belongs to its alternative, not the top-level split.
fn split_alternatives(group: &[String]) -> Vec<Vec<String>> {
    let mut alts: Vec<Vec<String>> = vec![Vec::new()];
    let mut depth = 0i32;
    let last = group.len() - 1;
    for (i, tok) in group.iter().enumerate() {
        let t = if i == 0 {
            tok.trim_start_matches('(')
        } else {
            tok.as_str()
        };
        let t = if i == last {
            t.trim_end_matches(')')
        } else {
            t
        };
        if t.is_empty() {
            continue;
        }
        if depth == 0 && t == "|" {
            alts.push(Vec::new());
            continue;
        }
        depth += (t.matches('[').count() + t.matches('{').count() + t.matches('(').count()) as i32;
        depth -= (t.matches(']').count() + t.matches('}').count() + t.matches(')').count()) as i32;
        alts.last_mut()
            .expect("the alternatives list always holds one")
            .push(t.to_string());
    }
    alts.retain(|a| !a.is_empty());
    alts
}

/// Pick the name for a flattened alternation's `rest` tail:
/// 1. the leading nameable token of the first *fully positional*
///    alternative — `TYPE NAME` in `(-f FILENAME | TYPE NAME)` reads
///    as a variadic `type` list;
/// 2. the head of the first alternative that merely *leads* with a
///    positional token — `TYPE [(NAME | -l label | --all)]`;
/// 3. the metavar following the first flag token — `[-f FILENAME]`
///    reads as a `filename` list (kubectl apply's `(-f FILENAME | -k
///    DIRECTORY)`);
/// 4. `args`.
fn alternation_rest_name(alts: &[Vec<String>]) -> String {
    for alt in alts {
        if alt.iter().all(|t| !alt_flag_like(t))
            && let Some(n) = alt.iter().find_map(|t| positional_name(t))
        {
            return n;
        }
    }
    for alt in alts {
        if let Some(first) = alt.first()
            && !alt_flag_like(first)
            && let Some(n) = positional_name(first)
        {
            return n;
        }
    }
    let toks: Vec<&String> = alts.iter().flatten().collect();
    for (i, t) in toks.iter().enumerate() {
        if alt_flag_like(t) {
            if let Some((_, mv)) = t.split_once('=')
                && let Some(n) = positional_name(mv)
            {
                return n;
            }
            if let Some(n) = toks[i + 1..].iter().find_map(|t| positional_name(t)) {
                return n;
            }
        }
    }
    "args".to_string()
}

/// Model a buffered `(`-group once it closes. The pieces are
/// alternatives, not a positional sequence, so the honest shapes are:
///
/// - **Every alternative positional-shaped** — `(TYPE NAME |
///   TYPE/NAME)` (kubectl rollout undo), `(docker-registry | generic |
///   tls)` (kubectl create secret): the group stands where one
///   argument chain could, so alternative[0]'s nameable tokens become
///   positionals — the least-wrong reading of the same argv slot.
/// - **Some alternative carries a flag** — `(-f FILENAME | TYPE NAME)`
///   (kubectl set/scale/annotate), `(TYPE NAME | -f FILE)`: no single
///   word covers both shapes, so the group flattens to the variadic
///   `rest` tail (`TYPE NAME`'s two words fit a `type...` list) named
///   by [`alternation_rest_name`], with a warning noting the
///   alternation was flattened.
/// - **Fewer than two `|` alternatives** — a conjunction
///   (`([--limits=X & --requests=Y]`, never closed on kubectl's
///   malformed usage line) or a single token
///   (`[(-o|--output=)json|yaml]`, an option-value choice list): not
///   an alternation the IR can read; counted for the "skipped" warning
///   as before.
fn finish_alternation(
    group: &[String],
    out: &mut ParsedHelp,
    seen: &mut HashSet<String>,
    skipped_groups: &mut usize,
) {
    let alts = split_alternatives(group);
    if alts.len() < 2 {
        *skipped_groups += 1;
        return;
    }
    if alts.iter().all(|alt| alt.iter().all(|t| !alt_flag_like(t))) {
        // All positional-shaped — emit alternative[0]'s tokens. A
        // `...` on or after a token marks it the variadic tail.
        let mut names: Vec<ParsedPositional> = Vec::new();
        for t in &alts[0] {
            let variadic = t.contains("...");
            match positional_name(t) {
                Some(name) => names.push(ParsedPositional {
                    name,
                    variadic,
                    doc: None,
                }),
                None if variadic => {
                    if let Some(last) = names.last_mut() {
                        last.variadic = true;
                    }
                }
                None => {}
            }
        }
        for pos in names {
            emit_positional(out, seen, pos);
        }
        return;
    }
    let name = alternation_rest_name(&alts);
    let slot = if out.rest.is_none() {
        "rest"
    } else {
        "positional"
    };
    emit_positional(
        out,
        seen,
        ParsedPositional {
            name: name.clone(),
            variadic: true,
            doc: None,
        },
    );
    out.warnings.push(format!(
        "alternation `{}` flattened to a {slot} argument `{name}`",
        group.join(" ")
    ));
}

/// Parse one option-table row: `-t, --title <string>   PR title` →
/// `(title, --title, string)` plus how many *extra* flag spellings the
/// row carried (rows like `-D -O -P -debug` capture only one).
///
/// The caller drops `-h`/`--help` results — the SDK wraps programmatic
/// use, and probing would otherwise give every command a `help`
/// option — and warns on `dropped > 0`.
fn parse_option_row(trimmed: &str) -> Option<(ParsedOption, usize)> {
    if !trimmed.starts_with('-') {
        return None;
    }
    // The description column is separated from the flag spec by a run
    // of two or more spaces; a row without one is flag-spec only. The
    // description is kept — "can be used multiple times"-style phrasing
    // is a `strings` kind signal.
    let (spec, desc) = match split_columns(trimmed) {
        Some((spec, desc)) => (spec, desc),
        None => (trimmed, ""),
    };
    let mut flags: Vec<String> = Vec::new();
    let mut metavar: Option<String> = None;
    for tok in spec.split([',', ' ', '\t']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some((flag, value)) = tok.split_once('=') {
            // `--track[=(direct|inherit)]` — the optional-value bracket
            // is not part of the spelling.
            let flag = flag.split('[').next().unwrap_or(flag);
            if flag.starts_with('-') {
                flags.push(clean_flag(flag));
            }
            if !value.is_empty() {
                metavar.get_or_insert(value.to_string());
            }
        } else if tok.starts_with('-') {
            // `--track[=(direct|inherit)]` carries an optional value
            // attached with `[` — split it off like `=`.
            if let Some((flag, _value)) = tok.split_once('[') {
                flags.push(clean_flag(flag));
                metavar.get_or_insert("value".to_string());
            } else {
                flags.push(clean_flag(tok));
            }
        } else if metavar.is_none() {
            metavar = Some(tok.to_string());
        }
    }
    // Long spellings win: `--x` and single-dash words (`-cipher`,
    // `-version`, openssl/java style); a lone `-x` is a short flag.
    let long = flags.iter().find(|f| f.len() > 2 || f.starts_with("--"));
    let short = flags.iter().find(|f| f.len() <= 2);
    // `flags` is never empty — a row that reached here starts with
    // `-`, so its first spec token is always a flag spelling.
    let flag = long.or(short).expect("a flag row always yields a spelling");
    let name = flag.trim_start_matches('-').to_snake_case();
    if name.is_empty() {
        return None;
    }
    // A short+long pair (`-t, --title`) is the canonical shape; more
    // spellings than that mean the row packed several options into one
    // line and all but the captured one are lost.
    let dropped = if flags.len() <= 1 || (flags.len() == 2 && long.is_some() && short.is_some()) {
        0
    } else {
        flags.len() - 1
    };
    // Repeatability — `strings` emits `flag <v>` once per element —
    // reads from three signals: the spec column (`--verbose...`,
    // `--filename=[]`, `--tag[=VAL]`, a plural `VALS` metavar), the
    // metavar itself, and the row's own description ("can be used
    // multiple times", "(repeatable)", "comma-separated"). Only
    // value-taking options promote: a repeatable no-arg switch
    // (`-v...`) is a counted flag, not a list.
    let repeatable = spec_is_repeatable(spec)
        || metavar.as_deref().is_some_and(is_list_metavar)
        || desc_is_repeatable(desc);
    let kind = match &metavar {
        None => "flag",
        Some(m) if is_bool_metavar(m) => "flag",
        Some(_) if repeatable => "strings",
        Some(m) if is_int_metavar(m) => "int",
        Some(_) => "string",
    };
    Some((
        ParsedOption {
            name,
            flag: flag.clone(),
            kind: kind.to_string(),
        },
        dropped,
    ))
}

/// Flag spelling cleanup: clap spells repeatable flags `--verbose...`
/// and java spells optional metavars `-verbose:[...]` — the `...` and
/// `:` are arity decoration, not part of the spelling.
fn clean_flag(flag: &str) -> String {
    flag.trim_end_matches(['.', ':']).to_string()
}

/// Split `spec   description` on the first run of 2+ spaces.
fn split_columns(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b' ' && bytes[i + 1] == b' ' {
            return Some((line[..i].trim_end(), line[i..].trim()));
        }
        i += 1;
    }
    None
}

/// Repeatability markers in the flag-spec column: a `...` ellipsis on
/// the spelling or metavar (`--verbose...`, `--exclude PATTERN...`), an
/// empty list bracket (kubectl spells StringSlice options
/// `--filename=[]`), or a bracketed `=metavar` tail (`--tag[=VAL]`). A
/// `[=(a|b)]` choice group is an optional-value annotation, not a
/// list — git's `--track[=(direct|inherit)]` stays `string`.
fn spec_is_repeatable(spec: &str) -> bool {
    spec.contains("...")
        || spec.contains("[]")
        || spec
            .split_once("[=")
            .is_some_and(|(_, tail)| !tail.starts_with('('))
}

/// A metavar that reads as a plural — all-caps `VALS`, `TAGS`,
/// `FILES` — hints at a repeatable option. Singular-looking `…S` words
/// (units like `SECONDS`, `STATUS`, `CLASS`) are the exception list:
/// `--timeout SECONDS` is not a repeatable option.
fn is_list_metavar(m: &str) -> bool {
    let m = m.trim_matches(|c| "<>[]{}()'\":".contains(c));
    m.len() >= 3
        && m.ends_with('S')
        && m.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !SINGULAR_S_WORDS.contains(&m.to_ascii_lowercase().as_str())
}

/// Plural-looking metavar words that are actually singular.
const SINGULAR_S_WORDS: &[&str] = &[
    "seconds", "minutes", "hours", "days", "weeks", "months", "years", "status", "class", "bytes",
    "dns", "os", "ms", "ns", "us", "tls", "as", "is",
];

/// Repeatability phrases in an option row's description column —
/// clap's `(repeatable)`, git's "may be specified multiple times",
/// kubectl's "comma separated list", "can be used multiple times".
fn desc_is_repeatable(desc: &str) -> bool {
    let d = desc.to_lowercase();
    [
        "repeat",
        "multiple",
        "comma-separated",
        "comma separated",
        "more than once",
        "each occurrence",
    ]
    .iter()
    .any(|phrase| d.contains(phrase))
}

/// Boolean-ish metavars — kubectl documents flag defaults as
/// `--flag=false:`/`--flag=true:` and man pages annotate `--flag
/// (boolean)`; all mean a switch, not a value-taking option.
fn is_bool_metavar(metavar: &str) -> bool {
    let m = metavar
        .trim_matches(|c| "<>[]{}()'\":".contains(c))
        .to_lowercase();
    matches!(m.as_str(), "true" | "false" | "boolean" | "bool")
}

/// Integer-ish metavars — `<count>`, `NUM`, `=N`, `=500` all map to
/// `int`.
fn is_int_metavar(metavar: &str) -> bool {
    let m = metavar
        .trim_matches(|c| "<>[]{}()'\":".contains(c))
        .to_lowercase();
    !m.is_empty()
        && (m.bytes().all(|b| b.is_ascii_digit())
            || matches!(
                m.as_str(),
                "int" | "integer" | "count" | "number" | "num" | "n" | "jobs" | "depth" | "level"
            ))
}

/// Parse one subcommand-table row: `pr   Manage pull requests`.
fn parse_command_row(trimmed: &str) -> Option<(String, String)> {
    // `trimmed` is non-empty, so a first word and its first char exist.
    let first = trimmed.split_whitespace().next().unwrap_or(trimmed);
    if first.starts_with('-') || first.starts_with('*') {
        return None;
    }
    if !first.chars().next().unwrap_or(' ').is_alphanumeric() {
        return None;
    }
    let desc = trimmed[first.len()..].trim().to_string();
    // Keep the literal token — it is the argv word, not the fn name.
    // Cobra/gh spell rows `name:   desc` — the colon is table
    // punctuation, not part of the command word.
    Some((first.trim_end_matches(':').to_string(), desc))
}

/// Parse one `positional arguments:` row — either a named metavar
/// (`FILE   input file`) or a `{a|b}` group describing the
/// subcommands a usage-line choice group introduced.
fn parse_positional_row(trimmed: &str) -> Option<PositionalRow> {
    // `trimmed` is non-empty, so `head` always starts a metavar or a
    // `{a|b}` group.
    let (head, desc) = match split_columns(trimmed) {
        Some((h, d)) => (h.trim(), d.to_string()),
        None => (trimmed, String::new()),
    };
    if head.starts_with('{') {
        let names: Vec<String> = head
            .trim_matches(|c| "{}".contains(c))
            .split([',', '|'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            None
        } else {
            Some(PositionalRow::Group(names, desc))
        }
    } else {
        let name = head
            .trim_matches(|c| "[]{}<>".contains(c))
            .to_lowercase()
            .to_snake_case();
        if name.is_empty() || name.starts_with('-') {
            None
        } else {
            Some(PositionalRow::Named(name, desc))
        }
    }
}
