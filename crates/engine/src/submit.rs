//! Form submission (M26.9, `docs/design/events.md`, "Form submission",
//! HTML section 4.10.21.3): where a form goes and what it sends, for the
//! engine and its scripts' host alike. The events before it, `invalid`
//! and `submit`, are the caller's; so is the navigation after.

use dom::{Document, NodeId, forms};
use net::Url;
use net::url::form::{self, FormEntry, FormValue};

use crate::loader::Page;

/// A POST's body and its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Post {
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: String,
}

/// Where a submission goes: its URL, with the entries in its query for
/// a GET; with a body for a POST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Submission {
    pub(crate) url: Url,
    pub(crate) post: Option<Post>,
    /// The navigable it goes to (M33.2): the submitter's `formtarget`,
    /// the form's `target`, or the document's base target.
    pub(crate) target: String,
}

/// The submitter's `form*` attribute if it has it, else the form's.
fn attribute<'d>(
    document: &'d Document,
    form: NodeId,
    submitter: Option<NodeId>,
    name: &str,
) -> Option<&'d str> {
    submitter
        .and_then(|submitter| document.attr(submitter, &format!("form{name}")))
        .or_else(|| document.attr(form, name))
}

/// What submitting `form` with `submitter` does (HTML section
/// 4.10.21.3): its action, the submitter's `formaction` or the form's
/// `action`, resolved against the document's base URL, the document's
/// URL when empty; its method, `post`, or `get` for any other but
/// `dialog`, which goes nowhere (G65); its encoding, the first label of
/// `accept-charset` the `encoding` crate knows, else the document's; and
/// its entries, serialized as `enctype` says: in the action URL's query
/// for a GET, `application/x-www-form-urlencoded`, `multipart/form-data`
/// or `text/plain` in a POST's body. `None` where nothing is to be
/// done.
pub(crate) fn plan(
    document: &Document,
    page: &Page,
    form: NodeId,
    submitter: Option<NodeId>,
    seed: u64,
) -> Option<Submission> {
    let action = attribute(document, form, submitter, "action").unwrap_or("");
    let url = if action.is_empty() {
        page.url()?.clone()
    } else {
        page.resolve(document, action)?
    };
    let method = attribute(document, form, submitter, "method")
        .unwrap_or("")
        .to_ascii_lowercase();
    if method == "dialog" {
        return None;
    }
    let post = method == "post";
    let encoding = document
        .attr(form, "accept-charset")
        .and_then(|labels| {
            labels
                .split(|c: char| c.is_ascii_whitespace())
                .find_map(encoding::Encoding::for_label)
        })
        .unwrap_or_else(|| page.encoding())
        .output_encoding();
    let entries: Vec<FormEntry> = forms::entry_list(document, form, submitter, encoding.name())
        .into_iter()
        .map(|entry| FormEntry {
            name: entry.name,
            value: match entry.value {
                Some(text) => FormValue::Text(text),
                None => FormValue::File {
                    name: String::new(),
                    content_type: "application/octet-stream".to_owned(),
                    data: Vec::new(),
                },
            },
        })
        .collect();
    // A `javascript:` action runs as a link's does, the entries unsent.
    if url.scheme() == "javascript" {
        return Some(Submission {
            url,
            post: None,
            target: target(document, form, submitter),
        });
    }
    if !post {
        let query = form::urlencoded(&entries, encoding);
        let whole = url.as_str();
        let end = whole.find(['?', '#']).unwrap_or(whole.len());
        let fragment = url
            .fragment()
            .map_or(String::new(), |fragment| format!("#{fragment}"));
        let url = Url::parse(&format!("{}?{query}{fragment}", &whole[..end])).ok()?;
        return Some(Submission {
            url,
            post: None,
            target: target(document, form, submitter),
        });
    }
    let enctype = attribute(document, form, submitter, "enctype")
        .unwrap_or("")
        .to_ascii_lowercase();
    let post = match enctype.as_str() {
        "multipart/form-data" => {
            let boundary = form::boundary(seed);
            Post {
                body: form::multipart(&entries, encoding, &boundary),
                content_type: format!("multipart/form-data; boundary={boundary}"),
            }
        }
        "text/plain" => Post {
            body: form::text_plain(&entries, encoding),
            content_type: "text/plain".to_owned(),
        },
        _ => Post {
            body: form::urlencoded(&entries, encoding).into_bytes(),
            content_type: "application/x-www-form-urlencoded".to_owned(),
        },
    };
    Some(Submission {
        url,
        post: Some(post),
        target: target(document, form, submitter),
    })
}

