//! What the site sweep counts of a page (the scoreboard, 2026-11-13):
//! counters the engine keeps as a page runs, which `page.sweep` reads
//! over the DevTools' channel, for the shell's `--sweep-site`. Nothing
//! of them reaches the page, and they cost a page nothing but a few
//! additions where the engine meets what it does not do.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use devtools::Json;

/// The most names one map keeps: a page that hits more is counted in
/// [`SweepCounts::dropped`].
pub(crate) const MAX_NAMES: usize = 256;

/// The most bytes of a name kept.
const MAX_NAME: usize = 80;

/// A page's counts.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SweepCounts {
    /// Names a script reached for that this engine has not: an uncaught
    /// `ReferenceError`'s `x is not defined`, a `TypeError`'s `x is not a
    /// function` or `x is not a constructor`, by the name.
    pub(crate) missing: BTreeMap<String, u32>,
    /// Uncaught `RangeError`s of a stack too deep.
    pub(crate) stack_overflows: u32,
    /// Scripts stopped for running too long.
    pub(crate) budget_stops: u32,
    /// Another frame's window refused to a script of its own origin
    /// (G90: every frame is another origin's here).
    pub(crate) frame_refused_same_origin: u32,
    /// `contentDocument` read as null of a frame of the reader's origin
    /// (G90).
    pub(crate) frame_document_same_origin: u32,
    /// What the document's policies refused (Content Security Policy).
    pub(crate) policy_refusals: u32,
    /// Media types a script asked whether this engine plays, answered
    /// no, by the type.
    pub(crate) media_unsupported: BTreeMap<String, u32>,
    /// Names not kept, past [`MAX_NAMES`].
    pub(crate) dropped: u32,
    /// A script's flush whose styles from the frame differed from a
    /// whole cascade's: counted by a build with the `style-crosscheck`
    /// feature alone, none otherwise.
    pub(crate) style_mismatches: u32,
    /// The flushes from the frame's styles that build checked: how much
    /// of the page the mismatches' count covers.
    pub(crate) style_checks: u32,
    /// The tasks the page's scripts ran, by kind: what a page left alone
    /// still does ([`TASK_KINDS`]).
    pub(crate) tasks: [u32; TASK_KINDS.len()],
}

/// The kinds of the tasks [`SweepCounts::tasks`] counts: timers, frame
/// callbacks' turns, events dispatched, fetches answered, messages, and
/// the rest.
pub(crate) const TASK_KINDS: [&str; 6] = [
    "timers",
    "animationFrames",
    "events",
    "fetched",
    "messages",
    "other",
];

impl SweepCounts {
    /// Counts an error the console said, by its kind and its message as
    /// the reference browser words it.
    /// `callee`: what the source names where the error was thrown, when
    /// it was kept ([`KeptSources::callee`]), which a call of what is no
    /// function is counted by, QuickJS's message naming nothing.
    pub(crate) fn note_error(&mut self, kind: js::ErrorKind, message: &str, callee: Option<&str>) {
        match kind {
            js::ErrorKind::Budget => self.budget_stops += 1,
            js::ErrorKind::Exception | js::ErrorKind::Rejection => {
                if let Some(rest) = message.strip_prefix("RangeError: ")
                    && rest.starts_with("Maximum call stack size exceeded")
                {
                    self.stack_overflows += 1;
                } else if let Some(name) = missing_name(message) {
                    let name = match (name, callee) {
                        (UNNAMED_CALL, Some(callee)) => callee,
                        (name, _) => name,
                    };
                    self.name(Which::Missing, name.to_owned(), 1);
                }
            }
            _ => {}
        }
    }

    /// Counts a media type asked about and answered no.
    pub(crate) fn note_unsupported_media(&mut self, kind: &str) {
        let kind = kind.trim().to_ascii_lowercase();
        if !kind.is_empty() {
            self.name(Which::Media, kind, 1);
        }
    }

