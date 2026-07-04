//! End-to-end: `nbstrip install` wires a repository, and `git add` then
//! stages notebooks stripped while the working tree keeps its outputs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const NOTEBOOK: &str = r#"{
 "cells": [
  {
   "cell_type": "code",
   "execution_count": 9,
   "id": "cafe0001",
   "metadata": {},
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": ["huge plotly blob\n"]
    }
   ],
   "source": ["print('hi')"]
  }
 ],
 "metadata": {},
 "nbformat": 4,
 "nbformat_minor": 5
}
"#;

struct TempRepo(PathBuf);

impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = env::temp_dir().join(format!("nbstrip-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        run(&dir, "git", &["init", "-q"]);
        Self(dir)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("running {cmd}: {e}"));
    assert!(
        out.status.success(),
        "{cmd} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn install_then_git_add_stages_stripped_notebooks() {
    let repo = TempRepo::new("install");
    let dir = &repo.0;

    run(dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);

    // The filter is registered and the attribute written.
    let clean = run(dir, "git", &["config", "--get", "filter.nbstrip.clean"]);
    assert!(clean.contains("nbstrip"), "clean filter: {clean}");
    let smudge = run(dir, "git", &["config", "--get", "filter.nbstrip.smudge"]);
    assert_eq!(smudge.trim(), "cat");
    let required = run(dir, "git", &["config", "--get", "filter.nbstrip.required"]);
    assert_eq!(required.trim(), "true");
    let attrs = fs::read_to_string(dir.join(".git/info/attributes")).unwrap();
    assert!(attrs.contains("*.ipynb filter=nbstrip"));

    // Round trip: the index gets the stripped bytes, the worktree keeps outputs.
    fs::write(dir.join("nb.ipynb"), NOTEBOOK).unwrap();
    run(dir, "git", &["add", "nb.ipynb"]);
    let staged = run(dir, "git", &["show", ":nb.ipynb"]);
    assert!(staged.contains("\"outputs\": []"), "staged: {staged}");
    assert!(staged.contains("\"execution_count\": null"));
    assert!(!staged.contains("huge plotly blob"));
    let worktree = fs::read_to_string(dir.join("nb.ipynb")).unwrap();
    assert!(worktree.contains("huge plotly blob"));
}

#[test]
fn checkout_restores_notebooks_after_install() {
    // Regression: install used to set `filter.nbstrip.required` without a
    // smudge command; git treats the missing command as a failed filter, so
    // any checkout/restore of an .ipynb died with "smudge filter nbstrip
    // failed". The explicit `cat` pass-through keeps checkout working.
    let repo = TempRepo::new("checkout");
    let dir = &repo.0;
    run(dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);

    fs::write(dir.join("nb.ipynb"), NOTEBOOK).unwrap();
    run(dir, "git", &["add", "nb.ipynb"]);
    run(
        dir,
        "git",
        &[
            "-c",
            "user.email=nbstrip@test",
            "-c",
            "user.name=nbstrip",
            "commit",
            "-qm",
            "add notebook",
        ],
    );
    fs::remove_file(dir.join("nb.ipynb")).unwrap();
    run(dir, "git", &["checkout", "--", "nb.ipynb"]);
    let restored = fs::read_to_string(dir.join("nb.ipynb")).unwrap();
    assert!(restored.contains("\"outputs\": []"), "restored: {restored}");
}

#[test]
fn install_is_idempotent() {
    let repo = TempRepo::new("idempotent");
    let dir = &repo.0;
    run(dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);
    run(dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);
    let attrs = fs::read_to_string(dir.join(".git/info/attributes")).unwrap();
    let hits = attrs
        .lines()
        .filter(|l| l.trim() == "*.ipynb filter=nbstrip")
        .count();
    assert_eq!(hits, 1, "attribute duplicated:\n{attrs}");
}

#[test]
fn install_outside_a_repo_fails() {
    let dir = env::temp_dir().join(format!("nbstrip-test-{}-norepo", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_nbstrip"))
        .arg("install")
        .current_dir(&dir)
        .env("GIT_CEILING_DIRECTORIES", env::temp_dir())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let _ = fs::remove_dir_all(&dir);
}

fn hg_available() -> bool {
    Command::new("hg")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn hg_dir(name: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!("nbstrip-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    run(&dir, "hg", &["init"]);
    dir
}

#[test]
fn install_then_hg_commit_stores_stripped_notebooks() {
    if !hg_available() {
        eprintln!("hg not installed; skipping");
        return;
    }
    let dir = hg_dir("hg-install");

    run(&dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);
    let hgrc = fs::read_to_string(dir.join(".hg/hgrc")).unwrap();
    assert!(hgrc.contains("[encode]"), "hgrc: {hgrc}");
    assert!(hgrc.contains("**.ipynb = pipe: "), "hgrc: {hgrc}");
    assert!(hgrc.contains("[hooks]"), "hgrc: {hgrc}");
    assert!(hgrc.contains("precommit.nbstrip = "), "hgrc: {hgrc}");

    // Round trip: the repository stores stripped bytes, the working
    // directory keeps outputs.
    fs::write(dir.join("nb.ipynb"), NOTEBOOK).unwrap();
    run(&dir, "hg", &["add", "nb.ipynb"]);
    let out = Command::new("hg")
        .args(["commit", "-m", "add notebook"])
        .env("HGUSER", "nbstrip-test")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "hg commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stored = run(&dir, "hg", &["cat", "nb.ipynb"]);
    assert!(stored.contains("\"outputs\": []"), "stored: {stored}");
    assert!(!stored.contains("huge plotly blob"));
    let worktree = fs::read_to_string(dir.join("nb.ipynb")).unwrap();
    assert!(worktree.contains("huge plotly blob"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hg_install_is_idempotent_and_preserves_hgrc() {
    if !hg_available() {
        eprintln!("hg not installed; skipping");
        return;
    }
    let dir = hg_dir("hg-idempotent");
    fs::write(
        dir.join(".hg/hgrc"),
        "[ui]\nusername = keep me\n\n[hooks]\nprecommit.keepme = true\n\n[encode]\n**.gz = pipe: gunzip\n",
    )
    .unwrap();

    run(&dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);
    run(&dir, env!("CARGO_BIN_EXE_nbstrip"), &["install"]);

    let hgrc = fs::read_to_string(dir.join(".hg/hgrc")).unwrap();
    let ours = hgrc.lines().filter(|l| l.starts_with("**.ipynb")).count();
    assert_eq!(ours, 1, "filter line duplicated:\n{hgrc}");
    let hooks = hgrc
        .lines()
        .filter(|l| l.starts_with("precommit.nbstrip"))
        .count();
    assert_eq!(hooks, 1, "hook line duplicated:\n{hgrc}");
    assert!(hgrc.contains("username = keep me"), "hgrc: {hgrc}");
    assert!(hgrc.contains("precommit.keepme = true"), "hgrc: {hgrc}");
    assert!(hgrc.contains("**.gz = pipe: gunzip"), "hgrc: {hgrc}");
    assert_eq!(hgrc.matches("[encode]").count(), 1, "hgrc: {hgrc}");
    assert_eq!(hgrc.matches("[hooks]").count(), 1, "hgrc: {hgrc}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hg_commit_aborts_when_filter_binary_is_missing() {
    // Regression: hg's pipe filter ignores the command's exit status, so with
    // the binary gone `hg commit` used to succeed and store an EMPTY notebook
    // (silent data destruction on a later update/revert). The precommit hook
    // install writes must abort the commit instead.
    if !hg_available() {
        eprintln!("hg not installed; skipping");
        return;
    }
    let dir = hg_dir("hg-missing-binary");

    // Install from a copy of the binary, then delete the copy — the
    // moved/rebuilt/cleaned-away binary a wired clone can end up pointing at.
    let doomed = dir.join("nbstrip-doomed");
    fs::copy(env!("CARGO_BIN_EXE_nbstrip"), &doomed).unwrap();
    run(&dir, doomed.to_str().unwrap(), &["install"]);
    fs::remove_file(&doomed).unwrap();

    fs::write(dir.join("nb.ipynb"), NOTEBOOK).unwrap();
    run(&dir, "hg", &["add", "nb.ipynb"]);
    let out = Command::new("hg")
        .args(["commit", "-m", "add notebook"])
        .env("HGUSER", "nbstrip-test")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "commit should abort when the filter binary is gone: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    // Nothing reached history — the guard fired before content was stored.
    let log = run(&dir, "hg", &["log", "-T", "{node}"]);
    assert_eq!(log.trim(), "", "a commit slipped through: {log}");
    // And the working copy is untouched.
    let worktree = fs::read_to_string(dir.join("nb.ipynb")).unwrap();
    assert!(worktree.contains("huge plotly blob"));
    let _ = fs::remove_dir_all(&dir);
}
