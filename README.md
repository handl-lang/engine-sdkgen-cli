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
