//! A document's faces (CSS Fonts 4 sections 4 and 5, gap batch D, D54):
//! the `@font-face` rules and the `FontFace`s a page made, laid over the
//! system's fonts. A family the document names is matched among its own
//! faces first; a face matched before its data has come is asked for,
//! and the next family stands in for it meanwhile (`font-display:
//! swap`, which `auto` is in the reference browser for most of a load).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use super::faces::{self, FaceId, Font, FontRequest, Slant};
use super::{Fallback, FontError, FontStore};

/// The most faces a document may have.
pub const MAX_FONT_FACES: usize = 256;

/// The most bytes a face's data may have, as fetched or decoded.
pub const MAX_FONT_BYTES: usize = 32 << 20;

/// What a face is said to be (its descriptors): the family it is of, and
/// the weights, slant and widths it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceRule {
    pub family: String,
    /// The weights, the least and the most, 1 to 1000.
    pub weight: (u16, u16),
    pub slant: Slant,
    /// The widths, the least and the most, as percentages.
    pub stretch: (f32, f32),
}

impl Default for FaceRule {
    fn default() -> Self {
        FaceRule {
            family: String::new(),
            weight: (400, 400),
            slant: Slant::Normal,
            stretch: (100.0, 100.0),
        }
    }
}

/// Where a face's data is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceState {
    /// Not asked for.
    Unloaded,
    /// Asked for by matching, not yet fetched: [`DocumentFaces::wanted`]
    /// hands it out.
    Wanted,
    Loading,
    Loaded,
    /// Its data could not be had or is no font.
    Failed,
}

/// One face of a document's.
struct WebFace {
    rule: FaceRule,
    state: FaceState,
    /// The face once its data came, and the number it was lent.
    data: Option<(Arc<Fallback>, FaceId)>,
}

/// A document's faces, in the order the document gave them.
pub struct DocumentFaces {
    faces: Mutex<Vec<WebFace>>,
    /// The fonts chosen, by what they were asked for; forgotten when a
    /// face comes or goes.
    chosen: RwLock<HashMap<faces::Key, Font>>,
}

impl DocumentFaces {
    /// The faces `rules` say, none loaded; past [`MAX_FONT_FACES`] the
    /// rest are left out.
    pub fn new(rules: Vec<FaceRule>) -> Self {
        DocumentFaces {
            faces: Mutex::new(
                rules
                    .into_iter()
                    .take(MAX_FONT_FACES)
                    .map(|rule| WebFace {
                        rule,
                        state: FaceState::Unloaded,
                        data: None,
                    })
                    .collect(),
            ),
            chosen: RwLock::new(HashMap::new()),
        }
    }

    /// How many faces it has.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether it has none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds a face, its data to come; its number, or `None` past
    /// [`MAX_FONT_FACES`].
    pub fn push(&self, rule: FaceRule) -> Option<usize> {
        let mut faces = self.lock();
        if faces.len() >= MAX_FONT_FACES {
            return None;
        }
        faces.push(WebFace {
            rule,
            state: FaceState::Unloaded,
            data: None,
        });
        self.forget();
        Some(faces.len() - 1)
    }

    /// Face `index`'s state.
    pub fn state(&self, index: usize) -> Option<FaceState> {
        self.lock().get(index).map(|face| face.state)
    }

    /// Face `index`'s descriptors.
    pub fn rule(&self, index: usize) -> Option<FaceRule> {
        self.lock().get(index).map(|face| face.rule.clone())
    }

    /// Whether a face is asked for or loading.
    pub fn is_loading(&self) -> bool {
        self.lock()
            .iter()
            .any(|face| matches!(face.state, FaceState::Wanted | FaceState::Loading))
    }

    /// The faces matching asked for since last asked, now loading.
    pub fn wanted(&self) -> Vec<usize> {
        let mut faces = self.lock();
        let mut out = Vec::new();
        for (n, face) in faces.iter_mut().enumerate() {
            if face.state == FaceState::Wanted {
                face.state = FaceState::Loading;
                out.push(n);
            }
        }
        out
    }

