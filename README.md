# sdkgen-cli

`handl-plugin-sdkgen-cli` — CLI backend plus the `hnl-cli` emitter for the Handl SDK generator (ADR-0107): SDKs for tools that only ship a binary. Generated adapters run on the `cli`/`cli-c` packages (`handl-lang/lib-cli`).

Sibling-repo deps are git dependencies (`branch = "main"`); `[patch]` tables
resolve them from sibling checkouts.

## Usage

```bash
# From an authored spec (*.clispec.yaml):
hnl sdkgen cli gh.clispec.yaml --output handl-packages/generated/gh

# Or probe a tool's --help directly:
hnl sdkgen cli gh --output handl-packages/generated/gh
```

`--config tool.sdkgen.yaml` overlays `rename`/`drop`/`types`/`max_depth`;
`probe: { flag, max_depth }` in a spec file merges a live `--help` walk
with the authored command tree.

## Probing

The probe walks `<tool> [subcmd…] <flag>` and recurses into discovered
subcommands. If the configured flag yields no parseable help it falls
back through `--help` → `-h` → `help` → a bare invocation (git wants
`-h`; the openssl family wants `help`), recording the flag that won in
diagnostics. Each node's parse emits warnings for table-shaped text it
could not use (empty option sections, unparsed flag-like rows,
unrecognized headers carrying rows, `{a|b}` groups with no commands
table), and the worst per-node confidence — `high`/`medium`/`low` —
lands in `module.diagnostics` as `confidence: <level>`. `hnl sdkgen`
prints these diagnostics, and the generated port `.hnl` stamps
`--- confidence: <level>` plus a `--- warning:` note on low confidence
suggesting a `.clispec.yaml`/`--config` fix.

The parser covers cobra/clap/getopt/argparse grammar: multi-line
`Usage:` blocks (indented continuation lines, `or:` alternates
ignored), `Flags:`/`Options:`/`Valid options are:` tables including
single-dash longs (`-cipher`, `-version`),
`Commands:`/`The commands are:`/`Available Commands:` tables,
`positional arguments:`/`Arguments:`/`Args:` tables (row descriptions
annotate usage-line positionals and `{a|b}` groups), and `{a|b}`/`{a,b}`
subcommand groups. Commands offering `--json`/`--format`/`--output`/`-o`
are tagged `meta:json-capable` in the IR and stamped `--- json-capable`
in the adapter.

## emit_spec — probe then hand-tune

`emit_spec: <path>` in the config writes the resolved command tree
(post-probe, post-overlay) as a canonical `.clispec.yaml` — `emit_spec:
-` prints to stdout. The emitted spec is self-contained (`probe:` is
never written) and re-parses via `CliSpec::load` to the same tree, so
a low-confidence probe becomes the starting point for a hand-tuned
spec:

```yaml
# tool.sdkgen.yaml
emit_spec: tool.clispec.yaml
```

```bash
hnl sdkgen cli tool --config tool.sdkgen.yaml --output gen
# then edit tool.clispec.yaml and re-run against it
```
