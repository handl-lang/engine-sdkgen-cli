# Emission — `CommandNode` → `SdkgenModule` → `hnl-cli` files

Two phases: `map_spec` (`src/map.rs`) lowers the spec tree into the
generic IR; `HnlCliEmitter` (`src/emit_hnl.rs`) renders it. Between them
sits the `arg_descriptors` contract — emitters rebuild argv shape from
descriptors alone.

## `CommandNode` → `SdkgenModule`

- **One `Function` per leaf command.** A node with `commands` is a
  branch (`gh pr`) and produces no function of its own. A tool with no
  subcommands is a root leaf and emits a single `<tool>` function.
- `Function.name` is the argv path snake-cased (`gh pr create` →
  `gh_pr_create`); `port_name` is the UpperCamel version
  (`GhPrCreate`). `c_decl` is the literal invocation (`gh pr create`),
  rendered in the port as `--- cli: gh pr create` — provenance and the
  subcommand path the adapter rebuilds argv from.
- `module.client_package = "cli"`, `backend_id = "cli"`,
  `library = <spec name>`, `module = <spec name snake-cased>`,
  `imports = [from "cli" import { Output, Spawn }]`. `errno_guard` is
  `false` — child exit status is data (`Output.code`), not an error
  channel.

### The `arg_descriptors` contract

| Descriptor | Port arg | Meaning |
| --- | --- | --- |
| `pos:<name>` | `String`/`Int`/`Bool` per positional kind | a positional, in argv order |
| `opt:<kind>:<name>:<flag>` | one trailing `opts: Obj` iff any options exist | a named option; `kind` ∈ `flag`|`string`|`int`|`strings`, `flag` the literal spelling |
| `rest:<name>` | trailing `[T]` | the variadic tail (`variadic: true` on the Function) |
| `meta:<hint>` | — | emitter hints, **not** argv shape — emitters ignore `meta:` when rebuilding argv |

`meta:json-capable` is stamped when the command offers a `--json`/
`--format`/`--output`/`-o` flag (option name or spelling) — consumers
know a machine-readable mode exists.

`return_port` is `Output`, `ret_descriptor` `"cli"`, `out_struct` unset.

## `hnl-cli` output contract

`HnlCliEmitter` writes two files:

```text
<output>/<module>.hnl                       — port
<output>/../<lib>-cli-<slug>/<module>.hnl   — adapter
```

`<lib>` is the basename of `--output`; `<slug>` is
`target_slug(module.target)` (`host` stays `host`). Both run through
`format_source_with_language` (the injected `Language` from
`SdkgenRequest`) — unformatted text is written only on a formatter
error.

Port (`gh/gh.hnl`):

```handl
--- Generated port for gh.gh
--- cli target: host
--- confidence: high            -- when a probe ran; low also gets a --- warning:

from "result" import { Result }
from "cli" import { Output, Spawn }

--- Create a pull request
--- cli: gh pr create
export GhPrCreate: (String, Obj, [String]) ~[1]~> Result<Output, Spawn>
```

Adapter (`gh-cli-host/gh.hnl`):

```handl
--- Generated adapter for gh-cli-host/gh

from "cli-c" import { run, argv }
from "../gh/gh" import { GhPrCreate, GhPrList }

gh_pr_create_flags := [{ name: "draft", flag: "--draft", kind: "flag" }, …]

--- json-capable                -- when meta:json-capable was stamped
export gh_pr_create: GhPrCreate := (branch, opts, extras) ~> do {
  run(argv("gh", ["pr", "create"], [branch], gh_pr_create_flags, opts, extras))
}
```

Per function: a `<name>_flags` table of `FlagSpec` records (one per
`opt:` descriptor, spec-name order) and an `export <name>: <Port> :=
(args) ~> do { run(argv(…)) }` binding. `opts` is `null` when the
command has no options; `rest` is `[]` when no `rest:` descriptor.
Keyword-colliding arg names get a `_` suffix (`data` → `data_`).

## The `cli`/`cli-c` runtime contract

Generated adapters bind against the `cli` port package and `cli-c`
backend (`handl-lang/lib-cli`):

- `cli` exports the types: `Output := { code, stdout, stderr }`,
  `Spawn := { code, message }` (spawn-time failure — the child's own
  exit status is `Output.code`, `128 + signo` on signal death),
  `FlagSpec := { name, flag, kind }`.
- `cli-c` implements `run: ([String]) ~[1]~> Result<Output, Spawn>`
  over `posix_spawnp` — **no `sh -c`**, argv verbatim — and
  `argv: (prog, subpath, positionals, flag_specs, opts, rest) ~>
  [String]`, a pure argv assembler that stringifies scalars and expands
  `opts` fields per `FlagSpec.kind`: `flag` emits the flag iff truthy;
  `string`/`int` emit `flag value`; `strings` emits `flag v` per
  element.

Because `cli-c` spawns `argv[0]` via PATH lookup with no shell, the
generated SDK inherits the same guarantee the probe runner makes: no
shell interpolation anywhere in the pipeline.
