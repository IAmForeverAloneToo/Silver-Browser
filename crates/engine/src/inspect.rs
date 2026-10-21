//! The DevTools' two ends in an engine (M36, `docs/design/devtools.md`):
//! the panel's page, which the browser alone shows and whose window has
//! the bridge to it; and the page inspected, which answers the panel's
//! requests about itself. The browser's router passes messages between
//! the two; neither end trusts what the other's text says.

use std::cell::RefCell;
use std::collections::HashSet;

use devtools::{Json, Request};
use dom::{Document, NodeData, NodeId};
use paint::Rect;

use crate::loader::{Incoming, LoadState};
use crate::{Engine, FontMeasurer, Handling, Navigation, Url, cascade_context};

/// The most answers and events a page inspected may have waiting for the
/// browser: it takes them each turn.
pub const MAX_INSPECTED_MESSAGES: usize = 1024;

impl Engine {
    /// Shows the DevTools panel's page (M36): the browser's own document,
    /// its scripts on whatever the browser's setting, its window given
    /// the global `devtools`. No navigation reaches it: this is its only
    /// way in.
    pub fn show_devtools_panel(&mut self) {
        let url = Url::parse(devtools::PANEL_URL).expect("the panel's URL parses");
        self.scripting = true;
        self.show_own_page(url, &devtools::frontend::page(), Handling::Replace);
    }

    /// Shows the browser's own page `body` at `url` (M36, M37): made from
    /// a response of the engine's own under the panel's policy, for
    /// `handling` once shown, its window given the bridge.
    pub(crate) fn show_own_page(&mut self, url: Url, body: &str, handling: Handling) {
        self.panel = true;
        self.answer_navigation(url, body, Some(devtools::PANEL_CSP), handling);
    }

    /// Navigates to `url`, for `handling`, answered by the engine itself
    /// with the HTML `body`, under the policy `csp` when given (M37): a
    /// document parsed as it arrives, its scripts run as any page's.
    pub(crate) fn answer_navigation(
        &mut self,
        url: Url,
        body: &str,
        csp: Option<&str>,
        handling: Handling,
    ) {
        self.cancel_navigation();
        let id = self.new_page_id();
        self.navigation = Some(Navigation {
            url: url.clone(),
            incoming: Incoming::Fetching(id),
            handling,
            started: self.time_origin(),
            creator: None,
            sandbox: self.creation_sandbox(),
            referrer: String::new(),
        });
        self.load_state = LoadState::Loading;
        let mut headers = net::HeaderList::new();
        headers.append(b"Content-Type", b"text/html;charset=utf-8");
        if let Some(csp) = csp {
            headers.append(b"Content-Security-Policy", csp.as_bytes());
        }
        let head = net::Response {
            url_list: vec![url],
            status: 200,
            status_message: b"OK".to_vec(),
            headers,
            body: Vec::new(),
            mime_type: net::MimeType::parse("text/html;charset=utf-8"),
            address_space: None,
            response_type: net::ResponseType::Basic,
            cors_exposed: Vec::new(),
        };
        self.document_head(id, head.clone());
        self.document_body(id, body.as_bytes());
        self.document_arrived(id, Ok(head));
    }

    /// Shows the browser's page at `url` (M37), for `handling`: its
    /// scripts run whatever the user's setting, which comes back with
    /// the next page.
    pub(crate) fn show_internal(&mut self, url: Url, handling: Handling) {
        let Some(body) = crate::internal::page(&url) else {
            return;
        };
        if self.own_scripting.is_none() {
            self.own_scripting = Some(self.scripting);
        }
        self.scripting = true;
        self.show_own_page(url, body, handling);
    }

    /// The user's scripting setting back, as an internal page is left.
    pub(crate) fn leave_internal(&mut self) {
        if let Some(scripting) = self.own_scripting.take() {
            self.scripting = scripting;
        }
    }

