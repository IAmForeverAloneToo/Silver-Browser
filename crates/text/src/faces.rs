//! Choosing a face by the font properties (CSS Fonts 4 section 5, gap
//! batch D, D53): the families in order, each by fontdb's matching with
//! the weight, style and stretch, the generic families mapped to the
//! system's, then the primary font's family. A face chosen is known by a
//! [`FaceId`], a number its collection hands out once and keeps, shared
//! by the collection's clones; the primary font is face 0. Where the
//! face is lighter or more upright than asked for, the font says what
//! drawing makes up: an emboldening and a slant, as the reference
//! browser draws them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use super::{Fallback, PRIMARY_FAMILIES, file_of};

/// A face a collection has chosen: 0 is its primary font.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceId(pub u32);

/// The slant asked for (`font-style`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Slant {
    #[default]
    Normal,
    Italic,
    /// By the angle, in degrees.
    Oblique(f32),
}

/// The slant a document's face covers (its `font-style` descriptor):
/// italic, or the oblique angles from the least to the most, in degrees,
/// upright being 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaceSlant {
    Italic,
    Oblique(f32, f32),
}

impl FaceSlant {
    /// Upright.
    pub const NORMAL: FaceSlant = FaceSlant::Oblique(0.0, 0.0);
}

/// The font properties a face is chosen by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontRequest<'a> {
    /// The family list, the names separated by commas.
    pub families: &'a str,
    /// 1 to 1000.
    pub weight: u16,
    pub slant: Slant,
    /// A percentage of the normal width.
    pub stretch: f32,
    /// Whether the font's kerning is used (`font-kerning` but `none`).
    pub kerning: bool,
    /// The features set or cleared (`font-feature-settings`): each
    /// one's tag and value.
    pub features: &'a [([u8; 4], u32)],
    /// The text's language, a BCP 47 tag; empty for none.
    pub language: &'a str,
}

impl Default for FontRequest<'_> {
    fn default() -> Self {
        FontRequest {
            families: "",
            weight: 400,
            slant: Slant::Normal,
            stretch: 100.0,
            kerning: true,
            features: &[],
            language: "",
        }
    }
}

impl FontRequest<'_> {
    /// Whether it shapes as the font does by itself.
    pub(crate) fn plain_shaping(&self) -> bool {
        self.kerning && self.features.is_empty() && self.language.is_empty()
    }
}

/// How a font's text is shaped past the font itself: the features set or
/// cleared, in order, and the language.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ShapeOptions {
    pub(crate) features: Vec<rustybuzz::Feature>,
    pub(crate) language: Option<rustybuzz::Language>,
}

impl ShapeOptions {
    /// The options `request` asks for: `kern` cleared for no kerning,
    /// then the features it sets, as the reference browser orders them,
    /// a later one of a tag winning.
    fn of(request: &FontRequest<'_>) -> Self {
        let feature = |tag: [u8; 4], value: u32| {
            rustybuzz::Feature::new(ttf_parser::Tag::from_bytes(&tag), value, ..)
        };
        let mut features = Vec::new();
        if !request.kerning {
            features.push(feature(*b"kern", 0));
        }
        features.extend(request.features.iter().map(|&(tag, v)| feature(tag, v)));
        ShapeOptions {
            features,
            language: Some(request.language)
                .filter(|l| !l.is_empty())
                .and_then(|l| l.parse().ok()),
        }
    }
}

/// The most shaping options a registry keeps; past it a font shapes as
/// the font does by itself.
pub const MAX_SHAPINGS: usize = 4096;

/// A face chosen, with what drawing makes up for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Font {
    pub face: FaceId,
    /// Emboldened: bold was asked for and the face is not.
    pub bold: bool,
    /// Slanted by [`SYNTHETIC_SKEW`]: a slant was asked for with an
    /// upright face.
    pub slanted: bool,
    /// The faces a document's `unicode-range`s put before `face` for
    /// the code points they cover, and how its text is shaped past the
    /// face (its kerning, features and language), by the number the
    /// collection's registry gave them; 0 for none of either.
    pub chain: u16,
}

