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
