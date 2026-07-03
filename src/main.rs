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
use serde_json::ser::{PrettyFormatter, Serializer};
use serde_json::Value;

mod strip;

const USAGE: &str = "\
strip Jupyter notebook outputs (outputs, execution counts, transient metadata)

usage:
  nbstrip FILE...        rewrite files in place
  nbstrip -t FILE...     print stripped notebooks to stdout
  nbstrip < in > out     stdin to stdout (the git clean filter)
  nbstrip install        register as the current repository's filter for
                         *.ipynb — git (config + .git/info/attributes) or
                         Mercurial ([encode] pipe filter in .hg/hgrc)

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

/// Mercurial: the `.hg/hgrc` section and filter pattern `install` manages.
const HG_ENCODE_SECTION: &str = "[encode]";
const HG_ENCODE_PATTERN: &str = "**.ipynb";

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

/// Register this binary as the repository's filter for `*.ipynb` — git clean
/// filter or Mercurial `[encode]` filter, whichever repository we're inside
/// (git wins when nested).
///
/// Everything is repo-local and nothing needs committing: git config +
/// `.git/info/attributes`, or `.hg/hgrc`. Teams that want the git *routing*
/// to travel with the repo commit `*.ipynb filter=nbstrip` to `.gitattributes`
/// instead — each clone still runs `nbstrip install` (VCSes never ship
/// config).
fn install() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| format!("resolving own path: {e}"))?;
    let exe = exe
        .to_str()
        .ok_or("this executable's path is not valid UTF-8")?
        .to_owned();
    if let Ok(git_dir) = cmd_stdout("git", &["rev-parse", "--absolute-git-dir"]) {
        return install_git(&git_dir, &exe);
    }
    if let Ok(hg_root) = cmd_stdout("hg", &["root"]) {
        return install_hg(&hg_root, &exe);
    }
    Err("not inside a git or Mercurial repository".to_owned())
}

/// Git: filter config (per-clone) + the attribute line in `.git/info/attributes`.
fn install_git(git_dir: &str, exe: &str) -> Result<(), String> {
    // The config value is run by git through `sh`, so quote the path.
    let clean_cmd = shell_quote(exe);
    cmd_ok("git", &["config", FILTER_CLEAN_KEY, &clean_cmd])?;
    cmd_ok("git", &["config", FILTER_REQUIRED_KEY, "true"])?;

    let attributes = Path::new(git_dir).join("info").join("attributes");
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

/// Mercurial: an `[encode]` filter in `.hg/hgrc` — the same stdin→stdout
/// contract as a git clean filter, applied as content enters the repository.
/// Idempotent: an existing `**.ipynb` filter line is replaced (a reinstall
/// from a new binary location must win), anything else in the file is kept.
fn install_hg(hg_root: &str, exe: &str) -> Result<(), String> {
    let hgrc = Path::new(hg_root).join(".hg").join("hgrc");
    let filter_line = format!("{HG_ENCODE_PATTERN} = pipe: {}", shell_quote(exe));

    let existing = fs::read_to_string(&hgrc).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(str::to_owned).collect();
    let mut in_encode = false;
    let mut replaced = false;
    let mut insert_at = None; // after the [encode] header, or its last line
    for (i, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_encode = trimmed == HG_ENCODE_SECTION;
        }
        if in_encode {
            insert_at = Some(i + 1);
            if trimmed
                .split('=')
                .next()
                .is_some_and(|key| key.trim() == HG_ENCODE_PATTERN)
            {
                line.clone_from(&filter_line);
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        if let Some(at) = insert_at {
            lines.insert(at, filter_line.clone());
        } else {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(HG_ENCODE_SECTION.to_owned());
            lines.push(filter_line.clone());
        }
    }
    fs::write(&hgrc, format!("{}\n", lines.join("\n")))
        .map_err(|e| format!("writing {}: {e}", hgrc.display()))?;

    println!("wrote {}:", hgrc.display());
    println!("  {HG_ENCODE_SECTION}");
    println!("  {filter_line}");
    println!("notebooks now strip on `hg commit`; the working directory keeps its outputs.");
    Ok(())
}

/// Run a command, require success, return trimmed stdout.
fn cmd_stdout(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("running {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Run a command for its side effect, requiring success.
fn cmd_ok(cmd: &str, args: &[&str]) -> Result<(), String> {
    cmd_stdout(cmd, args).map(|_| ())
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