    fn name(&mut self, which: Which, mut name: String, amount: u32) {
        if name.len() > MAX_NAME {
            let mut end = MAX_NAME;
            while !name.is_char_boundary(end) {
                end -= 1;
            }
            name.truncate(end);
        }
        let map = match which {
            Which::Missing => &mut self.missing,
            Which::Media => &mut self.media_unsupported,
        };
        let room = map.len() < MAX_NAMES;
        match map.get_mut(&name) {
            Some(count) => *count += amount,
            None if room => {
                map.insert(name, amount);
            }
            None => self.dropped += amount,
        }
    }

    /// Adds `other`'s counts to these.
    pub(crate) fn add(&mut self, other: &SweepCounts) {
        for (name, count) in &other.missing {
            self.name(Which::Missing, name.clone(), *count);
        }
        for (name, count) in &other.media_unsupported {
            self.name(Which::Media, name.clone(), *count);
        }
        self.stack_overflows += other.stack_overflows;
        self.budget_stops += other.budget_stops;
        self.frame_refused_same_origin += other.frame_refused_same_origin;
        self.frame_document_same_origin += other.frame_document_same_origin;
        self.policy_refusals += other.policy_refusals;
        self.dropped += other.dropped;
        self.style_mismatches += other.style_mismatches;
        self.style_checks += other.style_checks;
        for (mine, theirs) in self.tasks.iter_mut().zip(other.tasks) {
            *mine += theirs;
        }
    }

    /// The counts as `page.sweep` answers them, with the layouts run and
    /// the frames drawn by what wanted them (`crate::FRAME_CAUSES`, then
    /// the caller's).
    pub(crate) fn json(&self, layouts: u64, frames: &[u64]) -> Json {
        let causes = crate::FRAME_CAUSES
            .iter()
            .chain(std::iter::once(&"asked"))
            .zip(frames)
            .filter(|(_, count)| **count > 0)
            .map(|(name, count)| (*name, Json::from(*count as f64)));
        let tasks = TASK_KINDS
            .iter()
            .zip(self.tasks)
            .map(|(name, count)| (*name, Json::from(f64::from(count))));
        let map = |map: &BTreeMap<String, u32>| {
            Json::object(
                map.iter()
                    .map(|(name, count)| (name.as_str(), Json::from(f64::from(*count)))),
            )
        };
        Json::object([
            ("missing", map(&self.missing)),
            (
                "stackOverflows",
                Json::from(f64::from(self.stack_overflows)),
            ),
            ("budgetStops", Json::from(f64::from(self.budget_stops))),
            (
                "frameRefusedSameOrigin",
                Json::from(f64::from(self.frame_refused_same_origin)),
            ),
            (
                "frameDocumentSameOrigin",
                Json::from(f64::from(self.frame_document_same_origin)),
            ),
            (
                "policyRefusals",
                Json::from(f64::from(self.policy_refusals)),
            ),
            ("mediaUnsupported", map(&self.media_unsupported)),
            ("dropped", Json::from(f64::from(self.dropped))),
            (
                "styleFastPathMismatches",
                Json::from(f64::from(self.style_mismatches)),
            ),
            (
                "styleFastPathChecks",
                Json::from(f64::from(self.style_checks)),
            ),
            ("layouts", Json::from(layouts as f64)),
            ("frames", Json::from(frames.iter().sum::<u64>() as f64)),
            ("frameCauses", Json::object(causes)),
            ("tasks", Json::object(tasks)),
        ])
    }
}

enum Which {
    Missing,
    Media,
}

/// What QuickJS says of a call of what is no function, naming nothing:
/// counted as one name, unless the source says what was called.
pub(crate) const UNNAMED_CALL: &str = "(not a function)";

/// The most scripts [`KeptSources`] keeps, and the most bytes of them.
const MAX_KEPT_SOURCES: usize = 64;
const MAX_KEPT_SOURCE_BYTES: usize = 16 << 20;

/// The texts of the classic scripts a document ran last, by their URL
/// and the line of it they start on, what an error's place is read in:
/// the name a call of what is no function was made by, which QuickJS's
/// message leaves out (the site sweep's "(not a function)").
#[derive(Default)]
pub(crate) struct KeptSources {
    kept: VecDeque<(String, u32, Arc<str>)>,
    bytes: usize,
}