    /// An engine showing the DevTools panel's page (M36), on this one's
    /// fonts, clock, waker, preferences and scale, drawing in software.
    pub fn devtools_panel(&self) -> Engine {
        let mut panel = Engine::with_fonts(self.layout_fonts.clone());
        panel.set_clock(self.clock.clone());
        if let Some(waker) = &self.waker {
            panel.use_waker(waker.clone());
        }
        panel.set_preferences(self.preferences);
        panel.set_scale_factor(self.scale);
        panel.show_devtools_panel();
        panel
    }

    /// Whether the engine shows the DevTools panel's page.
    pub fn is_devtools_panel(&self) -> bool {
        self.panel
    }

    /// The panel's page hears `message` from the browser, as a task.
    pub fn devtools_send(&mut self, message: String) {
        if !self.panel || !self.shown.page.is_panel() {
            return;
        }
        let now = self.now();
        self.shown.scripts.post(now, js::Task::DevTools(message));
        self.wake();
    }

    /// What the panel's page sent the browser since this was last asked.
    pub fn take_devtools(&mut self) -> Vec<String> {
        if !self.shown.page.is_panel() {
            return Vec::new();
        }
        self.shown.scripts.take_devtools()
    }

    /// The panel's request `message` about the page shown: its answer is
    /// in [`Engine::take_inspected`]. What is not a request is dropped.
    pub fn inspect(&mut self, message: &str) {
        let Ok(request) = Request::parse(message) else {
            return;
        };
        let answer = match self.answer(&request) {
            Ok(result) => devtools::result(request.id, result),
            Err(why) => devtools::error(request.id, &why),
        };
        self.tell_panel(answer);
    }

    /// The answers and events for the panel since this was last asked.
    pub fn take_inspected(&mut self) -> Vec<String> {
        self.tell_dom_changes();
        std::mem::take(&mut self.inspected)
    }

    /// Queues `message` for the panel, unless too many wait.
    fn tell_panel(&mut self, message: String) {
        if self.inspected.len() < MAX_INSPECTED_MESSAGES {
            self.inspected.push(message);
        }
    }

    /// The DevTools console's `expression` run in the page shown: its
    /// value's preview, or its exception's.
    fn evaluate(&mut self, expression: &str) -> Result<Json, String> {
        if !self.scripts_run() || self.shown.made {
            return Err("scripts do not run on this page".to_owned());
        }
        let ready = self.ready_state();
        let window = self.window_env();
        let selected = self
            .selected
            .filter(|&node| self.shown.document.contains(node));
        let answer = self.shown.evaluate(
            &mut self.network,
            window,
            ready,
            expression.to_owned(),
            selected,
        );
        self.scripts_ran();
        let answer = answer.ok_or("the expression ran out of time")?;
        Json::parse(&answer).map_err(|why| why.to_string())
    }

    /// The result of `request`, or why there is none.
    fn answer(&mut self, request: &Request) -> Result<Json, String> {
        if let Some(answer) = self.inspect_dom(request) {
            return answer;
        }
        match request.method.as_str() {
            // The page's URL and title.
            "page.info" => Ok(Json::object([
                ("url", Json::from(self.url().map(Url::to_string))),
                ("title", Json::from(self.title())),
            ])),
            // An expression run in the page (M36.1).
            "console.evaluate" => {
                let expression = request
                    .param("expression")
                    .and_then(Json::as_str)
                    .ok_or("no expression")?;
                self.evaluate(expression)
            }
            method => Err(format!("no method {method}")),
        }
    }
}

/// The most nodes one `dom.tree` answer holds: past it, a node's
/// children are left for the panel to ask for.
pub const MAX_TREE_NODES: usize = 5000;

/// The most characters of a text node a tree's answer holds.
const MAX_TREE_TEXT: usize = 200;