/// One face of a chain ([`Font::chain`]): a document's face for the
/// code points in its ranges, or the number of one of its faces not had
/// yet, which a text needing those code points asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Link {
    pub(crate) font: Option<Font>,
    pub(crate) pending: usize,
    pub(crate) ranges: Arc<[(u32, u32)]>,
}

impl Link {
    /// Whether the link's ranges hold `c`.
    pub(crate) fn covers(&self, c: char) -> bool {
        let c = u32::from(c);
        self.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
    }
}

/// The most chains a registry keeps; past it a font has none, its
/// ranged faces passed over.
pub const MAX_CHAINS: usize = 4096;

impl Font {
    /// The horizontal skew drawing makes up: x moves right by this much
    /// of the height above the baseline.
    pub fn skew(self) -> f32 {
        if self.slanted { SYNTHETIC_SKEW } else { 0.0 }
    }
}

/// The skew of a made-up slant, the reference browser's: a quarter,
/// some 14 degrees.
pub const SYNTHETIC_SKEW: f32 = 0.25;

/// The weight from which a face is bold.
const BOLD: u16 = 600;

/// A face of the system's as matching reads it, what a process that
/// scanned the system's fonts gives another with their files
/// ([`super::FontFiles`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceRecord {
    pub families: Vec<String>,
    pub path: String,
    pub index: u32,
    pub weight: u16,
    /// 0 upright, 1 italic, 2 oblique.
    pub style: u8,
    /// 1 (ultra-condensed) to 9 (ultra-expanded).
    pub stretch: u8,
}

/// The most choices a collection keeps; past it they are forgotten and
/// made again.
pub const MAX_CHOICES: usize = 4096;

/// The faces a collection chooses from and has chosen, shared by its
/// clones.
pub(crate) struct Faces {
    /// The system's faces; none for a collection of one font.
    db: Option<fontdb::Database>,
    /// Each family's name as the database has it, by its name in lower
    /// case: CSS's names match whatever their case.
    names: HashMap<String, String>,
    /// The primary font's file and face index.
    primary: Option<(PathBuf, u32)>,
    /// The primary font's weight and whether it is slanted.
    primary_style: (u16, bool),
    /// The faces handed out past the primary font: face `n` is the
    /// `n - 1`th. The system's are kept; a document's are lent, and gone
    /// with the document.
    loaded: RwLock<Vec<Slot>>,
    /// The fonts chosen, by what they were asked for.
    chosen: Choices,
    /// The chains handed out ([`Font::chain`]): chain `n` is the
    /// `n - 1`th, each kept once.
    chains: RwLock<Chains>,
    /// The shaping options handed out ([`Font::shaping`]), as chains
    /// are, by the request's kerning, features and language.
    shapings: RwLock<Shapings>,
    /// The face `monospace` is, in a collection of bundled faces with
    /// no system's: the tests' fixed-width font.
    monospace: Option<FaceId>,
}

/// The shaping options handed out, in order, and each one's number by
/// what was asked.
type Shapings = (
    Vec<Arc<ShapeOptions>>,
    HashMap<(bool, Vec<([u8; 4], u32)>, String), u16>,
);

/// The chains handed out, in order, and each one's number by its links
/// and the number of its shaping options.
type Chains = (Vec<(Arc<[Link]>, u16)>, HashMap<(Arc<[Link]>, u16), u16>);

/// What a choice is kept by: the families, weight, slant and stretch.
type Key = (Box<str>, u16, u32, u32);

/// The key of `request`, its families still borrowed.
fn key_of<'a>(request: &FontRequest<'a>) -> (&'a str, u16, u32, u32) {
    let slant = match request.slant {
        Slant::Normal => 0,
        Slant::Italic => 1,
        Slant::Oblique(a) => 2 | (a.to_bits() & !3),
    };
    (
        request.families,
        request.weight,
        slant,
        request.stretch.to_bits(),
    )
}

/// Fonts chosen, by what they were asked for, up to [`MAX_CHOICES`].
/// Layout asks for the same font for word after word, so the last
/// choice each thread was given is kept apart, and asking for it again
/// is a comparison rather than a lookup; a number each map has, and
/// one counted up whenever it forgets, say whether that choice is
/// still the map's.
pub(crate) struct Choices {
    id: u64,
    forgotten: AtomicU64,
    chosen: RwLock<HashMap<Key, Font>>,
}

