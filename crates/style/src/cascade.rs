//! The cascade: origin sorting, specificity, inheritance, and property
//! parsing — turning a `dom::Node` plus its stylesheets into a
//! [`StyledDom`] (roadmap M5).
//!
//! Scope of this milestone, deliberately:
//!
//! * At-rules (`@media`, `@supports`, ...) are not evaluated — their
//!   rules are skipped entirely for matching purposes (M20 adds real
//!   `@media` evaluation; applying them unconditionally in the meantime
//!   would risk *wrong* results, e.g. `@media print` rules bleeding into
//!   screen rendering, which is worse than not applying them at all).
//! * Colors: hex (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`) and a small,
//!   high-confidence named-color set — not the full CSS Color 148-name
//!   table, and not `rgb()`/`hsl()` functions yet.
//! * Lengths: `px` and `em` only (`%`, `rem`, `vw`/`vh`, ... land with
//!   M10.1).
//! * `display`: `block` / `inline` / `none` only (M5.1's own v1 scope).

use css::Token;
use dom::Node;

use crate::computed::{
    BorderStyle, Color, ComputedStyle, Direction, Display, Edges, LengthPercentageAuto,
    MarginEdges, TextAlign, TextTransform, WhiteSpace, BOTTOM, LEFT, RIGHT, TOP,
};
use crate::selector_matching::{matches_selector, parse_selector_list, FlatChild, FlatDom, Specificity};

/// The built-in default stylesheet (M5.6), compiled in — loading it is
/// not resource fetching (no I/O at runtime), so it doesn't bend the
/// "parsers never fetch" rule.
const UA_STYLESHEET_SRC: &str = include_str!("../assets/ua.css");

/// A `FlatDom` with each node's [`ComputedStyle`] resolved alongside it:
/// `styles[i]` is the computed style for `flat.nodes[i]`, including
/// index 0 (the virtual document root — never itself painted, but real
/// top-level elements inherit from its always-initial-values style).
pub struct StyledDom {
    /// The structural view (also used for selector matching).
    pub flat: FlatDom,
    /// Parallel to `flat.nodes` — `styles[i]` is `flat.nodes[i]`'s style.
    pub styles: Vec<ComputedStyle>,
}

/// Resolves computed styles for every element in `document`, cascading
/// the built-in UA stylesheet (lowest precedence) under `author_sheets`
/// (in document order) and each element's inline `style` attribute
/// (highest precedence short of `!important` — see [`tier`]).
pub fn style_document(document: &Node, author_sheets: &[css::Stylesheet]) -> StyledDom {
    let flat = FlatDom::build(document);
    let ua = css::parse_stylesheet(UA_STYLESHEET_SRC)
        .expect("the built-in UA stylesheet (assets/ua.css) is valid CSS");
    let mut styles = vec![ComputedStyle::default(); flat.nodes.len()];
    compute_recursive(&flat, 0, &ua, author_sheets, &mut styles);
    StyledDom { flat, styles }
}

/// Computes styles for `idx`'s element children, then recurses — a
/// parent's style is always finalized before any child needs it for
/// inheritance.
fn compute_recursive(
    flat: &FlatDom,
    idx: usize,
    ua: &css::Stylesheet,
    author_sheets: &[css::Stylesheet],
    styles: &mut Vec<ComputedStyle>,
) {
    let parent_style = styles[idx].clone();
    let child_element_indices: Vec<usize> = flat.nodes[idx]
        .children
        .iter()
        .filter_map(|c| match c {
            FlatChild::Element(i) => Some(*i),
            FlatChild::Text(_) => None,
        })
        .collect();
    for child_idx in child_element_indices {
        let applied = declarations_for(flat, child_idx, ua, author_sheets);
        styles[child_idx] = compute_style(&applied, &parent_style, flat.attr(child_idx, "dir"));
        compute_recursive(flat, child_idx, ua, author_sheets, styles);
    }
}

/// A declaration plus everything needed to sort it into cascade order.
struct Applied {
    /// See [`tier`] — encodes origin *and* importance in one order.
    tier: u8,
    specificity: Specificity,
    /// Strictly increasing in the order this declaration was examined
    /// (UA sheet, then each author sheet, in document order) — the
    /// final tiebreak once tier and specificity are equal.
    order: u32,
    decl: css::Declaration,
}

/// Where a declaration came from, before importance is folded in.
#[derive(Clone, Copy)]
enum TierBase {
    UserAgent,
    AuthorSheet,
    AuthorInline,
}

/// The cascade order (css-cascade-5 §6), collapsed to a single `u8` so
/// declarations can be sorted by `(tier, specificity, order)` and
/// applied ascending — later always overwrites earlier, no special
/// cases at apply time. Lowest to highest precedence: UA normal, author
/// normal (inline beats any selector-matched normal rule, regardless of
/// specificity), author `!important` (inline again beats sheet rules),
/// UA `!important` (unused by `assets/ua.css` today, but modeled for
/// correctness — a UA sheet is allowed to use `!important`).
fn tier(base: TierBase, important: bool) -> u8 {
    match (base, important) {
        (TierBase::UserAgent, false) => 0,
        (TierBase::AuthorSheet, false) => 1,
        (TierBase::AuthorInline, false) => 2,
        (TierBase::AuthorSheet, true) => 3,
        (TierBase::AuthorInline, true) => 4,
        (TierBase::UserAgent, true) => 5,
    }
}