    /// Marks face `index` as loading, a script having asked for it.
    pub fn loading(&self, index: usize) {
        if let Some(face) = self.lock().get_mut(index)
            && matches!(face.state, FaceState::Unloaded | FaceState::Wanted)
        {
            face.state = FaceState::Loading;
        }
    }

    /// Face `index` could not be had.
    pub fn failed(&self, index: usize) {
        if let Some(face) = self.lock().get_mut(index) {
            face.state = FaceState::Failed;
            face.data = None;
        }
        self.forget();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<WebFace>> {
        self.faces
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn forget(&self) {
        if let Ok(mut chosen) = self.chosen.write() {
            chosen.clear();
        }
    }

    /// The document's face of `family` for `request`: `Some(Some(font))`
    /// for a face had, `Some(None)` when the family is the document's
    /// but its face is not had yet (asked for), `None` for a family the
    /// document has no face of.
    fn family_font(&self, family: &str, request: &FontRequest<'_>) -> Option<Option<Font>> {
        let mut faces = self.lock();
        let of_family: Vec<usize> = (0..faces.len())
            .filter(|&n| {
                faces[n].state != FaceState::Failed
                    && faces[n].rule.family.eq_ignore_ascii_case(family)
            })
            .collect();
        if of_family.is_empty() {
            return None;
        }
        let best = best_match(&of_family, |n| &faces[n].rule, request)?;
        let face = &mut faces[best];
        match &face.data {
            Some((_, id)) => {
                let rule = &face.rule;
                let italic = rule.slant != Slant::Normal;
                Some(Some(faces::synthesized(
                    *id,
                    request,
                    rule.weight.1,
                    italic,
                )))
            }
            None => {
                if face.state == FaceState::Unloaded {
                    face.state = FaceState::Wanted;
                }
                Some(None)
            }
        }
    }
}

/// The best of `candidates` for `request` by CSS Fonts 4's matching
/// (section 5.2, step 4): the width, then the slant, then the weight.
fn best_match<'r>(
    candidates: &[usize],
    rule: impl Fn(usize) -> &'r FaceRule,
    request: &FontRequest<'_>,
) -> Option<usize> {
    let stretch = |n: usize| {
        let (lo, hi) = rule(n).stretch;
        distance(request.stretch, lo, hi, request.stretch <= 100.0)
    };
    let best_stretch = candidates
        .iter()
        .map(|&n| stretch(n))
        .fold(f32::INFINITY, f32::min);
    let left: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&n| stretch(n) == best_stretch)
        .collect();
    // The slants in the order each asks them.
    let order: [u8; 3] = match request.slant {
        Slant::Italic => [1, 2, 0],
        Slant::Oblique(_) => [2, 1, 0],
        Slant::Normal => [0, 2, 1],
    };
    let kind = |slant: Slant| match slant {
        Slant::Normal => 0,
        Slant::Italic => 1,
        Slant::Oblique(_) => 2,
    };
    let left: Vec<usize> = order
        .iter()
        .map(|&k| {
            left.iter()
                .copied()
                .filter(|&n| kind(rule(n).slant) == k)
                .collect::<Vec<_>>()
        })
        .find(|found| !found.is_empty())?;
    let w = f32::from(request.weight);
    let weight = |n: usize| {
        let (lo, hi) = rule(n).weight;
        let (lo, hi) = (f32::from(lo), f32::from(hi));
        if (lo..=hi).contains(&w) {
            return 0.0;
        }
        // CSS Fonts 4: from 400 to 500, heavier up to 500 first, then
        // lighter, then heavier; under 400 lighter first; over 500
        // heavier first.
        if (400.0..=500.0).contains(&w) {
            if lo > w && lo <= 500.0 {
                lo - w
            } else if hi < w {
                1000.0 + (w - hi)
            } else {
                2000.0 + (lo - w)
            }
        } else {
            distance(w, lo, hi, w < 400.0)
        }
    };
    left.iter()
        .copied()
        .min_by(|&a, &b| weight(a).total_cmp(&weight(b)))
}

/// How far `value` is from the range `lo` to `hi`, those below it first
/// when `below_first`, then those above.
fn distance(value: f32, lo: f32, hi: f32, below_first: bool) -> f32 {
    if (lo..=hi).contains(&value) {
        0.0
    } else if hi < value {
        (value - hi) + if below_first { 0.0 } else { 100_000.0 }
    } else {
        (lo - value) + if below_first { 100_000.0 } else { 0.0 }
    }
}

