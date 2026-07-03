//! `nbstrip` — strip Jupyter notebook outputs so they never reach git.
//!
//! Modes:
//! - `nbstrip FILE...` rewrites files in place (already-clean files untouched)
//! - `nbstrip -t FILE...` prints the stripped notebooks to stdout
//! - `nbstrip < in.ipynb > out.ipynb` — stdin to stdout, the git clean filter
//! - `nbstrip install` — wire this binary into the current repository as the
//!   clean filter for `*.ipynb` (git config + `.git/info/attributes`)
//!
//! After `install`, `git add` stages notebooks stripped while the working tree
//! keeps its outputs for Jupyter. `filter.nbstrip.required=true` makes a
//! missing binary fail the add loudly instead of silently committing outputs.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::{env, fs, io};

use serde::Serialize;
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer};

mod strip;

const USAGE: &str = "\
strip Jupyter notebook outputs (outputs, execution counts, transient metadata)

usage:
  nbstrip FILE...        rewrite files in place
  nbstrip -t FILE...     print stripped notebooks to stdout
  nbstrip < in > out     stdin to stdout (the git clean filter)
  nbstrip install        register as the current git repository's clean filter
                         for *.ipynb (git config + .git/info/attributes)

options:
  -t, --textconv         print stripped notebooks to stdout instead of
                         rewriting them in place
  -h, --help             print this help
  -V, --version          print the version

Cells or notebooks marked `keep_output` (metadata or cell tag) keep outputs.
";

const FLAG_TEXTCONV: &str = "--textconv";
const FLAG_TEXTCONV_SHORT: &str = "-t";
const FLAG_HELP: &str = "--help";
const FLAG_HELP_SHORT: &str = "-h";
const FLAG_VERSION: &str = "--version";
const FLAG_VERSION_SHORT: &str = "-V";
const CMD_INSTALL: &str = "install";

const FILTER_CLEAN_KEY: &str = "filter.nbstrip.clean";
const FILTER_REQUIRED_KEY: &str = "filter.nbstrip.required";
/// The attribute line `install` writes into `.git/info/attributes`.
const ATTRIBUTES_LINE: &str = "*.ipynb filter=nbstrip";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // A failing clean filter aborts the `git add` (filter.required):
            // better no staging than outputs slipping into history silently.
            eprintln!("nbstrip: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut textconv = false;
    let mut files = Vec::new();
    for arg in env::args().skip(1) {
        match arg.as_str() {
            FLAG_TEXTCONV | FLAG_TEXTCONV_SHORT => textconv = true,
            FLAG_HELP | FLAG_HELP_SHORT => {
                print!("{USAGE}");
                return Ok(());
            }
            FLAG_VERSION | FLAG_VERSION_SHORT => {
                println!("nbstrip {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown flag `{arg}`\n\n{USAGE}"));
            }
            _ => files.push(arg),
        }
    }

    // `install` is a subcommand, not a file. (A notebook literally named
    // `install` can still be stripped as `./install`.)
    if !textconv && files.len() == 1 && files[0] == CMD_INSTALL {
        return install();
    }

    if files.is_empty() {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| format!("reading stdin: {e}"))?;
        let stripped = strip_to_string(&input)?;
        // Always emit, changed or not: a clean filter's stdout IS the staged
        // content, so "no change, print nothing" would truncate the file.
        io::stdout()
            .write_all(stripped.as_bytes())
            .map_err(|e| format!("writing stdout: {e}"))?;
        return Ok(());
    }

    for file in &files {
        let input = fs::read_to_string(file).map_err(|e| format!("reading {file}: {e}"))?;
        let stripped = strip_to_string(&input).map_err(|e| format!("{file}: {e}"))?;
        if textconv {
            io::stdout()
                .write_all(stripped.as_bytes())
                .map_err(|e| format!("writing stdout: {e}"))?;
        } else if stripped != input {
            fs::write(file, stripped).map_err(|e| format!("writing {file}: {e}"))?;
        }
    }
    Ok(())
}

