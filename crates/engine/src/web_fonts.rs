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
    /// How many script faces they were made with, after the rules'.
    scripted: usize,
}

/// A face a script added to `document.fonts` (CSS Font Loading 3): its
/// number, descriptors and data, a font.
pub(crate) struct ScriptFace {
    id: u32,
    rule: FaceRule,
    data: Vec<u8>,
}

/// What a script did to the document's fonts.
pub(crate) enum FontChange {
    Add(ScriptFace),
    Delete(u32),
}

/// The answer to a script's `fonts` call (`js::Call::Fonts`): the
/// fonts' status by `loading`, whether data is a font, and the faces
/// added or taken away, kept in `changes` for the engine.
pub(crate) fn answer(
    op: &str,
    data: Vec<u8>,
    loading: bool,
    changes: &mut Vec<FontChange>,
) -> js::Answer {
    let mut words = op.splitn(8, ' ');
    match words.next() {
        Some("status") => js::Answer::String(if loading { "loading" } else { "loaded" }.into()),
        Some("valid") => js::Answer::Bool(text::is_font(&data)),
        Some("add") => {
            let mut number = || words.next().and_then(|w| w.parse::<f32>().ok());
            let (Some(id), Some(w0), Some(w1)) = (number(), number(), number()) else {
                return js::Answer::Bool(false);
            };
            let slant = match words.next() {
                Some("italic") => Slant::Italic,
                Some("oblique") => Slant::Oblique(14.0),
                _ => Slant::Normal,
            };
            let mut number = || words.next().and_then(|w| w.parse::<f32>().ok());
            let (Some(s0), Some(s1)) = (number(), number()) else {
                return js::Answer::Bool(false);
            };
            let family = words.next().unwrap_or_default().to_owned();
            if family.is_empty() || !text::is_font(&data) {
                return js::Answer::Bool(false);
            }
            let clamp = |w: f32| w.clamp(1.0, 1000.0) as u16;
            changes.push(FontChange::Add(ScriptFace {
                id: id as u32,
                rule: FaceRule {
                    family,
                    weight: (clamp(w0.min(w1)), clamp(w0.max(w1))),
                    slant,
                    stretch: (s0.min(s1), s0.max(s1)),
                },
                data,
            }));
            js::Answer::Bool(true)
        }
        Some("delete") => {
            if let Some(id) = words.next().and_then(|w| w.parse().ok()) {
                changes.push(FontChange::Delete(id));
            }
            js::Answer::Nothing
        }
        _ => js::Answer::Nothing,
    }
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

    /// Gathers the shown document's `@font-face` rules: when they or the
    /// faces its scripts added changed, its faces are made anew, the
    /// rules' none loaded, and it is laid out again.
    pub(crate) fn update_web_fonts(&mut self) {
        let rules = self
            .shown
            .stylesheets
            .font_faces(&self.media(self.viewport));
        let scripted = self.shown.script_fonts.len();
        let same = match &self.shown.web_fonts {
            Some(web) => web.rules == rules && web.scripted == scripted,
            None => rules.is_empty() && scripted == 0,
        };
        if same {
            return;
        }
        self.remake_web_fonts(rules);
    }

    /// Makes the shown document's faces anew from `rules` and its
    /// scripts' faces, and lays it out again.
    fn remake_web_fonts(&mut self, rules: Vec<FontFaceRule>) {
        let scripts = &self.shown.script_fonts;
        self.shown.web_fonts = (!rules.is_empty() || !scripts.is_empty()).then(|| {
            self.next_font_set += 1;
            let faces: Vec<FaceRule> = rules
                .iter()
                .map(face_rule)
                .chain(scripts.iter().map(|face| face.rule.clone()))
                .collect();
            let fonts = self
                .layout_fonts
                .with_document(Arc::new(DocumentFaces::new(faces)));
            for (n, face) in scripts.iter().enumerate() {
                let _ = fonts.face_arrived(rules.len() + n, face.data.clone());
            }
            WebFonts {
                id: self.next_font_set,
                next: vec![0; rules.len()],
                scripted: scripts.len(),
                rules,
                fonts,
            }
        });
        self.shown.layout_cache.clear();
        self.shown.layout = None;
        self.changed = true;
    }

    /// Applies what scripts did to the document's fonts.
    pub(crate) fn script_fonts_changed(&mut self, changes: Vec<FontChange>) {
        for change in changes {
            match change {
                FontChange::Add(face) => {
                    self.shown.script_fonts.retain(|kept| kept.id != face.id);
                    if self.shown.script_fonts.len() < text::MAX_FONT_FACES {
                        self.shown.script_fonts.push(face);
                    }
                }
                FontChange::Delete(id) => self.shown.script_fonts.retain(|kept| kept.id != id),
            }
        }
        let rules = self
            .shown
            .web_fonts
            .as_ref()
            .map_or_else(Vec::new, |web| web.rules.clone());
        self.remake_web_fonts(rules);
    }

    /// Whether the shown document's web fonts are loading: a face asked
    /// for or coming, or `@font-face` rules not yet laid out with.
    pub(crate) fn fonts_loading(&self) -> bool {
        match &self.shown.web_fonts {
            Some(web) => {
                self.shown.layout.is_none() && !web.rules.is_empty()
                    || web.fonts.document().is_some_and(|faces| faces.is_loading())
            }
            None => !self
                .shown
                .stylesheets
                .font_faces(&self.media(self.viewport))
                .is_empty(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_a_script_adds_is_a_font_with_its_descriptors() {
        let mut changes = Vec::new();
        let added = answer(
            "add 7 700 300 italic 125 75 My Font",
            text::TEST_FONT.to_vec(),
            false,
            &mut changes,
        );
        assert!(matches!(added, js::Answer::Bool(true)));
        let [FontChange::Add(face)] = changes.as_slice() else {
            panic!("one face added");
        };
        assert_eq!(face.id, 7);
        assert_eq!(
            face.rule,
            FaceRule {
                family: "My Font".into(),
                weight: (300, 700),
                slant: Slant::Italic,
                stretch: (75.0, 125.0),
            }
        );
        // What is no font, or has no family, is not added.
        let mut changes = Vec::new();
        for op in [
            "add 1 400 400 normal 100 100 X",
            "add 1 400 400 normal 100 100",
        ] {
            let data = if op.ends_with('X') {
                b"junk".to_vec()
            } else {
                text::TEST_FONT.to_vec()
            };
            assert!(matches!(
                answer(op, data, false, &mut changes),
                js::Answer::Bool(false)
            ));
        }
        assert!(changes.is_empty());
        assert!(matches!(
            answer("status", Vec::new(), true, &mut changes),
            js::Answer::String(s) if s == "loading"
        ));
    }
}