impl KeptSources {
    /// Keeps `text`, a script of `url` starting on its line `line`; the
    /// oldest go past [`MAX_KEPT_SOURCES`] or [`MAX_KEPT_SOURCE_BYTES`].
    pub(crate) fn keep(&mut self, url: &str, line: u32, text: &str) {
        if text.len() > MAX_KEPT_SOURCE_BYTES {
            return;
        }
        self.bytes += text.len();
        self.kept.push_back((url.to_owned(), line, Arc::from(text)));
        while self.kept.len() > MAX_KEPT_SOURCES || self.bytes > MAX_KEPT_SOURCE_BYTES {
            let Some((_, _, gone)) = self.kept.pop_front() else {
                break;
            };
            self.bytes -= gone.len();
        }
    }

    /// The name called at line `line`, column `column` of `url`, as
    /// QuickJS places a call of what is no function: the identifier
    /// there, followed by the call's `(` (or a template's backtick, or the
    /// `)` of `(0, f)()`); none for a member named by brackets, or where
    /// no kept script lies. QuickJS counts the columns of a script's first
    /// line from 0, and of the others from 1, within the script's own
    /// text.
    pub(crate) fn callee(&self, url: &str, line: u32, column: u32) -> Option<&str> {
        let (_, start, text) = self.kept.iter().rev().find(|(kept, start, text)| {
            kept == url && line >= *start && (line - start) as usize <= text.lines().count()
        })?;
        let row = text.lines().nth((line - start) as usize)?;
        let column = match line == *start {
            true => column,
            false => column.checked_sub(1)?,
        };
        callee_at(row, column as usize)
    }
}

/// The identifier at `column` (in characters, from 0) of `row`, when a
/// call follows it.
fn callee_at(row: &str, column: usize) -> Option<&str> {
    let at = row.char_indices().nth(column)?.0;
    let rest = &row[at..];
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let end = rest.find(|c: char| !word(c)).unwrap_or(rest.len());
    let name = &rest[..end];
    let next = rest[end..].trim_start().chars().next();
    let starts = name.chars().next().is_some_and(|c| !c.is_ascii_digit());
    (starts && matches!(next, Some('(' | ')' | '`'))).then_some(name)
}