/// Whether `data` is a font file this crate reads: an sfnt, or WOFF 1.
pub fn is_font(data: &[u8]) -> bool {
    super::woff::sfnt(data.to_vec()).is_ok_and(|data| FontStore::from_bytes(data, 0).is_ok())
}

impl super::FontCollection {
    /// This collection with `document`'s faces over the system's: what
    /// a document's layout measures with and its display list is drawn
    /// in. Shares everything else with this collection.
    pub fn with_document(&self, document: Arc<DocumentFaces>) -> Self {
        let mut fonts = self.clone();
        fonts.document = Some(document);
        fonts
    }

    /// The document's faces, when it has its own.
    pub fn document(&self) -> Option<&Arc<DocumentFaces>> {
        self.document.as_ref()
    }

    /// Face `index` of the document's has come: `data`, a font file, or
    /// a WOFF 1 one. Says whether it is a font; a face that is not is
    /// failed.
    pub fn face_arrived(&self, index: usize, data: Vec<u8>) -> Result<(), FontError> {
        let Some(document) = &self.document else {
            return Err(FontError::NoFontFound);
        };
        let store = super::woff::sfnt(data).and_then(|data| FontStore::from_bytes(data, 0));
        let store = match store {
            Ok(store) => store,
            Err(error) => {
                document.failed(index);
                return Err(error);
            }
        };
        let face = Arc::new(Fallback {
            path: std::path::PathBuf::new(),
            index: 0,
            font: std::sync::OnceLock::from(Some(store)),
        });
        let id = self.faces.lend(&face);
        if let Some(web) = document.lock().get_mut(index) {
            web.state = FaceState::Loaded;
            web.data = Some((face, id));
        }
        document.forget();
        Ok(())
    }

    /// The font for `request` with the document's faces first.
    pub(crate) fn document_font(
        &self,
        document: &DocumentFaces,
        request: &FontRequest<'_>,
    ) -> Font {
        let slant = match request.slant {
            Slant::Normal => 0,
            Slant::Italic => 1,
            Slant::Oblique(a) => 2 | (a.to_bits() & !3),
        };
        let key = (
            Box::<str>::from(request.families),
            request.weight,
            slant,
            request.stretch.to_bits(),
        );
        if let Ok(chosen) = document.chosen.read()
            && let Some(font) = chosen.get(&key)
        {
            return *font;
        }
        let mut font = None;
        for family in faces::families(request.families) {
            font = match document.family_font(family, request) {
                Some(Some(font)) => Some(font),
                // Not had yet: the next family stands in.
                Some(None) => None,
                None => self.faces.family_font(family, request),
            };
            if font.is_some() {
                break;
            }
        }
        let font = font.unwrap_or_else(|| self.faces.fallback_font(request));
        if let Ok(mut chosen) = document.chosen.write() {
            if chosen.len() >= faces::MAX_CHOICES {
                chosen.clear();
            }
            chosen.insert(key, font);
        }
        font
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FontCollection;

    fn rule(family: &str, weight: (u16, u16), slant: Slant) -> FaceRule {
        FaceRule {
            family: family.into(),
            weight,
            slant,
            ..FaceRule::default()
        }
    }

    fn request(families: &str, weight: u16, slant: Slant) -> FontRequest<'_> {
        FontRequest {
            families,
            weight,
            slant,
            stretch: 100.0,
        }
    }