/// Why `form` of a sandboxed document is not submitted (M33.2): its
/// flags without `allow-forms`, the console's line saying so; `None`
/// when it may be.
pub(crate) fn blocked(document: &Document, page: &Page, form: NodeId) -> Option<String> {
    if !page.sandbox().has(crate::sandbox::Sandbox::FORMS) {
        return None;
    }
    let action = document.attr(form, "action").unwrap_or_default();
    let action = page
        .resolve(document, action)
        .map(|url| url.to_string())
        .unwrap_or_default();
    Some(format!(
        "Blocked form submission to '{action}' because the form's frame is sandboxed and the \
         'allow-forms' permission is not set."
    ))
}

/// A submission's target (HTML section 4.10.21.3): the submitter's
/// `formtarget`, else the form's `target`, else the document's base
/// target (M33.2).
fn target(document: &Document, form: NodeId, submitter: Option<NodeId>) -> String {
    match attribute(document, form, submitter, "target") {
        Some(target) => target.to_owned(),
        None => crate::frames::base_target(document),
    }
}

impl crate::Engine {
    /// Submits `form` (HTML section 4.10.21.3) with `submitter`: unless
    /// `form.submit()` asked, a cancelable `submit` naming the submitter
    /// first; then the navigation its plan makes, a link's, refused
    /// where a link would be, a `javascript:` action run.
    pub(crate) fn submit_form(
        &mut self,
        form: NodeId,
        submitter: Option<NodeId>,
        from_method: bool,
    ) {
        if let Some(line) = blocked(&self.shown.document, &self.shown.page, form) {
            self.shown.scripts.say(js::ConsoleLevel::Error, line);
            return;
        }
        if !from_method && !self.validate_interactively(form, submitter) {
            return;
        }
        if !from_method {
            let event = js::EventInit::new("SubmitEvent", "submit")
                .with("bubbles", js::Field::Bool(true))
                .with("cancelable", js::Field::Bool(true))
                .with(
                    "submitter",
                    js::Field::Node(submitter.map(crate::scripts::handle)),
                );
            if self.fire(js::Target::Node(crate::scripts::handle(form)), event) == Some(true) {
                return;
            }
        }
        let document = &self.shown.document;
        if !document.is_inclusive_ancestor(document.root(), form) {
            return;
        }
        let seed = document.version() ^ (self.clock)().as_nanos() as u64;
        let Some(submission) = plan(document, &self.shown.page, form, submitter, seed) else {
            return;
        };
        self.follow_submission(submission);
    }

    /// HTML's interactive validation of a form about to be submitted
    /// (section 4.10.20.2, M26.9b), unless it or its submitter says
    /// `novalidate`: every control's user validity set; an `invalid` at
    /// each that fails; the first whose `invalid` was not canceled
    /// focused, scrolled to and its message given as the notice. Whether
    /// the submission goes on.
    fn validate_interactively(&mut self, form: NodeId, submitter: Option<NodeId>) -> bool {
        let document = &self.shown.document;
        if document.attr(form, "novalidate").is_some()
            || submitter
                .is_some_and(|submitter| document.attr(submitter, "formnovalidate").is_some())
        {
            return true;
        }
        dom::validity::mark_submitted(&mut self.shown.document, form);
        self.state_changed();
        let invalid = dom::validity::invalid_controls(&self.shown.document, form);
        if invalid.is_empty() {
            return true;
        }
        let mut first = None;
        for control in invalid {
            let event =
                js::EventInit::new("Event", "invalid").with("cancelable", js::Field::Bool(true));
            let canceled = self.fire(js::Target::Node(crate::scripts::handle(control)), event);
            if canceled != Some(true) && first.is_none() && self.shown.document.contains(control) {
                first = Some(control);
            }
        }
        if let Some(first) = first {
            let message = dom::validity::validation_message(&self.shown.document, first);
            let visible = forms::is_text_control(&self.shown.document, first);
            self.focus(Some(first), visible, true);
            self.notice = Some(message);
        }
        false
    }

