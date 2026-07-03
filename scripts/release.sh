#!/usr/bin/env bash
#
# release.sh — publish nbstrip to crates.io + GitHub.
#
# Usage:
#   scripts/release.sh [--yes]
#
#   --yes     skip the confirmation prompt before the irreversible steps
#
# The version is read from Cargo.toml — bump it (and cut the version section in
# CHANGELOG.md from [Unreleased]) and commit BEFORE running this. Tag scheme:
# bare `vX.Y.Z`.
#
# This automates the mechanical half of docs/crate_release_process.md (it
# assumes the review, validation gate, changelog, and version bump are already
# done and committed). In order it:
#   1. pre-flight: on master, clean tree, tools authenticated, tag not taken,
#      version not already on crates.io;
#   2. `git pull --rebase` — absorb the coverage-badge CI commit BEFORE tagging,
#      so the tag lands on the final commit and never has to be moved;
#   3. `cargo publish --dry-run` — verify the exact artifact builds in isolation;
#   4. tag the release commit, push the branch + tag;
#   5. `cargo publish` (IRREVERSIBLE — crates.io versions are immutable);
#   6. `gh release create` (notes from CHANGELOG.md).
#
set -euo pipefail

die() { echo "release: $*" >&2; exit 1; }
step() { printf '\n=== %s ===\n' "$*"; }

CRATE="nbstrip"

# --- parse args ---------------------------------------------------------------
ASSUME_YES=0
for arg in "$@"; do
    case "$arg" in
        --yes | -y) ASSUME_YES=1 ;;
        *) die "unknown option: $arg (usage: release.sh [--yes])" ;;
    esac
done

# --- run from the repo root ---------------------------------------------------
ROOT="$(git rev-parse --show-toplevel)" || die "not inside a git repository"
cd "$ROOT"

# Version: the first `version = "..."` inside the [package] table.
VERSION="$(sed -n '/^\[package\]/,/^\[/{s/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p;}' Cargo.toml | head -n1)"
[ -n "$VERSION" ] || die "could not read [package] version from Cargo.toml"
TAG="v$VERSION"

echo "release: $CRATE $VERSION  ->  tag $TAG"

# --- pre-flight ---------------------------------------------------------------
step "pre-flight"
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = "master" ] || die "not on master (on '$BRANCH')"
git diff --quiet && git diff --cached --quiet \
    || die "working tree is dirty — commit the version bump + changelog first"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    die "tag $TAG already exists — the version is already released, or delete the stale local tag (git tag -d $TAG)"
fi
# Refuse a version already on crates.io. `cargo publish --dry-run` does NOT check
# this (it only builds), so without it the clash would only surface at the final
# upload — after the rebase, tag, and push. Best-effort: needs curl + network; if
# either is missing we fall through to cargo's own check at publish time.
if command -v curl >/dev/null 2>&1; then
    INDEX_URL="https://index.crates.io/${CRATE:0:2}/${CRATE:2:2}/$CRATE"
    if curl -fsSL "$INDEX_URL" 2>/dev/null | grep -Fq "\"vers\":\"$VERSION\""; then
        die "$CRATE $VERSION is already published on crates.io (versions are immutable) — bump the version in Cargo.toml"
    fi
fi
command -v gh >/dev/null 2>&1 || die "the 'gh' CLI is required"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated — run 'gh auth login'"
[ -n "${CARGO_REGISTRY_TOKEN:-}" ] || ls ~/.cargo/credentials* >/dev/null 2>&1 \
    || die "no crates.io token — run 'cargo login' or set CARGO_REGISTRY_TOKEN"

# --- sync with the remote BEFORE tagging --------------------------------------
# The CI coverage job pushes a "[skip ci]" badge commit after every master push.
# Rebasing now means the release commit reaches its FINAL hash before we tag it,
# so the tag never has to be deleted and re-created.
step "git pull --rebase origin master"
git fetch origin master
git pull --rebase origin master

# --- verify the artifact (no upload) ------------------------------------------
step "cargo publish --dry-run"
cargo publish --dry-run

# --- confirm before anything irreversible -------------------------------------
echo
echo "About to PUSH and PUBLISH — this is IRREVERSIBLE (crates.io versions are immutable):"
echo "  crate:   $CRATE $VERSION"
echo "  tag:     $TAG -> $(git rev-parse --short HEAD)  ($(git log -1 --format=%s))"
echo "  remote:  $(git remote get-url origin)"
if [ "$ASSUME_YES" -ne 1 ]; then
    printf 'Continue? [y/N] '
    read -r reply </dev/tty
    case "$reply" in y | Y | yes | YES) ;; *) die "aborted by user" ;; esac
fi

# --- tag, publish, then push the tag -------------------------------------------
# The tag is created BEFORE publishing (it marks exactly what gets published)
# but pushed only AFTER `cargo publish` succeeds: a failed publish then leaves
# just a local tag to clean up (`git tag -d`), never a stale remote tag that a
# post-rebase retry can't overwrite (the 0.2.0 release hit exactly that).
step "tag + push branch"
git tag -a "$TAG" -m "$CRATE $VERSION"
git push origin master

step "cargo publish"
cargo publish

step "push tag"
git push origin "$TAG"

# --- GitHub release -----------------------------------------------------------
step "gh release create $TAG"
NOTES="$(mktemp)"
trap 'rm -f "$NOTES"' EXIT
if [ -f CHANGELOG.md ]; then
    awk -v v="$VERSION" '
        $0 ~ "^## \\[" v "\\]" { f = 1; next }
        /^## \[/ { f = 0 }
        f { print }
    ' CHANGELOG.md > "$NOTES"
fi
if [ -s "$NOTES" ]; then
    gh release create "$TAG" --title "$TAG" --notes-file "$NOTES"
else
    gh release create "$TAG" --title "$TAG" \
        --notes "\`$CRATE\` $VERSION. See CHANGELOG.md and <https://crates.io/crates/$CRATE/$VERSION>."
fi

REPO_URL="$(git remote get-url origin | sed 's#git@github.com:#https://github.com/#; s#\.git$##')"
echo
echo "release: done — $CRATE $VERSION"
echo "  crates.io: https://crates.io/crates/$CRATE/$VERSION"
echo "  github:    $REPO_URL/releases/tag/$TAG"