impl Engine {
    /// The node of the protocol's number `value`, if it is one of the
    /// shown document's.
    fn node_param(&self, request: &Request, name: &str) -> Result<NodeId, String> {
        request
            .param(name)
            .and_then(Json::as_u64)
            .and_then(NodeId::from_bits)
            .filter(|&node| self.shown.document.contains(node))
            .ok_or_else(|| format!("no node {name}"))
    }

    /// The inspector's answers (M36.2), or `None` for a method of
    /// another domain's.
    fn inspect_dom(&mut self, request: &Request) -> Option<Result<Json, String>> {
        Some(match request.method.as_str() {
            "dom.tree" => self.dom_tree(request),
            // A node's ancestors, the document first.
            "dom.path" => self.node_param(request, "node").map(|node| {
                let document = &self.shown.document;
                let mut path: Vec<Json> = document
                    .ancestors(node)
                    .map(|ancestor| Json::from(ancestor.to_bits()))
                    .collect();
                path.reverse();
                Json::Array(path)
            }),
            "dom.select" => self.node_param(request, "node").map(|node| {
                self.selected = Some(node);
                self.set_highlight(Some(node));
                Json::Null
            }),
            "dom.highlight" => match request.param("node") {
                Some(Json::Null) | None => {
                    self.set_highlight(None);
                    Ok(Json::Null)
                }
                Some(_) => self.node_param(request, "node").map(|node| {
                    self.set_highlight(Some(node));
                    Json::Null
                }),
            },
            "dom.pick" => {
                self.picking = request.param("on").and_then(Json::as_bool) == Some(true);
                if !self.picking {
                    self.set_highlight(self.selected);
                }
                Ok(Json::Null)
            }
            "dom.setAttribute" => self.node_param(request, "node").and_then(|node| {
                let name = request
                    .param("name")
                    .and_then(Json::as_str)
                    .unwrap_or_default();
                let value = request
                    .param("value")
                    .and_then(Json::as_str)
                    .unwrap_or_default();
                self.set_attribute_for_devtools(node, name, Some(value))
            }),
            "dom.removeAttribute" => self.node_param(request, "node").and_then(|node| {
                let name = request
                    .param("name")
                    .and_then(Json::as_str)
                    .unwrap_or_default();
                self.set_attribute_for_devtools(node, name, None)
            }),
            "css.setStyle" => self.node_param(request, "node").and_then(|node| {
                let text = request
                    .param("text")
                    .and_then(Json::as_str)
                    .unwrap_or_default();
                let value = (!text.trim().is_empty()).then_some(text);
                self.set_attribute_for_devtools(node, "style", value)
            }),
            "css.matched" => self
                .node_param(request, "node")
                .and_then(|node| self.matched(node)),
            "css.computed" => self
                .node_param(request, "node")
                .and_then(|node| self.computed(node)),
            _ => return None,
        })
    }

    /// The tree under `node`, the document when none is named, opened
    /// where the panel opened it (`open`): the node itself is, and the
    /// root element and the body are when the panel asks for the whole.
    fn dom_tree(&mut self, request: &Request) -> Result<Json, String> {
        let root = match request.param("node") {
            Some(_) => self.node_param(request, "node")?,
            None => self.shown.document.root(),
        };
        let mut open: HashSet<NodeId> = request
            .param("open")
            .and_then(Json::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Json::as_u64)
            .filter_map(NodeId::from_bits)
            .collect();
        open.insert(root);
        let document = &self.shown.document;
        if request.param("node").is_none()
            && let Some(html) = document.document_element()
        {
            open.insert(html);
            if let Some(body) = document
                .children(html)
                .find(|&child| document.tag(child) == Some("body"))
            {
                open.insert(body);
            }
        }
        let mut budget = MAX_TREE_NODES;
        let tree = tree_json(document, root, &open, &mut budget);
        // The panel hears of the document's changes from now on.
        self.dom_told = Some(document.version());
        Ok(tree)
    }

