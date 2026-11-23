# Design rationale

Why the `cli` backend is shaped the way it is. This covers only the
choices local to this crate — the registry, emitter contract, and
`SdkgenModule` IR are documented in `host/docs/codegen/`.

## One unified help parser

The probe treats `--help` text as *one* grammar family, not
per-generator parsers. Cobra, clap, getopt, argparse, and man-page
help all draw from the same handful of conventions — a `Usage:`/
`SYNOPSIS` line, a flags table, a commands table, a positional-args
table — so a single `parse_help` (`src/help.rs`) recognizes headers by
suffix-normalization (`Flags:`/`Global Flags:`/`Valid options are:` all
land) rather than by tool identity. The payoff: tools the authors have
never seen parse on first contact, and every fix (kubectl's
`--flag=<default>:`, java's `or:` alternates, groff's ANSI escapes)
improves all future scrapes at once. `tests/fixtures/help/` is a
corpus of real-world violators kept as the regression suite.

The parser is pure — process spawning lives only in `probe.rs`'s
`CommandRunner`, injected as `&dyn Fn` — so the entire grammar is
tested without a single subprocess, and the walk's policy (flag
fallback order, timeouts, visited-set pruning) is separable from the
text problem.

## Confidence and warnings are the API

The aCLImatise/cli-to-js lesson: a `--help` scraper that silently
emits a half-empty SDK is worse than no scraper. So every
recognized-but-unusable shape produces a *named warning* — an
unrecognized section header, an unparsed flag-like row, an alternation
that had to be flattened — and each node carries a confidence
(`high`/`medium`/`low`) that surfaces as `confidence:` first in
`module.diagnostics` and a `--- confidence:` header in the emitted
port. `low` is deliberately not an error: it's the signal to switch
workflow — emit the resolved spec (`--emit-spec`), hand-tune it, and
generate from yaml (see
[howto-tune-a-probe.md](./howto-tune-a-probe.md)). `lib-cli-gen`
operates exactly this way.

## A descriptor-string IR

`SdkgenModule` is shared across all codegen sources and carries no
CLI-specific fields — a CLI command's argv *shape* (positionals,
flag/kind/spellings, a variadic tail, emitter hints) can't be a typed
IR member without polluting the generic contract. The resolution is
`arg_descriptors`: colon-separated strings (`pos:`, `opt:`, `rest:`,
`meta:`) that each emitter re-parses. Strings are the extensible
point — the trailing `:<doc>` component that threads help text into
generated SDK docs was added without touching the IR at all; consumers
that predate it simply never see a doc (empty docs emit nothing, so
doc-less descriptors are byte-identical to before). Validation keeps
names/kinds/flags colon-free upstream, which is what makes "everything
past the Nth separator" an unambiguous encoding for free-text fields.
The contract itself is in [emission.md](./emission.md).

## Port + adapter split

Emission produces two packages on purpose:

- **port** (`<pkg>/<module>.hnl`) — `export` arrow *types* against the
  `cli` package: the consumer-facing API. It names the contract, not
  the mechanism.
- **adapter** (`<output>/<pkg>-cli-<target>/<module>.hnl`) — per-target, nested inside the output store (PL-004 `ArtifactRef` constraint)
  bindings that assemble argv via `cli-c`'s pure `argv` and spawn via
  `run` (`posix_spawnp` — never `sh -c`).

Keeping the port target-free means the same signature file can front
several adapter targets, and the adapter being generated trivial
`run(argv(...))` bodies means all spawn policy (no shell, verbatim
argv, exit-status-as-data) lives in one place: `cli-c`, in
`handl-lang/lib-cli`. The generated SDK inherits the same guarantee
the probe runner makes — nothing in the pipeline ever touches a
shell.

Two related rejections worth knowing:

- **Branch commands emit no function** — `gh pr` is a namespace, not a
  call; one `Function` per leaf keeps the port's surface honest.
- **No `x.json()` convenience methods** — `meta:json-capable` is a
  doc annotation pointing at lib-json's free `parse` arrow, because
  composable free functions treeshake cleanly and keep the `~>`
  purity boundary visible where a method chain would hide it.