/// Every declaration that applies to element `idx`, tagged with its
/// cascade key — unsorted; [`compute_style`] sorts before applying.
fn declarations_for(
    flat: &FlatDom,
    idx: usize,
    ua: &css::Stylesheet,
    author_sheets: &[css::Stylesheet],
) -> Vec<Applied> {
    let mut order = 0u32;
    let mut out = Vec::new();
    collect_from_sheet(flat, idx, ua, TierBase::UserAgent, &mut order, &mut out);
    for sheet in author_sheets {
        collect_from_sheet(flat, idx, sheet, TierBase::AuthorSheet, &mut order, &mut out);
    }
    if let Some(style_attr) = flat.attr(idx, "style")
        && let Ok(decls) = css::parse_style_attribute(style_attr)
    {
        for decl in decls {
            out.push(Applied {
                tier: tier(TierBase::AuthorInline, decl.important),
                // Inline declarations aren't selector-matched, so there's
                // nothing to compare within this tier by specificity —
                // `order` alone (declaration order in the attribute)
                // breaks ties, matching real engines.
                specificity: Specificity::default(),
                order,
                decl,
            });
            order += 1;
        }
    }
    out
}

/// Matches `sheet`'s qualified rules against element `idx`, pushing one
/// [`Applied`] per declaration in every rule that matches. At-rules are
/// skipped (see the module doc); an unparseable prelude drops that one
/// rule, per css-syntax-3 — M4.10, now actually exercised end to end.
fn collect_from_sheet(
    flat: &FlatDom,
    idx: usize,
    sheet: &css::Stylesheet,
    base: TierBase,
    order: &mut u32,
    out: &mut Vec<Applied>,
) {
    for rule in &sheet.rules {
        let css::Rule::Qualified(qr) = rule else { continue };
        let Ok(list) = parse_selector_list(&qr.prelude) else { continue };
        // A comma-separated list is cascade-equivalent to separate rules
        // sharing one declaration block — each matching branch
        // contributes independently, so the specificity that matters is
        // the highest among the branches that actually matched here.
        let specificity = list
            .0
            .iter()
            .filter(|s| matches_selector(flat, idx, s))
            .map(|s| s.specificity)
            .max();
        let Some(specificity) = specificity else { continue };
        for decl in &qr.declarations {
            out.push(Applied {
                tier: tier(base, decl.important),
                specificity,
                order: *order,
                decl: decl.clone(),
            });
            *order += 1;
        }
    }
}

/// Sorts `applied` into cascade order and folds it into a fresh
/// `ComputedStyle` inherited from `parent`.
fn compute_style(applied: &[Applied], parent: &ComputedStyle, dir_attr: Option<&str>) -> ComputedStyle {
    let mut sorted: Vec<&Applied> = applied.iter().collect();
    sorted.sort_by_key(|a| (a.tier, a.specificity, a.order));

    let mut style = ComputedStyle::inherited_from(parent);

    // `dir` (M8.3): a presentational HTML attribute, not a cascaded CSS
    // declaration — applied directly, the same way the inline `style`
    // attribute gets read in `declarations_for`, rather than through
    // `apply_declaration`'s property-name dispatch (there's no CSS
    // `direction` property parser yet to route it through). `auto`
    // (content-sniffed direction) isn't implemented — unrecognized
    // values are ignored, same forward-compat pattern as everywhere
    // else, leaving the inherited direction in place.
    match dir_attr.map(str::to_ascii_lowercase).as_deref() {
        Some("ltr") => style.direction = Direction::Ltr,
        Some("rtl") => style.direction = Direction::Rtl,
        _ => {}
    }

    // `font-size` first: every other length's `em` resolves against
    // *this* element's own font-size, which itself must already be
    // final before those are processed.
    for a in &sorted {
        if a.decl.name == "font-size"
            && let Some(px) = parse_length_px(&a.decl.value, parent.font_size_px)
        {
            style.font_size_px = px;
        }
    }

    let em_basis = style.font_size_px;
    let mut border_color_set = false;
    for a in &sorted {
        apply_declaration(&mut style, &a.decl, em_basis, &mut border_color_set);
    }
    if !border_color_set {
        // `border-color`'s initial value is `currentcolor` (css-color-4
        // §4.4) — resolved here now that `color` itself is final.
        style.border_color = style.color;
    }
    style
}

