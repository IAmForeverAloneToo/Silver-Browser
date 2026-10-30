//! A document's web fonts (gap batch D, D54): the faces of the
//! `@font-face` rules in force, laid over the engine's fonts, each
//! fetched when matching first asks for it, its sources tried in turn.
//! Until a face comes the next family stands in, and its arrival lays
//! the document out again.

use std::sync::Arc;

use style::font_face::{FaceStyle, FontFaceRule, FontSource};
use text::{DocumentFaces, FaceRule, FaceSlant, FaceState, FontCollection};

use crate::Engine;
use crate::loader::{LoadError, Target};

/// The shown document's web fonts.
pub(crate) struct WebFonts {
    /// The number of this set, never another's: an answer for an older
    /// set is dropped.
    id: u64,
    /// The rules they were made from.
    rules: Arc<Vec<FontFaceRule>>,
    /// Each rule's face's data once had, and its index in its file: a
    /// set made anew with the same rule has it at once.
    data: Vec<Option<(Vec<u8>, u32)>>,
    /// The engine's fonts with the faces over them.
    fonts: FontCollection,
    /// Each face's source to try next.
    next: Vec<usize>,
    /// How many script faces they were made with, after the rules'.
    scripted: usize,
    /// Whether `document.fonts` was last told its faces are loading.
    loading: bool,
    /// The rules' faces had, and failed, since it was.
    period: (Vec<usize>, Vec<usize>),
    /// Whether a script asked for a rule's face since `document.fonts`
    /// was last told: it is told once the face is had or failed, even
    /// when nothing began loading (a face had already, or found at once
    /// by `local()`), so that its `load()` settles.
    due: bool,
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
    /// A script asked for the face of the document's rule of this
    /// number (`FontFace.load()` of a CSS-connected face).
    Load(usize),
}

/// What a script's `fonts` calls read: whether the document's fonts are
/// loading, its `@font-face` rules' faces, and the fonts it lays out
/// with, those faces among them.
#[derive(Clone, Copy, Default)]
pub(crate) struct FontsEnv<'a> {
    pub(crate) loading: bool,
    pub(crate) rules: &'a [FontFaceRule],
    pub(crate) fonts: Option<&'a FontCollection>,
}

