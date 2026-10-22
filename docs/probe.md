# Probing — live `--help` scraping

When the `cli` input is a bare tool name (or a spec carrying a `probe:`
block), `probe_tool` (`src/probe.rs`) walks `<tool> [subcmd…] <flag>`
and parses each page with `parse_help` (`src/help.rs`) into
`CommandNode` trees.

## The walk

- For each node, candidate help flags are tried in order: the configured
  `plan.flag`, then `--help`, `-h`, `help`, and a bare invocation. The
  first output that `parse_help` judges **usable** wins (`has_usage`, or
  any options/subcommands parsed). git wants `-h`; the openssl family
  wants `help`; tools that print usage on a no-arg run hit the bare
  fallback. When a fallback won, a `{path}: \`--help\` gave no parseable
  help — used \`-h\`` diagnostic records it.
- A node whose every candidate fails adds `probe \`<tool> <path>
  <flag>\` failed: <err>` to diagnostics and confidence drops to `low` —
  the walk continues; failures never abort.
- Discovered subcommands recurse while `depth < max_depth`
  (default 1 — top-level only). A visited-set keyed by dotted path
  guards re-entry (aliases, `help` loops); already-probed names are
  skipped, not re-run.
- `CommandRunner` is `std::process::Command` — **never a shell** —
  `stdin` null, exit code ignored, stdout preferred with **stderr
  fallback** when stdout is empty (many tools print help to stderr), and
  a **10 s timeout** per invocation (`with_timeout` for tests). The
  runner is injected as `&dyn Fn`, so tests never spawn.
- Output that leads with `unknown command`, `Error:`, or `error:` is
  rejected as error-page output — cobra answers `gh nosuch --help` with
  exit 0 and the *root* command table, which would otherwise re-parent
  the root under a bogus path.

## What `parse_help` recognizes

Cobra/clap/getopt/argparse conventions:

- `Usage:` lines — `<x>`/`[x]` positionals, `x...`/`[x]...` variadic →
  `rest`, `{a,b}`/`{a|b}` choice groups → subcommands, marker tokens
  (`[OPTIONS]`, `[COMMAND]`, …) skipped. `Usage:` alone takes the next
  indented line; indented continuation lines repeating the program word
  are parsed as alternate forms; `or:`/`or ` lines ignored. `[-D
  debugopts]`-style option groups are skipped to their `]`. Multi-word
  metavariables like `<a|b>` are not positionals.
- Option tables — `Flags:`/`Options:`/`Global Flags:`/`Valid options
  are:`/`Optional arguments`/`Named arguments` headers
  (suffix-normalized: `… are`/`is`/`include`/`follows` stripped,
  parenthesized qualifiers dropped). Rows like `-t, --title <string>`,
  `--opt=VAL`, `--track[=(…)]`, clap's `--verbose...`, java's
  `-opt:[...]`, and single-dash longs (`-cipher`, `-version`). The long
  spelling wins; short+long is one option; `-h`/`--help` rows are
  dropped (every command would otherwise grow a `help` option).
  Metavars `int|integer|count|number|num|n|jobs|depth|level` → `int`
  kind, other metavars → `string`, none → `flag`. Probed options never
  get `strings` — repeatability is a spec-only refinement.
- Command tables — `Commands:`/`Available Commands:`/`The commands
  are:`/`SUBCOMMANDS:` — `name   description` rows (`name:` colons
  trimmed). Table rows fill descriptions onto `{a|b}`-discovered names
  instead of duplicating them.
- Positional tables — `positional arguments:`/`Arguments:`/`Args:` —
  row descriptions annotate usage-line positionals; `{a|b}` group rows
  describe usage-group subcommands.
- Positionals probed from usage are always `kind: string`.

## Confidence and warnings

Every recognized-but-unusable shape lands in `warnings` — the point is
to report *how* the scrape went, not to emit a half-empty SDK silently:

- `no usage line found`
- `error output, not help text: \`<first line>\``
- `options section \`<h>\` produced no option rows` — header seen, no rows parsed
- `unrecognized section \`<h>\` carried N table rows` — an unknown `Foo:` header had indented column-split rows under it
- `unparsed flag-like row: \`<row>\`` — a `-`-led row in an options section that didn't parse
- `row \`<row>\` lists several flag spellings; only \`<f>\` captured` — packed rows like `-D -O -P -debug` lose all but one spelling
- `N table rows appeared outside a recognized section` / `N flag-like rows appeared outside an options section`
- `usage group names \`a\`, \`b\` unmatched by any commands table` — `{a|b}` usage groups with no corresponding table rows

Per-node confidence (`ParsedHelp::confidence`):

| Level | Condition |
| --- | --- |
| `high` | Options or subcommands parsed, zero warnings |
| `medium` | Usable parse with warnings |
| `low` | Only usage/positionals recovered — no options, no subcommands; or the node failed to probe at all |

`ProbeResult.confidence` is the worst per-node level. The backend puts
`confidence: <level>` first in `module.diagnostics` (followed by the
warnings), `hnl sdkgen` prints them, and the `hnl-cli` emitter stamps
`--- confidence: <level>` on the port plus, on `low`, `--- warning:
…--help probe is low-confidence; author a .clispec.yaml or pass
--config to correct it`.

**Low confidence is the escape-hatch signal**: write a
`*.clispec.yaml` by hand, or probe once with `emit_spec:` and hand-tune
the dump (see [clispec.md](./clispec.md), "the `--config` overlay").
`lib-cli-gen`'s spec set is the convention in practice: probe-emitted
where usable, hand-authored baselines where the help grammar is outside
the parser (groff man-page help, prose `or:` alternates).