    /// Goes where a submission says, as a link would.
    fn follow_submission(&mut self, submission: Submission) {
        let url = submission.url;
        if url.scheme() == "javascript" {
            if self.scripts_run() {
                self.run_javascript_url(&url);
            }
            return;
        }
        let scheme = self.shown.page.url().map(Url::scheme);
        match crate::refusal(&url, scheme, self.shown.made) {
            Some(why) => self.notice = Some(why),
            // Its policies' `form-action` decides, before and at each
            // redirect (M33.1).
            None if !self.form_action_allows(&url) => {}
            None => {
                let source = crate::NavigationSource {
                    form: true,
                    ..self.navigation_source(true)
                };
                // Another frame named by its target (M33.2).
                match self.other_navigable(&submission.target, &url, true) {
                    Ok(crate::frames::Chosen::Frame(frame)) => {
                        self.navigate_other(frame, url, false, submission.post, source.origin);
                    }
                    // A form sent to a new tab (M37): a GET opens one; a
                    // POST is sent here (a gap).
                    Ok(crate::frames::Chosen::NewTab) if submission.post.is_none() => {
                        self.open_tab(url, true, false);
                    }
                    Ok(_) => self.submit_to(url, submission.post, source),
                    Err(line) => self.shown.scripts.say(js::ConsoleLevel::Error, line),
                }
            }
        }
    }

    /// [`crate::Engine::navigate`], a POST of `post` when given (M26.9):
    /// a new entry, after the current one but in the initial document's
    /// place, never a fragment's navigation.
    pub(crate) fn navigate_with(
        &mut self,
        url: Url,
        post: Option<Post>,
        source: crate::NavigationSource,
    ) {
        let Some(post) = post else {
            self.navigate_replacing(url, false, Some(source));
            return;
        };
        let handling = if self.shown.initial {
            crate::Handling::Replace
        } else {
            crate::Handling::Push
        };
        self.start_navigation_with(url, handling, Some(post), Some(source));
    }

    /// Sends a form's submission to `url` in this navigable, as
    /// [`crate::Engine::navigate_with`] does, unless it would go from a
    /// secure document to an `http:` URL that is not potentially
    /// trustworthy (M33.3): then the browser's page says so in its place,
    /// in an entry of its own that is never fetched again, and keeps the
    /// submission for that page's "Send anyway".
    pub(crate) fn submit_to(
        &mut self,
        url: Url,
        post: Option<Post>,
        source: crate::NavigationSource,
    ) {
        // Any URL that is not potentially trustworthy: `http:`, the
        // other schemes being refused before.
        let mixed = self.shown.page.is_secure_context() && !url.is_potentially_trustworthy();
        if !mixed {
            self.navigate_with(url, post, source);
            return;
        }
        self.cancel_navigation();
        let handling = if self.shown.initial {
            crate::Handling::Replace
        } else {
            crate::Handling::Push
        };
        let shown = self.made(crate::pages::mixed_form(&url), None);
        self.posting = true;
        self.commit(shown, Some(url.clone()), handling);
        self.pending_form = Some(crate::PendingForm { url, post, source });
    }

    /// Resets `form` (HTML section 4.10.22) when no script runs its
    /// reset button's activation: its controls back to their defaults.
    /// With no realm there is no listener to hear a `reset`.
    pub(crate) fn reset_form(&mut self, form: NodeId) {
        forms::reset_form(&mut self.shown.document, form);
        self.state_changed();
    }

    /// Implicit submission (HTML section 4.10.21.2) from Enter in the
    /// field `field`: its form's default button clicked, as a key clicks
    /// one; with none, the form submitted when no other field of it
    /// blocks that.
    pub(crate) fn implicit_submission(&mut self, field: NodeId) {
        let document = &self.shown.document;
        let Some(form) = forms::form_owner(document, field) else {
            return;
        };
        match forms::default_button(document, form) {
            Some(button) => {
                if !forms::actually_disabled(document, button) {
                    self.key_click(button);
                }
            }
            None => {
                if forms::implicit_submission_blockers(document, form) <= 1 {
                    self.submit_form(form, None, false);
                }
            }
        }
    }
}