thread_local! {
    /// The last choice this thread was given: the map's number and how
    /// often it had forgotten, the key and the font.
    static LAST: RefCell<Option<(u64, u64, Key, Font)>> = const { RefCell::new(None) };
}

impl Choices {
    pub(crate) fn new() -> Self {
        static IDS: AtomicU64 = AtomicU64::new(0);
        Choices {
            id: IDS.fetch_add(1, Ordering::Relaxed),
            forgotten: AtomicU64::new(0),
            chosen: RwLock::new(HashMap::new()),
        }
    }

    /// The font chosen for `request` before, if any.
    pub(crate) fn get(&self, request: &FontRequest<'_>) -> Option<Font> {
        let (families, weight, slant, stretch) = key_of(request);
        let forgotten = self.forgotten.load(Ordering::Acquire);
        let last = LAST.with_borrow(|last| match last {
            Some((id, when, (f, w, s, t), font))
                if *id == self.id
                    && *when == forgotten
                    && (*w, *s, *t) == (weight, slant, stretch)
                    && **f == *families =>
            {
                Some(*font)
            }
            _ => None,
        });
        if last.is_some() {
            return last;
        }
        let key: Key = (families.into(), weight, slant, stretch);
        let font = *self.chosen.read().ok()?.get(&key)?;
        LAST.set(Some((self.id, forgotten, key, font)));
        Some(font)
    }

    /// Keeps `font` as `request`'s, forgetting all the others first
    /// when there are [`MAX_CHOICES`]: they would be chosen the same
    /// again, so the threads' last choices stand.
    pub(crate) fn keep(&self, request: &FontRequest<'_>, font: Font) {
        let (families, weight, slant, stretch) = key_of(request);
        if let Ok(mut chosen) = self.chosen.write() {
            if chosen.len() >= MAX_CHOICES {
                chosen.clear();
            }
            chosen.insert((families.into(), weight, slant, stretch), font);
        }
    }

    /// Forgets every choice.
    pub(crate) fn forget(&self) {
        if let Ok(mut chosen) = self.chosen.write() {
            chosen.clear();
            self.forgotten.fetch_add(1, Ordering::Release);
        }
    }
}

impl Faces {
    /// The faces of `db`, the primary font in `primary`, whose weight and
    /// slant are `primary_style`.
    pub(crate) fn new(
        mut db: Option<fontdb::Database>,
        primary: Option<(PathBuf, u32)>,
        primary_style: (u16, bool),
    ) -> Self {
        let mut names = HashMap::new();
        if let Some(db) = db.as_mut() {
            for face in db.faces() {
                for (name, _) in &face.families {
                    names
                        .entry(name.to_lowercase())
                        .or_insert_with(|| name.clone());
                }
            }
            // The reference browser's fixed-width font, where there is
            // one.
            if names.contains_key("consolas") {
                db.set_monospace_family("Consolas");
            }
        }
        Faces {
            db,
            names,
            primary,
            primary_style,
            loaded: RwLock::new(Vec::new()),
            chosen: Choices::new(),
            chains: RwLock::new((Vec::new(), HashMap::new())),
            shapings: RwLock::new((Vec::new(), HashMap::new())),
            monospace: None,
        }
    }

    /// The number of the shaping options `request` asks for, handed out
    /// once; 0 for none, and past [`MAX_SHAPINGS`].
    pub(crate) fn shaping(&self, request: &FontRequest<'_>) -> u16 {
        if request.plain_shaping() {
            return 0;
        }
        let key = (
            request.kerning,
            request.features.to_vec(),
            request.language.to_owned(),
        );
        if let Ok(kept) = self.shapings.read()
            && let Some(&n) = kept.1.get(&key)
        {
            return n;
        }
        let Ok(mut kept) = self.shapings.write() else {
            return 0;
        };
        if let Some(&n) = kept.1.get(&key) {
            return n;
        }
        if kept.0.len() >= MAX_SHAPINGS {
            return 0;
        }
        kept.0.push(Arc::new(ShapeOptions::of(request)));
        let n = kept.0.len() as u16;
        kept.1.insert(key, n);
        n
    }