    #[test]
    fn a_face_is_asked_for_when_matched_and_stood_in_for_until_it_comes() {
        let fonts = FontCollection::from_bytes(crate::TEST_FONT.to_vec(), 0).expect("parses");
        let document = Arc::new(DocumentFaces::new(vec![
            rule("Web", (400, 400), Slant::Normal),
            rule("Web", (700, 700), Slant::Normal),
        ]));
        let fonts = fonts.with_document(document.clone());
        let regular = request("Web, serif", 400, Slant::Normal);
        assert_eq!(fonts.font(&regular), Font::default());
        assert_eq!(document.wanted(), [0]);
        assert_eq!(document.wanted(), Vec::<usize>::new(), "handed out once");
        assert_eq!(document.state(0), Some(FaceState::Loading));
        fonts
            .face_arrived(0, crate::TEST_FONT.to_vec())
            .expect("a font");
        // The choice made while it was away is made again.
        let web = fonts.font(&regular);
        assert_ne!(web.face, FaceId(0));
        assert_eq!(fonts.font(&request("web, serif", 400, Slant::Normal)), web);
        assert!(fonts.face(web.face).measure("a", 16.0) > 0.0);
        // Bold asks for the other face; not yet had, the regular one is
        // not taken in its stead: the next family is.
        let bold = fonts.font(&request("Web, serif", 700, Slant::Normal));
        assert_eq!(bold.face, FaceId(0));
        assert_eq!(document.wanted(), [1]);
        // What is no font fails, and is passed over.
        assert!(fonts.face_arrived(1, b"not a font".to_vec()).is_err());
        assert_eq!(document.state(1), Some(FaceState::Failed));
        let bold = fonts.font(&request("Web", 700, Slant::Normal));
        assert_eq!((bold.face, bold.bold), (web.face, true));
        // The registry lends the face: gone with the document.
        let id = web.face;
        drop(fonts);
        drop(document);
        let base = FontCollection::from_bytes(crate::TEST_FONT.to_vec(), 0).expect("parses");
        assert!(base.faces.loaded(id).is_none());
    }

    #[test]
    fn a_family_of_the_documents_hides_the_systems_while_its_face_comes() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf");
        let files = crate::FontFiles {
            primary: (path.to_owned(), 0),
            fallbacks: Vec::new(),
            faces: vec![crate::FaceRecord {
                families: vec!["Shared".into()],
                path: "elsewhere.ttf".into(),
                index: 0,
                weight: 400,
                style: 0,
                stretch: 5,
            }],
        };
        let system = FontCollection::from_files(&files).expect("read");
        assert_ne!(
            system.font(&request("Shared", 400, Slant::Normal)).face,
            FaceId(0)
        );
        let document = Arc::new(DocumentFaces::new(vec![rule(
            "Shared",
            (400, 400),
            Slant::Normal,
        )]));
        let fonts = system.with_document(document);
        assert_eq!(
            fonts.font(&request("Shared", 400, Slant::Normal)).face,
            FaceId(0)
        );
    }

    #[test]
    fn faces_match_by_width_then_slant_then_weight() {
        let rules = [
            rule("a", (100, 100), Slant::Normal),
            rule("a", (300, 300), Slant::Normal),
            rule("a", (450, 450), Slant::Normal),
            rule("a", (600, 900), Slant::Normal),
            rule("a", (400, 400), Slant::Italic),
            rule("a", (400, 400), Slant::Oblique(10.0)),
        ];
        let pick = |weight: u16, slant: Slant| {
            let all: Vec<usize> = (0..rules.len()).collect();
            best_match(&all, |n| &rules[n], &request("a", weight, slant))
        };
        assert_eq!(pick(450, Slant::Normal), Some(2));
        assert_eq!(pick(400, Slant::Normal), Some(2), "up to 500 first");
        assert_eq!(pick(500, Slant::Normal), Some(2), "then lighter");
        assert_eq!(pick(200, Slant::Normal), Some(0), "lighter first under 400");
        assert_eq!(pick(550, Slant::Normal), Some(3), "heavier first over 500");
        assert_eq!(pick(1000, Slant::Normal), Some(3));
        assert_eq!(pick(400, Slant::Italic), Some(4));
        assert_eq!(pick(400, Slant::Oblique(5.0)), Some(5));
        let wide = [
            FaceRule {
                stretch: (50.0, 75.0),
                ..rule("a", (400, 400), Slant::Normal)
            },
            FaceRule {
                stretch: (125.0, 150.0),
                ..rule("a", (400, 400), Slant::Normal)
            },
        ];
        let width = |stretch: f32| {
            best_match(
                &[0, 1],
                |n| &wide[n],
                &FontRequest {
                    stretch,
                    ..request("a", 400, Slant::Normal)
                },
            )
        };
        assert_eq!(width(100.0), Some(0), "narrower first at normal");
        assert_eq!(width(110.0), Some(1), "wider first past normal");
    }
}
