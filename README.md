# sdkgen-cli

`handl-plugin-sdkgen-cli` — the `cli` backend and `hnl-cli` emitter for
the Handl SDK generator (ADR-0107): SDKs for tools that only ship a
binary. Inputs are authored `*.clispec.yaml`, foreign specs (fig /
cli-schema v1 / usage-KDL), or a bare tool name probed via `--help`;
output is a `.hnl` port + adapter pair against the `cli`/`cli-c`
packages (`handl-lang/lib-cli`).

```bash
hnl sdkgen cli gh --output generated/gh          # probe a tool
hnl sdkgen cli gh.clispec.yaml --output generated/gh   # or a spec
```

Sibling-repo deps are git dependencies (`branch = "main"`); `[patch]`
tables resolve them from sibling checkouts.

## Docs

| | |
| --- | --- |
| Tutorial | [docs/tutorial.md](docs/tutorial.md) — generate your first CLI SDK |
| How-to | [docs/howto-tune-a-probe.md](docs/howto-tune-a-probe.md) · [docs/howto-foreign-specs.md](docs/howto-foreign-specs.md) |
| Reference | [docs/clispec.md](docs/clispec.md) (spec schema + `--config` overlay + foreign-format mappings) · [docs/probe.md](docs/probe.md) (`--help` scraper) · [docs/emission.md](docs/emission.md) (`SdkgenModule` mapping + output contract) |
| Explanation | [docs/design.md](docs/design.md) — design rationale |

Framework-level docs (IR, registry, `hnl sdkgen` flags) live in the
host repo:
[`host/docs/sdkgen/`](https://github.com/handl-lang/host/blob/main/docs/sdkgen/index.md).