    /// The shaping options numbered `n`, none for 0 or a number never
    /// handed out.
    pub(crate) fn shaping_options(&self, n: u16) -> Option<Arc<ShapeOptions>> {
        let i = n.checked_sub(1)? as usize;
        self.shapings.read().ok()?.0.get(i).cloned()
    }

    /// Keeps `face` as the face of `monospace` and `ui-monospace`, where
    /// the collection has no system's faces.
    pub(crate) fn keep_monospace(&mut self, face: super::FontStore) {
        let slots = self.loaded.get_mut().unwrap_or_else(|e| e.into_inner());
        slots.push(Slot::Kept(Arc::new(Fallback {
            path: PathBuf::new(),
            index: 0,
            font: std::sync::OnceLock::from(Some(face)),
        })));
        self.monospace = Some(FaceId(slots.len() as u32));
    }

    /// The number of the chain `links` shaped by the options numbered
    /// `shaping`, handed out once; 0 for neither, and past
    /// [`MAX_CHAINS`].
    pub(crate) fn chain(&self, links: Arc<[Link]>, shaping: u16) -> u16 {
        if links.is_empty() && shaping == 0 {
            return 0;
        }
        let key = (links, shaping);
        if let Ok(chains) = self.chains.read()
            && let Some(&n) = chains.1.get(&key)
        {
            return n;
        }
        let Ok(mut chains) = self.chains.write() else {
            return 0;
        };
        if let Some(&n) = chains.1.get(&key) {
            return n;
        }
        if chains.0.len() >= MAX_CHAINS {
            return 0;
        }
        chains.0.push(key.clone());
        let n = chains.0.len() as u16;
        chains.1.insert(key, n);
        n
    }

    /// The number of chain `n`'s shaping options; 0 for none.
    pub(crate) fn chain_shaping(&self, n: u16) -> u16 {
        let Some(i) = n.checked_sub(1) else {
            return 0;
        };
        self.chains
            .read()
            .ok()
            .and_then(|chains| chains.0.get(usize::from(i)).map(|(_, s)| *s))
            .unwrap_or(0)
    }

    /// The system's faces, when the collection has them.
    pub(crate) fn database(&self) -> Option<&fontdb::Database> {
        self.db.as_ref()
    }

    /// The links of chain `n`, none for 0, a number never handed out
    /// and a chain of shaping options alone.
    pub(crate) fn links(&self, n: u16) -> Option<Arc<[Link]>> {
        let i = n.checked_sub(1)? as usize;
        let links = self.chains.read().ok()?.0.get(i)?.0.clone();
        (!links.is_empty()).then_some(links)
    }

    /// The faces of the system's fonts in `records`, read as asked for.
    pub(crate) fn database_of(records: &[FaceRecord]) -> fontdb::Database {
        let mut db = fontdb::Database::new();
        for record in records {
            db.push_face_info(fontdb::FaceInfo {
                id: fontdb::ID::dummy(),
                source: fontdb::Source::File(PathBuf::from(&record.path)),
                index: record.index,
                families: record
                    .families
                    .iter()
                    .map(|name| (name.clone(), fontdb::Language::English_UnitedStates))
                    .collect(),
                post_script_name: String::new(),
                style: match record.style {
                    1 => fontdb::Style::Italic,
                    2 => fontdb::Style::Oblique,
                    _ => fontdb::Style::Normal,
                },
                weight: fontdb::Weight(record.weight),
                stretch: stretch_of_number(record.stretch),
                monospaced: false,
            });
        }
        db
    }