/// Applies one already-cascade-ordered declaration to `style`, ignoring
/// (never erroring on) any property name or value this engine doesn't
/// understand yet — the M3 forward-compatibility pattern: a browser
/// encounters unsupported properties on every real page, constantly.
fn apply_declaration(style: &mut ComputedStyle, decl: &css::Declaration, em_basis: f32, border_color_set: &mut bool) {
    match decl.name.as_str() {
        "color" => {
            if let Some(c) = parse_color(&decl.value) {
                style.color = c;
            }
        }
        "background-color" => {
            if let Some(c) = parse_color(&decl.value) {
                style.background_color = c;
            }
        }
        "font-size" => {} // first pass, above
        "font-family" => {
            if let Some(f) = parse_font_family(&decl.value) {
                style.font_family = f;
            }
        }
        "display" => {
            if let Some(d) = parse_display(&decl.value) {
                style.display = d;
            }
        }
        "text-align" => {
            if let Some(a) = parse_text_align(&decl.value) {
                style.text_align = a;
            }
        }
        "text-transform" => {
            if let Some(t) = parse_text_transform(&decl.value) {
                style.text_transform = t;
            }
        }
        "white-space" => {
            if let Some(w) = parse_white_space(&decl.value) {
                style.white_space = w;
            }
        }
        "letter-spacing" => {
            if let Some(v) = parse_spacing(&decl.value, em_basis) {
                style.letter_spacing = v;
            }
        }
        "word-spacing" => {
            if let Some(v) = parse_spacing(&decl.value, em_basis) {
                style.word_spacing = v;
            }
        }
        "width" => {
            if let Some(w) = parse_length_percentage_or_auto(&decl.value, em_basis) {
                style.width = w;
            }
        }
        "height" => {
            // `<percentage>` deliberately isn't accepted here yet
            // (roadmap M6.4 defers percentage heights to M10) —
            // `parse_length_or_auto` only ever produces `Auto`/`Px`.
            if let Some(h) = parse_length_or_auto(&decl.value, em_basis) {
                style.height = h;
            }
        }
        "margin" => {
            if let Some(e) = parse_margin_shorthand(&decl.value, em_basis) {
                style.margin = e;
            }
        }
        "margin-top" => set_margin_edge(&mut style.margin, TOP, &decl.value, em_basis),
        "margin-right" => set_margin_edge(&mut style.margin, RIGHT, &decl.value, em_basis),
        "margin-bottom" => set_margin_edge(&mut style.margin, BOTTOM, &decl.value, em_basis),
        "margin-left" => set_margin_edge(&mut style.margin, LEFT, &decl.value, em_basis),
        "padding" => {
            if let Some(e) = parse_box_shorthand(&decl.value, em_basis) {
                style.padding = e;
            }
        }
        "padding-top" => set_edge(&mut style.padding, TOP, &decl.value, em_basis),
        "padding-right" => set_edge(&mut style.padding, RIGHT, &decl.value, em_basis),
        "padding-bottom" => set_edge(&mut style.padding, BOTTOM, &decl.value, em_basis),
        "padding-left" => set_edge(&mut style.padding, LEFT, &decl.value, em_basis),
        // padding/border-width reject negative lengths per spec;
        // `parse_length_px` doesn't enforce that yet (margin needs
        // negatives, so the parser stays sign-agnostic) — a known v1 gap.
        "border-width" => {
            if let Some(e) = parse_box_shorthand(&decl.value, em_basis) {
                style.border_width = e;
            }
        }
        "border-top-width" => set_edge(&mut style.border_width, TOP, &decl.value, em_basis),
        "border-right-width" => set_edge(&mut style.border_width, RIGHT, &decl.value, em_basis),
        "border-bottom-width" => set_edge(&mut style.border_width, BOTTOM, &decl.value, em_basis),
        "border-left-width" => set_edge(&mut style.border_width, LEFT, &decl.value, em_basis),
        "border-color" => {
            if let Some(c) = parse_color(&decl.value) {
                style.border_color = c;
                *border_color_set = true;
            }
        }
        "border-style" => {
            if let Some(s) = parse_border_style(&decl.value) {
                style.border_style = s;
            }
        }
        "border" => apply_border_shorthand(style, &decl.value, em_basis, border_color_set),
        _ => {} // not in the v1 property set (or not a real property) — ignored, never an error
    }
}

/// Parses and writes a single-length longhand (`padding-top`,
/// `border-*-width`, ...); a value that fails to parse leaves that edge
/// untouched, exactly like any other declaration this engine can't make
/// sense of.
fn set_edge(edges: &mut Edges, which: usize, tokens: &[Token], em_basis: f32) {
    if let Some(v) = parse_length_px(tokens, em_basis) {
        edges[which] = v;
    }
}

/// Like [`set_edge`], for a `margin-*` longhand (which additionally
/// accepts `auto`).
fn set_margin_edge(edges: &mut MarginEdges, which: usize, tokens: &[Token], em_basis: f32) {
    if let Some(v) = parse_length_or_auto(tokens, em_basis) {
        edges[which] = v;
    }
}

/// Expands a 1–4 value list into `[top, right, bottom, left]` per CSS2
/// §8.3's box-shorthand expansion rule (shared by `margin`, `padding`,
/// and `border-width` — only the per-value parser differs).
fn expand_1_to_4<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    match values {
        [a] => Some([*a, *a, *a, *a]),
        [a, b] => Some([*a, *b, *a, *b]),
        [a, b, c] => Some([*a, *b, *c, *b]),
        [a, b, c, d] => Some([*a, *b, *c, *d]),
        _ => None,
    }
}

/// Expands the `padding`/`border-width` box shorthand. Any value that
/// fails to parse as a length invalidates the whole shorthand (matching
/// real CSS: a shorthand with one bad component is dropped entirely, not
/// partially applied).
fn parse_box_shorthand(tokens: &[Token], em_basis_px: f32) -> Option<Edges> {
    let groups = split_whitespace_groups(tokens);
    let lengths: Option<Vec<f32>> =
        groups.iter().map(|g| parse_length_px(g, em_basis_px)).collect();
    expand_1_to_4(&lengths?)
}

