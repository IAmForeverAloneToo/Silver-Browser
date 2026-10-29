//! Choosing a face by the font properties (CSS Fonts 4 section 5, gap
//! batch D, D53): the families in order, each by fontdb's matching with
//! the weight, style and stretch, the generic families mapped to the
//! system's, then the primary font's family. A face chosen is known by a
//! [`FaceId`], a number its collection hands out once and keeps, shared
//! by the collection's clones; the primary font is face 0. Where the
//! face is lighter or more upright than asked for, the font says what
//! drawing makes up: an emboldening and a slant, as the reference
//! browser draws them.

use std::collections::HashMap;
use std::path::PathBuf;
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
}

impl Default for FontRequest<'_> {
    fn default() -> Self {
        FontRequest {
            families: "",
            weight: 400,
            slant: Slant::Normal,
            stretch: 100.0,
        }
    }
}

/// A face chosen, with what drawing makes up for it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Font {
    pub face: FaceId,
    /// Emboldened: bold was asked for and the face is not.
    pub bold: bool,
    /// The horizontal skew a slant was asked for with an upright face:
    /// x moves right by this much of the height above the baseline.
    pub skew: f32,
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
    /// `n - 1`th.
    loaded: RwLock<Vec<Arc<Fallback>>>,
    /// The fonts chosen, by what they were asked for.
    chosen: RwLock<HashMap<Key, Font>>,
}

/// What a choice is kept by: the families, weight, slant and stretch.
type Key = (Box<str>, u16, u32, u32);

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
            chosen: RwLock::new(HashMap::new()),
        }
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
        let slant = match request.slant {
            Slant::Normal => 0,
            Slant::Italic => 1,
            Slant::Oblique(a) => 2 | (a.to_bits() & !3),
        };
        let key: Key = (
            request.families.into(),
            request.weight,
            slant,
            request.stretch.to_bits(),
        );
        if let Ok(chosen) = self.chosen.read()
            && let Some(font) = chosen.get(&key)
        {
            return *font;
        }
        let font = self.matched(request);
        if let Ok(mut chosen) = self.chosen.write() {
            if chosen.len() >= MAX_CHOICES {
                chosen.clear();
            }
            chosen.insert(key, font);
        }
        font
    }

    /// The font CSS's matching gives `request`.
    fn matched(&self, request: &FontRequest<'_>) -> Font {
        let slanted = request.slant != Slant::Normal;
        let Some(db) = &self.db else {
            let (weight, italic) = self.primary_style;
            return synthesized(FaceId(0), request, weight, italic);
        };
        let query = |families: &[fontdb::Family<'_>]| {
            db.query(&fontdb::Query {
                families,
                weight: fontdb::Weight(request.weight),
                style: if slanted {
                    fontdb::Style::Italic
                } else {
                    fontdb::Style::Normal
                },
                stretch: stretch_of_percentage(request.stretch),
            })
        };
        let primary: Vec<fontdb::Family<'_>> = PRIMARY_FAMILIES
            .iter()
            .map(|name| fontdb::Family::Name(name))
            .collect();
        let mut found = None;
        for family in request.families.split(',').map(str::trim) {
            let lower = family.to_ascii_lowercase();
            found = match lower.as_str() {
                "" => None,
                "sans-serif" | "system-ui" | "ui-sans-serif" | "-apple-system" => query(&primary),
                "serif" | "ui-serif" => query(&[fontdb::Family::Serif]),
                "monospace" | "ui-monospace" => query(&[fontdb::Family::Monospace]),
                "cursive" => query(&[fontdb::Family::Cursive]),
                "fantasy" => query(&[fontdb::Family::Fantasy]),
                _ => self
                    .names
                    .get(&family.to_lowercase())
                    .and_then(|name| query(&[fontdb::Family::Name(name)])),
            };
            if found.is_some() {
                break;
            }
        }
        let Some(id) = found.or_else(|| query(&primary)) else {
            let (weight, italic) = self.primary_style;
            return synthesized(FaceId(0), request, weight, italic);
        };
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
        let number = |faces: &[Arc<Fallback>]| {
            faces
                .iter()
                .position(|face| face.path == path && face.index == index)
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
        loaded.push(Arc::new(Fallback {
            path,
            index,
            font: std::sync::OnceLock::new(),
        }));
        FaceId(loaded.len() as u32)
    }

    /// The number `face` was handed out with; 0 for one never handed
    /// out.
    pub(crate) fn id_of(&self, face: &Arc<Fallback>) -> FaceId {
        self.loaded
            .read()
            .ok()
            .and_then(|loaded| loaded.iter().position(|f| Arc::ptr_eq(f, face)))
            .map_or(FaceId(0), |n| FaceId(n as u32 + 1))
    }

    /// The face handed out as `id`, when there is one past the primary
    /// font.
    pub(crate) fn loaded(&self, id: FaceId) -> Option<Arc<Fallback>> {
        let n = id.0.checked_sub(1)? as usize;
        self.loaded.read().ok()?.get(n).cloned()
    }
}

/// `face`, whose weight is `weight` and which is slanted when `italic`,
/// with what drawing makes up for `request`.
fn synthesized(face: FaceId, request: &FontRequest<'_>, weight: u16, italic: bool) -> Font {
    Font {
        face,
        bold: request.weight >= BOLD && weight < BOLD,
        skew: if request.slant != Slant::Normal && !italic {
            SYNTHETIC_SKEW
        } else {
            0.0
        },
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
            stretch: 100.0,
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
                skew: SYNTHETIC_SKEW
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
            (path, font.bold, font.skew)
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
        // A face is handed out once.
        let a = faces.choose(&request("serif", 400, Slant::Normal));
        let b = faces.choose(&request("Times New Roman, serif", 400, Slant::Normal));
        assert_eq!(a.face, b.face);
        assert_eq!(faces.records().len(), records.len());
    }
}