    /// Sets or removes attribute `name` of element `node` as the user
    /// asked in the inspector: a change of the document's, as a script's
    /// is, but for its `style`, which no policy of the page's refuses.
    fn set_attribute_for_devtools(
        &mut self,
        node: NodeId,
        name: &str,
        value: Option<&str>,
    ) -> Result<Json, String> {
        if !self.shown.document.is_element(node) {
            return Err("not an element".to_owned());
        }
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name.contains(|c: char| c.is_whitespace() || "\"'>/=".contains(c)) {
            return Err(format!("'{name}' is not an attribute's name"));
        }
        if name == "style" {
            self.shown.page.set_cssom_style(node, true);
        }
        let done = self.mutate(|document| match value {
            Some(value) => document.set_attribute(node, &name, value),
            None => document.remove_attribute(node, &name),
        });
        done.map(|_| Json::Null).map_err(|why| format!("{why:?}"))
    }

    /// The rules that matched element `node`, the style attribute's apart.
    fn matched(&mut self, node: NodeId) -> Result<Json, String> {
        if !self.shown.document.is_element(node) {
            return Err("not an element".to_owned());
        }
        self.cached_styles();
        let viewport = self.viewport;
        let animations = RefCell::new(self.shown.animations.borrow().clone());
        let fonts = FontMeasurer(&self.layout_fonts);
        let context = cascade_context(
            viewport,
            &fonts,
            &self.shown.page,
            self.media(viewport),
            &animations,
        );
        let Some(cached) = &self.shown.styled else {
            return Err("no styles".to_owned());
        };
        let rules = self
            .shown
            .stylesheets
            .recascader(&self.shown.document, &cached.styles, &context)
            .matched_rules(node);
        let names = self.shown.page.sheet_names();
        let mut inline = Json::Null;
        let mut out = Vec::new();
        for rule in rules {
            let declarations = Json::Array(
                rule.declarations
                    .iter()
                    .map(|decl| {
                        Json::object([
                            ("name", Json::from(decl.name.as_str())),
                            ("value", Json::from(decl.value.as_str())),
                            ("important", Json::from(decl.important)),
                            ("overridden", Json::from(decl.overridden)),
                        ])
                    })
                    .collect(),
            );
            let source = match rule.origin {
                style::RuleOrigin::Inline => {
                    inline = Json::object([("declarations", declarations)]);
                    continue;
                }
                style::RuleOrigin::UserAgent => "user agent stylesheet".to_owned(),
                style::RuleOrigin::Hints => "attributes".to_owned(),
                style::RuleOrigin::Author(sheet) => names
                    .get(sheet)
                    .cloned()
                    .unwrap_or_else(|| "a stylesheet".to_owned()),
            };
            let (a, b, c) = rule.specificity;
            out.push(Json::object([
                ("source", Json::from(source)),
                (
                    "selector",
                    Json::from(match rule.origin {
                        style::RuleOrigin::Hints => "element".to_owned(),
                        _ => rule.selector,
                    }),
                ),
                (
                    "specificity",
                    Json::Array(vec![a.into(), b.into(), c.into()]),
                ),
                (
                    "conditions",
                    Json::Array(rule.conditions.into_iter().map(Json::from).collect()),
                ),
                ("declarations", declarations),
            ]));
        }
        Ok(Json::object([
            ("inline", inline),
            ("rules", Json::Array(out)),
        ]))
    }

    /// Element `node`'s computed values, as `getComputedStyle` gives them.
    fn computed(&mut self, node: NodeId) -> Result<Json, String> {
        let styles = self.cached_styles();
        let style = styles.get(node).ok_or("no style")?;
        Ok(Json::Array(
            style::serialize::COMPUTED_PROPERTIES
                .iter()
                .filter_map(|&name| {
                    let value = style::serialize::computed_value(style, name)?;
                    Some(Json::Array(vec![Json::from(name), Json::from(value)]))
                })
                .collect(),
        ))
    }

