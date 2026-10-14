//! The renderers' crash dumps (M34.4, D34, docs/design/processes.md,
//! "Crashes"): the folder they go to, beside the profile, their names,
//! how many are kept, and the text file of a panic's message beside a
//! dump. The browser writes them (the platform module); nothing sends
//! them anywhere.

use std::path::{Path, PathBuf};

/// The most dumps kept: past it, the oldest go.
pub(crate) const MAX_CRASH_DUMPS: usize = 8;

/// The folder the dumps go to beside the profile in `profile`.
pub(crate) fn beside(profile: &Path) -> Option<PathBuf> {
    Some(profile.parent()?.join("Crashes"))
}

/// The name of the dump of renderer process `id`, made `millis` after the
/// epoch: in the order of their making, as their names sort.
pub(crate) fn dump_name(millis: u128, id: u32) -> String {
    format!("renderer-{millis:013}-{id}.dmp")
}

/// Removes the oldest dumps in `dir`, and their messages, past the
/// newest `keep`.
pub(crate) fn prune(dir: &Path, keep: usize) {
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
    let past = dumps.len().saturating_sub(keep);
    for dump in &dumps[..past] {
        let _ = std::fs::remove_file(dump);
        let _ = std::fs::remove_file(dump.with_extension("txt"));
    }
}

/// A renderer's panic message as the file beside its dump keeps it: what
/// the renderer wrote, which is not trusted, its control characters but
/// line breaks and tabs dropped.
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
    fn the_oldest_dumps_go_past_the_most_kept() {
        let dir = std::env::temp_dir().join(format!("silver-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a folder");
        for at in 0..10u128 {
            let dump = dir.join(dump_name(1_000 + at * 100, 7));
            std::fs::write(&dump, b"dump").expect("written");
            std::fs::write(dump.with_extension("txt"), b"message").expect("written");
        }
        std::fs::write(dir.join("other.dmp"), b"not ours").expect("written");
        prune(&dir, MAX_CRASH_DUMPS);
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
        assert_eq!(left.len(), MAX_CRASH_DUMPS * 2 + 1, "{left:?}");
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
