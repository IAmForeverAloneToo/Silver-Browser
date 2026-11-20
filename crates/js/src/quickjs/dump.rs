//! Sweep builds' dump of the scripts pages run (`dump-scripts`): a corpus
//! of real-world sources for parsers to read. Never in a shipped or
//! default build: off, compiled out.
//!
//! The directory `SILVER_DUMP_SCRIPTS` names holds one file per source,
//! named by a hash of its URL and text (`.js` for a classic script,
//! `.mjs` for a module), and `index.tsv`, a line per file: its name,
//! `script` or `module`, its URL, and how it came (`inline`, `external`
//! or `worker`). A source dumped once, by any process, is not again;
//! one too large, or past the total a process writes, is not dumped, a
//! line in `dropped.tsv` saying so. Inline scripts can carry a page's
//! data: the corpus stays on the machine that made it.
//!
//! A sandboxed renderer can write nothing: it gives its sources to the
//! browser ([`set_sink`]), which writes them ([`write`]), naming each
//! file itself from its own hash, so that nothing a renderer sends
//! becomes part of a path.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The largest source dumped, in bytes.
pub const MAX_SOURCE: usize = 8 << 20;

/// The most a process dumps in all, in bytes: a sweep's browser, a site
/// at a time.
pub const MAX_TOTAL: u64 = 512 << 20;

/// How a source came: an inline script's text, an external script's or
/// module's response, a worker's, or code from a string (`eval`, a
/// function constructor's source), the document's that compiled it.
pub const KINDS: &[&str] = &["inline", "external", "worker", "eval"];

/// Where a process's sources go instead of its own files: a renderer's
/// link to the browser.
pub type Sink = Box<dyn Fn(&str, &str, bool, &str) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Sends the sources this process meets to `sink` (its URL, its text,
/// whether a module, and its kind) rather than writing them.
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// The directory to dump into, made if need be; none without the
/// variable.
fn directory() -> Option<&'static PathBuf> {
    static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            std::env::var_os("SILVER_DUMP_SCRIPTS")
                .map(PathBuf::from)
                .filter(|directory| std::fs::create_dir_all(directory).is_ok())
        })
        .as_ref()
}

/// FNV-1a over `url`, a NUL and `text`: the same name in every process.
fn hash(url: &str, text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in url.bytes().chain([0]).chain(text.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// A source a realm runs: to this process's sink, if it has one, else
/// written.
pub fn dump(text: &str, url: &str, module: bool, kind: &str) {
    match SINK.get() {
        Some(sink) => sink(url, text, module, kind),
        None => write(text, url, module, kind),
    }
}

/// Writes a source into the dump directory, if there is one.
pub fn write(text: &str, url: &str, module: bool, kind: &str) {
    if let Some(directory) = directory() {
        static TOTAL: AtomicU64 = AtomicU64::new(0);
        write_in(directory, &TOTAL, text, url, module, kind);
    }
}

/// [`write`] into `directory`, `total` the bytes written so far: a kind
/// not of [`KINDS`] is dropped unsaid; a source over [`MAX_SOURCE`], or
/// past [`MAX_TOTAL`], is dropped with a line in `dropped.tsv`.
fn write_in(directory: &Path, total: &AtomicU64, text: &str, url: &str, module: bool, kind: &str) {
    let Some(&kind) = KINDS.iter().find(|known| **known == kind) else {
        return;
    };
    let url = url.replace(['\t', '\r', '\n', '\0'], " ");
    let bytes = text.len() as u64;
    let past = total.load(Ordering::Relaxed).saturating_add(bytes) > MAX_TOTAL;
    if text.len() > MAX_SOURCE || past {
        let why = if past { "total" } else { "size" };
        append(
            directory,
            "dropped.tsv",
            &format!("{why}\t{bytes}\t{url}\n"),
        );
        return;
    }
    let name = format!(
        "{:016x}.{}",
        hash(&url, text),
        if module { "mjs" } else { "js" }
    );
    let made = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(&name))
        .and_then(|mut file| file.write_all(text.as_bytes()));
    if made.is_err() {
        return;
    }
    total.fetch_add(bytes, Ordering::Relaxed);
    let line = format!(
        "{name}\t{}\t{url}\t{kind}\n",
        if module { "module" } else { "script" }
    );
    append(directory, "index.tsv", &line);
}

/// Appends `line` to the file `name` of `directory`, in one write: the
/// processes of a sweep share it.
fn append(directory: &Path, name: &str, line: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .append(true)
        .create(true)
        .open(directory.join(name))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    #[test]
    fn a_sources_name_is_its_urls_and_texts() {
        assert_eq!(super::hash("a", "b"), super::hash("a", "b"));
        assert_ne!(super::hash("a", "b"), super::hash("ab", ""));
        assert_ne!(super::hash("a", "b"), super::hash("a", "c"));
    }

    /// What a renderer sends names no path: a URL of `..`, a drive, a
    /// NUL or a line break lands as a hash-named file in the directory,
    /// its index line one line; an unknown kind and a source too large
    /// are dropped.
    #[test]
    fn a_hostile_url_writes_only_a_hash_named_file_in_the_directory() {
        let directory =
            std::env::temp_dir().join(format!("silver-dump-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a directory");
        let total = AtomicU64::new(0);
        let hostile = [
            "../../outside.js",
            r"C:\Windows\evil.js",
            "a\0b.js",
            "x\ninjected\tmodule\turl\tinline",
        ];
        for url in hostile {
            super::write_in(&directory, &total, "1;", url, false, "external");
        }
        super::write_in(&directory, &total, "2;", "u", false, "../../kind");
        let big = "x".repeat(super::MAX_SOURCE + 1);
        super::write_in(&directory, &total, &big, "big", true, "inline");
        let mut names: Vec<String> = std::fs::read_dir(&directory)
            .expect("listed")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .into_string()
                    .expect("a name")
            })
            .collect();
        names.sort();
        let files: Vec<&String> = names.iter().filter(|name| name.ends_with(".js")).collect();
        assert_eq!(files.len(), 4, "{names:?}");
        for name in &files {
            let (stem, _) = name.split_once('.').expect("an extension");
            assert_eq!(stem.len(), 16);
            assert!(stem.chars().all(|c| c.is_ascii_hexdigit()), "{name}");
        }
        assert!(names.contains(&"index.tsv".to_owned()));
        assert!(names.contains(&"dropped.tsv".to_owned()));
        assert!(
            !directory
                .parent()
                .expect("a parent")
                .join("outside.js")
                .exists()
        );
        let index = std::fs::read_to_string(directory.join("index.tsv")).expect("the index");
        assert_eq!(index.lines().count(), 4, "{index}");
        assert!(
            index.lines().all(|line| line.split('\t').count() == 4),
            "{index}"
        );
        let dropped = std::fs::read_to_string(directory.join("dropped.tsv")).expect("dropped");
        assert!(dropped.starts_with("size\t"), "{dropped}");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
