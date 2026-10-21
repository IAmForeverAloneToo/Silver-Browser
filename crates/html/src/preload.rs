//! The preload scanner (M38, `docs/design/http.md`): a second tokenizer
//! given the document's text as it is decoded, which reads ahead of the
//! parser, past a script that blocks it, and gives back hints of what
//! the document will ask for: its stylesheets, scripts and images, its
//! `<link rel=preload>`s, and its first `<base href>`. It builds no tree
//! and fetches nothing; the engine decides what each hint is worth.
//!
//! It tokenizes as the parser would where that is simple: an element
//! whose content is text (`script`, `style`, `title`, `textarea`, `xmp`,
//! `iframe`, `noembed`, `noframes`, `plaintext`, and `noscript` with
//! scripting on) is read as text; inside `<template>` nothing is a hint
//! (its content is inert), nor inside `<svg>` or `<math>`, whose
//! `script` and `style` are elements like any other. A document whose
//! markup the tree builder would treat otherwise only loses or gains a
//! speculative request, which the page's own requests then do not take.

use crate::tokenizer::{TextState, Token, Tokenizer, content_state};

/// What a hint asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintKind {
    /// A `<link rel=stylesheet>`.
    Stylesheet,
    /// A `<script src>`.
    Script,
    /// An `<img src>`.
    Image,
    /// A `<link rel=preload>`, its `as` among its attributes.
    Preload,
    /// The first `<base href>`: what the URLs after it are resolved
    /// against.
    Base,
    /// A `<meta>` that sets a policy requests are held to: a Content
    /// Security Policy (or its report-only one), or a referrer policy.
    /// The engine preloads nothing after it, whose requests it may not
    /// make as the policy would. Its URL is empty.
    Policy,
}

/// A hint: its kind, its URL as written (not resolved), and the
/// element's attributes, which say how it is to be requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub kind: HintKind,
    pub url: String,
    pub attrs: Vec<(String, String)>,
}

impl Hint {
    /// The value of the attribute `name`, if the element had it.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Whether the space-separated `list` holds `token`, ASCII case
/// ignored.
fn has_token(list: &str, token: &str) -> bool {
    list.split(|c: char| c.is_ascii_whitespace())
        .any(|t| t.eq_ignore_ascii_case(token))
}

/// The scanner of one document.
pub struct Scanner {
    tokenizer: Tokenizer,
    scripting: bool,
    /// How deep in `<template>`, and in `<svg>` or `<math>`, it is.
    template: usize,
    foreign: usize,
    base_seen: bool,
    hints: Vec<Hint>,
}

impl Scanner {
    /// A scanner for a document parsed with HTML's scripting flag
    /// `scripting`.
    pub fn new(scripting: bool) -> Scanner {
        Scanner {
            tokenizer: Tokenizer::streaming(),
            scripting,
            template: 0,
            foreign: 0,
            base_seen: false,
            hints: Vec::new(),
        }
    }

    /// Scans `text`, the next of the document's.
    pub fn feed(&mut self, text: &str) {
        self.tokenizer.feed(text);
        self.scan();
    }

    /// The end of the document's text.
    pub fn end(&mut self) {
        self.tokenizer.end();
        self.scan();
    }

    /// The hints found since they were last taken.
    pub fn take_hints(&mut self) -> Vec<Hint> {
        std::mem::take(&mut self.hints)
    }