/// Parse, strip, and re-serialize a notebook the way `nbformat` writes it:
/// 1-space indent, sorted keys (`serde_json`'s map is ordered), and the
/// input's trailing newline preserved — so strip-then-save-in-Jupyter and
/// save-then-strip produce identical bytes.
fn strip_to_string(input: &str) -> Result<String, String> {
    let mut nb: Value =
        serde_json::from_str(input).map_err(|e| format!("not valid notebook JSON: {e}"))?;
    strip::strip(&mut nb);

    let mut buf = Vec::new();
    let mut ser = Serializer::with_formatter(&mut buf, PrettyFormatter::with_indent(b" "));
    nb.serialize(&mut ser)
        .map_err(|e| format!("serializing notebook: {e}"))?;
    let mut out = String::from_utf8(buf).map_err(|e| format!("serialized non-UTF-8: {e}"))?;
    if input.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Register this binary as the repository's clean filter for `*.ipynb`.
///
/// Everything is repo-local and nothing needs committing: the filter command
/// goes to `git config` (per-clone by design) and the attribute line to
/// `.git/info/attributes`. Teams that want the attribute to travel with the
/// repo commit `*.ipynb filter=nbstrip` to `.gitattributes` instead — each
/// clone still runs `nbstrip install` (git never ships config).
fn install() -> Result<(), String> {
    let git_dir = git_stdout(&["rev-parse", "--absolute-git-dir"])
        .map_err(|e| format!("not inside a git repository? {e}"))?;

    let exe = env::current_exe().map_err(|e| format!("resolving own path: {e}"))?;
    let exe = exe
        .to_str()
        .ok_or("this executable's path is not valid UTF-8")?;
    // The config value is run by git through `sh`, so quote the path.
    let clean_cmd = shell_quote(exe);
    git_ok(&["config", FILTER_CLEAN_KEY, &clean_cmd])?;
    git_ok(&["config", FILTER_REQUIRED_KEY, "true"])?;

    let attributes = Path::new(&git_dir).join("info").join("attributes");
    let existing = fs::read_to_string(&attributes).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == ATTRIBUTES_LINE) {
        println!("attributes already present: {}", attributes.display());
    } else {
        if let Some(dir) = attributes.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        }
        let newline = if existing.is_empty() || existing.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        fs::write(
            &attributes,
            format!("{existing}{newline}{ATTRIBUTES_LINE}\n"),
        )
        .map_err(|e| format!("writing {}: {e}", attributes.display()))?;
        println!("wrote {}: {ATTRIBUTES_LINE}", attributes.display());
    }

    println!("configured {FILTER_CLEAN_KEY} = {clean_cmd}");
    println!("configured {FILTER_REQUIRED_KEY} = true");
    println!("notebooks now strip on `git add`; the working tree keeps its outputs.");
    Ok(())
}

/// Run git, require success, return trimmed stdout.
fn git_stdout(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| format!("running git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Run git for its side effect, requiring success.
fn git_ok(args: &[&str]) -> Result<(), String> {
    git_stdout(args).map(|_| ())
}

/// Quote a path for the POSIX shell git uses to run filter commands.
fn shell_quote(path: &str) -> String {
    let safe = path
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+'));
    if safe && !path.is_empty() {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{shell_quote, strip_to_string};

    const NOTEBOOK: &str = r#"{
 "cells": [
  {
   "cell_type": "code",
   "execution_count": 3,
   "id": "aa11bb22",
   "metadata": {},
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": ["hi\n"]
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

    #[test]
    fn strips_and_matches_nbformat_style() {
        let out = strip_to_string(NOTEBOOK).unwrap();
        assert!(out.contains("\"outputs\": []"));
        assert!(out.contains("\"execution_count\": null"));
        // 1-space indent and the preserved trailing newline.
        assert!(out.starts_with("{\n \"cells\""));
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn stable_under_a_second_pass() {
        let once = strip_to_string(NOTEBOOK).unwrap();
        assert_eq!(strip_to_string(&once).unwrap(), once);
    }

    #[test]
    fn rejects_garbage() {
        assert!(strip_to_string("not json").is_err());
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(
            shell_quote("/usr/local/bin/nbstrip"),
            "/usr/local/bin/nbstrip"
        );
        assert_eq!(
            shell_quote("/opt/my tools/nbstrip"),
            "'/opt/my tools/nbstrip'"
        );
        assert_eq!(shell_quote("/o'dd/nbstrip"), r"'/o'\''dd/nbstrip'");
    }
}