    /// The records of the database's faces that are in files named in
    /// Unicode.
    pub(crate) fn records(&self) -> Vec<FaceRecord> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        db.faces()
            .filter_map(|face| {
                let (path, index) = file_of(db, face.id)?;
                Some(FaceRecord {
                    families: face.families.iter().map(|(name, _)| name.clone()).collect(),
                    path: path.to_str()?.to_owned(),
                    index,
                    weight: face.weight.0,
                    style: match face.style {
                        fontdb::Style::Normal => 0,
                        fontdb::Style::Italic => 1,
                        fontdb::Style::Oblique => 2,
                    },
                    stretch: face.stretch.to_number() as u8,
                })
            })
            .collect()
    }

    /// The font for `request`: the one chosen before, or one chosen and
    /// kept.
    pub(crate) fn choose(&self, request: &FontRequest<'_>) -> Font {
        if let Some(font) = self.chosen.get(request) {
            return font;
        }
        let font = self.matched(request);
        self.chosen.keep(request, font);
        font
    }

    /// The font CSS's matching gives `request`: the first of its
    /// families the system has, else the primary font's family.
    fn matched(&self, request: &FontRequest<'_>) -> Font {
        families(request.families)
            .find_map(|family| self.family_font(&family, request))
            .unwrap_or_else(|| self.fallback_font(request))
    }

    /// The system's face of `family` that best matches `request`, when
    /// it has the family.
    pub(crate) fn family_font(
        &self,
        family: &Family<'_>,
        request: &FontRequest<'_>,
    ) -> Option<Font> {
        if let (Family::Generic("monospace" | "ui-monospace"), Some(face), None) =
            (family, self.monospace, &self.db)
        {
            return Some(synthesized(face, request, 400, false));
        }
        let db = self.db.as_ref()?;
        let id = match family {
            Family::Generic("sans-serif" | "system-ui" | "ui-sans-serif") => {
                query(db, &primary_families(), request)
            }
            Family::Generic("serif" | "ui-serif") => query(db, &[fontdb::Family::Serif], request),
            Family::Generic("monospace" | "ui-monospace") => {
                query(db, &[fontdb::Family::Monospace], request)
            }
            Family::Generic("cursive") => query(db, &[fontdb::Family::Cursive], request),
            Family::Generic("fantasy") => query(db, &[fontdb::Family::Fantasy], request),
            Family::Generic(_) => None,
            Family::Name(name) => {
                let lower = name.to_lowercase();
                self.names
                    .get(&lower)
                    .or_else(|| alternate_family(&lower).and_then(|other| self.names.get(other)))
                    .and_then(|name| query(db, &[fontdb::Family::Name(name)], request))
            }
        }?;
        Some(self.font_of_id(db, id, request))
    }

    /// The font for `request` when none of its families is had: the
    /// primary font's family's best face.
    pub(crate) fn fallback_font(&self, request: &FontRequest<'_>) -> Font {
        let (weight, italic) = self.primary_style;
        let Some(db) = &self.db else {
            return synthesized(FaceId(0), request, weight, italic);
        };
        match query(db, &primary_families(), request) {
            Some(id) => self.font_of_id(db, id, request),
            None => synthesized(FaceId(0), request, weight, italic),
        }
    }

    /// The font of the database's face `id` for `request`.
    fn font_of_id(&self, db: &fontdb::Database, id: fontdb::ID, request: &FontRequest<'_>) -> Font {
        let Some(info) = db.face(id) else {
            return Font::default();
        };
        let (weight, italic) = (info.weight.0, info.style != fontdb::Style::Normal);
        let face = match file_of(db, id) {
            Some(file) if Some(&file) == self.primary.as_ref() => FaceId(0),
            Some((path, index)) => self.handed_out(path, index),
            None => FaceId(0),
        };
        synthesized(face, request, weight, italic)
    }

    /// The face in `path` at `index`: the number it was handed out with,
    /// or a new one.
    pub(crate) fn handed_out(&self, path: PathBuf, index: u32) -> FaceId {
        let number = |faces: &[Slot]| {
            faces
                .iter()
                .position(|slot| {
                    matches!(slot, Slot::Kept(face) if face.path == path && face.index == index)
                })
                .map(|n| FaceId(n as u32 + 1))
        };
        if let Ok(loaded) = self.loaded.read()
            && let Some(id) = number(&loaded)
        {
            return id;
        }
        let Ok(mut loaded) = self.loaded.write() else {
            return FaceId(0);
        };
        if let Some(id) = number(&loaded) {
            return id;
        }
        loaded.push(Slot::Kept(Arc::new(Fallback {
            path,
            index,
            font: std::sync::OnceLock::new(),
        })));
        FaceId(loaded.len() as u32)
    }

    /// A number for `face`, a document's: the registry does not keep it.
    pub(crate) fn lend(&self, face: &Arc<Fallback>) -> FaceId {
        let Ok(mut loaded) = self.loaded.write() else {
            return FaceId(0);
        };
        loaded.push(Slot::Lent(Arc::downgrade(face)));
        FaceId(loaded.len() as u32)
    }

    /// The number `face` was handed out with; 0 for one never handed
    /// out.
    pub(crate) fn id_of(&self, face: &Arc<Fallback>) -> FaceId {
        self.loaded
            .read()
            .ok()
            .and_then(|loaded| {
                loaded.iter().position(|slot| match slot {
                    Slot::Kept(f) => Arc::ptr_eq(f, face),
                    Slot::Lent(f) => std::ptr::eq(f.as_ptr(), Arc::as_ptr(face)),
                })
            })
            .map_or(FaceId(0), |n| FaceId(n as u32 + 1))
    }

    /// The face handed out as `id`, when there is one past the primary
    /// font and it is still had.
    pub(crate) fn loaded(&self, id: FaceId) -> Option<Arc<Fallback>> {
        let n = id.0.checked_sub(1)? as usize;
        match self.loaded.read().ok()?.get(n)? {
            Slot::Kept(face) => Some(face.clone()),
            Slot::Lent(face) => face.upgrade(),
        }
    }
}

