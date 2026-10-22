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
  the walk continues; failures never abort. One refinement: a child the
  parent discovered *only* through loose-block promotion (see
  `promoted` below) is **dropped** on a total probe failure — `git
  help -a` lists `git help <topic>` concept pages (`attributes`,
  `namespaces`, `protocol-v2`) in sections visually identical to its
  command lists, and the probe is what separates them from real argv
  words. A dropped node leaves a `{parent}: dropped \`<name>\` — …`
  diagnostic, keeps the underlying probe-failure note, and does **not**
  drag confidence down (its failure is restored out). Table- or
  usage-discovered children keep their about-only stub on failure —
  a real `Commands:` row is evidence the word exists.
- Discovered subcommands recurse while `depth < max_depth`
  (default 1 — top-level only). A visited-set keyed by dotted path
  guards re-entry (aliases, `help` loops); already-probed names are
  skipped, not re-run.
- `CommandRunner` is `std::process::Command` — **never a shell** —
  `stdin` null, exit code ignored, stdout preferred with **stderr
  fallback** when stdout is empty (many tools print help to stderr), and
  a **10 s timeout** per invocation (`with_timeout` for tests). The
  runner is injected as `&dyn Fn`, so tests never spawn.
- Output that leads with `unknown command`, `Error:`/`error:`, an
  `[ERROR]`/`[error]` banner, or `the following arguments are
  required` is rejected as error-page output — cobra answers `gh
  nosuch --help` with exit 0 and the *root* command table, and `aws
  --help` answers a ParamValidation error + usage; parsing either
  would re-parent or stall the walk on a bogus page.
- ANSI SGR escapes are stripped first — man-page help (aws) is
  groff-formatted with bold/underline markup.

## What `parse_help` recognizes

Cobra/clap/getopt/argparse/man-page conventions:

- `Usage:` lines (and man `SYNOPSIS` heads) — `<x>`/`[x]` positionals,
  `x...`/`[x]...` variadic → `rest`, `{a,b}`/`{a|b}` choice groups →
  subcommands, marker tokens (`[OPTIONS]`, `[COMMAND]`, `[parameters]`,
  …) skipped. `Usage:`/`SYNOPSIS` alone takes the next indented line;
  indented continuation lines repeating the program word are parsed as
  alternate forms; `or:`/`or ` lines ignored. `[-D debugopts]`-style
  option groups are skipped to their `]`; `'…'`/`"…"` quoted option
  values stay single tokens. Parenthesized alternation groups
  (`(TYPE NAME | -f FILE)`) are *modeled*, not skipped: an
  all-positional alternation emits alternative[0]'s positionals
  (`(TYPE NAME | TYPE/NAME)` → `type`, `name`); a flag-bearing one
  flattens to a named variadic `rest` tail with a warning, since no
  single word covers both shapes. Groups that aren't `|`-alternations
  (`[(-o|--output=)json|yaml]`, a truncated conjunction) are skipped
  and counted in warnings. `NAME=VALUE` operands name only their left
  side; punctuation-only tokens never become `''` positionals.
- Option tables — `Flags:`/`Options:`/`Global Flags:`/`Valid options
  are:`/`Optional arguments`/`Named arguments` headers
  (suffix-normalized: `… are`/`is`/`include`/`follows` stripped,
  parenthesized qualifiers dropped). Rows like `-t, --title <string>`,
  `--opt=VAL`, `--track[=(…)]`, clap's `--verbose...`, java's
  `-opt:[...]`, kubectl's `--flag=<default>:` (boolean defaults read as
  `flag`), man pages' `--flag (type)`, and single-dash longs
  (`-cipher`, `-version`). A blank line ends a table only when the
  next non-blank line is unindented — kubectl and groff blank-separate
  every row from its description paragraph. The long spelling wins;
  short+long is one option; `-h`/`--help` rows are dropped (every
  command would otherwise grow a `help` option). Metavars
  `int|integer|count|number|num|n|jobs|depth|level` or all-digit →
  `int` kind, `true|false|boolean|bool` → `flag`, other metavars →
  `string`, none → `flag`. A **value-taking** option whose row proves
  repeatability infers `strings` (repeat-emits `flag <v>` per
  element) instead of `string`/`int`: a `...`/`[]`/`[=VAL]` marker in
  the flag spec (`--verbose...`, kubectl's `--filename=[]`,
  `--tag[=VAL]`), a plural all-caps metavar (`VALS`, `TAGS`,
  `FILES` — singular lookalikes like `SECONDS`/`STATUS` are
  excepted), or "multiple times"/"(repeatable)"/"comma-separated"
  phrasing in the row's description (`git -C`, `git -L`,
  `kubectl --label` rows all infer this way). A repeatable *no-arg*
  switch (`-v...`) stays `flag` — it is a count, not a list. The row's
  description column becomes the option's `doc` (threaded to the
  FlagSpec `doc` field and the port's `opts:` table). When the guess
  is wrong, `types:` in the config overrides it.
- Command tables — `Commands:`/`Available Commands:`/`The commands
  are:`/`SUBCOMMANDS:` — `name   description` rows (`name:` colons
  trimmed). Unrecognized-header or headerless stretches whose
  table-shaped rows are dominated (≥2 rows, ≥half) by command-shaped
  rows — a `name   desc` column row or a `* name` bullet — are
  promoted to command lists: this is what recovers git's prose-headed
  groups (`start a working area (see also: …)`, `Ancillary Commands /
  Manipulators`) and aws's `* <service>` bullets under `Available
  Services`. Table rows fill descriptions onto `{a|b}`-discovered
  names instead of duplicating them. Names promoted this way are
  recorded in `ParsedHelp.promoted` — the weaker provenance the
  probe's concept-topic pruning relies on (a later real `Commands:`
  row for the same name un-marks it).
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
- `alternation \`(…)\` flattened to a rest/positional argument \`x\`` — a flag-bearing `(A | B)` group (kubectl's `(TYPE NAME | -f FILE)`) had to collapse into the variadic tail
- `usage line carried N parenthesized alternation groups that were skipped` — `(…)` groups that are not `|`-alternations at all (option-value choices, truncated conjunctions like `[--limits=X & --requests=Y]`)

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
the parser (deeply-nested alternation usage, prose `or:` alternates)
or where the probed shape needs curation (aws's ~300-service list).
