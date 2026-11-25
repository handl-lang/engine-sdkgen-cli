# Agent guide — tool-codegen-cli

`handl-plugin-codegen-cli` — the `cli` source and `hnl-cli` emitter for
the Handl codegen pipeline (PL-004): SDKs for tools that only ship a
binary. Inputs are authored `*.clispec.yaml`, foreign specs (fig /
cli-schema v1 / usage-KDL), or a bare tool name probed via `--help`;
output is a `.hnl` port + adapter pair against the `cli`/`cli-c`
packages (`handl-lang/lib-cli`). Rust 2024.

## Layout

| path | role |
| --- | --- |
| `src/lib.rs` | crate docs + public API |
| `src/spec.rs`, `src/config.rs` | `*.clispec.yaml` model + config |
| `src/foreign/` | foreign-spec frontends — `fig`, `cli_schema`, `usage` (KDL) |
| `src/probe.rs`, `src/help.rs` | `--help` probing of real binaries |
| `src/map.rs`, `src/emit_hnl.rs`, `src/backend.rs` | spec → IR map + `.hnl` emission |
| `tests/` | `backend_probe`, `corpus`, `foreign_spec`, `golden`, `probe_real`, `registry` + `fixtures/` |
| `docs/` | backend/emitter doc |
| `xtask/`, `workflows/gate.yaml`, `bupha.toml`, `.cargo/` | vendored repo gate + CI wiring |

## Contract

- Deps resolve from git (`handl-plugin-codegen`, `handl-plugin-ext-fmt`
  → `handl-lang/host`; `handl-language` → `handl-lang/kernel`, all
  `branch = "main"`); `[patch]` tables map siblings locally.
- Probe tests may depend on real binaries on PATH — check
  `tests/probe_real.rs`/`corpus.rs` before assuming hermeticity.

## Verification

```bash
cargo xtask gate   # fmt --check → clippy -D warnings → test → doc
```


<!-- myco:agent-awareness -->

## Agent awareness

- Session start / any work discussion: run `myco status --prompt`
  before proposing next steps — know the board first.
- User describes new work → offer to capture it (`myco todo add`,
  `issue new`, `proposal new`). Don't wait to be asked.
- Picking up a task: `myco resume <ID>` is the briefing.
- Wrapping up: `myco handoff <ID>` anything resumable; keep statuses
  honest (claimed → in-progress → blocked/done).

<!-- /myco:agent-awareness -->
