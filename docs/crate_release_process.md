# Crate release process

`nbstrip` publishes **one crate to crates.io**, tagged `vX.Y.Z`. The version
lives in `Cargo.toml`; the changelog is `CHANGELOG.md` (Keep a Changelog
format — unshipped work accumulates under `## [Unreleased]` and is cut into a
dated version section at release time).

crates.io versions are **immutable**: once `cargo publish` succeeds you cannot
overwrite or re-upload that version (only *yank* it). Everything before the
publish step exists to make the artifact correct *before* it goes out. No
prebuilt binaries are shipped — the GitHub release is just notes; users install
with `cargo install nbstrip`.

## Steps

1. **Validation gate** (what CI runs — run it locally first):

   ```bash
   cargo fmt --all --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --all-features
   cargo doc --no-deps   # with RUSTDOCFLAGS="-D warnings"
   ```

2. **Changelog + version bump, one commit.** Move the `[Unreleased]` content
   into a new `## [X.Y.Z] - YYYY-MM-DD` section (leave `[Unreleased]` present
   and empty), bump `version` in `Cargo.toml` to match, commit both together
   on `master`.

3. **Release** — the mechanical half is scripted:

   ```bash
   scripts/release.sh          # add --yes to skip the confirmation
   ```

   The script pre-flights (master, clean tree, tag free, version not already
   on crates.io, `gh` + cargo credentials), rebases onto origin (absorbing the
   CI coverage-badge commit so the tag lands on the final hash), runs
   `cargo publish --dry-run`, then — after one confirmation — tags, pushes,
   publishes, and creates the GitHub release with this version's changelog
   section as notes.

4. **Verify**: the crates.io page renders, `cargo install nbstrip` works from
   a clean environment, docs.rs builds.

## If something goes wrong

- **Before `cargo publish`**: nothing is irreversible — delete the local tag
  (`git tag -d vX.Y.Z`), fix, redo.
- **After `cargo publish`**: the version is live forever. A broken release is
  *yanked* (`cargo yank --version X.Y.Z`), which stops new dependents without
  breaking existing lockfiles, and a fixed `X.Y.Z+1` is released.
