//! Pure parser for `--help` text — usage line, option tables, and
//! subcommand tables. No process spawning lives here; [`crate::probe`]
//! feeds captured output to [`parse_help`].
//!
//! Recognized shapes (cobra/clap/getopt/argparse conventions):
//! - `Usage: gh pr create [<flags>] <branch> [<revs>...]` — `<x>`/`[x]`
//!   positionals, `x...`/`[x]...` variadic → `rest`, `{a,b}` choice
//!   groups → subcommands, `[OPTIONS]`/`[COMMAND]` markers skipped.
//! - `Flags:`/`Options:`/`Global Flags:`/`optional arguments:` tables —
//!   rows like `-t, --title <string>`, `--draft`, `--opt=VAL`.
//! - `Commands:`/`Available Commands:`/`SUBCOMMANDS:` tables — rows like
//!   `pr   Manage pull requests`.

use heck::ToSnakeCase;

/// A positional placeholder parsed from a usage line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPositional {
    /// Snake-cased argument name.
    pub name: String,
    /// Whether it was a variadic (`x...`) placeholder.
    pub variadic: bool,
}

/// An option row parsed from a flags/options table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOption {
    /// Spec-map key — the long flag snake-cased, or the short letter.
    pub name: String,
    /// The flag spelling handed back to the tool.
    pub flag: String,
    /// `flag` | `string` | `int`.
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Options,
    Commands,
}

/// Parse one `tool --help` output. `prog_words` is how many leading
/// usage tokens are the program itself (`["gh","pr","create"]` → 3).
pub fn parse_help(text: &str, prog_words: usize) -> ParsedHelp {
    let mut out = ParsedHelp::default();
    let mut section = Section::None;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            section = Section::None;
            continue;
        }
        if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("usage:") {
            let mut body = &trimmed[6..];
            // Cobra puts `Usage:` alone and the command on the next,
            // indented line.
            if body.trim().is_empty() {
                if let Some(next) = lines.next() {
                    body = next.trim();
                }
            }
            parse_usage(body, prog_words, &mut out);
            section = Section::None;
            continue;
        }
        if line == line.trim_start() {
            // Unindented lines are headers or prose — a table row is
            // always indented, so leaving a section here is safe.
            section = section_header(trimmed);
            continue;
        }
        match section {
            Section::Options => {
                if let Some(opt) = parse_option_row(trimmed) {
                    out.options.push(opt);
                }
            }
            Section::Commands => {
                if let Some(row) = parse_command_row(trimmed) {
                    out.subcommands.push(row);
                }
            }
            Section::None => {}
        }
    }
    out
}

/// Map a non-indented header line to a section kind.
fn section_header(trimmed: &str) -> Section {
    let head = trimmed.trim_end_matches(':').to_lowercase();
    if head.ends_with("flags")
        || head.ends_with("options")
        || head.ends_with("optionals")
        || head == "optional arguments"
        || head == "named arguments"
    {
        Section::Options
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
];

/// Parse a usage body like `gh pr create [<flags>] <branch> [revs...]`,
/// skipping the first `prog_words` program tokens.
fn parse_usage(body: &str, prog_words: usize, out: &mut ParsedHelp) {
    let mut seen = std::collections::HashSet::new();
    for tok in body.split_whitespace().skip(prog_words) {
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
        // are option placeholders, not positionals.
        if inner.is_empty() || inner.starts_with('-') || inner.contains('|') || inner.contains(',')
        {
            continue;
        }
        let lower = inner.to_lowercase();
        if USAGE_MARKERS.contains(&lower.as_str()) {
            continue;
        }
        let pos = ParsedPositional {
            name: lower.to_snake_case(),
            variadic,
        };
        if variadic {
            out.rest = Some(pos);
        } else if seen.insert(pos.name.clone()) {
            out.positionals.push(pos);
        }
    }
}

/// Parse one option-table row: `-t, --title <string>   PR title` →
/// `(title, --title, string)`. Returns `None` for `-h`/`--help` —
/// the SDK wraps programmatic use, and probing would otherwise give
/// every command a `help` option.
fn parse_option_row(trimmed: &str) -> Option<ParsedOption> {
    if !trimmed.starts_with('-') {
        return None;
    }
    // The description column is separated from the flag spec by a run
    // of two or more spaces; a row without one is flag-spec only.
    let spec = match split_columns(trimmed) {
        Some((spec, _desc)) => spec,
        None => trimmed,
    };
    let mut long: Option<String> = None;
    let mut short: Option<String> = None;
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
                remember_flag(flag, &mut long, &mut short);
            }
            if !value.is_empty() {
                metavar.get_or_insert(value.to_string());
            }
        } else if tok.starts_with('-') {
            // `--track[=(direct|inherit)]` carries an optional value
            // attached with `[` — split it off like `=`.
            if let Some((flag, _value)) = tok.split_once('[') {
                remember_flag(flag, &mut long, &mut short);
                metavar.get_or_insert("value".to_string());
            } else {
                remember_flag(tok, &mut long, &mut short);
            }
        } else if metavar.is_none() {
            metavar = Some(tok.to_string());
        }
    }
    let flag = long.or(short)?;
    if flag == "-h" || flag == "--help" {
        return None;
    }
    let name = flag.trim_start_matches('-').to_snake_case();
    if name.is_empty() {
        return None;
    }
    let kind = match &metavar {
        None => "flag",
        Some(m) if is_int_metavar(m) => "int",
        Some(_) => "string",
    };
    Some(ParsedOption {
        name,
        flag,
        kind: kind.to_string(),
    })
}

fn remember_flag(flag: &str, long: &mut Option<String>, short: &mut Option<String>) {
    // clap spells repeatable flags `--verbose...`; the `...` is arity
    // decoration, not part of the spelling.
    let flag = flag.trim_end_matches('.');
    if flag.starts_with("--") {
        if long.is_none() {
            *long = Some(flag.to_string());
        }
    } else if short.is_none() {
        *short = Some(flag.to_string());
    }
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

/// Integer-ish metavars — `<count>`, `NUM`, `=N` all map to `int`.
fn is_int_metavar(metavar: &str) -> bool {
    let m = metavar
        .trim_matches(|c| "<>[]{}()".contains(c))
        .to_lowercase();
    matches!(
        m.as_str(),
        "int" | "integer" | "count" | "number" | "num" | "n" | "jobs" | "depth" | "level"
    )
}

/// Parse one subcommand-table row: `pr   Manage pull requests`.
fn parse_command_row(trimmed: &str) -> Option<(String, String)> {
    let first = trimmed.split_whitespace().next()?;
    if first.starts_with('-') || first.starts_with('*') {
        return None;
    }
    if !first.chars().next()?.is_alphanumeric() {
        return None;
    }
    let desc = trimmed[first.len()..].trim().to_string();
    // Keep the literal token — it is the argv word, not the fn name.
    Some((first.to_string(), desc))
}
