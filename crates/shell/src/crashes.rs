//! The renderers' crash dumps (M34.4, D34, docs/design/processes.md,
//! "Crashes"): the folder they go to, beside the profile, their names,
//! how much of them is kept, and the text file of a panic's message
//! beside a dump. The browser writes them (the platform module), to
//! that folder alone; nothing sends them anywhere: no code reads them
//! but the process checks', nor does any reach the network. For whoever
//! gives `MiniDumpWriteDump` a callback: `windows-sys`'s x86-64
//! `MINIDUMP_THREAD_CALLBACK` is not packed to four bytes as dbghelp's
//! is (`pshpack4.h`), so its stack's bounds read through it are not the
//! thread's (found trying to cut an overflow's stack from its dump, G253).

use std::path::{Path, PathBuf};

/// The most the dumps and their messages may hold together, in bytes:
/// past it, the oldest go (G253; a dump of a renderer's stack overflow
/// was its whole stack's, some 40 MiB, before its stack was cut).
pub(crate) const MAX_CRASH_BYTES: u64 = 200 << 20;

/// The folder the dumps go to beside the profile in `profile`.
#[cfg_attr(feature = "single-process", allow(dead_code))]
pub(crate) fn beside(profile: &Path) -> Option<PathBuf> {
    Some(profile.parent()?.join("Crashes"))
}

/// The name of the dump of renderer process `id`, made `millis` after the
/// epoch: in the order of their making, as their names sort.
pub(crate) fn dump_name(millis: u128, id: u32) -> String {
    format!("renderer-{millis:013}-{id}.dmp")
}

/// Removes the oldest dumps in `dir`, and their messages, past the
/// newest that hold `keep` bytes together at most. Only Windows writes
/// dumps yet (G113).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn prune(dir: &Path, keep: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dumps: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "dmp")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("renderer-"))
        })
        .collect();
    dumps.sort();
    let size = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
    let mut held = 0u64;
    let mut kept = dumps.len();
    for (at, dump) in dumps.iter().enumerate().rev() {
        held = held.saturating_add(size(dump) + size(&dump.with_extension("txt")));
        if held > keep {
            break;
        }
        kept = at;
    }
    for dump in &dumps[..kept] {
        let _ = std::fs::remove_file(dump);
        let _ = std::fs::remove_file(dump.with_extension("txt"));
    }
}

/// A renderer's panic message as the file beside its dump keeps it: what
/// the renderer wrote, which is not trusted, its control characters but
/// line breaks and tabs dropped.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn message_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dumps_go_beside_the_profile() {
        let profile = Path::new("data").join("SilverBrowser").join("Profile");
        assert_eq!(
            beside(&profile),
            Some(Path::new("data").join("SilverBrowser").join("Crashes"))
        );
    }

    #[test]
    fn the_oldest_dumps_go_past_the_most_bytes_kept() {
        let dir = std::env::temp_dir().join(format!("silver-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        // Ten dumps of 90 bytes and their messages of 10: the newest
        // eight fit in 800.
        for at in 0..10u128 {
            let dump = dir.join(dump_name(1_000 + at * 100, 7));
            std::fs::write(&dump, [0u8; 90]).expect("written");
            std::fs::write(dump.with_extension("txt"), [b'm'; 10]).expect("written");
        }
        std::fs::write(dir.join("other.dmp"), b"not ours").expect("written");
        prune(&dir, 800);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("listed")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        left.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left.len(), 8 * 2 + 1, "{left:?}");
        assert!(!left.contains(&dump_name(1_000, 7)), "{left:?}");
        assert!(!left.contains(&"renderer-0000000001100-7.txt".to_owned()));
        assert!(left.contains(&dump_name(1_200, 7)), "{left:?}");
        assert!(left.contains(&"other.dmp".to_owned()));
    }

    #[test]
    fn a_message_keeps_no_control_characters_but_line_breaks() {
        assert_eq!(
            message_text(b"panicked at\n\tx\x1b[31m\x07 \xff"),
            "panicked at\n\tx[31m \u{FFFD}"
        );
    }
}
