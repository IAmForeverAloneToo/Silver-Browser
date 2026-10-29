//! A document's web fonts (gap batch D, D54): the faces of the
//! `@font-face` rules in force, laid over the engine's fonts, each
//! fetched when matching first asks for it, its sources tried in turn.
//! Until a face comes the next family stands in, and its arrival lays
//! the document out again.

use std::sync::Arc;

use style::computed::FontStyle;
use style::font_face::{FontFaceRule, FontSource};
use text::{DocumentFaces, FaceRule, FontCollection, Slant};

use crate::Engine;
use crate::loader::{LoadError, Target};

/// The shown document's web fonts.
pub(crate) struct WebFonts {
    /// The number of this set, never another's: an answer for an older
    /// set is dropped.
    id: u64,
    /// The rules they were made from.
    rules: Vec<FontFaceRule>,
    /// The engine's fonts with the faces over them.
    fonts: FontCollection,
    /// Each face's source to try next.
    next: Vec<usize>,
}

/// The descriptors of `rule`'s face, as matching takes them.
fn face_rule(rule: &FontFaceRule) -> FaceRule {
    FaceRule {
        family: rule.family.clone(),
        weight: rule.weight,
        slant: match rule.style {
            FontStyle::Normal => Slant::Normal,
            FontStyle::Italic => Slant::Italic,
            FontStyle::Oblique(angle) => Slant::Oblique(angle),
        },
        stretch: rule.stretch,
    }
}

impl Engine {
    /// The fonts the shown document lays out with: the engine's, with
    /// its web fonts over them.
    pub(crate) fn document_fonts(&self) -> &FontCollection {
        self.shown
            .web_fonts
            .as_ref()
            .map_or(&self.layout_fonts, |web| &web.fonts)
    }

    /// Gathers the shown document's `@font-face` rules: when they changed,
    /// its faces are made anew, none loaded, and it is laid out again.
    pub(crate) fn update_web_fonts(&mut self) {
        let rules = self
            .shown
            .stylesheets
            .font_faces(&self.media(self.viewport));
        let same = match &self.shown.web_fonts {
            Some(web) => web.rules == rules,
            None => rules.is_empty(),
        };
        if same {
            return;
        }
        self.shown.web_fonts = (!rules.is_empty()).then(|| {
            self.next_font_set += 1;
            let faces = Arc::new(DocumentFaces::new(rules.iter().map(face_rule).collect()));
            WebFonts {
                id: self.next_font_set,
                next: vec![0; rules.len()],
                rules,
                fonts: self.layout_fonts.with_document(faces),
            }
        });
        self.shown.layout_cache.clear();
        self.shown.layout = None;
    }

    /// Fetches the faces the last layout asked for.
    pub(crate) fn request_fonts(&mut self) {
        let wanted = match &self.shown.web_fonts {
            Some(web) => web
                .fonts
                .document()
                .map_or_else(Vec::new, |faces| faces.wanted()),
            None => return,
        };
        for face in wanted {
            self.fetch_font(face);
        }
    }

    /// Fetches face `face`'s next source; with none left, the face fails.
    fn fetch_font(&mut self, face: usize) {
        if self.shown.made {
            return;
        }
        self.sync_csp();
        let initiator = self.shown.page.initiator();
        let page = self.shown.page.id();
        let shown = &mut self.shown;
        let Some(web) = &mut shown.web_fonts else {
            return;
        };
        let (Some(rule), Some(next)) = (web.rules.get(face), web.next.get_mut(face)) else {
            return;
        };
        while let Some(source) = rule.sources.get(*next) {
            *next += 1;
            // A face of the system's by its name is not looked for: the
            // url after it is fetched.
            let FontSource::Url(url) = source else {
                continue;
            };
            // A fetched sheet's urls are made absolute against it; a
            // `style` element's are of the document's base.
            let url = shown.page.resolve(&shown.document, url);
            let (Some(url), Some(initiator)) = (url, initiator.clone()) else {
                continue;
            };
            let request = net::Request::get(url, net::Destination::Font)
                .initiated_by(initiator)
                .with_body_limit(text::MAX_FONT_BYTES);
            let target = Target::Font { set: web.id, face };
            self.network.fetch(page, target, request);
            return;
        }
        if let Some(faces) = web.fonts.document() {
            faces.failed(face);
        }
    }

    /// Takes the answer to a face's fetch: a font lays the document out
    /// again; anything else has the face's next source tried.
    pub(crate) fn font_arrived(
        &mut self,
        page: crate::loader::PageId,
        (set, face): (u64, usize),
        result: Result<net::Response, LoadError>,
    ) {
        if page != self.shown.page.id() {
            return;
        }
        let Some(web) = &self.shown.web_fonts else {
            return;
        };
        if web.id != set {
            return;
        }
        let body = match result {
            Ok(response) if (200..300).contains(&response.status) => Some(response.body),
            _ => None,
        };
        let taken = body.is_some_and(|body| web.fonts.face_arrived(face, body.to_vec()).is_ok());
        if !taken {
            self.fetch_font(face);
            return;
        }
        self.shown.layout_cache.clear();
        self.shown.layout = None;
        self.changed = true;
    }
}
