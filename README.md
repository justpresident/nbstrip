# nbstrip

![CI](https://github.com/justpresident/nbstrip/actions/workflows/ci.yml/badge.svg?branch=master)
![Coverage](https://raw.githubusercontent.com/justpresident/nbstrip/master/.github/badges/coverage.svg)
[![Crates.io](https://img.shields.io/crates/v/nbstrip.svg)](https://crates.io/crates/nbstrip)
[![Docs.rs](https://docs.rs/nbstrip/badge.svg)](https://docs.rs/nbstrip)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Fast Jupyter notebook output stripper and git clean filter, in Rust with only two
dependencies (`serde`, `serde_json`). Cell outputs, execution counts, and
transient UI metadata never reach git; your working tree keeps everything for
Jupyter. Diffs stay readable, repositories stay small, and megabytes of
embedded plotly figures stop hiding real changes.

## Install

```bash
cargo install nbstrip
```

MSRV is **Rust 1.70**. On older toolchains (< 1.85) use
`cargo install --locked nbstrip` — the shipped lockfile pins
dependency versions that still support 1.70; unlocked installs on
current toolchains resolve the latest.

## Wire a repository (once per clone)

```bash
cd your-repo
nbstrip install    # detects git or Mercurial
```

**Mercurial:** inside an hg repository, `install` writes an `[encode]` filter
to `.hg/hgrc` (`**.ipynb = pipe: /path/to/nbstrip`) — same effect: commits
store stripped notebooks, the working directory keeps its outputs. Repo-local
and idempotent; a reinstall updates the binary path in place. When a git and
an hg repository are nested, git wins.

**Git:** `install` registers the binary as the repository's clean filter for `*.ipynb`
(`filter.nbstrip.clean` + `filter.nbstrip.required` in the repo's git config,
the attribute line in `.git/info/attributes`). Nothing needs committing. From
then on `git add` stages notebooks stripped, `git diff`/`git status` compare
through the filter, and the working tree keeps its outputs.

Because git config never travels with a clone — by design — **every clone runs
`nbstrip install` once**. A clone that skipped it strips nothing and warns
about nothing. Teams can commit `*.ipynb filter=nbstrip` to `.gitattributes`
so the *routing* travels with the repo; each clone still needs the one-time
`nbstrip install` to define the filter itself (it is idempotent, and
`filter.nbstrip.required=true` then makes an unwired clone fail the `git add`
loudly instead of silently committing outputs).

## What gets stripped

- **cell outputs** — cleared
- **execution counts** — nulled (the key stays, as the nbformat schema requires)
- **transient metadata** — notebook-level `signature`, `widgets`, `vscode`;
  cell-level `collapsed`, `scrolled`, `execution`, `ExecuteTime`,
  `heading_collapsed`, `hidden`

Sources, markdown cells, and authored metadata pass through untouched. Output
is byte-compatible with how `nbformat` writes files (1-space indent, sorted
keys, trailing newline preserved) and stripping is idempotent, so the filter
never churns.

**Keeping an output deliberately:** tag a cell `keep_output` (or set
`keep_output: true` in its metadata; notebook-level metadata keeps every cell).
An explicit `keep_output: false` beats a tag. This is the `nbstripout`
convention, so notebooks move between the two tools cleanly.

## Other modes

```bash
nbstrip notebooks/*.ipynb        # rewrite files in place (Clear All Outputs, but scriptable)
nbstrip -t nb.ipynb              # print the stripped notebook to stdout
nbstrip < in.ipynb > out.ipynb   # stdin to stdout (what git calls)
```

In-place mode is handy when the *local* file should shrink too — e.g. after a
heavy plotting session; re-running the notebook regenerates everything.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The lint policy is strict (`pedantic`/`nursery`/`cargo` plus
`unwrap_used`/`expect_used`/`panic`); fix findings rather than allowing them.
Releases follow [docs/crate_release_process.md](docs/crate_release_process.md)
via [scripts/release.sh](scripts/release.sh).

## License

Apache-2.0. Modeled on [nbstripout](https://github.com/kynan/nbstripout) and
[nbstripout-fast](https://github.com/deshaw/nbstripout-fast); reimplemented
from scratch.
