# `*.clispec.yaml` — the authored spec

The `cli` backend's native input is a `*.clispec.yaml` file describing a
tool's command tree. Anything that is not a `*.yaml`/`*.yml` file is a
tool name and is probed instead (see [probe.md](./probe.md)).

```yaml
name: gh                      # required — module name basis + probed argv[0]
about: GitHub CLI             # module doc
probe:                        # optional — overlay a live --help walk
  flag: --help                #   help flag tried first (default --help)
  max_depth: 1                #   subcommand levels below root (default 1)
options: {…}                  # the root node is the tool itself
positionals: […]
rest: {…}
commands:                     # nested subcommands, keyed by argv word
  pr:
    about: Manage pull requests
    commands:
      create:
        about: Create a pull request
        options:
          title: { flag: "--title", kind: string, doc: "PR title" }
          draft:  { flag: "--draft", kind: flag }
          label:  { flag: "--label", kind: strings }
        positionals:
          - { name: branch, kind: string, doc: "base branch" }
        rest: { name: extras, kind: string }
```

## Schema

Top level (`CliSpec`, `src/spec.rs`):

| Key | Type | Meaning |
| --- | --- | --- |
| `name` | string, required | Tool/binary name — `module.library`, the emitted module stem, and argv\[0\] when probing |
| `about` | string | Module doc |
| `probe` | map | Presence (even `{}`) triggers a live probe merged under the authored tree; **never written by `emit_spec`** — a dumped spec is already resolved |
| `options` / `positionals` / `rest` / `commands` | — | The root `CommandNode` — the tool is its own command |

`probe:` (`ProbeSpec`): `flag` (help flag tried first, default
`--help`), `max_depth` (subcommand levels below root, default 1;
config `max_depth` overrides).

`CommandNode` — every command, recursively:

| Key | Type | Meaning |
| --- | --- | --- |
| `about` | string | Doc for the emitted function |
| `options` | map `name → OptionSpec` | Named options; the map key is the spec name and the `opts` record field |
| `positionals` | list of `PositionalSpec` | Positional args, in argv order |
| `rest` | `PositionalSpec` | Optional variadic tail (`args...`) |
| `commands` | map `name → CommandNode` | Subcommands — a node with `commands` is a branch and emits no function |

`OptionSpec`: `flag` (required spelling handed back to the tool —
must start with `-`, must not contain `:` since the IR descriptor is
colon-separated), `kind` ∈ `flag | string | int | strings` (default
`flag`; `strings` is repeatable), `doc`.

`PositionalSpec`: `name`, `kind` ∈ `string | int | bool` (default
`string`), `doc`. Positionals map to port args `String`/`Int`/`Bool`;
`rest` maps to a trailing `[T]`.

## Validation

`CliSpec::load` validates (`spec.validate()`): non-empty `name`; option
names non-empty without `:`; flags start with `-`, contain no `:`, and
are unique per node; option kinds and positional kinds from the lists
above; positional names non-empty, unique, no `:`; subcommand names
non-empty. Errors carry the dotted command path (`gh.pr.create: …`).

## Spec + probe merge

When `probe:` is present, the authored tree overlays the discovered one
**field-by-field, authored wins** (`merge_commands` in `src/backend.rs`):
a matching command merges `about`/options per name; authored
`positionals` replace probed ones only when non-empty; authored `rest`
wins when present. Root `options`/`positionals`/`rest` fill from the
probe only when the spec left them empty. A spec can therefore declare
only tree shape and let the probe supply options.

## `*.sdkgen.yaml` — the `--config` overlay

`CliConfig` (`src/config.rs`) is loaded from `--config` and applied
**after** spec load/probe, before validation. All dotted paths refer to
the *original* command names — order is `drop`, then `types`, then
`rename`, so a renamed command's new name is not a valid `types` key.
Unmatched paths produce `config: … matched no command` diagnostics.

```yaml
rename: { "pr.create": "make" }            # dotted path → new leaf name
drop: ["pr.delete"]                      # remove command subtrees
types: { "pr.create.label": strings }    # re-kind an option or positional
max_depth: 2                             # probe cap — overrides probe.max_depth
emit_spec: gh.clispec.yaml               # dump resolved tree ("-" = stdout)
```

| Key | Type | Meaning |
| --- | --- | --- |
| `rename` | map `path → name` | Rename the leaf of a dotted command path |
| `drop` | list of paths | Remove command subtrees entirely |
| `types` | map `path.arg → kind` | Override an option kind (`flag`/`string`/`int`/`strings`) or positional/rest kind (`string`/`int`/`bool`) |
| `max_depth` | int | Probe recursion cap; wins over `probe.max_depth` |
| `emit_spec` | path or `-` | Write the resolved tree (post-probe, post-overlay) as canonical `.clispec.yaml` — the "probe then hand-tune" workflow |

`emit_spec` output is `CliSpec::dump()` — self-contained (no `probe:`),
re-parses via `CliSpec::load` to the same tree. The pipeline: probe a
noisy tool, dump the spec, hand-tune, then generate from the spec file.