    fn scan(&mut self) {
        while let Some(token) = self.tokenizer.next_token() {
            match token {
                Token::StartTag {
                    name,
                    attrs,
                    self_closing,
                } => self.start(name, attrs, self_closing),
                Token::EndTag(name) => match name.as_str() {
                    "template" if self.foreign == 0 => {
                        self.template = self.template.saturating_sub(1)
                    }
                    "svg" | "math" => self.foreign = self.foreign.saturating_sub(1),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn start(&mut self, name: String, attrs: Vec<(String, String)>, self_closing: bool) {
        if self.foreign > 0 {
            if matches!(name.as_str(), "svg" | "math") && !self_closing {
                self.foreign += 1;
            }
            return;
        }
        let state = match name.as_str() {
            "noscript" if self.scripting => Some(TextState::Rawtext),
            "svg" | "math" => {
                if !self_closing {
                    self.foreign += 1;
                    self.tokenizer.set_cdata_allowed(true);
                }
                None
            }
            "template" => {
                self.template += 1;
                None
            }
            name => content_state(name),
        };
        if let Some(state) = state {
            self.tokenizer.set_state(state);
        }
        self.tokenizer.set_cdata_allowed(self.foreign > 0);
        if self.template > 0 {
            return;
        }
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|(n, _)| n == key)
                .map(|(_, v)| v.as_str())
        };
        if name == "meta" {
            let equiv = attr("http-equiv").unwrap_or_default();
            if equiv.eq_ignore_ascii_case("content-security-policy")
                || equiv.eq_ignore_ascii_case("content-security-policy-report-only")
                || attr("name").is_some_and(|n| n.eq_ignore_ascii_case("referrer"))
            {
                self.hints.push(Hint {
                    kind: HintKind::Policy,
                    url: String::new(),
                    attrs,
                });
            }
            return;
        }
        let (kind, url) = match name.as_str() {
            "base" if !self.base_seen => {
                let Some(href) = attr("href") else {
                    return;
                };
                self.base_seen = true;
                (HintKind::Base, href)
            }
            "link" => {
                let rel = attr("rel").unwrap_or_default();
                let kind = if has_token(rel, "stylesheet") && !has_token(rel, "alternate") {
                    HintKind::Stylesheet
                } else if has_token(rel, "preload") {
                    HintKind::Preload
                } else {
                    return;
                };
                match attr("href") {
                    Some(href) => (kind, href),
                    None => return,
                }
            }
            "script" => match attr("src") {
                Some(src) => (HintKind::Script, src),
                None => return,
            },
            "img" => match attr("src") {
                Some(src) => (HintKind::Image, src),
                None => return,
            },
            _ => return,
        };
        let url = url
            .trim_matches(|c: char| c.is_ascii_whitespace())
            .to_owned();
        if !url.is_empty() {
            self.hints.push(Hint { kind, url, attrs });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(html: &str, scripting: bool) -> Vec<(HintKind, String)> {
        let mut scanner = Scanner::new(scripting);
        // In pieces, as text arrives.
        let chars: Vec<char> = html.chars().collect();
        for piece in chars.chunks(3) {
            scanner.feed(&piece.iter().collect::<String>());
        }
        scanner.end();
        scanner
            .take_hints()
            .into_iter()
            .map(|hint| (hint.kind, hint.url))
            .collect()
    }

    fn of(pairs: &[(HintKind, &str)]) -> Vec<(HintKind, String)> {
        pairs.iter().map(|(k, u)| (*k, (*u).to_owned())).collect()
    }

    #[test]
    fn hints_come_in_the_documents_order_past_a_blocking_script() {
        use HintKind::*;
        let html = "<base href=/b/><base href=/ignored/>\
            <link rel='Stylesheet' href=a.css><link rel='alternate stylesheet' href=no.css>\
            <link rel=preload href=f.woff as=font crossorigin>\
            <script src=s.js></script><script>document.write('<img src=w.png>')</script>\
            <img src=' i.png '><img src=''><img><link rel=icon href=i.ico>";
        assert_eq!(
            scan(html, true),
            of(&[
                (Base, "/b/"),
                (Stylesheet, "a.css"),
                (Preload, "f.woff"),
                (Script, "s.js"),
                (Image, "i.png"),
            ])
        );
    }

    #[test]
    fn text_elements_templates_and_foreign_content_give_none() {
        use HintKind::*;
        let html = "<title><img src=t.png></title><textarea><img src=t.png></textarea>\
            <style><img src=s.png></style><script><img src=s.png></script>\
            <xmp><img src=x.png></xmp><noscript><img src=n.png></noscript>\
            <template><img src=t.png><template></template><img src=t2.png></template>\
            <svg><script><img src=svg.png></script><svg></svg><link rel=stylesheet href=svg.css></svg>\
            <img src=after.png><plaintext><img src=p.png>";
        assert_eq!(scan(html, true), of(&[(Image, "after.png")]));
        // Scripting off: noscript's content is markup.
        assert_eq!(
            scan("<noscript><img src=n.png></noscript>", false),
            of(&[(Image, "n.png")])
        );
    }

    #[test]
    fn a_meta_setting_a_policy_is_a_hint_in_its_place() {
        use HintKind::*;
        let html = "<img src=a.png><meta http-equiv=Content-Security-Policy content=x>\
            <meta name=Referrer content=no-referrer><meta name=viewport content=x>\
            <meta http-equiv=content-security-policy-report-only content=x><img src=b.png>";
        assert_eq!(
            scan(html, true),
            of(&[
                (Image, "a.png"),
                (Policy, ""),
                (Policy, ""),
                (Policy, ""),
                (Image, "b.png")
            ])
        );
    }

    #[test]
    fn a_hint_keeps_its_elements_attributes() {
        let mut scanner = Scanner::new(true);
        scanner.feed(
            "<img src=a.png crossorigin=use-credentials loading=lazy referrerpolicy=no-referrer>",
        );
        scanner.end();
        let hints = scanner.take_hints();
        assert_eq!(hints[0].attr("crossorigin"), Some("use-credentials"));
        assert_eq!(hints[0].attr("loading"), Some("lazy"));
        assert_eq!(hints[0].attr("referrerpolicy"), Some("no-referrer"));
        assert_eq!(hints[0].attr("nomodule"), None);
        assert!(scanner.take_hints().is_empty(), "taken");
    }
}