/// A face handed out: kept by the registry, or lent by a document.
enum Slot {
    Kept(Arc<Fallback>),
    Lent(std::sync::Weak<Fallback>),
}

/// One entry of a family list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Family<'a> {
    /// A generic family's keyword, lowercase.
    Generic(&'a str),
    /// A family's name, unquoted.
    Name(std::borrow::Cow<'a, str>),
}

/// The generic families' keywords: a name written as one unquoted is
/// the generic family, quoted it is a name.
const GENERIC: [&str; 11] = [
    "serif",
    "sans-serif",
    "cursive",
    "fantasy",
    "monospace",
    "system-ui",
    "math",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
];

/// The entries of a family list as the style crate writes it, in
/// order: names separated by commas, each a generic keyword, words, or
/// a string in double quotes with backslash escapes.
pub(crate) fn families(list: &str) -> impl Iterator<Item = Family<'_>> {
    let mut rest = list;
    std::iter::from_fn(move || {
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                return None;
            }
            if let Some(quoted) = rest.strip_prefix('"') {
                let (name, after) = unquote(quoted);
                rest = after
                    .trim_start()
                    .strip_prefix(',')
                    .unwrap_or_else(|| after.trim_start());
                return Some(Family::Name(name.into()));
            }
            let end = rest.find(',').unwrap_or(rest.len());
            let word = rest[..end].trim();
            rest = rest.get(end + 1..).unwrap_or("");
            if word.is_empty() {
                continue;
            }
            let lower = word.to_ascii_lowercase();
            if GENERIC.contains(&lower.as_str()) || lower.starts_with("generic(") {
                return Some(Family::Generic(
                    GENERIC.iter().find(|g| **g == lower).copied().unwrap_or(""),
                ));
            }
            return Some(Family::Name(word.into()));
        }
    })
}

/// Whether `c` is a private-use code point (Unicode section 23.5).
pub fn is_private_use(c: char) -> bool {
    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// The family list `list` without its generic families, as the style
/// crate writes one: what a private-use code point matches against (CSS
/// Fonts 4 section 5.4); `None` when it names none but generics.
pub fn named_families(list: &str) -> Option<String> {
    let names: Vec<String> = families(list)
        .filter_map(|family| match family {
            Family::Name(name) => Some(format!(
                "\"{}\"",
                name.replace('\\', "\\\\").replace('"', "\\\"")
            )),
            Family::Generic(_) => None,
        })
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// A string's text after its opening quote, unescaped, and what is
/// after its closing quote.
fn unquote(text: &str) -> (String, &str) {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (out, &text[i + 1..]),
            '\\' => {
                let mut hex = String::new();
                let mut next = None;
                for (_, d) in chars.by_ref() {
                    if d.is_ascii_hexdigit() && hex.len() < 6 {
                        hex.push(d);
                        continue;
                    }
                    next = Some(d);
                    break;
                }
                if hex.is_empty() {
                    if let Some(d) = next {
                        out.push(d);
                    }
                } else {
                    let code = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                    out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    match next {
                        Some(' ') | None => {}
                        Some('"') => return (out, chars.as_str()),
                        Some(d) => out.push(d),
                    }
                }
            }
            c => out.push(c),
        }
    }
    (out, "")
}

