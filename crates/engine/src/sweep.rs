//! What the site sweep counts of a page (the scoreboard, 2026-11-13):
//! counters the engine keeps as a page runs, which `page.sweep` reads
//! over the DevTools' channel, for the shell's `--sweep-site`. Nothing
//! of them reaches the page, and they cost a page nothing but a few
//! additions where the engine meets what it does not do.

use std::collections::BTreeMap;

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
}

impl SweepCounts {
    /// Counts an error the console said, by its kind and its message as
    /// the reference browser words it.
    pub(crate) fn note_error(&mut self, kind: js::ErrorKind, message: &str) {
        match kind {
            js::ErrorKind::Budget => self.budget_stops += 1,
            js::ErrorKind::Exception | js::ErrorKind::Rejection => {
                if let Some(rest) = message.strip_prefix("RangeError: ")
                    && rest.starts_with("Maximum call stack size exceeded")
                {
                    self.stack_overflows += 1;
                } else if let Some(name) = missing_name(message) {
                    let name = name.to_owned();
                    self.name(Which::Missing, name, 1);
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
    }

    /// The counts as `page.sweep` answers them, with the layouts run.
    pub(crate) fn json(&self, layouts: u64) -> Json {
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
            ("layouts", Json::from(layouts as f64)),
        ])
    }
}

enum Which {
    Missing,
    Media,
}

/// What QuickJS says of a call of what is no function, naming nothing:
/// counted as one name.
pub(crate) const UNNAMED_CALL: &str = "(not a function)";

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
        );
        counts.note_error(js::ErrorKind::Rejection, "ReferenceError: x is not defined");
        counts.note_error(js::ErrorKind::Exception, "ReferenceError: x is not defined");
        counts.note_error(js::ErrorKind::Budget, "");
        counts.note_error(js::ErrorKind::Handled, "ReferenceError: y is not defined");
        assert_eq!(counts.stack_overflows, 1);
        assert_eq!(counts.budget_stops, 1);
        assert_eq!(counts.missing.get("x"), Some(&2));
        assert_eq!(counts.missing.get("y"), None, "handled by the page");
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
        let json = total.json(7).write();
        assert!(json.contains("\"layouts\":7"), "{json}");
    }
}
