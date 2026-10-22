# How to tune a probe

A live `--help` probe is a scrape — the parser reports how well it
went (`confidence: high|medium|low` plus per-node diagnostics on
stderr, mirrored as `--- confidence:` / `--- diagnostic:` headers in
the emitted port). Two workflows correct it:

- **Curate a dumped spec** — the usual path for anything you ship.
- **Overlay the probe** — for small, targeted corrections that should
  keep tracking the live help.

## Dump, edit, generate

1. Probe once, writing the resolved tree as a spec:

   ```bash
   hnl sdkgen cli gh --output generated/gh --emit-spec gh.clispec.yaml
   ```

   `--emit-spec -` prints the spec to stdout instead (diagnostics stay
   on stderr). `emit_spec:` in a `--config` file does the same; the
   flag wins.

2. Hand-edit `gh.clispec.yaml`. Typical fixes:

   - delete bogus commands the scraper promoted (`commands:` keys);
   - rename spec keys — they become `opts` field names and (for
     commands) the argv word;
   - re-kind options (`flag`/`string`/`int`/`strings`) and positionals
     (`string`/`int`/`bool`) the parser guessed wrong;
   - add `rest: { name: args, kind: string }` to leaves so flags the
     spec doesn't model still pass through verbatim;
   - fill `about`/`doc` fields — they land in the emitted SDK docs.

   The schema is [clispec.md](./clispec.md). Note `probe:` is never
   written into the dump — the file is already the resolved tree.

3. Generate from the spec — spec mode is deterministic (no probing),
   so the output now reflects your edits exactly:

   ```bash
   hnl sdkgen cli gh.clispec.yaml --output generated/gh
   ```

   `lib-cli-gen`'s `specs/*.clispec.yaml` are the working convention:
   probe-emitted where usable, hand-corrected baselines (`git`,
   `ffmpeg`, `aws`) where the help grammar defeats the parser.

## Overlay instead of curating

When the probe is mostly right and you want it to keep tracking new
releases, put corrections *around* it rather than forking the tree:

- `probe:` in a spec file — declare only the tree shape (or the
  commands you care about) and let the probe fill options. Authored
  nodes win field-by-field over the probed ones (see *Spec + probe
  merge* in [clispec.md](./clispec.md)).
- `--config tool.sdkgen.yaml` — `drop`/`rename`/`types`/`max_depth`
  over the resolved tree without touching a spec at all (see
  *`*.sdkgen.yaml` — the `--config` overlay* in
  [clispec.md](./clispec.md)).

Rule of thumb: `types:`/`drop:`/`rename:` for a handful of fixes;
`--emit-spec` + hand-tuning when confidence is `low` or the tree
needs real curation.
