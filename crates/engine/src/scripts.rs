//! Scripts (M24, `docs/design/scripting.md`): a document's realm, the
//! host its scripts reach the document through, its console, and what
//! a `script` element is (HTML section 4.12.1.1).
//!
//! The host owns the document for the length of a run, moved in and
//! back: a script's changes go through `dom::Document`'s methods and
//! journal as any other change does. A realm is made at a document's
//! first script and kept with the document; one whose heap stays full
//! after a failed run is dropped, and the document runs no script again.

use std::collections::VecDeque;

use dom::{Document, NodeId};
use js::{
    Completion, ConsoleLevel, ErrorKind, Limits, QuickJs, Realm, Script, ScriptEngine, ScriptError,
    ScriptingHost,
};

/// The most entries a document's console keeps, the oldest dropped.
pub const MAX_CONSOLE_ENTRIES: usize = 1_000;
/// The most bytes of an entry's text, cut at a character boundary.
pub const MAX_CONSOLE_MESSAGE: usize = 16 << 10;

/// A line of a document's console (M24.7): what a script wrote, or an
/// error it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    pub level: ConsoleLevel,
    pub text: String,
}

/// Where the shown document's scripting stands (M24.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptState {
    /// Scripting is off, as it is unless asked for (risk R2).
    Off,
    /// Scripts run; `errors` have been reported.
    Running { errors: usize },
    /// The document's realm ran out of memory and was dropped: no
    /// further script runs in it.
    Stopped { errors: usize },
}

/// The host a document's scripts reach it through (M24.4): the document,
/// moved in for a run, and what they wrote to the console meanwhile.
#[derive(Default)]
pub(crate) struct DocumentHost {
    pub(crate) document: Document,
    console: Vec<ConsoleEntry>,
}

impl ScriptingHost for DocumentHost {
    fn console(&mut self, level: ConsoleLevel, message: String) {
        self.console.push(ConsoleEntry {
            level,
            text: cut(message),
        });
    }

    fn title(&mut self) -> String {
        document_title(&self.document)
    }

    fn set_title(&mut self, title: String) {
        set_document_title(&mut self.document, &title);
    }
}