/// Expands the `margin` box shorthand — like [`parse_box_shorthand`),
/// but each value may also be `auto`.
fn parse_margin_shorthand(tokens: &[Token], em_basis_px: f32) -> Option<MarginEdges> {
    let groups = split_whitespace_groups(tokens);
    let values: Option<Vec<LengthPercentageAuto>> =
        groups.iter().map(|g| parse_length_or_auto(g, em_basis_px)).collect();
    expand_1_to_4(&values?)
}

/// The `border` shorthand: any order, any subset, of a width, a style
/// keyword, and a color. Using it always resets the parts it doesn't
/// mention to their initial value (real shorthand semantics) — a
/// omitted color re-enables the `currentcolor` default applied once the
/// whole cascade for this element is done (see [`compute_style`]).
fn apply_border_shorthand(
    style: &mut ComputedStyle,
    tokens: &[Token],
    em_basis_px: f32,
    border_color_set: &mut bool,
) {
    let mut width = None;
    let mut border_style = None;
    let mut color = None;
    for g in split_whitespace_groups(tokens) {
        if width.is_none() && let Some(w) = parse_length_px(&g, em_basis_px) {
            width = Some(w);
        } else if border_style.is_none() && let Some(s) = parse_border_style(&g) {
            border_style = Some(s);
        } else if color.is_none() && let Some(c) = parse_color(&g) {
            color = Some(c);
        } else {
            return; // an unrecognized (or repeated) part: drop the whole shorthand
        }
    }
    style.border_width = [width.unwrap_or(0.0); 4];
    style.border_style = border_style.unwrap_or(BorderStyle::None);
    match color {
        Some(c) => {
            style.border_color = c;
            *border_color_set = true;
        }
        None => *border_color_set = false,
    }
}

/// Splits `tokens` on whitespace into non-empty groups, preserving each
/// group's internal token order — shared by `border`'s and the box
/// shorthands' "one value per space-separated word" parsing.
fn split_whitespace_groups(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for t in tokens {
        if matches!(t, Token::Whitespace) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            current.push(t.clone());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// A length in CSS pixels: `px` as-is, `em` relative to `em_basis_px`,
/// or the unitless `0` (the one number CSS allows without a unit for
/// length properties). Anything else (a nonzero unitless number, `%`,
/// `rem`, ...) isn't v1 scope yet.
fn parse_length_px(tokens: &[Token], em_basis_px: f32) -> Option<f32> {
    match single_token(tokens)? {
        Token::Dimension { value, unit } if unit.eq_ignore_ascii_case("px") => Some(*value as f32),
        Token::Dimension { value, unit } if unit.eq_ignore_ascii_case("em") => {
            Some(*value as f32 * em_basis_px)
        }
        Token::Number { value, .. } if *value == 0.0 => Some(0.0),
        _ => None,
    }
}

/// `auto`, or a length (`px`/`em`/unitless `0`) — used for `margin` and
/// `height`. Neither accepts `<percentage>` yet (deferred to M10 —
/// margin-percentage alongside percentage heights, kept to a matching
/// scope rather than implementing one and not the other).
fn parse_length_or_auto(tokens: &[Token], em_basis_px: f32) -> Option<LengthPercentageAuto> {
    if is_auto(tokens) {
        return Some(LengthPercentageAuto::Auto);
    }
    parse_length_px(tokens, em_basis_px).map(LengthPercentageAuto::Px)
}

/// `auto`, a length, or a percentage — used for `width` (M6.2's explicit
/// scope: "auto, fixed px, percentage of containing block").
fn parse_length_percentage_or_auto(tokens: &[Token], em_basis_px: f32) -> Option<LengthPercentageAuto> {
    if is_auto(tokens) {
        return Some(LengthPercentageAuto::Auto);
    }
    if let Some(Token::Percentage { value }) = single_token(tokens) {
        return Some(LengthPercentageAuto::Percent(*value as f32));
    }
    parse_length_px(tokens, em_basis_px).map(LengthPercentageAuto::Px)
}

fn is_auto(tokens: &[Token]) -> bool {
    single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("auto"))
}

fn parse_display(tokens: &[Token]) -> Option<Display> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "none" => Some(Display::None),
        _ => None, // flex/grid/inline-block/... land later
    }
}

fn parse_text_align(tokens: &[Token]) -> Option<TextAlign> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "left" => Some(TextAlign::Left),
        "right" => Some(TextAlign::Right),
        "center" => Some(TextAlign::Center),
        _ => None, // `justify`/`start`/`end` land with M21
    }
}

fn parse_text_transform(tokens: &[Token]) -> Option<TextTransform> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None, // full-width/full-size-kana land later
    }
}

fn parse_white_space(tokens: &[Token]) -> Option<WhiteSpace> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "normal" => Some(WhiteSpace::Normal),
        "pre" => Some(WhiteSpace::Pre),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "pre-line" => Some(WhiteSpace::PreLine),
        _ => None, // `break-spaces` (css-text-4) lands later
    }
}

