# sdkgen-cli

`handl-plugin-sdkgen-cli` — CLI backend plus the `hnl-cli` emitter for the Handl SDK generator (ADR-0107): SDKs for tools that only ship a binary. Generated adapters run on the `cli`/`cli-c` packages (`handl-lang/lib-cli`).

Sibling-repo deps are git dependencies (`branch = "main"`); `[patch]` tables
resolve them from sibling checkouts.

## Usage

```bash
# From an authored spec (*.clispec.yaml):
hnl sdkgen cli gh.clispec.yaml --output handl-packages/generated/gh

# From a foreign spec — fig completion spec or cli-schema v1:
hnl sdkgen cli gh.fig.json --output handl-packages/generated/gh
hnl sdkgen cli gh.cli-schema.json --output handl-packages/generated/gh

# Or probe a tool's --help directly:
hnl sdkgen cli gh --output handl-packages/generated/gh
```

Foreign spec formats are detected by file name (`*.fig.json`,
`*.figspec`, `*.cli-schema.json`) or by content markers
(`schemaVersion`/`cli-schema-version`, a `subcommands` array, fig-shaped
`options` rows) and lowered into the same spec IR — constructs with no
IR analog are reported as diagnostics, not errors.

`--config tool.sdkgen.yaml` overlays `rename`/`drop`/`types`/`max_depth`;
`probe: { flag, max_depth }` in a spec file merges a live `--help` walk
with the authored command tree; `emit_spec:` in the config writes the
resolved tree back out as a `.clispec.yaml` to hand-tune.

## Docs

- [docs/clispec.md](docs/clispec.md) — spec schema reference and the
  `--config` overlay (`rename`/`drop`/`types`/`max_depth`/`emit_spec`)
- [docs/probe.md](docs/probe.md) — the `--help` scraper: fallback flags,
  parser coverage, confidence/warnings model
- [docs/emission.md](docs/emission.md) — `CommandNode` → `SdkgenModule`
  mapping, the `pos:`/`opt:`/`rest:`/`meta:` descriptors, and the
  `hnl-cli` port/adapter + `cli-c` output contract

Framework-level docs (IR, registry, CLI flags) live in the host repo:
[`host/docs/sdkgen/`](https://github.com/handl-lang/host/blob/main/docs/sdkgen/index.md).