    /// Highlights `node` in the page, or nothing.
    fn set_highlight(&mut self, node: Option<NodeId>) {
        if self.highlighted != node {
            self.highlighted = node;
            self.changed = true;
            self.wake();
        }
    }

    /// While the inspector picks (M36.2), the pointer at `(x, y)` in the
    /// viewport highlights what is under it, and a press there picks it,
    /// the page hearing of neither: whether it was taken.
    pub(crate) fn pick_at(&mut self, x: f32, y: f32, pressed: bool) -> bool {
        if !self.picking {
            return false;
        }
        let node = self
            .hit_test(x, y)
            .filter(|&node| self.shown.document.is_element(node));
        self.set_highlight(node);
        if pressed && let Some(node) = node {
            self.picking = false;
            self.picked_press = true;
            self.selected = Some(node);
            let document = &self.shown.document;
            let mut path: Vec<Json> = document
                .ancestors(node)
                .map(|ancestor| Json::from(ancestor.to_bits()))
                .collect();
            path.reverse();
            self.tell_panel(devtools::event(
                "dom.picked",
                Json::object([
                    ("node", Json::from(node.to_bits())),
                    ("path", Json::Array(path)),
                ]),
            ));
        }
        true
    }

    /// The release of the press that picked, which the page does not hear.
    pub(crate) fn picked_release(&mut self) -> bool {
        std::mem::take(&mut self.picked_press)
    }

    /// The panel's event that the document changed since it was last
    /// read, once.
    fn tell_dom_changes(&mut self) {
        if let Some(told) = self.dom_told
            && told != self.shown.document.version()
        {
            self.dom_told = Some(self.shown.document.version());
            self.tell_panel(devtools::event("dom.updated", Json::object([])));
        }
    }

    /// `frame`, `width` by `height` raster pixels, with the highlighted
    /// node's boxes drawn over it, as the reference browser draws them:
    /// its content box blue, its padding green, its border yellow and
    /// its margin orange.
    pub(crate) fn overlaid(&mut self, width: u32, height: u32) -> Option<&[u8]> {
        let node = self.highlighted?;
        let (border, style) = self.box_model(node)?;
        let scale = self.scale;
        let side = |v: [f32; 4]| v.map(|v| v * scale);
        let rect = |r: Rect| [r.x * scale, r.y * scale, r.w * scale, r.h * scale];
        let px = |l: &style::computed::LengthPercentage| l.resolve(border.w);
        let margin = side([
            style.margin[0].resolve(border.w, 0.0),
            style.margin[1].resolve(border.w, 0.0),
            style.margin[2].resolve(border.w, 0.0),
            style.margin[3].resolve(border.w, 0.0),
        ]);
        let padding = side([
            px(&style.padding[0]),
            px(&style.padding[1]),
            px(&style.padding[2]),
            px(&style.padding[3]),
        ]);
        let borders = side(style.border_width);
        let outer = |r: [f32; 4], e: [f32; 4]| {
            [
                r[0] - e[3],
                r[1] - e[0],
                r[2] + e[1] + e[3],
                r[3] + e[0] + e[2],
            ]
        };
        let inner = |r: [f32; 4], e: [f32; 4]| {
            [
                r[0] + e[3],
                r[1] + e[0],
                (r[2] - e[1] - e[3]).max(0.0),
                (r[3] - e[0] - e[2]).max(0.0),
            ]
        };
        let border_box = rect(border);
        let margin_box = outer(border_box, margin);
        let padding_box = inner(border_box, borders);
        let content_box = inner(padding_box, padding);
        let frame = self.backend.last_frame();
        if frame.len() != width as usize * height as usize * 4 {
            return None;
        }
        self.overlay.clear();
        self.overlay.extend_from_slice(frame);
        let ring = [
            (margin_box, border_box, [246, 178, 107]),
            (border_box, padding_box, [255, 229, 153]),
            (padding_box, content_box, [147, 196, 125]),
        ];
        for (outside, inside, colour) in ring {
            fill_between(
                &mut self.overlay,
                width,
                height,
                outside,
                Some(inside),
                colour,
            );
        }
        fill_between(
            &mut self.overlay,
            width,
            height,
            content_box,
            None,
            [111, 168, 220],
        );
        Some(&self.overlay)
    }

