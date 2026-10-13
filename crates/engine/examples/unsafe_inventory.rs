//! Counts the `unsafe` of the third-party crates the browser builds
//! (M33.5, docs/security.md): for each registry package of a lock file,
//! the `unsafe` blocks, functions, traits and impls, and extern blocks
//! in its Rust sources as Cargo keeps them, written as a Markdown table.
//!
//!     cargo run -p engine --example unsafe_inventory -- [Cargo.lock]
//!
//! The sources are those `cargo fetch` put under `$CARGO_HOME/registry/
//! src`; a package not there is listed as not found. A count of words in
//! code, comments left out: what to read, not a proof of anything.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

fn main() {
    let lock = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Cargo.lock".to_owned());
    let text = std::fs::read_to_string(&lock).unwrap_or_else(|error| {
        eprintln!("cannot read {lock}: {error}");
        std::process::exit(1);
    });
    let Some(home) = cargo_home() else {
        eprintln!("no CARGO_HOME, nor a home directory");
        std::process::exit(1);
    };
    let sources: Vec<PathBuf> = std::fs::read_dir(home.join("registry").join("src"))
        .map(|dirs| {
            dirs.filter_map(|dir| dir.ok().map(|dir| dir.path()))
                .collect()
        })
        .unwrap_or_default();
    println!("| crate | version | blocks | fns | traits and impls | externs |");
    println!("|---|---|---:|---:|---:|---:|");
    let (mut with, mut without) = (0, 0);
    for (name, version) in packages(&text) {
        let dir = sources
            .iter()
            .map(|source| source.join(format!("{name}-{version}")))
            .find(|dir| dir.is_dir());
        let Some(dir) = dir else {
            println!("| {name} | {version} | not found | | | |");
            continue;
        };
        let mut counts = Counts::default();
        count_dir(&dir, &mut counts);
        if counts.total() == 0 {
            without += 1;
            continue;
        }
        with += 1;
        println!(
            "| {name} | {version} | {} | {} | {} | {} |",
            counts.blocks, counts.fns, counts.impls, counts.externs
        );
    }
    println!();
    println!("{with} crates write `unsafe`; {without} write none.");
}

/// Where Cargo keeps its registry.
fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".cargo"))
}

/// The registry packages of a lock file, by name and version.
fn packages(lock: &str) -> Vec<(String, String)> {
    let mut packages = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |key: &str| {
            block.lines().find_map(|line| {
                let value = line.strip_prefix(key)?.trim().strip_prefix('=')?.trim();
                Some(value.trim_matches('"').to_owned())
            })
        };
        let from_registry = field("source").is_some_and(|source| source.starts_with("registry+"));
        if let (true, Some(name), Some(version)) = (from_registry, field("name"), field("version"))
        {
            packages.push((name, version));
        }
    }
    packages
}

#[derive(Default)]
struct Counts {
    blocks: usize,
    fns: usize,
    impls: usize,
    externs: usize,
}

impl Counts {
    fn total(&self) -> usize {
        self.blocks + self.fns + self.impls + self.externs
    }
}

fn count_dir(dir: &Path, counts: &mut Counts) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_dir(&path, counts);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && let Ok(source) = std::fs::read_to_string(&path)
        {
            count_source(&source, counts);
        }
    }
}

/// Counts the word `unsafe` in `source`'s code, by what follows it; a
/// line comment's are left out.
fn count_source(source: &str, counts: &mut Counts) {
    for line in source.lines() {
        let code = line.split("//").next().unwrap_or_default();
        let mut rest = code;
        while let Some(at) = rest.find("unsafe") {
            let before = rest[..at].chars().next_back();
            let after = &rest[at + "unsafe".len()..];
            rest = after;
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '\'')
                || after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-')
            {
                continue;
            }
            let next = after.trim_start();
            if next.starts_with('{') {
                counts.blocks += 1;
            } else if next.starts_with("fn") {
                counts.fns += 1;
            } else if next.starts_with("impl") || next.starts_with("trait") {
                counts.impls += 1;
            } else if next.starts_with("extern") {
                counts.externs += 1;
            }
        }
    }
}