/// The answer to a script's `fonts` call (`js::Call::Fonts`): the
/// fonts' status, whether data is a font, the faces added or taken
/// away and the rules' faces asked for, kept in `changes` for the
/// engine; the rules' faces as `FontFace`s see them; a descriptor
/// checked and written back; a `src`'s sources; a system face's data
/// by its name; and the families of a `font` shorthand. What an op
/// reads as text is in `op`, after its name.
pub(crate) fn answer(
    op: &str,
    data: Vec<u8>,
    env: &FontsEnv<'_>,
    changes: &mut Vec<FontChange>,
) -> js::Answer {
    let mut words = op.splitn(9, ' ');
    let rest = op.split_once(' ').map_or("", |(_, rest)| rest);
    let text = || rest.to_owned();
    match words.next() {
        Some("status") => js::Answer::String(if env.loading { "loading" } else { "loaded" }.into()),
        Some("valid") => js::Answer::Bool(text::is_font(&data)),
        Some("add") => {
            let mut number = || words.next().and_then(|w| w.parse::<f32>().ok());
            let (Some(id), Some(w0), Some(w1)) = (number(), number(), number()) else {
                return js::Answer::Bool(false);
            };
            let slant = match words.next() {
                Some("italic") => FaceSlant::Italic,
                Some("oblique") => FaceSlant::Oblique(14.0, 14.0),
                _ => FaceSlant::NORMAL,
            };
            let mut number = || words.next().and_then(|w| w.parse::<f32>().ok());
            let (Some(s0), Some(s1)) = (number(), number()) else {
                return js::Answer::Bool(false);
            };
            let ranges = words.next().map_or_else(Vec::new, ranges_of);
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
                    ranges,
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
        Some("load") => {
            if let Some(n) = words.next().and_then(|w| w.parse().ok()) {
                changes.push(FontChange::Load(n));
            }
            js::Answer::Nothing
        }
        Some("rules") => {
            let faces = env.fonts.and_then(|fonts| fonts.document());
            let mut out = Vec::new();
            for (n, rule) in env.rules.iter().enumerate() {
                let state = match faces.and_then(|faces| faces.state(n)) {
                    Some(FaceState::Loaded) => "loaded",
                    Some(FaceState::Failed) => "error",
                    Some(FaceState::Wanted | FaceState::Loading) => "loading",
                    _ => "unloaded",
                };
                let t = &rule.text;
                out.extend([
                    rule.family.clone(),
                    t.style.clone(),
                    t.weight.clone(),
                    t.stretch.clone(),
                    t.unicode_range.clone(),
                    t.feature_settings.clone(),
                    t.display.clone(),
                    state.to_owned(),
                ]);
            }
            js::Answer::Strings(out)
        }
        Some("descriptor") => {
            let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
            js::Answer::MaybeString(descriptor(name, value))
        }
        // An `@font-face` rule's block: name, value and an empty
        // priority for each descriptor it takes, as a rule's `style`
        // lists them.
        Some("parse") => {
            let list = css::parse_style_attribute(&text()).unwrap_or_default();
            let mut out = Vec::new();
            for (name, value) in crate::scripts::rules::descriptor_list(&list) {
                out.extend([name.to_owned(), value, String::new()]);
            }
            js::Answer::Strings(out)
        }
        Some("sources") => {
            let sources = style::font_face::sources(&css::tokenize(&text()));
            let mut out = Vec::new();
            for source in sources.unwrap_or_default() {
                match source {
                    FontSource::Url(url) => out.extend(["url".to_owned(), url]),
                    FontSource::Local(name) => out.extend(["local".to_owned(), name]),
                }
            }
            js::Answer::Strings(out)
        }
        Some("local") => match env.fonts.and_then(|fonts| fonts.local_face(&text())) {
            Some((data, 0)) => js::Answer::Bytes(data),
            _ => js::Answer::Nothing,
        },
        Some("families") => {
            let families = style::font_shorthand_families(&css::tokenize(&text()));
            js::Answer::Strings(families.map_or_else(Vec::new, |families| {
                std::iter::once("ok".to_owned())
                    .chain(families.into_iter().filter_map(|family| match family {
                        style::font_family::Family::Name(name) => Some(name),
                        style::font_family::Family::Generic(_) => None,
                    }))
                    .collect()
            }))
        }
        _ => js::Answer::Nothing,
    }
}

/// Applies what a worker's scripts did to its fonts (D54): a face added
/// lies over the fonts its creator gave it, as a document's do, and one
/// deleted is passed over. `faces` holds each face's number by its
/// place among them.
pub(crate) fn worker_fonts_changed(
    fonts: &mut FontCollection,
    faces: &mut Vec<u32>,
    changes: Vec<FontChange>,
) {
    for change in changes {
        let id = match &change {
            FontChange::Add(face) => face.id,
            FontChange::Delete(id) => *id,
            // A worker has no rules' faces to load.
            FontChange::Load(_) => continue,
        };
        if let Some(document) = fonts.document().filter(|_| !faces.is_empty()) {
            for (n, _) in faces.iter().enumerate().filter(|(_, kept)| **kept == id) {
                document.failed(n);
            }
        }
        let FontChange::Add(face) = change else {
            continue;
        };
        // Its own faces, none of a creator's: a worker's first.
        if faces.is_empty() {
            *fonts = fonts.with_document(Arc::new(DocumentFaces::new(Vec::new())));
        }
        let Some(n) = fonts
            .document()
            .and_then(|document| document.push(face.rule))
        else {
            continue;
        };
        faces.push(face.id);
        let _ = fonts.face_arrived(n, face.data);
    }
}

/// Lays the faces `changes` add over `fonts`, the window's for a script
/// run, until the engine makes its faces anew after the run; says
/// whether there were any.
pub(crate) fn lay_over(fonts: &mut FontCollection, changes: &[FontChange]) -> bool {
    let mut any = false;
    for change in changes {
        let FontChange::Add(face) = change else {
            continue;
        };
        if fonts.document().is_none() {
            *fonts = fonts.with_document(Arc::new(DocumentFaces::new(Vec::new())));
        }
        let Some(n) = fonts
            .document()
            .and_then(|document| document.push(face.rule.clone()))
        else {
            continue;
        };
        any |= fonts.face_arrived(n, face.data.clone()).is_ok();
    }
    any
}

/// `value` of the descriptor `name` written back as an `@font-face`
/// rule's would be; `None` for one the descriptor does not take.
fn descriptor(name: &str, value: &str) -> Option<String> {
    let sheet = css::parse_stylesheet(&format!("@font-face {{ {name}: {value} }}")).ok()?;
    let css::Rule::At(at) = sheet.rules.first()? else {
        return None;
    };
    let Some(css::Block::Declarations(list)) = &at.block else {
        return None;
    };
    let [declaration] = list.as_slice() else {
        return None;
    };
    // A value that ends the declaration early is not the value.
    if value.contains([';', '{', '}']) {
        return None;
    }
    style::font_face::descriptor(&declaration.name, &declaration.value).map(|(_, text)| text)
}

/// The ranges a script's face is for, as the bindings write them:
/// `-` for all, else each range's first and last code points in hex,
/// `first-last`, parted by commas.
fn ranges_of(text: &str) -> Vec<(u32, u32)> {
    if text == "-" {
        return Vec::new();
    }
    text.split(',')
        .filter_map(|range| {
            let (lo, hi) = range.split_once('-')?;
            Some((
                u32::from_str_radix(lo, 16).ok()?,
                u32::from_str_radix(hi, 16).ok()?,
            ))
        })
        .take(style::font_face::MAX_UNICODE_RANGES)
        .collect()
}

/// The descriptors of `rule`'s face, as matching takes them.
fn face_rule(rule: &FontFaceRule) -> FaceRule {
    FaceRule {
        family: rule.family.clone(),
        weight: rule.weight,
        slant: match rule.style {
            FaceStyle::Italic => FaceSlant::Italic,
            FaceStyle::Oblique(lo, hi) => FaceSlant::Oblique(lo, hi),
        },
        stretch: rule.stretch,
        ranges: rule.ranges.clone(),
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
    /// faces its scripts added changed, its faces are made anew, those
    /// of rules it had before keeping their data, and it is laid out
    /// again.
    pub(crate) fn update_web_fonts(&mut self) {
        let rules = self
            .shown
            .stylesheets
            .font_faces(&self.media(self.viewport));
        let scripted = self.shown.script_fonts.len();
        let same = match &self.shown.web_fonts {
            Some(web) => *web.rules == rules && web.scripted == scripted,
            None => rules.is_empty() && scripted == 0,
        };
        if same {
            return;
        }
        self.remake_web_fonts(rules);
    }

    /// Makes the shown document's faces anew from `rules` and its
    /// scripts' faces, and lays it out again. A rule's face had before
    /// is had still.
    fn remake_web_fonts(&mut self, rules: Vec<FontFaceRule>) {
        let old = self.shown.web_fonts.take();
        let mut refetch = Vec::new();
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
            let mut data = vec![None; rules.len()];
            if let Some(old) = &old {
                let old_faces = old.fonts.document();
                for (n, rule) in rules.iter().enumerate() {
                    let was = old.rules.iter().position(|was| was == rule);
                    let had = was.and_then(|w| old.data[w].as_ref());
                    if let Some((bytes, index)) = had
                        && fonts.face_arrived_at(n, bytes.to_vec(), *index).is_ok()
                    {
                        data[n] = Some((bytes.clone(), *index));
                        continue;
                    }
                    // One coming for the old set is fetched again for
                    // this one, whose number the answer must carry.
                    let state = was.and_then(|w| old_faces.and_then(|faces| faces.state(w)));
                    if matches!(state, Some(FaceState::Wanted | FaceState::Loading)) {
                        refetch.push(n);
                    }
                }
            }
            for (n, face) in scripts.iter().enumerate() {
                let _ = fonts.face_arrived(rules.len() + n, face.data.clone());
            }
            WebFonts {
                id: self.next_font_set,
                next: vec![0; rules.len()],
                scripted: scripts.len(),
                rules: Arc::new(rules),
                data,
                fonts,
                loading: old.as_ref().is_some_and(|old| old.loading),
                due: old.as_ref().is_some_and(|old| old.due),
                period: old.map(|old| old.period).unwrap_or_default(),
            }
        });
        for n in refetch {
            let faces = self
                .shown
                .web_fonts
                .as_ref()
                .and_then(|web| web.fonts.document().cloned());
            if let Some(faces) = faces {
                faces.loading(n);
                self.fetch_font(n);
            }
        }
        self.shown.layout_cache.clear();
        self.shown.layout = None;
        self.changed = true;
    }

    /// Applies what scripts did to the document's fonts: faces added or
    /// taken away make its faces anew; a rule's face asked for is
    /// fetched.
    pub(crate) fn script_fonts_changed(&mut self, changes: Vec<FontChange>) {
        let mut remake = false;
        let mut load = Vec::new();
        for change in changes {
            match change {
                FontChange::Add(face) => {
                    self.shown.script_fonts.retain(|kept| kept.id != face.id);
                    if self.shown.script_fonts.len() < text::MAX_FONT_FACES {
                        self.shown.script_fonts.push(face);
                    }
                    remake = true;
                }
                FontChange::Delete(id) => {
                    self.shown.script_fonts.retain(|kept| kept.id != id);
                    remake = true;
                }
                FontChange::Load(n) => load.push(n),
            }
        }
        if remake {
            let rules = self
                .shown
                .web_fonts
                .as_ref()
                .map_or_else(Vec::new, |web| web.rules.to_vec());
            self.remake_web_fonts(rules);
        }
        if !load.is_empty() {
            // The faces asked for are of the rules in force now.
            self.sheets_changed();
            self.update_web_fonts();
        }
        for n in load {
            let faces = self.shown.web_fonts.as_ref().and_then(|web| {
                (n < web.rules.len())
                    .then(|| web.fonts.document().cloned())
                    .flatten()
            });
            if let Some(faces) = faces
                && matches!(
                    faces.state(n),
                    Some(FaceState::Unloaded | FaceState::Wanted)
                )
            {
                faces.loading(n);
                self.fetch_font(n);
            }
            if let Some(web) = &mut self.shown.web_fonts {
                web.due = true;
            }
        }
        self.tell_fonts();
    }

    /// Whether the shown document's web fonts are loading (CSS Font
    /// Loading 3's "pending on the environment"): a face asked for or
    /// coming, or `@font-face` rules its faces are not yet made from.
    pub(crate) fn fonts_loading(&self) -> bool {
        let rules = || {
            self.shown
                .stylesheets
                .font_faces(&self.media(self.viewport))
        };
        match &self.shown.web_fonts {
            Some(web) => {
                web.fonts.document().is_some_and(|faces| faces.is_loading())
                    || self.shown.recascade && *web.rules != rules()
            }
            None => !rules().is_empty(),
        }
    }

    /// Whether a style or layout is still to come that could ask for a
    /// face of the document's `@font-face` rules: `document.fonts`
    /// lays the document out for itself before it says it is done,
    /// so that `ready` waits until a layout asked for nothing new and
    /// its faces came.
    pub(crate) fn fonts_unsettled(&self) -> bool {
        self.shown
            .web_fonts
            .as_ref()
            .is_some_and(|web| !web.rules.is_empty())
            && (self.shown.layout.is_none()
                || self.shown.recascade
                || self.shown.styled.is_none()
                || !self.shown.pending.is_empty())
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
        self.tell_fonts();
    }

    /// Tells the document's `document.fonts` when its faces start
    /// loading and when they are done (CSS Font Loading 3 section 2.4):
    /// a `FontFaceSetLoadEvent` the bindings take for the set, with the
    /// numbers of the rules' faces had and failed since loading began.
    pub(crate) fn tell_fonts(&mut self) {
        let loading = self
            .shown
            .web_fonts
            .as_ref()
            .and_then(|web| web.fonts.document())
            .is_some_and(|faces| faces.is_loading());
        let Some(web) = &mut self.shown.web_fonts else {
            return;
        };
        let settled = !web.period.0.is_empty() || !web.period.1.is_empty() || web.due;
        if loading == web.loading && (loading || !settled) {
            return;
        }
        web.loading = loading;
        if !loading {
            web.due = false;
        }
        let event = if loading {
            js::EventInit::new("FontFaceSetLoadEvent", "loading")
        } else {
            let (loaded, failed) = std::mem::take(&mut web.period);
            let list = |faces: Vec<usize>| {
                faces
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let mut event = js::EventInit::new("FontFaceSetLoadEvent", "loadingdone");
            event.fields = vec![
                ("loaded", js::Field::String(list(loaded))),
                ("failed", js::Field::String(list(failed))),
            ];
            event
        };
        let task = js::Task::Dispatch {
            target: js::Target::Window,
            event,
        };
        let now = self.now();
        self.shown.scripts.post(now, task);
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
            let url = match source {
                // A face of the system's by its full or PostScript name;
                // without one, the next source.
                FontSource::Local(name) => {
                    let Some((data, index)) = self.layout_fonts.local_face(name) else {
                        continue;
                    };
                    if web
                        .fonts
                        .face_arrived_at(face, data.clone(), index)
                        .is_err()
                    {
                        continue;
                    }
                    web.data[face] = Some((data, index));
                    web.period.0.push(face);
                    shown.layout_cache.clear();
                    shown.layout = None;
                    self.changed = true;
                    return;
                }
                FontSource::Url(url) => url,
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
        web.period.1.push(face);
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
        let Some(web) = &mut self.shown.web_fonts else {
            return;
        };
        if web.id != set {
            return;
        }
        let body = match result {
            Ok(response) if (200..300).contains(&response.status) => Some(response.body),
            _ => None,
        };
        let taken = body.filter(|body| web.fonts.face_arrived(face, body.to_vec()).is_ok());
        let Some(body) = taken else {
            self.fetch_font(face);
            self.tell_fonts();
            return;
        };
        if let Some(data) = web.data.get_mut(face) {
            *data = Some((body.to_vec(), 0));
        }
        web.period.0.push(face);
        self.shown.layout_cache.clear();
        self.shown.layout = None;
        self.changed = true;
        self.tell_fonts();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(op: &str, data: &[u8], env: &FontsEnv<'_>, changes: &mut Vec<FontChange>) -> js::Answer {
        answer(op, data.to_vec(), env, changes)
    }

    #[test]
    fn a_face_a_script_adds_is_a_font_with_its_descriptors() {
        let env = FontsEnv::default();
        let mut changes = Vec::new();
        let added = ask(
            "add 7 700 300 italic 125 75 41-5a,e9-e9 My Font",
            text::TEST_FONT,
            &env,
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
                slant: FaceSlant::Italic,
                stretch: (75.0, 125.0),
                ranges: vec![(0x41, 0x5A), (0xE9, 0xE9)],
            }
        );
        // What is no font, or has no family, is not added.
        let mut changes = Vec::new();
        for op in [
            "add 1 400 400 normal 100 100 - X",
            "add 1 400 400 normal 100 100 -",
        ] {
            let data: &[u8] = if op.ends_with('X') {
                b"junk"
            } else {
                text::TEST_FONT
            };
            assert!(matches!(
                ask(op, data, &env, &mut changes),
                js::Answer::Bool(false)
            ));
        }
        assert!(changes.is_empty());
        let loading = FontsEnv {
            loading: true,
            ..FontsEnv::default()
        };
        assert!(matches!(
            ask("status", b"", &loading, &mut changes),
            js::Answer::String(s) if s == "loading"
        ));
        ask("load 3", b"", &env, &mut changes);
        assert!(matches!(changes.as_slice(), [FontChange::Load(3)]));
    }

    #[test]
    fn a_face_added_is_laid_over_the_runs_fonts() {
        let mut changes = Vec::new();
        ask(
            "add 1 400 400 normal 100 100 - Web",
            text::TEST_FONT,
            &FontsEnv::default(),
            &mut changes,
        );
        let mut fonts = FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0).unwrap();
        let request = text::FontRequest {
            families: "Web, serif",
            ..text::FontRequest::default()
        };
        assert_eq!(fonts.font(&request).face, text::FaceId(0));
        assert!(!lay_over(&mut fonts, &[]));
        assert!(lay_over(&mut fonts, &changes));
        assert_ne!(fonts.font(&request).face, text::FaceId(0));
    }

    #[test]
    fn descriptors_sources_and_families_are_read_for_scripts() {
        let env = FontsEnv::default();
        let mut changes = Vec::new();
        let written = |name: &str, value: &str| match answer(
            &format!("descriptor {name} {value}"),
            Vec::new(),
            &env,
            &mut Vec::new(),
        ) {
            js::Answer::MaybeString(text) => text,
            _ => panic!("a string or none"),
        };
        assert_eq!(
            written("font-weight", "900 100").as_deref(),
            Some("900 100")
        );
        assert_eq!(
            written("font-style", "oblique 0deg").as_deref(),
            Some("normal")
        );
        assert_eq!(written("font-weight", "bolder"), None);
        assert_eq!(written("font-weight", "100; color: red"), None);
        assert_eq!(
            written("unicode-range", "u+0-7f").as_deref(),
            Some("U+0-7F")
        );
        let js::Answer::Strings(sources) = ask(
            "sources local(A), url(a.woff2) format(woff2), url(\"b.ttf\")",
            b"",
            &env,
            &mut changes,
        ) else {
            panic!("strings");
        };
        assert_eq!(sources, ["local", "A", "url", "b.ttf", "url", "a.woff2"]);
        let js::Answer::Strings(families) = ask(
            "families bold 12px \"Web\", serif, B c",
            b"",
            &env,
            &mut changes,
        ) else {
            panic!("strings");
        };
        assert_eq!(families, ["ok", "Web", "B c"]);
        let js::Answer::Strings(none) = ask("families 12px", b"", &env, &mut changes) else {
            panic!("strings");
        };
        assert!(none.is_empty());
        let js::Answer::Strings(parsed) = ask(
            "parse src: url(a); font-width: condensed; color: red; font-stretch: 50%",
            b"",
            &env,
            &mut changes,
        ) else {
            panic!("strings");
        };
        assert_eq!(parsed, ["src", "url(\"a\")", "", "font-stretch", "50%", ""]);
        // The rules' faces, each with its descriptors and state.
        let sheet = css::parse_stylesheet(
            "@font-face { font-family: a; src: url(a); font-weight: 900 100 }",
        )
        .unwrap();
        let rules =
            style::Stylesheets::new(vec![sheet]).font_faces(&style::Media::screen(800.0, 600.0));
        let env = FontsEnv {
            rules: &rules,
            ..FontsEnv::default()
        };
        let js::Answer::Strings(flat) = ask("rules", b"", &env, &mut changes) else {
            panic!("strings");
        };
        assert_eq!(
            flat,
            [
                "a",
                "normal",
                "900 100",
                "normal",
                "U+0-10FFFF",
                "normal",
                "auto",
                "unloaded"
            ]
        );
        assert_eq!(ranges_of("-"), []);
        assert_eq!(ranges_of("0-7f,zz,e9-e9"), [(0, 0x7F), (0xE9, 0xE9)]);
    }
}
