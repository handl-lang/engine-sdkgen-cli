# `*.clispec.yaml` — the authored spec

The `cli` backend's native input is a `*.clispec.yaml` file describing a
tool's command tree. Recognized *foreign* spec formats (see
[below](#foreign-input-formats)) are lowered into the same IR; anything
else that is not a spec is a tool name and is probed instead (see
[probe.md](./probe.md)).

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
`flag`; `strings` is repeatable — it emits `flag <v>` once per list
element), `doc` (one-line description — threads into the emitted
FlagSpec `doc` field and the port's `opts:` doc table; probed options
pick it up from the help row's description column). Probed option rows infer `strings` from repeatability
signals in the help text itself — `...`/`[]`/`[=VAL]` spec markers,
plural `VALS` metavars, "multiple times"/"(repeatable)"/
"comma-separated" phrasing — so `git -C`-style repeatable flags map
`strings` without a `types:` override (see
[probe.md](./probe.md) for the full signal list).

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
| `emit_spec` | path or `-` | Write the resolved tree (post-probe, post-overlay) as canonical `.clispec.yaml` — the "probe then hand-tune" workflow. The `--emit-spec <path>` CLI flag sets the same value and wins when both are present |

`emit_spec` output is `CliSpec::dump()` — self-contained (no `probe:`),
re-parses via `CliSpec::load` to the same tree. The pipeline: probe a
noisy tool, dump the spec (`--emit-spec out.clispec.yaml`, or `-` for
stdout — the dump is the only thing printed to stdout; diagnostics go
to stderr), hand-tune, then generate from the spec file.

## Foreign input formats

Three external spec ecosystems ingest directly (`src/foreign/`, one
file per dialect) — the result is the same `CliSpec` IR, so
`hnl codegen cli gh.fig.json …` works exactly like the yaml input, and
`emit_spec` dumps a canonical `.clispec.yaml` of the imported tree.

Detection order:

1. **File name** — `*.fig.json` / `*.figspec` → fig;
   `*.cli-schema.json` → cli-schema; `*.kdl` / `*.usage.kdl` → usage.
   A claimed name that is missing or malformed is a hard error, never
   a probe.
2. **Content markers** — a JSON object with `schemaVersion` or
   `cli-schema-version` → cli-schema; a `subcommands` array, or an
   `options` array of fig-shaped rows (`{"name": ["--long", "-s"], …}`)
   → fig; a document that parses as KDL with top-level
   `cmd`/`flag`/`arg`/`option`/`usage` nodes → usage.
3. Anything else falls through to the probe path (a binary, a bare tool
   name).

### fig (withfig/autocomplete)

Serialized [fig completion specs](https://github.com/withfig/autocomplete)
— the de-facto standard, 600+ tools — are `subcommands` / `options` /
`args` trees.

| fig field | IR mapping |
| --- | --- |
| `name` (root, string or `string[]`) | `spec.name` (first element; extras warn) |
| `description` | `about` (root) / `doc` (options, args) |
| `subcommands[].name` | command key (first of an array; the rest → `[aliases: …]` annotation); duplicate names merge |
| `options[].name` | the first `--long` spelling is `OptionSpec.flag`; other spellings → `[aliases: …]`; spec key is the flag ident (`--dry-run` → `dry_run`) |
| `options[].args` | present → `kind: string`; absent → `kind: flag` |
| `options[].isRepeatable` (`true` or `n > 1`) | `strings` when the option takes a value; `[repeatable]` annotation on a flag |
| `isRequired`, `isPersistent`, `exclusiveOn`, `dependsOn` | `[required]` / `[persistent]` / `[exclusive with: …]` / `[depends on: …]` doc annotations |
| `args[]` | `positionals` (in order); `isVariadic` → `rest`; `isOptional`/`default` → annotations |
| everything else (`suggestions`, `template`, `generators`, `loadSpec`, `generateSpec`, `parserDirectives`, `displayName`, …) | dropped — one warning per node listing the fields |

### cli-schema v1

[cli-schema](https://cli-schema.github.io) is the emerging agent-facing
JSON schema. `schemaVersion` must be `1` (`"1"` string or
`cli-schema-version` accepted).

| cli-schema field | IR mapping |
| --- | --- |
| `name`, `description` | `spec.name`, `spec.about` (`version` is read but unused) |
| `globalOptions` | flag parameters propagated into *every* command node (and the root leaf); a command's own spelling wins ties |
| `rootDefault` | `parameters` → the root node's own options/positionals; `summary`/`notes` → `about` fallback |
| `namespaces[]` | `segment` → command key; `options` → flags inherited by the whole subtree; `defaultCommand.parameters` → the namespace node's own; `commands`/`namespaces` recurse |
| `commands[]` | `name` → command key; `path` → namespace-word prefix (`{"name": "delete", "path": ["repo"]}` is `repo delete`) |
| `parameters[]` | `role` `flag`/`confirmationSkip`/`dryRun` → option (`flag: --{name}`); `positional` → positional or `rest` when `variadic` |
| `type` | `boolean`→`flag`, `integer`/`number`→`int`, `array`→`strings`, `string`/`enum`→`string` (positionals: `int`/`bool`/`string`) |
| `required`, `shortName`, `aliases`, `defaultValue`, `enumValues`, `separator`, `elementType`, `hidden`, `deprecated`, `validations`, `repeatable` on a boolean | `[…]` doc annotations |
| `intent`, `output`, `streaming`, `usage`, `examples`, `environment`, `requiresAuth`, … | dropped — one warning per node listing the fields (`x-*` vendor extensions pass silently) |

### usage (KDL)

[usage](https://usage.jdx.dev) specs — KDL documents (`flag "-f
--force"`, `arg "<file>"`, `cmd "name" { … }`; mise ships one) — read
by the in-crate KDL subset reader (`src/foreign/kdl.rs`), which also
accepts the bare-word values real `*.usage.kdl` files use.

| usage construct | IR mapping |
| --- | --- |
| `bin` / `name` | `spec.name` (`bin` wins) |
| `about`/`about_md`/`long_about`, `usage`, top-level `alias` | `spec.about` (+ `[aliases: …]`/`[usage: …]` annotations) |
| `flag`/`option "…spellings <value>"` | option — first `--long` spelling is `OptionSpec.flag`, the rest `[aliases: …]`; a value token/child `arg` → `string`, `var`/`...` → `strings`, none → `flag` |
| `arg "<x>"` | `positionals` in order; `...`/`var=#true` on the last → `rest` |
| `cmd "name" { … }` | command key; `alias`/`help`/`long_help` → `about` + annotations; children recurse |
| `flagset "n" { … }` + `use "n"` | the set's flags/args spliced into the using scope |
| `global=#true` on a flag | inherited by the whole subtree (own spelling wins) |
| `required`, `default`, `env`, `count`, `hide`, `deprecated`, `negate`, `conflicts`, `overrides`, `requires`, `choices`, `delimiter`, `action`, `var_min`/`var_max`, `validate`, `note`/`warning` children | `[…]` doc annotations |
| `complete`/`mount`/`exec`, `help_template`/`display_order`, `output`/`exit_code`/`config`/`group`/`clause`, unknown nodes/props | dropped — one warning per scope |

Warnings land in `module.diagnostics` prefixed `fig {path}: …` /
`cli-schema {path}: …` / `usage {path}: …`; the lowered spec runs
through the same `CliSpec::validate` as authored yaml, so an
unmappable flag spelling is a normal validation error.
