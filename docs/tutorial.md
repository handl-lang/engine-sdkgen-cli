# Tutorial — generate your first CLI SDK

This walks a real tool — `gh` — from a bare binary to a callable Handl
SDK: probe its `--help`, emit the port/adapter pair, and invoke a
command. The only inputs are a tool on `PATH` and `hnl` with the
`cli` sdkgen target registered (the default feature set — see
`host/docs/sdkgen/cli.md`).

## 1. Probe the tool

```bash
hnl sdkgen cli gh --output generated/gh
```

The positional argument is a bare tool name, so the backend probes
`gh --help`, `gh <sub> --help`, … and parses each page into a command
tree. Watch stderr: every probe warning lands as a
`sdkgen: diagnostic: …` line, led by `confidence: high|medium|low`.
Low confidence means the scrape is unreliable — see
[howto-tune-a-probe.md](./howto-tune-a-probe.md) before trusting the
output.

Two files appear:

```text
generated/gh/gh.hnl            — the port: types + signatures
generated/gh-cli-host/gh.hnl   — the adapter over cli-c
```

## 2. Read the port

`generated/gh/gh.hnl` holds one `export` per *leaf* command — branch
nodes like `gh pr` are namespaces, not functions:

```handl
--- List pull requests
--- opts:
---   state (string): filter by state
---   limit (int): maximum number to fetch
--- cli: gh pr list
export GhPrList: (Obj, [String]) ~[1]~> Result<Output, Spawn>
```

The signature reads `({positionals…}, opts: Obj, rest: [T])`: a
positional arg per `pos:` descriptor, one `opts` record iff the
command has named options, and a trailing list when the command is
variadic. `Output`/`Spawn` come from the `cli` package; the child
exit status is data (`Output.code`), never an exception.

The sibling adapter binds each port to `cli-c`'s `run`/`argv` — no
shell anywhere; argv is assembled verbatim.

## 3. Call it

Generated adapters are ordinary `.hnl` modules. Import the adapter
(not the port) and call the snake-cased leaf name:

```handl
--- demo.hnl
from "generated/gh-cli-host/gh" import * as m

--- `opts` fields are the spec names; flag-kind options take booleans.
r := tag.untag(m.gh_pr_list({ state: "open" }, []))
out := r.value.stdout
```

`opts` is `null`-able by omission — call `m.gh_pr_list(null, [])` for
no options. Functions return `Result<Output, Spawn>`; `tag.untag`
unwraps to `.value` (`Output`) on `Ok` — see `lib-cli-gen`'s
`src/*.test.hnl` for runnable smoke-test shapes.

For a command whose stdout is JSON (`meta:json-capable` — any
`--json`/`--format`/`--output`/`-o` flag), compose the free `parse`
arrow from lib-json as the adapter's `--- json-capable:` comment
suggests; there is deliberately no `x.json()` helper (see
[design.md](./design.md)).

## Where next

- The probe got a shape wrong → [howto-tune-a-probe.md](./howto-tune-a-probe.md)
- You already have a fig/cli-schema/usage spec → [howto-foreign-specs.md](./howto-foreign-specs.md)
- Authoring the yaml by hand → [clispec.md](./clispec.md)
- What the emitted files are built from → [emission.md](./emission.md)