/// The name an error's message says a script reached for and did not
/// find: `x` of `ReferenceError: x is not defined`, the last part of
/// `a.b.x` of `TypeError: a.b.x is not a function` (or a constructor);
/// [`UNNAMED_CALL`] for QuickJS's `TypeError: not a function`.
pub(crate) fn missing_name(message: &str) -> Option<&str> {
    if message == "TypeError: not a function" {
        return Some(UNNAMED_CALL);
    }
    let name = if let Some(rest) = message.strip_prefix("ReferenceError: ") {
        rest.strip_suffix(" is not defined")?
    } else {
        let rest = message.strip_prefix("TypeError: ")?;
        rest.strip_suffix(" is not a function")
            .or_else(|| rest.strip_suffix(" is not a constructor"))?
    };
    let name = name.rsplit('.').next()?.trim();
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    plain.then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_names_what_was_missing_and_a_deep_stack_is_counted() {
        assert_eq!(
            missing_name("ReferenceError: MediaSource is not defined"),
            Some("MediaSource")
        );
        assert_eq!(
            missing_name("TypeError: document.body.animate is not a function"),
            Some("animate")
        );
        assert_eq!(
            missing_name("TypeError: ResizeObserver is not a constructor"),
            Some("ResizeObserver")
        );
        // A call of what a script computed names nothing of the engine's.
        assert_eq!(missing_name("TypeError: a[0](...) is not a function"), None);
        assert_eq!(missing_name("TypeError: x is null"), None);
        assert_eq!(
            missing_name("TypeError: not a function"),
            Some(UNNAMED_CALL)
        );
        let mut counts = SweepCounts::default();
        counts.note_error(
            js::ErrorKind::Exception,
            "RangeError: Maximum call stack size exceeded",
            None,
        );
        counts.note_error(
            js::ErrorKind::Rejection,
            "ReferenceError: x is not defined",
            None,
        );
        counts.note_error(
            js::ErrorKind::Exception,
            "ReferenceError: x is not defined",
            None,
        );
        counts.note_error(js::ErrorKind::Budget, "", None);
        counts.note_error(
            js::ErrorKind::Handled,
            "ReferenceError: y is not defined",
            None,
        );
        assert_eq!(counts.stack_overflows, 1);
        assert_eq!(counts.budget_stops, 1);
        assert_eq!(counts.missing.get("x"), Some(&2));
        assert_eq!(counts.missing.get("y"), None, "handled by the page");
        // A call of what is no function, by the name its source calls it
        // by when kept, else unnamed.
        counts.note_error(
            js::ErrorKind::Exception,
            "TypeError: not a function",
            Some("observe"),
        );
        counts.note_error(js::ErrorKind::Exception, "TypeError: not a function", None);
        assert_eq!(counts.missing.get("observe"), Some(&1));
        assert_eq!(counts.missing.get(UNNAMED_CALL), Some(&1));
    }

    #[test]
    fn a_call_of_what_is_no_function_is_named_by_its_source() {
        // Where QuickJS places it: the called name's first character,
        // from 1, in the script's own line; the line the document's.
        let mut sources = KeptSources::default();
        sources.keep(
            "https://a.test/",
            10,
            "
foo123(1, 2);
(0, o.a.other)();
p['x y']();
zz.call(1); n.x `t`;",
        );
        sources.keep(
            "https://a.test/s.js",
            1,
            "a.b.missing();
  c.gone();",
        );
        let at = |url, line, column| sources.callee(url, line, column);
        assert_eq!(at("https://a.test/", 11, 1), Some("foo123"));
        assert_eq!(at("https://a.test/", 12, 9), Some("other"));
        assert_eq!(
            at("https://a.test/", 13, 1),
            None,
            "a member named by brackets"
        );
        assert_eq!(at("https://a.test/", 14, 4), Some("call"));
        assert_eq!(at("https://a.test/", 14, 15), Some("x"), "a tag");
        // Its first line counted from 0.
        assert_eq!(at("https://a.test/s.js", 1, 4), Some("missing"));
        assert_eq!(at("https://a.test/s.js", 2, 5), Some("gone"));
        assert_eq!(at("https://a.test/s.js", 3, 1), None, "past its lines");
        assert_eq!(at("https://b.test/", 11, 1), None, "no script kept");
        // Kept to their bound, the oldest gone.
        for n in 0..MAX_KEPT_SOURCES {
            sources.keep(&format!("https://c.test/{n}"), 1, "f();");
        }
        assert_eq!(sources.callee("https://a.test/s.js", 1, 4), None);
        assert_eq!(sources.callee("https://c.test/63", 1, 0), Some("f"));
    }

    #[test]
    fn a_page_of_many_names_keeps_as_many_as_the_cap() {
        let mut counts = SweepCounts::default();
        for n in 0..MAX_NAMES + 10 {
            counts.note_unsupported_media(&format!("video/x{n}"));
        }
        assert_eq!(counts.media_unsupported.len(), MAX_NAMES);
        assert_eq!(counts.dropped, 10);
        counts.note_unsupported_media(" Video/X0 ");
        assert_eq!(counts.media_unsupported.get("video/x0"), Some(&2));
        let mut total = SweepCounts::default();
        total.add(&counts);
        total.add(&counts);
        assert_eq!(total.media_unsupported.get("video/x0"), Some(&4));
        let json = total.json(7, &[]).write();
        assert!(json.contains("\"layouts\":7"), "{json}");
    }

    #[test]
    fn the_frames_are_said_by_their_cause_and_the_tasks_by_their_kind() {
        let mut counts = SweepCounts::default();
        counts.tasks[0] = 3;
        let mut frames = [0; crate::FRAME_CAUSES.len() + 1];
        frames[0] = 2;
        frames[crate::FRAME_CAUSES.len()] = 1;
        let json = counts.json(0, &frames).write();
        assert!(json.contains("\"frames\":3"), "{json}");
        assert!(
            json.contains("\"frameCauses\":{\"changed\":2,\"asked\":1}"),
            "{json}"
        );
        assert!(json.contains("\"timers\":3"), "{json}");
    }
}