/// `text`, cut to [`MAX_CONSOLE_MESSAGE`] bytes at a character boundary.
fn cut(mut text: String) -> String {
    if text.len() > MAX_CONSOLE_MESSAGE {
        let mut end = MAX_CONSOLE_MESSAGE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// A document's scripts: its realm, once it has one, and its console.
pub(crate) struct Scripts {
    realm: Option<Box<dyn Realm<DocumentHost>>>,
    limits: Limits,
    console: VecDeque<ConsoleEntry>,
    errors: usize,
    /// Its realm died: it runs nothing more.
    stopped: bool,
    /// It stopped since this was last asked.
    just_stopped: bool,
}

impl Default for Scripts {
    fn default() -> Scripts {
        Scripts::with_limits(Limits::default())
    }
}

impl Scripts {
    /// A document's scripts, its realm to be made with `limits`.
    pub(crate) fn with_limits(limits: Limits) -> Scripts {
        Scripts {
            realm: None,
            limits,
            console: VecDeque::new(),
            errors: 0,
            stopped: false,
            just_stopped: false,
        }
    }

    /// Runs `script` in the document's realm, made if it has none, with
    /// `document` lent to the host, then the microtask checkpoint that
    /// follows a script (HTML section 8.1.4.4); its errors and those of
    /// the jobs the checkpoint runs go to the console. What it ended
    /// with, unless it failed or the document runs no scripts.
    pub(crate) fn run(
        &mut self,
        document: &mut Document,
        script: &Script<'_>,
    ) -> Option<Completion> {
        if self.stopped {
            return None;
        }
        if self.realm.is_none() {
            match QuickJs.realm::<DocumentHost>(self.limits) {
                Ok(realm) => self.realm = Some(realm),
                Err(error) => {
                    self.report(&error);
                    self.stop();
                    return None;
                }
            }
        }
        let realm = self.realm.as_mut()?;
        let mut host = DocumentHost {
            document: std::mem::take(document),
            console: Vec::new(),
        };
        let result = realm.run(script, &mut host);
        let errors = realm.checkpoint(&mut host);
        let dead = realm.dead().is_some();
        *document = std::mem::take(&mut host.document);
        for entry in host.console {
            self.push(entry);
        }
        let completion = match result {
            Ok(completion) => Some(completion),
            Err(error) => {
                self.report(&error);
                None
            }
        };
        for error in &errors {
            self.report(error);
        }
        if dead {
            self.stop();
        }
        completion
    }

    /// Whether the back-forward cache may keep the document: not when
    /// its realm died.
    pub(crate) fn keepable(&self) -> bool {
        !self.stopped
    }

    /// Collects the realm's garbage, as when the document is kept.
    pub(crate) fn collect(&mut self) {
        if let Some(realm) = &mut self.realm {
            realm.collect();
        }
    }

    /// The bytes of heap the realm holds; none without one.
    #[cfg(test)]
    pub(crate) fn heap(&self) -> usize {
        self.realm.as_ref().map_or(0, |realm| realm.heap())
    }

    /// The document started again (M24.8): its realm goes, and the next
    /// script runs in a new one; its console stays, since what was run
    /// was run.
    pub(crate) fn restart(&mut self) {
        *self = Scripts {
            console: std::mem::take(&mut self.console),
            ..Scripts::with_limits(self.limits)
        };
    }

    fn stop(&mut self) {
        self.realm = None;
        self.stopped = true;
        self.just_stopped = true;
        self.push(ConsoleEntry {
            level: ConsoleLevel::Error,
            text: "Scripts stopped on this page: out of memory".to_owned(),
        });
    }

    /// Reports `error` as HTML section 8.1.4.6 does, to the console: an
    /// uncaught exception and where it was thrown, or a rejection with no
    /// handler; a dead realm's refusal is not an error of the page's.
    pub(crate) fn report(&mut self, error: &ScriptError) {
        let text = match error.kind {
            ErrorKind::Dead => return,
            ErrorKind::Rejection => format!("Uncaught (in promise) {}", error.message),
            ErrorKind::Budget => error.message.clone(),
            ErrorKind::Exception | ErrorKind::Heap => format!("Uncaught {}", error.message),
        };
        let text = if error.url.is_empty() {
            text
        } else {
            format!("{text} at {}:{}:{}", error.url, error.line, error.column)
        };
        self.errors += 1;
        self.push(ConsoleEntry {
            level: ConsoleLevel::Error,
            text,
        });
    }

    fn push(&mut self, entry: ConsoleEntry) {
        if self.console.len() == MAX_CONSOLE_ENTRIES {
            self.console.pop_front();
        }
        self.console.push_back(entry);
    }

    /// The console's entries since it was last taken.
    pub(crate) fn take_console(&mut self) -> Vec<ConsoleEntry> {
        self.console.drain(..).collect()
    }

    /// Where the document's scripting stands, scripting being on.
    pub(crate) fn state(&self) -> ScriptState {
        if self.stopped {
            ScriptState::Stopped {
                errors: self.errors,
            }
        } else {
            ScriptState::Running {
                errors: self.errors,
            }
        }
    }

    /// Whether the realm died since this was last asked.
    pub(crate) fn take_just_stopped(&mut self) -> bool {
        std::mem::take(&mut self.just_stopped)
    }
}

/// `document.title`'s getter (HTML section 3.1.7): the text of the
/// first `title` element in tree order, its ASCII white space stripped
/// and collapsed; empty without one.
pub(crate) fn document_title(document: &Document) -> String {
    let Some(title) = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("title"))
    else {
        return String::new();
    };
    let text: String = document
        .children(title)
        .filter_map(|child| document.text(child))
        .collect();
    text.split(|c: char| c.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `document.title`'s setter (HTML section 3.1.7): the first `title`
/// element's children replaced by `value`'s text, a `title` made last in
/// the head if there is none; nothing without a document element, or
/// without a head to put a `title` in.
pub(crate) fn set_document_title(document: &mut Document, value: &str) {
    let Some(html) = document.document_element() else {
        return;
    };
    let existing = document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("title"));
    let title = match existing {
        Some(title) => title,
        None => {
            let Some(head) = document
                .element_children(html)
                .find(|&child| document.tag(child) == Some("head"))
            else {
                return;
            };
            let Ok(title) = document.create_element("title") else {
                return;
            };
            if document.append_child(head, title).is_err() {
                let _ = document.free(title);
                return;
            }
            title
        }
    };
    // String replace all: the children go, the text comes in their place.
    let children: Vec<NodeId> = document.children(title).collect();
    for child in children {
        if document.remove_child(title, child).is_ok() {
            let _ = document.free(child);
        }
    }
    if !value.is_empty()
        && let Ok(text) = document.create_text(value)
        && document.append_child(title, text).is_err()
    {
        let _ = document.free(text);
    }
}

/// What a `script` element is (HTML section 4.12.1.1, "prepare the
/// script element", step 9 on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptType {
    Classic,
    Module,
    ImportMap,
    SpeculationRules,
}

/// The type of the script `element` is, by its `type` and `language`
/// attributes; `None` when it is not a script at all.
pub(crate) fn script_type(document: &Document, element: NodeId) -> Option<ScriptType> {
    let type_attr = document.attr(element, "type");
    let language = document.attr(element, "language");
    let block_type = match (type_attr, language) {
        (Some(""), _) | (None, Some("")) | (None, None) => "text/javascript".to_owned(),
        (Some(value), _) => value.to_owned(),
        (None, Some(language)) => format!("text/{language}"),
    };
    let stripped = type_attr.map(|value| value.trim_matches(|c: char| c.is_ascii_whitespace()));
    if is_javascript_essence(&block_type) || stripped.is_some_and(is_javascript_essence) {
        return Some(ScriptType::Classic);
    }
    match block_type.to_ascii_lowercase().as_str() {
        "module" => Some(ScriptType::Module),
        "importmap" => Some(ScriptType::ImportMap),
        "speculationrules" => Some(ScriptType::SpeculationRules),
        _ => None,
    }
}

/// Whether `value` is a JavaScript MIME type essence match: one of the
/// essences, in any case, with no parameters (MIME Sniffing section 4.6).
fn is_javascript_essence(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    net::MimeType::parse(&lower).is_some_and(|mime| mime.essence() == lower && mime.is_javascript())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Document {
        Document::from_node(&html::parse_document(source).expect("parsing never fails"))
    }

    fn script(source: &str) -> Script<'_> {
        Script {
            source,
            url: "https://example.com/",
            line: 1,
            muted: false,
        }
    }

    #[test]
    fn the_title_reads_and_writes_as_html_says() {
        let mut document = parsed("<title>  a \n b </title><p>x");
        assert_eq!(document_title(&document), "a b");
        set_document_title(&mut document, "new");
        assert_eq!(document_title(&document), "new");
        // None yet: one is made last in the head.
        let mut document = parsed("<meta charset=utf-8><p>x");
        assert_eq!(document_title(&document), "");
        set_document_title(&mut document, "  made  ");
        assert_eq!(document_title(&document), "made");
        let head = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("head"))
            .unwrap();
        let last = document.last_child(head).unwrap();
        assert_eq!(document.tag(last), Some("title"));
        // The empty string leaves the title element empty.
        set_document_title(&mut document, "");
        assert_eq!(document.children(last).count(), 0);
        // No document element: nothing.
        let mut empty = Document::new();
        set_document_title(&mut empty, "x");
        assert_eq!(empty.len(), 1);
    }

    #[test]
    fn a_script_reads_and_writes_the_document_and_logs() {
        let mut document = parsed("<title>Before</title>");
        let mut scripts = Scripts::default();
        let done = scripts.run(
            &mut document,
            &script(
                "console.log('hello', 1); document.title = document.title + ' and after'; 'ok'",
            ),
        );
        assert_eq!(done, Some(Completion::String("ok".to_owned())));
        assert_eq!(document_title(&document), "Before and after");
        let console = scripts.take_console();
        assert_eq!(console.len(), 1);
        assert_eq!(console[0].text, "hello 1");
        assert!(scripts.take_console().is_empty());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
    }

    #[test]
    fn errors_go_to_the_console_and_the_next_script_runs() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::default();
        let inline = Script {
            source: "\nmissing()",
            url: "https://example.com/page.html",
            line: 4,
            muted: false,
        };
        assert_eq!(scripts.run(&mut document, &inline), None);
        scripts.run(&mut document, &script("Promise.reject(new Error('later'))"));
        assert!(scripts.run(&mut document, &script("1")).is_some());
        let console = scripts.take_console();
        assert_eq!(
            console[0].text,
            "Uncaught ReferenceError: missing is not defined at https://example.com/page.html:5:1"
        );
        assert_eq!(console[0].level, ConsoleLevel::Error);
        assert!(
            console[1]
                .text
                .starts_with("Uncaught (in promise) Error: later")
        );
        assert_eq!(scripts.state(), ScriptState::Running { errors: 2 });
    }

    #[test]
    fn a_realm_out_of_memory_stops_the_documents_scripts() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            heap: 8 << 20,
            ..Limits::default()
        });
        scripts.run(
            &mut document,
            &script("var kept = []; for (let i = 0; ; i++) kept.push('x'.repeat(1000) + i);"),
        );
        assert!(matches!(scripts.state(), ScriptState::Stopped { .. }));
        assert!(scripts.take_just_stopped());
        assert!(!scripts.take_just_stopped());
        assert_eq!(scripts.run(&mut document, &script("1")), None);
        let console = scripts.take_console();
        assert_eq!(
            console.last().unwrap().text,
            "Scripts stopped on this page: out of memory"
        );
        // The document is the engine's again.
        assert_eq!(document.text_content(document.root()), "x");
    }

    #[test]
    fn the_console_keeps_its_last_entries_each_cut() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::default();
        scripts.run(
            &mut document,
            &script(
                "for (let i = 0; i < 1004; i++) console.log(i); \
                 console.log('y'.repeat(20000)); console.log('y'.repeat(16383) + '\\u00e9');",
            ),
        );
        let console = scripts.take_console();
        assert_eq!(console.len(), MAX_CONSOLE_ENTRIES);
        assert_eq!(console[0].text, "6");
        assert_eq!(console[998].text.len(), MAX_CONSOLE_MESSAGE);
        // Cut short of the character that would cross the limit.
        assert_eq!(console[999].text.len(), MAX_CONSOLE_MESSAGE - 1);
    }

    #[test]
    fn a_script_that_runs_too_long_is_stopped_and_said_so() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            steps: 10,
            ..Limits::default()
        });
        assert_eq!(scripts.run(&mut document, &script("for (;;) {}")), None);
        let console = scripts.take_console();
        assert!(
            console[0]
                .text
                .starts_with("Script ran too long and was stopped at https://example.com/:1:"),
            "{}",
            console[0].text
        );
        // The realm lives on.
        assert!(scripts.run(&mut document, &script("1")).is_some());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 1 });
    }

    #[test]
    fn a_dead_realms_refusal_is_no_error_of_the_pages() {
        let mut scripts = Scripts::default();
        scripts.report(&ScriptError {
            kind: ErrorKind::Dead,
            message: "dead".to_owned(),
            url: String::new(),
            line: 0,
            column: 0,
        });
        assert!(scripts.take_console().is_empty());
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
    }

    #[test]
    fn a_restart_makes_a_new_realm_and_keeps_the_console() {
        let mut document = parsed("<p>x");
        let mut scripts = Scripts::with_limits(Limits {
            heap: 8 << 20,
            ..Limits::default()
        });
        let source = "console.log(typeof seen); var seen = 1; missing()";
        scripts.run(&mut document, &script(source));
        scripts.restart();
        assert_eq!(scripts.state(), ScriptState::Running { errors: 0 });
        scripts.run(&mut document, &script(source));
        let console: Vec<String> = scripts.take_console().into_iter().map(|e| e.text).collect();
        assert_eq!(console[0], "undefined");
        assert_eq!(console[2], "undefined");
        // The limits stay.
        scripts.run(
            &mut document,
            &script(
                "var kept = []; for (let i = 0; i < 20000; i++) kept.push('x'.repeat(1000) + i);",
            ),
        );
        assert!(matches!(scripts.state(), ScriptState::Stopped { .. }));
    }

    #[test]
    fn a_script_elements_type_is_its_type_or_language() {
        let types = |source: &str| {
            let document = parsed(source);
            document
                .subtree(document.root())
                .filter(|&n| document.tag(n) == Some("script"))
                .map(|n| script_type(&document, n))
                .collect::<Vec<_>>()
        };
        use ScriptType::*;
        assert_eq!(
            types(
                "<script></script><script type=''></script><script type=' TEXT/JavaScript '></script>\
                 <script language=javascript></script><script language=''></script>\
                 <script type=application/x-ecmascript language=vbscript></script>"
            ),
            [Some(Classic); 6]
        );
        assert_eq!(
            types(
                "<script type='text/javascript;charset=utf-8'></script><script type=text/plain></script>\
                 <script language=vbscript></script><script type=module></script>\
                 <script type=ImportMap></script><script type=speculationrules></script>"
            ),
            [
                None,
                None,
                None,
                Some(Module),
                Some(ImportMap),
                Some(SpeculationRules)
            ]
        );
    }
}
