# Changelog

All notable changes to `nbstrip` are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Mercurial support: `nbstrip install` inside an hg repository writes an
  `[encode]` pipe filter for `**.ipynb` to `.hg/hgrc` (repo-local, idempotent,
  preserves existing hgrc content; replaces the filter line on reinstall).
  Auto-detected — git wins when repositories are nested.

### Changed

- MSRV lowered from 1.88 to **1.70** (edition 2024 -> 2021; no functional
  change). The committed lockfile is v3 and pins MSRV-compatible dependency
  versions (`serde_json` 1.0.149, `zmij` 1.0.19), so `cargo install --locked
  nbstrip` works on rustc 1.70+; unlocked installs on modern toolchains
  resolve the latest.

### Fixed

- `--help` now lists the options (`-t`/`--textconv`, `-h`/`--help`,
  `-V`/`--version`); the flags existed but were undocumented.

## [0.1.0] - 2026-07-03

### Added

- Stripping engine: clears cell outputs, nulls execution counts (schema-valid),
  and removes transient UI metadata (notebook-level `signature`/`widgets`/
  `vscode`; cell-level `collapsed`/`scrolled`/`execution`/`ExecuteTime`/
  `heading_collapsed`/`hidden`). Honors the `nbstripout` `keep_output`
  convention (cell tag or metadata, notebook-level metadata; an explicit
  `false` beats a tag). Output is `nbformat`-byte-compatible (1-space indent,
  sorted keys, trailing newline preserved) and idempotent.
- CLI modes: in-place (`nbstrip FILE...`), stdout (`-t`/`--textconv`), and
  stdin→stdout for use as a git clean filter (always emits — a no-change
  shortcut would truncate the staged file).
- `nbstrip install`: registers the binary as the current repository's clean
  filter for `*.ipynb` — `filter.nbstrip.clean` (shell-quoted absolute path)
  and `filter.nbstrip.required=true` in the repo config, plus the attribute
  line in `.git/info/attributes`. Repo-local, idempotent, nothing to commit.