    /// The border box of `node` in the viewport, and its style.
    fn box_model(&mut self, node: NodeId) -> Option<(Rect, style::ComputedStyle)> {
        let (border, _) = self.border_box(node)?;
        let style = self.cached_styles().get(node)?.clone();
        Some((border, style))
    }
}

/// Blends `colour` at two thirds over the pixels of `outside` but not of
/// `inside`, rectangles `[x, y, w, h]` in raster pixels.
fn fill_between(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    outside: [f32; 4],
    inside: Option<[f32; 4]>,
    colour: [u8; 3],
) {
    let span = |from: f32, len: f32, max: u32| {
        let start = from.round().clamp(0.0, max as f32) as u32;
        let end = (from + len).round().clamp(0.0, max as f32) as u32;
        (start, end)
    };
    let (x0, x1) = span(outside[0], outside[2], width);
    let (y0, y1) = span(outside[1], outside[3], height);
    let hole = inside.map(|r| (span(r[0], r[2], width), span(r[1], r[3], height)));
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(((hx0, hx1), (hy0, hy1))) = hole
                && (hx0..hx1).contains(&x)
                && (hy0..hy1).contains(&y)
            {
                continue;
            }
            let at = (y as usize * width as usize + x as usize) * 4;
            for (channel, c) in pixels[at..at + 3].iter_mut().zip(colour) {
                *channel = ((u16::from(*channel) + 2 * u16::from(c)) / 3) as u8;
            }
        }
    }
}

/// `node` and the tree under it as the panel reads it: each node its
/// number, type, name, attributes or text, and its children's count, and
/// the children of those `open` while `budget` lasts.
fn tree_json(
    document: &Document,
    node: NodeId,
    open: &HashSet<NodeId>,
    budget: &mut usize,
) -> Json {
    *budget = budget.saturating_sub(1);
    // White space between elements is left out, as the reference
    // browser leaves it out.
    let children: Vec<NodeId> = document
        .children(node)
        .filter(|&child| {
            document
                .text(child)
                .is_none_or(|text| !text.trim().is_empty())
        })
        .collect();
    let mut members = vec![("id", Json::from(node.to_bits()))];
    match document.data(node) {
        Some(NodeData::Element { tag, attrs, .. }) => {
            members.push(("type", Json::from(1u32)));
            members.push(("name", Json::from(tag.as_str())));
            members.push((
                "attributes",
                Json::Array(
                    attrs
                        .iter()
                        .map(|(name, value)| {
                            Json::Array(vec![Json::from(name.as_str()), Json::from(value.as_str())])
                        })
                        .collect(),
                ),
            ));
        }
        Some(NodeData::Text(text)) => {
            members.push(("type", Json::from(3u32)));
            members.push(("name", Json::from("#text")));
            let cut: String = text.chars().take(MAX_TREE_TEXT).collect();
            members.push(("text", Json::from(cut)));
        }
        Some(NodeData::Document) | None => {
            members.push(("type", Json::from(9u32)));
            members.push(("name", Json::from("#document")));
        }
    }
    members.push(("childCount", Json::from(children.len() as u64)));
    if open.contains(&node) && *budget > 0 {
        let kids = children
            .into_iter()
            .map(|child| tree_json(document, child, open, budget))
            .collect();
        members.push(("children", Json::Array(kids)));
    }
    Json::Object(
        members
            .into_iter()
            .map(|(n, v)| (n.to_owned(), v))
            .collect(),
    )
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;