/// `normal` (resets to `0`) or a length — `letter-spacing`/`word-spacing`
/// share this grammar.
fn parse_spacing(tokens: &[Token], em_basis_px: f32) -> Option<f32> {
    if is_normal(tokens) {
        return Some(0.0);
    }
    parse_length_px(tokens, em_basis_px)
}

fn is_normal(tokens: &[Token]) -> bool {
    single_ident(tokens).is_some_and(|s| s.eq_ignore_ascii_case("normal"))
}

fn parse_border_style(tokens: &[Token]) -> Option<BorderStyle> {
    match single_ident(tokens)?.to_ascii_lowercase().as_str() {
        "none" | "hidden" => Some(BorderStyle::None),
        "solid" => Some(BorderStyle::Solid),
        _ => None, // dashed/dotted/double/... land with box-model completion (M10)
    }
}

/// A comma-joined font-family list, kept as declared — still not
/// consumed for real font *selection*. M8.1 landed per-*script*
/// fallback (`text::FontCollection`: Latin primary + resolved CJK
/// fallbacks), which is what that milestone's own wording asked for,
/// but it's driven entirely by Unicode script detection on the text
/// itself, never by this property: an author's `font-family: "Noto
/// Sans CJK SC", sans-serif` declaration is parsed and stored here and
/// then simply never read by `text`. Properly honoring it needs more
/// than M8.1 did — `FontCollection` is one primary font per `Engine`,
/// not one per declared family list, so per-element family selection
/// would need its own query-and-cache mechanism alongside the
/// per-script one, plus a fallback-order decision when both a declared
/// family *and* a script-appropriate one could apply to the same run.
/// Not scheduled under a specific milestone yet — flagged here instead
/// of silently staying unread.
fn parse_font_family(tokens: &[Token]) -> Option<String> {
    let mut families = Vec::new();
    let mut current = String::new();
    for t in tokens {
        match t {
            Token::Whitespace => {
                if !current.is_empty() {
                    current.push(' ');
                }
            }
            Token::Comma => {
                let name = current.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                families.push(name);
                current.clear();
            }
            Token::Ident(s) => current.push_str(s),
            Token::String { value, bad: false } => current.push_str(value),
            _ => return None,
        }
    }
    let name = current.trim().to_string();
    if name.is_empty() {
        return None;
    }
    families.push(name);
    Some(families.join(", "))
}

/// Hex colors and a small, deliberately-not-exhaustive named-color set
/// (see the module doc) — filters whitespace, then requires exactly one
/// remaining token (trailing garbage invalidates the value, matching
/// real CSS: a malformed value drops the whole declaration).
fn parse_color(tokens: &[Token]) -> Option<Color> {
    match single_token(tokens)? {
        Token::Hash { value, .. } => parse_hex_color(value),
        Token::Ident(name) => named_color(name),
        _ => None,
    }
}

fn single_token(tokens: &[Token]) -> Option<&Token> {
    let mut it = tokens.iter().filter(|t| !matches!(t, Token::Whitespace));
    let first = it.next()?;
    match it.next() {
        None => Some(first),
        Some(_) => None,
    }
}

