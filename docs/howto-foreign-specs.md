# How to ingest a foreign spec

Three spec ecosystems lower into the same `CliSpec` IR, so generation
works exactly like a `.clispec.yaml` input:

- **fig** — serialized [withfig/autocomplete](https://github.com/withfig/autocomplete)
  completion specs (600+ tools).
- **cli-schema v1** — the [cli-schema](https://cli-schema.github.io)
  agent-facing document.
- **usage** — [usage.jdx.dev](https://usage.jdx.dev) KDL specs (mise
  ships one).

## Steps

1. Name the file so detection claims it, or rely on content markers:

   | Dialect | Claimed by name | Sniffed by content |
   | --- | --- | --- |
   | fig | `*.fig.json`, `*.figspec` | `subcommands` array, or fig-shaped `options` rows (`{"name": ["--long", "-s"], …}`) |
   | cli-schema | `*.cli-schema.json` | `schemaVersion` / `cli-schema-version` key |
   | usage | `*.kdl`, `*.usage.kdl` | parses as KDL with top-level `cmd`/`flag`/`arg`/`option`/`usage` nodes |

   A claimed name that is missing or malformed is a hard error — never
   silently probed as a tool.

2. Generate:

   ```bash
   hnl codegen cli gh.fig.json --output generated/gh
   ```

3. Read the warnings. Every construct with no IR analog is reported,
   not swallowed — `codegen: diagnostic: fig gh.checkout: unsupported
   subcommand fields dropped: loadSpec`. Constructs that have meaning
   but no argv shape (`aliases`, `required`, `exclusiveOn`, `intent`,
   `environment`, suggestion `generators`, …) survive as bracketed doc
   annotations (`[aliases: -m]`, `[intent: destructive]`) on the
   generated docs. The field-by-field mapping tables are in
   [clispec.md §foreign input formats](./clispec.md#foreign-input-formats).

4. Optional — convert to the canonical spec for curation:

   ```bash
   hnl codegen cli gh.fig.json --output generated/gh --emit-spec gh.clispec.yaml
   ```

   The dump is an ordinary `.clispec.yaml`; edit it as in
   [howto-tune-a-probe.md](./howto-tune-a-probe.md) and generate from
   it thereafter.