/// The primary font's families.
fn primary_families() -> Vec<fontdb::Family<'static>> {
    PRIMARY_FAMILIES
        .iter()
        .map(|name| fontdb::Family::Name(name))
        .collect()
}

/// The family the reference browser takes for a family the system does
/// not have, by its lowercase name (Blink's `AlternateFamilyName`):
/// Times and Times New Roman, Courier and Courier New, Helvetica and
/// Arial, each for the other.
fn alternate_family(lower: &str) -> Option<&'static str> {
    Some(match lower {
        "times" => "times new roman",
        "times new roman" => "times",
        "courier" => "courier new",
        "courier new" => "courier",
        "helvetica" => "arial",
        "arial" => "helvetica",
        _ => return None,
    })
}

/// The face of `families` in `db` that best matches `request`.
fn query(
    db: &fontdb::Database,
    families: &[fontdb::Family<'_>],
    request: &FontRequest<'_>,
) -> Option<fontdb::ID> {
    db.query(&fontdb::Query {
        families,
        weight: fontdb::Weight(request.weight),
        style: if request.slant != Slant::Normal {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        },
        stretch: stretch_of_percentage(request.stretch),
    })
}

/// `face`, whose weight is `weight` and which is slanted when `italic`,
/// with what drawing makes up for `request`.
pub(crate) fn synthesized(
    face: FaceId,
    request: &FontRequest<'_>,
    weight: u16,
    italic: bool,
) -> Font {
    Font {
        face,
        bold: request.weight >= BOLD && weight < BOLD,
        slanted: request.slant != Slant::Normal && !italic,
        chain: 0,
    }
}

/// The width class nearest a `font-stretch` percentage.
fn stretch_of_percentage(percentage: f32) -> fontdb::Stretch {
    const CLASSES: [(f32, fontdb::Stretch); 9] = [
        (50.0, fontdb::Stretch::UltraCondensed),
        (62.5, fontdb::Stretch::ExtraCondensed),
        (75.0, fontdb::Stretch::Condensed),
        (87.5, fontdb::Stretch::SemiCondensed),
        (100.0, fontdb::Stretch::Normal),
        (112.5, fontdb::Stretch::SemiExpanded),
        (125.0, fontdb::Stretch::Expanded),
        (150.0, fontdb::Stretch::ExtraExpanded),
        (200.0, fontdb::Stretch::UltraExpanded),
    ];
    let mut best = CLASSES[4];
    for class in CLASSES {
        if (class.0 - percentage).abs() < (best.0 - percentage).abs() {
            best = class;
        }
    }
    best.1
}

/// The width class numbered `n`, 1 to 9.
fn stretch_of_number(n: u8) -> fontdb::Stretch {
    match n {
        1 => fontdb::Stretch::UltraCondensed,
        2 => fontdb::Stretch::ExtraCondensed,
        3 => fontdb::Stretch::Condensed,
        4 => fontdb::Stretch::SemiCondensed,
        6 => fontdb::Stretch::SemiExpanded,
        7 => fontdb::Stretch::Expanded,
        8 => fontdb::Stretch::ExtraExpanded,
        9 => fontdb::Stretch::UltraExpanded,
        _ => fontdb::Stretch::Normal,
    }
}

/// The weight and slant of a face, from its OS/2 table.
pub(crate) fn style_of(data: &[u8], index: u32) -> (u16, bool) {
    match ttf_parser::Face::parse(data, index) {
        Ok(face) => (
            face.weight().to_number(),
            face.is_italic() || face.is_oblique(),
        ),
        Err(_) => (400, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(families: &str, weight: u16, slant: Slant) -> FontRequest<'_> {
        FontRequest {
            families,
            weight,
            slant,
            ..FontRequest::default()
        }
    }

    #[test]
    fn a_collection_of_one_font_makes_up_bold_and_slant() {
        let faces = Faces::new(None, None, (400, false));
        assert_eq!(
            faces.choose(&request("serif", 400, Slant::Normal)),
            Font::default()
        );
        let bold = faces.choose(&request("serif", 700, Slant::Italic));
        assert_eq!(
            bold,
            Font {
                face: FaceId(0),
                bold: true,
                slanted: true,
                chain: 0
            }
        );
        assert!(!faces.choose(&request("a", 599, Slant::Normal)).bold);
        assert!(faces.choose(&request("a", 600, Slant::Normal)).bold);
        // A bold face is not made bolder.
        let bold_face = Faces::new(None, None, (700, true));
        assert_eq!(
            bold_face.choose(&request("a", 900, Slant::Oblique(10.0))),
            Font::default()
        );
    }

    #[test]
    fn widths_are_the_nearest_class() {
        assert_eq!(stretch_of_percentage(100.0), fontdb::Stretch::Normal);
        assert_eq!(stretch_of_percentage(70.0), fontdb::Stretch::Condensed);
        assert_eq!(stretch_of_percentage(10.0), fontdb::Stretch::UltraCondensed);
        assert_eq!(
            stretch_of_percentage(1000.0),
            fontdb::Stretch::UltraExpanded
        );
        for n in 1..=9 {
            assert_eq!(stretch_of_number(n).to_number(), u16::from(n));
        }
    }

    #[test]
    fn faces_are_matched_by_family_weight_and_slant() {
        let record = |family: &str, path: &str, weight: u16, style: u8| FaceRecord {
            families: vec![family.to_owned()],
            path: path.to_owned(),
            index: 0,
            weight,
            style,
            stretch: 5,
        };
        let records = [
            record("Segoe UI", "segoe.ttf", 400, 0),
            record("Segoe UI", "segoeb.ttf", 700, 0),
            record("Times New Roman", "times.ttf", 400, 0),
            record("Times New Roman", "timesi.ttf", 400, 1),
            record("Courier New", "cour.ttf", 400, 0),
        ];
        let db = Faces::database_of(&records);
        let faces = Faces::new(
            Some(db),
            Some((PathBuf::from("segoe.ttf"), 0)),
            (400, false),
        );
        let pick = |families: &str, weight: u16, slant: Slant| {
            let font = faces.choose(&request(families, weight, slant));
            let path = faces
                .loaded(font.face)
                .map_or("primary".to_owned(), |f| f.path.display().to_string());
            (path, font.bold, font.skew())
        };
        assert_eq!(
            pick("sans-serif", 400, Slant::Normal),
            ("primary".into(), false, 0.0)
        );
        assert_eq!(
            pick("Unknown, sans-serif", 700, Slant::Normal),
            ("segoeb.ttf".into(), false, 0.0)
        );
        assert_eq!(
            pick("TIMES New Roman", 400, Slant::Italic),
            ("timesi.ttf".into(), false, 0.0)
        );
        assert_eq!(
            pick("serif", 700, Slant::Normal),
            ("times.ttf".into(), true, 0.0)
        );
        assert_eq!(
            pick("monospace", 400, Slant::Oblique(5.0)),
            ("cour.ttf".into(), false, SYNTHETIC_SKEW)
        );
        assert_eq!(
            pick("Nothing", 400, Slant::Normal),
            ("primary".into(), false, 0.0)
        );
        // Times and Courier, which the system has not, are its Times New
        // Roman and Courier New, as in the reference browser.
        assert_eq!(
            pick("Times", 400, Slant::Normal),
            ("times.ttf".into(), false, 0.0)
        );
        assert_eq!(
            pick("courier, serif", 400, Slant::Normal),
            ("cour.ttf".into(), false, 0.0)
        );
        // A face is handed out once.
        let a = faces.choose(&request("serif", 400, Slant::Normal));
        let b = faces.choose(&request("Times New Roman, serif", 400, Slant::Normal));
        assert_eq!(a.face, b.face);
        assert_eq!(faces.records().len(), records.len());
    }
}