fn single_ident(tokens: &[Token]) -> Option<&str> {
    match single_token(tokens)? {
        Token::Ident(s) => Some(s.as_str()),
        _ => None,
    }
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (css-color-4 §4.2.1) — any
/// other length, or any non-hex-digit character, is invalid.
fn parse_hex_color(hex: &str) -> Option<Color> {
    let digit = |c: u8| (c as char).to_digit(16);
    let bytes = hex.as_bytes();
    let one = |i: usize| -> Option<f32> { Some(digit(*bytes.get(i)?)? as f32 / 15.0) };
    let two = |i: usize| -> Option<f32> {
        let hi = digit(*bytes.get(i)?)?;
        let lo = digit(*bytes.get(i + 1)?)?;
        Some((hi * 16 + lo) as f32 / 255.0)
    };
    match hex.len() {
        3 => Some(Color { r: one(0)?, g: one(1)?, b: one(2)?, a: 1.0 }),
        4 => Some(Color { r: one(0)?, g: one(1)?, b: one(2)?, a: one(3)? }),
        6 => Some(Color { r: two(0)?, g: two(2)?, b: two(4)?, a: 1.0 }),
        8 => Some(Color { r: two(0)?, g: two(2)?, b: two(4)?, a: two(6)? }),
        _ => None,
    }
}

/// A small, high-confidence set of named colors (CSS1/CSS2's basic
/// keywords, plus `orange` and `transparent`) — not the full CSS Color
/// 148-name table (deliberately: getting an obscure one wrong silently
/// would be worse than not supporting it yet).
fn named_color(name: &str) -> Option<Color> {
    Some(match name.to_ascii_lowercase().as_str() {
        "black" => Color::rgb8(0, 0, 0),
        "silver" => Color::rgb8(192, 192, 192),
        "gray" | "grey" => Color::rgb8(128, 128, 128),
        "white" => Color::rgb8(255, 255, 255),
        "maroon" => Color::rgb8(128, 0, 0),
        "red" => Color::rgb8(255, 0, 0),
        "purple" => Color::rgb8(128, 0, 128),
        "fuchsia" | "magenta" => Color::rgb8(255, 0, 255),
        "green" => Color::rgb8(0, 128, 0),
        "lime" => Color::rgb8(0, 255, 0),
        "olive" => Color::rgb8(128, 128, 0),
        "yellow" => Color::rgb8(255, 255, 0),
        "navy" => Color::rgb8(0, 0, 128),
        "blue" => Color::rgb8(0, 0, 255),
        "teal" => Color::rgb8(0, 128, 128),
        "aqua" | "cyan" => Color::rgb8(0, 255, 255),
        "orange" => Color::rgb8(255, 165, 0),
        "transparent" => Color::TRANSPARENT,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(src: &str) -> css::Stylesheet {
        css::parse_stylesheet(src).expect("test stylesheets are valid")
    }

    /// Styles `html` and returns the computed style of the element found
    /// by `selector` (the first match, in document order).
    fn style_of(html: &str, sheets: &[css::Stylesheet], selector: &str) -> ComputedStyle {
        let document = html::parse_document(html).expect("html parsing never fails");
        let styled = style_document(&document, sheets);
        let list = parse_selector_list(&css::tokenize(selector)).expect("valid selector");
        styled
            .flat
            .element_indices()
            .find(|&i| matches_selector(&styled.flat, i, &list.0[0]))
            .map(|i| styled.styles[i].clone())
            .unwrap_or_else(|| panic!("no element matched {selector:?}"))
    }

    /// Wraps four plain pixel values as `MarginEdges` (test convenience).
    fn px4(edges: [f32; 4]) -> MarginEdges {
        edges.map(LengthPercentageAuto::Px)
    }

    #[test]
    fn ua_stylesheet_parses_and_applies_with_no_author_sheets() {
        let s = style_of("<html><body><p>x</p></body></html>", &[], "p");
        assert_eq!(s.display, Display::Block); // from ua.css
        assert_eq!(s.margin, px4([16.0, 0.0, 16.0, 0.0]));
    }

    #[test]
    fn author_beats_ua_at_equal_specificity_tier() {
        // ua.css sets `p { margin: 16px 0; }`; author overrides fully.
        let sheets = [sheet("p { margin: 5px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin, px4([5.0; 4]));
    }

    #[test]
    fn specificity_breaks_ties_within_author_normal() {
        let sheets = [sheet("p { color: red; } p.a { color: blue; }")];
        let s = style_of("<p class='a'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn later_source_order_wins_at_equal_specificity() {
        let sheets = [sheet("p { color: red; } p { color: blue; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn low_specificity_important_beats_high_specificity_normal() {
        let sheets = [sheet("p { color: red !important; } p#x { color: blue; }")];
        let s = style_of("<p id='x'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn important_author_beats_important_reversed_for_normal() {
        // Sanity check on the *reversal*: normal author beats normal UA
        // (author { display: block } over UA's implicit inline default
        // for a made-up tag), which is the opposite of the !important case.
        let sheets = [sheet("made-up-tag { display: block; }")];
        let s = style_of("<made-up-tag>x</made-up-tag>", &sheets, "made-up-tag");
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn inline_style_beats_any_selector_matched_normal_rule() {
        let sheets = [sheet("#x { color: blue; }")]; // higher specificity than any selector could beat via inline... except inline always wins anyway
        let s = style_of("<p id='x' style='color: red'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn author_important_beats_inline_normal() {
        let sheets = [sheet("p { color: blue !important; }")];
        let s = style_of("<p style='color: red'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn inline_important_beats_author_important() {
        let sheets = [sheet("p { color: blue !important; }")];
        let s = style_of("<p style='color: red !important'>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn color_and_font_size_inherit_but_margin_does_not() {
        let sheets = [sheet("div { color: red; font-size: 20px; margin: 10px; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.color, Color::rgb8(255, 0, 0));
        assert_eq!(s.font_size_px, 20.0);
        assert_eq!(s.margin, px4([0.0; 4])); // initial, not inherited
    }

    #[test]
    fn em_on_font_size_resolves_against_parent() {
        let sheets = [sheet("div { font-size: 20px; } span { font-size: 2em; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.font_size_px, 40.0);
    }

    #[test]
    fn em_on_other_properties_resolves_against_self() {
        let sheets = [sheet("p { font-size: 10px; margin: 2em; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin, px4([20.0; 4]));
    }

    #[test]
    fn box_shorthand_expands_one_to_four_values() {
        // `padding`/`border-width`'s shorthand (`margin`'s own, which
        // additionally allows `auto` per value, is tested separately
        // below).
        assert_eq!(parse_box_shorthand(&css::tokenize("10px"), 16.0), Some([10.0; 4]));
        assert_eq!(
            parse_box_shorthand(&css::tokenize("10px 20px"), 16.0),
            Some([10.0, 20.0, 10.0, 20.0])
        );
        assert_eq!(
            parse_box_shorthand(&css::tokenize("10px 20px 30px"), 16.0),
            Some([10.0, 20.0, 30.0, 20.0])
        );
        assert_eq!(
            parse_box_shorthand(&css::tokenize("10px 20px 30px 40px"), 16.0),
            Some([10.0, 20.0, 30.0, 40.0])
        );
        assert_eq!(parse_box_shorthand(&css::tokenize("10px 20px 30px 40px 50px"), 16.0), None);
    }

    #[test]
    fn margin_shorthand_expands_and_allows_auto_per_value() {
        use LengthPercentageAuto::{Auto, Px};
        assert_eq!(
            parse_margin_shorthand(&css::tokenize("10px auto"), 16.0),
            Some([Px(10.0), Auto, Px(10.0), Auto])
        );
        assert_eq!(
            parse_margin_shorthand(&css::tokenize("0 auto"), 16.0),
            Some([Px(0.0), Auto, Px(0.0), Auto])
        );
    }

    #[test]
    fn margin_auto_reaches_computed_style() {
        let sheets = [sheet("p { margin: 0 auto; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.margin[LEFT], LengthPercentageAuto::Auto);
        assert_eq!(s.margin[RIGHT], LengthPercentageAuto::Auto);
        assert_eq!(s.margin[TOP], LengthPercentageAuto::Px(0.0));
    }

    #[test]
    fn text_align_parses_and_inherits() {
        let sheets = [sheet("div { text-align: center; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.text_align, TextAlign::Center, "inherited from div");
    }

    #[test]
    fn text_align_unknown_value_is_ignored() {
        let sheets = [sheet("p { text-align: justify; }")]; // not v1 yet
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.text_align, TextAlign::Left); // initial value, untouched
    }

    #[test]
    fn text_transform_parses_and_inherits() {
        let sheets = [sheet("div { text-transform: uppercase; }")];
        let s = style_of("<div><span>x</span></div>", &sheets, "span");
        assert_eq!(s.text_transform, TextTransform::Uppercase);
    }

    #[test]
    fn white_space_parses_all_four_values() {
        for (value, expected) in [
            ("normal", WhiteSpace::Normal),
            ("pre", WhiteSpace::Pre),
            ("pre-wrap", WhiteSpace::PreWrap),
            ("pre-line", WhiteSpace::PreLine),
        ] {
            let sheets = [sheet(&format!("p {{ white-space: {value}; }}"))];
            assert_eq!(style_of("<p>x</p>", &sheets, "p").white_space, expected, "{value}");
        }
    }

    #[test]
    fn letter_and_word_spacing_accept_lengths_and_normal() {
        let sheets = [sheet("p { letter-spacing: 2px; word-spacing: 1em; font-size: 10px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.letter_spacing, 2.0);
        assert_eq!(s.word_spacing, 10.0); // 1em at font-size: 10px

        let sheets = [sheet("div { letter-spacing: 5px; } p { letter-spacing: normal; }")];
        let s = style_of("<div><p>x</p></div>", &sheets, "p");
        assert_eq!(s.letter_spacing, 0.0, "`normal` resets, it doesn't just fail to parse");
    }

    #[test]
    fn dir_attribute_sets_direction_and_inherits() {
        let s = style_of(r#"<div dir="rtl"><span>x</span></div>"#, &[], "span");
        assert_eq!(s.direction, Direction::Rtl, "inherited from the div's dir attribute");

        let s = style_of(r#"<p dir="ltr">x</p>"#, &[], "p");
        assert_eq!(s.direction, Direction::Ltr);

        // Unrecognized (e.g. `auto`, which v1 doesn't content-sniff for)
        // leaves the inherited/initial value alone rather than erroring.
        let s = style_of(r#"<p dir="auto">x</p>"#, &[], "p");
        assert_eq!(s.direction, Direction::Ltr);
    }

    #[test]
    fn width_accepts_auto_px_and_percent() {
        let auto = style_of("<p>x</p>", &[], "p");
        assert_eq!(auto.width, LengthPercentageAuto::Auto); // initial value

        let sheets = [sheet("p { width: 200px; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").width, LengthPercentageAuto::Px(200.0));

        let sheets = [sheet("p { width: 50%; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").width, LengthPercentageAuto::Percent(50.0));
    }

    #[test]
    fn height_accepts_auto_and_px_but_not_percent_yet() {
        let sheets = [sheet("p { height: 300px; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").height, LengthPercentageAuto::Px(300.0));

        // A `%` height is unrecognized *value* syntax for now (M10 adds
        // it) — ignored, per the forward-compatibility pattern, not an
        // error; the property keeps its initial value.
        let sheets = [sheet("p { height: 50%; }")];
        assert_eq!(style_of("<p>x</p>", &sheets, "p").height, LengthPercentageAuto::Auto);
    }

    #[test]
    fn border_shorthand_sets_width_style_and_color_in_any_order() {
        let sheets = [sheet("p { border: 2px solid red; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [2.0; 4]);
        assert_eq!(s.border_style, BorderStyle::Solid);
        assert_eq!(s.border_color, Color::rgb8(255, 0, 0));

        // Order-independence.
        let sheets = [sheet("p { border: red solid 2px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [2.0; 4]);
        assert_eq!(s.border_color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn border_color_defaults_to_currentcolor() {
        let sheets = [sheet("p { color: red; border: 1px solid; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn border_none_computes_to_no_visible_border_regardless_of_width() {
        // CSS2 §8.5.3: `border-style: none` means the border doesn't
        // paint even if a width was given — this engine still records
        // `border_width` faithfully; `border_style` is what callers must
        // check before treating that width as visible.
        let sheets = [sheet("p { border-width: 5px; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.border_width, [5.0; 4]);
        assert_eq!(s.border_style, BorderStyle::None);
    }

    #[test]
    fn unrecognized_properties_and_values_are_ignored_not_errors() {
        let sheets = [sheet("p { transform: rotate(5deg); color: red; margin: banana; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(255, 0, 0)); // the recognized one still applied
        // The bad value didn't override — `<p>` keeps ua.css's margin,
        // not [0;4] (that would actually be a bug: a bad author value
        // clobbering a previously-cascaded one instead of just not
        // applying).
        assert_eq!(s.margin, px4([16.0, 0.0, 16.0, 0.0]));
    }

    #[test]
    fn unparseable_selector_drops_its_rule_not_the_stylesheet() {
        // M4.10, exercised end to end: a bad selector's rule vanishes;
        // sibling rules in the same sheet still apply.
        let sheets = [sheet(":not() { color: red; } p { color: blue; }")];
        let s = style_of("<p>x</p>", &sheets, "p");
        assert_eq!(s.color, Color::rgb8(0, 0, 255));
    }

    #[test]
    fn display_none_is_recorded_but_children_still_get_styles() {
        let sheets = [sheet("div { display: none; } span { color: red; }")];
        let document = html::parse_document("<div><span>x</span></div>").unwrap();
        let styled = style_document(&document, &sheets);
        let div = 1;
        let span = 2;
        assert_eq!(styled.styles[div].display, Display::None);
        // Styling doesn't itself decide what's painted — that's paint's
        // job, walking `display` — so a display:none subtree's children
        // still have well-defined styles.
        assert_eq!(styled.styles[span].color, Color::rgb8(255, 0, 0));
    }

    #[test]
    fn hex_colors_all_four_lengths() {
        assert_eq!(parse_color(&css::tokenize("#f00")), Some(Color::rgb8(255, 0, 0)));
        assert_eq!(
            parse_color(&css::tokenize("#ff0000")),
            Some(Color::rgb8(255, 0, 0))
        );
        assert_eq!(
            parse_color(&css::tokenize("#f00f")),
            Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })
        );
        assert_eq!(
            parse_color(&css::tokenize("#ff000080")),
            Some(Color { r: 1.0, g: 0.0, b: 0.0, a: 128.0 / 255.0 })
        );
        assert_eq!(parse_color(&css::tokenize("#ggg")), None);
        assert_eq!(parse_color(&css::tokenize("#ff")), None); // invalid length
    }

    #[test]
    fn named_colors_are_case_insensitive() {
        assert_eq!(parse_color(&css::tokenize("RED")), Some(Color::rgb8(255, 0, 0)));
        assert_eq!(parse_color(&css::tokenize("CornflowerBlue")), None); // not in the small set — documented gap
    }

    #[test]
    fn font_family_list_preserves_multi_word_and_quoted_names() {
        assert_eq!(
            parse_font_family(&css::tokenize("Arial, sans-serif")),
            Some("Arial, sans-serif".to_string())
        );
        assert_eq!(
            parse_font_family(&css::tokenize("\"Segoe UI\", sans-serif")),
            Some("Segoe UI, sans-serif".to_string())
        );
        assert_eq!(
            parse_font_family(&css::tokenize("Segoe UI, sans-serif")),
            Some("Segoe UI, sans-serif".to_string())
        );
    }

    #[test]
    fn zero_length_needs_no_unit_but_nonzero_does() {
        assert_eq!(parse_length_px(&css::tokenize("0"), 16.0), Some(0.0));
        assert_eq!(parse_length_px(&css::tokenize("5"), 16.0), None);
        assert_eq!(parse_length_px(&css::tokenize("5px"), 16.0), Some(5.0));
    }

    #[test]
    fn deleting_the_style_block_falls_back_to_ua_styling() {
        // The M5.9 acceptance scenario, at the unit level: no author
        // sheets at all still yields real (UA) styling, not blank defaults.
        let with_author = style_of("<h1>x</h1>", &[sheet("h1 { color: red; }")], "h1");
        let ua_only = style_of("<h1>x</h1>", &[], "h1");
        assert_eq!(with_author.font_size_px, ua_only.font_size_px); // both from ua.css
        assert_ne!(with_author.color, Color::BLACK); // author override present
        assert_eq!(ua_only.display, Display::Block); // ua.css alone still styles it
    }

    #[test]
    fn non_rendering_elements_default_to_display_none() {
        // Without this, `<style>`'s own CSS source text (or `<title>`'s,
        // `<script>`'s, ...) would compute as ordinary visible inline
        // content — a real bug caught at the full-pipeline level (M6)
        // before being traced back to a missing UA rule here.
        for tag in ["head", "style", "script", "title", "meta", "link"] {
            let html = format!("<{tag}>x</{tag}>");
            let s = style_of(&html, &[], tag);
            assert_eq!(s.display, Display::None, "{tag} should default to display:none");
        }
    }

    #[test]
    fn pre_defaults_to_white_space_pre() {
        let s = style_of("<pre>x</pre>", &[], "pre");
        assert_eq!(s.white_space, WhiteSpace::Pre);
        assert_eq!(s.display, Display::Block);
    }

    #[test]
    fn adding_a_color_rule_visibly_changes_computed_style() {
        let before = style_of("<p>x</p>", &[], "p");
        let after = style_of("<p>x</p>", &[sheet("p { color: red; }")], "p");
        assert_ne!(before.color, after.color);
    }
}
