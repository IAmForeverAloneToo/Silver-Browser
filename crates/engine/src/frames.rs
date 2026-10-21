//! Frames (M33.2, `docs/design/hardening.md`): the document of an
//! `<iframe>`, shown by an engine of its own under the engine of the
//! document that has the element (D24), and the tree of a tab's frames
//! the engines share.
//!
//! A frame's engine shares its parent's fonts, cookies, cache, storage,
//! clock and waker, fetches on threads of its own, and draws in software;
//! its parent lays the element out as a replaced box, draws the frame's
//! pixels into its content box as it does a canvas's, waits for it in its
//! own `load`, and drives it: its tasks, its network, its frames.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dom::{Change, Document, NodeId};
use net::Url;

use crate::loader::{Creator, Incoming, LoadState, Page, PageId, Target, Waker};
use crate::sandbox::Sandbox;
use crate::scripts::ports::SharedRouter;
use crate::submit::Post;
use crate::{
    Engine, FrameTime, Handling, Navigation, Shown, fragment_of, parsed_or_empty, scripts,
};

/// The most frames a tab has, those of the documents its history keeps
/// included: past them an `iframe` stays empty (the reference browser
/// allows 1,000 a page; a frame here is a whole engine).
pub const MAX_FRAMES: usize = 100;

/// How deep frames nest: a frame this deep makes none of its own.
pub const MAX_FRAME_DEPTH: usize = 16;

/// The heap a frame's realm may use (M33.2), less than a top-level
/// document's `js::MAX_SCRIPT_HEAP`.
pub const MAX_FRAME_HEAP: usize = 64 << 20;

/// A frame's number in its tab: its navigable's, kept when it navigates.
pub(crate) type FrameId = u32;

/// The top-level document's frame.
pub(crate) const TOP: FrameId = 0;

/// The most tasks a frame's inbox holds: past them what other frames
/// send it is dropped.
pub const MAX_FRAME_TASKS: usize = 1024;

/// What one frame asked of another (M33.2), waiting in its inbox.
#[derive(Debug, Clone)]
pub(crate) enum FrameTask {
    /// A message for its window: its data serialized, the sender's
    /// origin serialized and its frame, and the target origin it was
    /// posted for, `*` or an origin's serialization.
    Message {
        data: String,
        origin: String,
        source: FrameId,
        target_origin: String,
    },
    /// A navigation.
    Navigate(Box<FrameNavigation>),
}

/// A navigation one frame asked of another (M33.2): to `url`, in place of
/// its entry when `replace`, with a form's POST, started by a document of
/// `origin`.
#[derive(Debug, Clone)]
pub(crate) struct FrameNavigation {
    pub(crate) url: Url,
    pub(crate) replace: bool,
    pub(crate) post: Option<Post>,
    pub(crate) origin: net::Origin,
}

/// A frame in the tree.
#[derive(Debug, Default)]
struct TreeNode {
    parent: Option<FrameId>,
    /// Its `iframe` in its parent's document.
    element: Option<NodeId>,
    /// Its element's `name`, which links, forms and named access find
    /// it by, and `window.name` as its scripts set it.
    name: String,
    window_name: Option<String>,
    /// Its document's origin and sandboxing flags.
    origin: Option<net::Origin>,
    sandbox: Sandbox,
    /// Its element's `sandbox` attribute's flags as it now stands, which
    /// its next navigation takes (M33.2).
    element_sandbox: Sandbox,
    /// The Local Network Access features its element's `allow` attribute
    /// names (M33.4).
    element_allow: Allowed,
    /// Its child frames, in the tree order of their elements.
    children: Vec<FrameId>,
    inbox: VecDeque<FrameTask>,
    /// Whether a script's question made it, which no engine has taken
    /// yet.
    reserved: bool,
}

/// The frames of a tab, which its engines share (M33.2).
pub(crate) struct FrameTree {
    next: FrameId,
    nodes: HashMap<FrameId, TreeNode>,
    /// The router of the ports of every frame's document and workers, so
    /// that a port goes from one frame to another.
    router: SharedRouter,
    /// In a renderer (M34): the browser's number for each frame, those
    /// whose documents other renderers show, and where what is posted to
    /// those goes.
    keys: HashMap<FrameId, u32>,
    remote: HashSet<FrameId>,
    forward: Option<Forward>,
}

/// Where a renderer sends what one of its frames posts to a frame another
/// renderer shows (M34): the browser's numbers of the frame posting, if
/// it has one, and of the frame posted to, and the task.
pub(crate) type Forward = Arc<dyn Fn(Option<u32>, u32, FrameTask) + Send + Sync>;

/// A tab's frame tree, as its engines hold it.
pub(crate) type SharedTree = Arc<Mutex<FrameTree>>;

impl FrameTree {
    /// A tab's tree: the top-level document's frame alone.
    pub(crate) fn shared() -> SharedTree {
        Arc::new(Mutex::new(FrameTree {
            next: TOP + 1,
            nodes: HashMap::from([(TOP, TreeNode::default())]),
            router: SharedRouter::default(),
            keys: HashMap::new(),
            remote: HashSet::new(),
            forward: None,
        }))
    }

    /// Frame `id` is the browser's frame `key` (M34).
    pub(crate) fn set_key(&mut self, id: FrameId, key: u32) {
        self.keys.insert(id, key);
    }

    /// The frame the browser numbers `key`, if it is in this tree.
    pub(crate) fn local_of(&self, key: u32) -> Option<FrameId> {
        self.keys
            .iter()
            .find(|(_, held)| **held == key)
            .map(|(id, _)| *id)
    }

    /// Frame `id`'s document is shown by another renderer, as the
    /// browser's frame `key`: what is posted to it goes there (M34).
    pub(crate) fn set_remote(&mut self, id: FrameId, key: u32) {
        self.keys.insert(id, key);
        self.remote.insert(id);
    }

    /// Frame `id`'s document is this renderer's again.
    pub(crate) fn set_local(&mut self, id: FrameId) {
        self.remote.remove(&id);
    }

    /// Sends what is posted to frames of other renderers through
    /// `forward`, once given.
    pub(crate) fn set_forward(&mut self, forward: Forward) {
        if self.forward.is_none() {
            self.forward = Some(forward);
        }
    }

    /// A frame of another renderer under `parent`, the top's when none
    /// (M34): the browser's frame `key`, its document of `origin`.
    pub(crate) fn add_stub(
        &mut self,
        parent: Option<FrameId>,
        key: u32,
        origin: net::Origin,
    ) -> FrameId {
        let id = match parent {
            None => TOP,
            Some(parent) => self.add(parent).unwrap_or(TOP),
        };
        if let Some(node) = self.nodes.get_mut(&id) {
            node.origin = Some(origin);
        }
        self.set_remote(id, key);
        id
    }

    /// A frame under `parent` for a renderer's frame root (M34), the
    /// browser's frame `key`.
    pub(crate) fn add_root(&mut self, parent: FrameId, key: u32) -> FrameId {
        let id = self.add(parent).unwrap_or(TOP);
        self.set_key(id, key);
        id
    }

    /// The tab's port router.
    pub(crate) fn router(&self) -> SharedRouter {
        self.router.clone()
    }

    /// A new frame under `parent`, unless the tab has [`MAX_FRAMES`].
    fn add(&mut self, parent: FrameId) -> Option<FrameId> {
        if self.frames() >= MAX_FRAMES {
            return None;
        }
        let mut id = self.next;
        while id == TOP || self.nodes.contains_key(&id) {
            id = id.wrapping_add(1);
        }
        self.next = id.wrapping_add(1);
        self.nodes.insert(
            id,
            TreeNode {
                parent: Some(parent),
                ..TreeNode::default()
            },
        );
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children.push(id);
        }
        Some(id)
    }

    /// Takes frame `id` out, and the frames under it.
    fn remove(&mut self, id: FrameId) {
        if id == TOP {
            return;
        }
        let mut gone = vec![id];
        let mut at = 0;
        while at < gone.len() {
            let parent = gone[at];
            gone.extend(
                self.nodes
                    .iter()
                    .filter(|(_, node)| node.parent == Some(parent))
                    .map(|(&child, _)| child),
            );
            at += 1;
        }
        for id in &gone {
            self.nodes.remove(id);
        }
        for node in self.nodes.values_mut() {
            node.children.retain(|child| !gone.contains(child));
        }
    }

    /// Frame `parent`'s children, in the tree order of their elements,
    /// each with its element and that element's `name`.
    pub(crate) fn set_children(&mut self, parent: FrameId, children: &[(FrameId, NodeId, String)]) {
        for (id, element, name) in children {
            if let Some(node) = self.nodes.get_mut(id) {
                node.element = Some(*element);
                node.name.clone_from(name);
            }
        }
        // A frame reserved for an element that has none now goes.
        let reserved: Vec<FrameId> = self
            .children(parent)
            .iter()
            .copied()
            .filter(|id| self.nodes.get(id).is_some_and(|node| node.reserved))
            .filter(|id| !children.iter().any(|(child, ..)| child == id))
            .collect();
        for id in reserved {
            self.remove(id);
        }
        // The document's frames alone: a document the history keeps
        // keeps its own out of the list until it is shown again.
        if let Some(node) = self.nodes.get_mut(&parent) {
            node.children = children.iter().map(|(id, ..)| *id).collect();
        }
    }

    /// The frame of `parent`'s `element`, made now in the tree when it
    /// has none yet (M33.2): an iframe a script inserted has its window
    /// before its parent's engine makes its frame, which takes the
    /// number. `None` past [`MAX_FRAMES`].
    pub(crate) fn reserve(
        &mut self,
        parent: FrameId,
        element: NodeId,
        name: &str,
    ) -> Option<FrameId> {
        if let Some(id) = self.of_element(parent, element) {
            return Some(id);
        }
        let id = self.add(parent)?;
        if let Some(node) = self.nodes.get_mut(&id) {
            node.element = Some(element);
            node.name = name.to_owned();
            node.reserved = true;
        }
        Some(id)
    }

    /// The frame `parent`'s engine makes for `element`: the number a
    /// script's question reserved for it, or a new one.
    fn claim(&mut self, parent: FrameId, element: NodeId) -> Option<FrameId> {
        if let Some(id) = self.of_element(parent, element)
            && let Some(node) = self.nodes.get_mut(&id)
            && node.reserved
        {
            node.reserved = false;
            return Some(id);
        }
        self.add(parent)
    }

    /// Frame `id`'s document's origin and sandboxing flags, a new
    /// document shown.
    pub(crate) fn set_document(&mut self, id: FrameId, origin: net::Origin, sandbox: Sandbox) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.origin = Some(origin);
            node.sandbox = sandbox;
        }
    }

    /// Frame `id`'s element's `sandbox` attribute's flags, and the
    /// features its `allow` attribute names (M33.4).
    pub(crate) fn set_element_policy(&mut self, id: FrameId, sandbox: Sandbox, allow: Allowed) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.element_sandbox = sandbox;
            node.element_allow = allow;
        }
    }

    /// The features a document of `origin` in frame `id` may use
    /// (M33.4), by Permissions Policy's default allowlist `'self'`: all
    /// at the top; in a frame, those its parent's document may use that
    /// its element's `allow` names, or all of those when its document is
    /// of its parent's document's origin.
    pub(crate) fn allowed(&self, id: FrameId, origin: &net::Origin) -> Allowed {
        let mut allowed = Allowed::ALL;
        let (mut at, mut origin) = (id, origin.clone());
        while let Some(parent) = self.parent(at) {
            let node = self.nodes.get(&at);
            let parents = self
                .nodes
                .get(&parent)
                .and_then(|node| node.origin.clone())
                .unwrap_or_else(net::Origin::new_opaque);
            if origin.is_opaque() || origin != parents {
                let named = node.map_or(Allowed::default(), |node| node.element_allow);
                allowed = allowed.and(named);
            }
            (at, origin) = (parent, parents);
        }
        allowed
    }

    /// The flags a new document of frame `id` is made with (HTML's
    /// "determine the creation sandboxing flags", M33.2): its element's
    /// and its parent's document's; none for the top's.
    pub(crate) fn creation_sandbox(&self, id: FrameId) -> Sandbox {
        let Some(node) = self.nodes.get(&id) else {
            return Sandbox::NONE;
        };
        let parent = node
            .parent
            .and_then(|parent| self.nodes.get(&parent))
            .map_or(Sandbox::NONE, |parent| parent.sandbox);
        node.element_sandbox.with(parent)
    }

    /// The origins of frame `id`'s ancestors' documents, its parent's
    /// first (M33.2): what `frame-ancestors` and `X-Frame-Options` check.
    pub(crate) fn ancestor_origins(&self, id: FrameId) -> Vec<net::Origin> {
        let mut origins = Vec::new();
        let mut at = self.parent(id);
        while let Some(ancestor) = at {
            let origin = self
                .nodes
                .get(&ancestor)
                .and_then(|node| node.origin.clone())
                .unwrap_or_else(net::Origin::new_opaque);
            origins.push(origin);
            at = self.parent(ancestor);
        }
        origins
    }

    pub(crate) fn contains(&self, id: FrameId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub(crate) fn parent(&self, id: FrameId) -> Option<FrameId> {
        self.nodes.get(&id)?.parent
    }

    /// Frame `id`'s top: the tab's top-level frame.
    pub(crate) fn top(&self, id: FrameId) -> FrameId {
        let mut at = id;
        while let Some(parent) = self.parent(at) {
            at = parent;
        }
        at
    }

    pub(crate) fn children(&self, id: FrameId) -> &[FrameId] {
        self.nodes.get(&id).map_or(&[], |node| &node.children)
    }

    /// Frame `id`'s child whose element is named `name`.
    pub(crate) fn child_named(&self, id: FrameId, name: &str) -> Option<FrameId> {
        if name.is_empty() {
            return None;
        }
        self.children(id)
            .iter()
            .copied()
            .find(|child| self.nodes.get(child).is_some_and(|node| node.name == name))
    }

    /// The child frame of frame `id` its document's `element` shows.
    pub(crate) fn of_element(&self, id: FrameId, element: NodeId) -> Option<FrameId> {
        self.children(id).iter().copied().find(|child| {
            self.nodes
                .get(child)
                .is_some_and(|node| node.element == Some(element))
        })
    }

    /// `window.name` of frame `id`: what its scripts set, else its
    /// element's `name`.
    pub(crate) fn window_name(&self, id: FrameId) -> String {
        self.nodes.get(&id).map_or_else(String::new, |node| {
            node.window_name
                .clone()
                .unwrap_or_else(|| node.name.clone())
        })
    }

    pub(crate) fn set_window_name(&mut self, id: FrameId, name: String) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.window_name = Some(name);
        }
    }

    /// Puts `task` in frame `id`'s inbox: false when the frame is gone or
    /// its inbox full.
    pub(crate) fn post(&mut self, id: FrameId, task: FrameTask) -> bool {
        if self.remote.contains(&id) {
            let (Some(to), Some(forward)) = (self.keys.get(&id).copied(), self.forward.clone())
            else {
                return false;
            };
            let from = match &task {
                FrameTask::Message { source, .. } => self.keys.get(source).copied(),
                FrameTask::Navigate(_) => None,
            };
            forward(from, to, task);
            return true;
        }
        match self.nodes.get_mut(&id) {
            Some(node) if node.inbox.len() < MAX_FRAME_TASKS => {
                node.inbox.push_back(task);
                true
            }
            _ => false,
        }
    }

    /// What other frames asked of frame `id` since it last looked.
    pub(crate) fn take_inbox(&mut self, id: FrameId) -> Vec<FrameTask> {
        self.nodes
            .get_mut(&id)
            .map_or_else(Vec::new, |node| node.inbox.drain(..).collect())
    }

    /// Whether frame `id` has tasks waiting.
    pub(crate) fn has_tasks(&self, id: FrameId) -> bool {
        self.nodes
            .get(&id)
            .is_some_and(|node| !node.inbox.is_empty())
    }

    /// Whether frame `source`, whose document is of `origin` and has the
    /// flags `sandbox`, may navigate frame `target` (M33.2), as the
    /// reference browser allows: itself,
    /// one of its descendants, its top, or a frame some ancestor of which,
    /// itself included, is of its origin. A sandboxed document only
    /// itself, its descendants, and its top as its flags allow, by the
    /// user's activation or not (HTML's "allowed by sandboxing to
    /// navigate").
    pub(crate) fn may_navigate(
        &self,
        source: FrameId,
        target: FrameId,
        (origin, sandbox): (&net::Origin, Sandbox),
        activated: bool,
    ) -> bool {
        if !self.contains(target) {
            return false;
        }
        if target == source {
            return true;
        }
        if target == self.top(source) {
            return !sandbox.has(match activated {
                true => Sandbox::TOP_NAVIGATION_BY_USER,
                false => Sandbox::TOP_NAVIGATION,
            });
        }
        let mut at = Some(target);
        while let Some(id) = at {
            if id == source {
                return true;
            }
            if !sandbox.has(Sandbox::NAVIGATION)
                && !origin.is_opaque()
                && self
                    .nodes
                    .get(&id)
                    .and_then(|node| node.origin.as_ref())
                    .is_some_and(|other| other == origin)
            {
                return true;
            }
            at = self.parent(id);
        }
        false
    }

    /// The top-level document's origin.
    pub(crate) fn top_origin(&self) -> net::Origin {
        self.nodes
            .get(&TOP)
            .and_then(|node| node.origin.clone())
            .unwrap_or_else(net::Origin::new_opaque)
    }

    /// Where frame `id`, whose document is of `origin`, is in the tab
    /// (M33.2): the top-level document's origin, and whether its
    /// document or an ancestor's is of another site.
    pub(crate) fn partition(&self, id: FrameId, origin: &net::Origin) -> net::Partition {
        let top = self.top_origin();
        let mut cross_site = !top.same_site(origin);
        let mut at = self.parent(id);
        while let Some(ancestor) = at.filter(|&ancestor| ancestor != TOP) {
            let other = self
                .nodes
                .get(&ancestor)
                .and_then(|node| node.origin.as_ref());
            cross_site |= other.is_none_or(|other| !top.same_site(other));
            at = self.parent(ancestor);
        }
        net::Partition { top, cross_site }
    }

    /// The frame of the tab named `name` by its element, the nearest the
    /// top first.
    pub(crate) fn find_named(&self, name: &str) -> Option<FrameId> {
        if name.is_empty() {
            return None;
        }
        let mut queue = VecDeque::from([TOP]);
        while let Some(id) = queue.pop_front() {
            if id != TOP && self.nodes.get(&id).is_some_and(|node| node.name == name) {
                return Some(id);
            }
            queue.extend(self.children(id).iter().copied());
        }
        None
    }

    /// How many frames the tab has, the top-level document's apart.
    pub(crate) fn frames(&self) -> usize {
        self.nodes.len() - 1
    }

    /// How deep the tab's frames nest: 0 with none.
    pub(crate) fn depth(&self) -> usize {
        let depth_of = |mut at: FrameId| {
            let mut depth = 0;
            while let Some(parent) = self.parent(at) {
                (depth, at) = (depth + 1, parent);
            }
            depth
        };
        self.nodes.keys().map(|&id| depth_of(id)).max().unwrap_or(0)
    }
}

/// The tree, locked.
pub(crate) fn lock(tree: &SharedTree) -> MutexGuard<'_, FrameTree> {
    tree.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where an engine is among its tab's frames.
pub(crate) struct FrameContext {
    pub(crate) tree: SharedTree,
    pub(crate) id: FrameId,
    /// The browser's number for the frame, in a renderer (M34); 0 in one
    /// process.
    pub(crate) key: u32,
    /// How many frames it is inside: 0 for the top-level document's.
    pub(crate) depth: usize,
    /// The URLs, without their fragments, of the documents of its
    /// ancestors, its parent's first: none of them is loaded in it (HTML's
    /// guard against a frame embedding itself).
    pub(crate) ancestors: Vec<String>,
    /// Its parent's document, which its navigations are made from for
    /// mixed content and the address spaces (none at the top), and that
    /// document's policies, whose `frame-src` they pass (M33.1), and what
    /// lets them ask their way into a more private space (M33.4).
    pub(crate) container: Option<net::Initiator>,
    pub(crate) container_csp: Option<net::csp::Csp>,
    pub(crate) container_local_network: Option<net::LocalNetworkAccess>,
}

/// The Local Network Access features a document may use (M33.4), and
/// those an `iframe`'s `allow` attribute names: Permissions Policy's for
/// these alone (G96).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Allowed {
    pub(crate) local: bool,
    pub(crate) loopback: bool,
}

impl Allowed {
    pub(crate) const ALL: Allowed = Allowed {
        local: true,
        loopback: true,
    };

    /// The features of an `allow` attribute's value: its directives split
    /// on `;`, each a feature's name and an allowlist. A feature named
    /// with any allowlist but `'none'` is taken, its origins not matched
    /// against the frame's; `local-network-access`, the reference
    /// browser's older name, is both.
    pub(crate) fn parse(value: &str) -> Allowed {
        let mut allowed = Allowed::default();
        for directive in value.split(';') {
            let mut tokens = directive.split_ascii_whitespace();
            let Some(feature) = tokens.next() else {
                continue;
            };
            if tokens.eq(["'none'"]) {
                continue;
            }
            match feature {
                "local-network" => allowed.local = true,
                "loopback-network" => allowed.loopback = true,
                "local-network-access" => allowed = Allowed::ALL,
                _ => {}
            }
        }
        allowed
    }

    fn and(self, other: Allowed) -> Allowed {
        Allowed {
            local: self.local && other.local,
            loopback: self.loopback && other.loopback,
        }
    }
}

/// What a run's scripts are lent of the tab's frames (M33.2): the tree,
/// their window's frame, and what wakes the tab when they send another
/// frame something.
#[derive(Clone)]
pub(crate) struct FramesEnv {
    pub(crate) tree: SharedTree,
    pub(crate) id: FrameId,
    pub(crate) wake: Option<Waker>,
}

impl FrameContext {
    /// A top-level document's: the tab's, in a new tree.
    pub(crate) fn top() -> FrameContext {
        FrameContext {
            tree: FrameTree::shared(),
            id: TOP,
            key: 0,
            depth: 0,
            ancestors: Vec::new(),
            container: None,
            container_csp: None,
            container_local_network: None,
        }
    }

    /// Whether the engine shows a frame's document, not a tab's.
    pub(crate) fn is_frame(&self) -> bool {
        self.id != TOP
    }
}

/// What an `iframe`'s attributes navigate its frame to (HTML section
/// 4.8.5, "process the iframe attributes").
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Source {
    /// No `src`, an empty one or `about:blank`: the frame's initial
    /// `about:blank` stays.
    Blank,
    /// A `srcdoc`, which wins over `src`.
    Srcdoc(String),
    /// A `src` parsed against the parent's base URL.
    Url(Url),
}

/// A frame whose document another renderer shows (M34): the browser's
/// number for it, and how many of its documents loaded, and how many of
/// those its element was told of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RemoteFrame {
    pub(crate) key: u32,
    loads: u32,
    told: u32,
}

/// What a frame of another renderer is drawn as in its parent's page
/// (M34, D32): a colour no page is taken to draw, which the browser
/// replaces with the frame's pixels where it shows.
pub const HOLE: [u8; 4] = [1, 254, 3, 255];

/// An `iframe`'s frame: its engine, and what its element knows of it.
pub(crate) struct Frame {
    pub(crate) id: FrameId,
    /// Its engine; once another renderer shows its documents, one that
    /// shows nothing.
    pub(crate) engine: Box<Engine>,
    pub(crate) remote: Option<RemoteFrame>,
    /// What the element's attributes last navigated it to.
    pub(crate) source: Source,
    /// The document of the frame whose load the element was told of: its
    /// `load` fired for it. Each new document is told again, whatever
    /// navigated the frame (M33.8).
    pub(crate) told: Option<PageId>,
    /// The frame's last drawing.
    pub(crate) drawn: Option<Drawn>,
    tree: SharedTree,
}

impl Frame {
    /// Whether the element was told its frame's document loaded.
    pub(crate) fn told_of_its_document(&self) -> bool {
        match &self.remote {
            Some(remote) => remote.told > 0 && remote.told == remote.loads,
            None => self.told == Some(self.engine.shown.page.id()),
        }
    }

    /// Whether the element is to be told its frame's document loaded:
    /// a load completed since it was last told.
    fn to_tell(&self) -> bool {
        match &self.remote {
            Some(remote) => remote.loads > remote.told,
            None => self.engine.completed.is_some() && self.engine.completed != self.told,
        }
    }

    /// Whether the frame has no navigation under way and its element was
    /// told its document loaded. A frame whose navigation was handed to
    /// another renderer has one under way until the browser makes it
    /// remote (M34).
    pub(crate) fn settled(&self) -> bool {
        match &self.remote {
            Some(_) => self.told_of_its_document(),
            None => {
                self.told_of_its_document()
                    && self.engine.frame_settled()
                    && !self.engine.handed_over
            }
        }
    }
}

impl Drop for Frame {
    /// The frame leaves the tree, and the frames under it.
    fn drop(&mut self) {
        lock(&self.tree).remove(self.id);
    }
}

/// A frame's drawing: its size in raster pixels, the frame it is shown
/// as, and its pixels.
pub(crate) struct Drawn {
    size: (u32, u32),
    frame: u32,
    pixels: Arc<images::Image>,
}

/// Moved on by each drawing of any frame in any engine: the frame its
/// image is shown as, so that no drawing is taken for another's, as
/// `SVG_FRAMES` does for an outer `<svg>`'s (the GPU's textures are kept
/// by image and frame).
static FRAME_DRAWINGS: AtomicU32 = AtomicU32::new(0);

/// The target of a link or a form's element (HTML section 4.6.4): its
/// `target`, else the document's base target (M33.2).
pub(crate) fn element_target(document: &Document, element: NodeId) -> String {
    match document.attr(element, "target") {
        Some(target) => target.to_owned(),
        None => base_target(document),
    }
}

/// The document's base target: the `target` of its first `<base>` that
/// has one, else empty.
pub(crate) fn base_target(document: &Document) -> String {
    document
        .subtree(document.root())
        .find(|&node| document.tag(node) == Some("base") && document.attr(node, "target").is_some())
        .and_then(|node| document.attr(node, "target"))
        .unwrap_or_default()
        .to_owned()
}

/// Whether `node` is an `iframe` in `document`.
fn is_iframe(document: &Document, node: NodeId) -> bool {
    document.tag(node) == Some("iframe")
}

/// Whether `change` may change the frames: an `iframe` in or out, or an
/// attribute its frame is navigated by.
pub(crate) fn may_touch_frames(document: &Document, change: &Change) -> bool {
    match change {
        Change::Inserted { child, .. } | Change::Removed { child, .. } => document
            .subtree(*child)
            .any(|node| is_iframe(document, node)),
        Change::AttributeChanged { element, name, .. } => {
            is_iframe(document, *element)
                && matches!(name.as_str(), "src" | "srcdoc" | "name" | "sandbox")
        }
        Change::Everything => true,
        Change::TextChanged { .. }
        | Change::StateChanged { .. }
        | Change::ControlChanged { .. } => false,
    }
}

/// The `iframe`s `changes` took out of `document`, put back or not:
/// their frames are dropped, a new one made for one put back, as HTML's
/// removal and insertion steps do.
pub(crate) fn removed_frames(document: &Document, changes: &[Change]) -> HashSet<NodeId> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Removed { child, .. } => Some(*child),
            _ => None,
        })
        .flat_map(|child| document.subtree(child))
        .filter(|&node| is_iframe(document, node))
        .collect()
}

impl Engine {
    /// An engine for frame `id` under this one's document (M33.2): the
    /// fonts, cookies, cache, storage, clock, waker and settings of this
    /// one, its own fetch threads and software rasterizer, and a smaller
    /// heap for its realm.
    fn child_engine(&mut self, id: FrameId) -> Option<Engine> {
        let fonts = self.layout_fonts.clone();
        let backend = Box::new(render::SoftwareBackend::with_fonts(fonts.clone()));
        let mut child = Engine::with_backend(fonts, backend);
        child.network = self.network.for_frame();
        child.clock = self.clock.clone();
        child.share_storage(self.storage.clone(), self.tab);
        child.areas = self.areas.clone();
        child.scripting = self.scripting;
        child.script_limits = js::Limits {
            heap: self.script_limits.heap.min(MAX_FRAME_HEAP),
            ..self.script_limits
        };
        child.compat_user_agent = self.compat_user_agent;
        child.preferences = self.preferences;
        child.media_type = self.media_type;
        child.scale = self.scale;
        child.hidden = self.hidden;
        // A frame's window has the focus only while the focus is in it.
        child.window_focused = false;
        child.viewport = (300, 150);
        child.modifiers = self.modifiers;
        if let Some(waker) = self.waker.clone() {
            child.use_waker(waker);
        }
        let mut ancestors = Vec::with_capacity(self.frame.ancestors.len() + 1);
        if let Some(url) = self.shown.page.url() {
            ancestors.push(url.without_fragment().to_owned());
        }
        ancestors.extend(self.frame.ancestors.iter().cloned());
        child.frame = FrameContext {
            tree: self.frame.tree.clone(),
            id,
            key: 0,
            depth: self.frame.depth + 1,
            ancestors,
            container: self.shown.page.initiator(),
            container_csp: Some(self.shown.page.csp(&self.shown.document).clone()),
            container_local_network: self.shown.page.local_network().cloned(),
        };
        // In a renderer (M34), the browser numbers the frame: its fetches,
        // cookies and storage are asked for by that number.
        if let Some(link) = self.link.clone() {
            let key = link.new_frame(self.frame.key)?;
            child.set_link(link, key);
        }
        Some(child)
    }

    /// The features the `iframe` `node`'s `allow` attribute names (M33.4).
    fn element_allow(&self, node: NodeId) -> Allowed {
        self.shown
            .document
            .attr(node, "allow")
            .map_or(Allowed::default(), Allowed::parse)
    }

    /// What lets a navigation of this engine's frame to `url` ask its way
    /// into a more private address space (M33.4): its container's
    /// document asks, by the features the frame's element and origins
    /// give a document of the URL's origin there.
    pub(crate) fn frame_local_network(&self, url: &Url) -> Option<net::LocalNetworkAccess> {
        let access = self.frame.container_local_network.clone()?;
        let allowed = lock(&self.frame.tree).allowed(self.frame.id, &url.origin());
        Some(net::LocalNetworkAccess {
            local: allowed.local,
            loopback: allowed.loopback,
            ..access
        })
    }

    /// Gives `page`, a document coming to this engine's frame, what lets
    /// its requests ask their way into a more private address space
    /// (M33.4), and its network the same: a secure context's, by the
    /// features its frames' origins and `allow` attributes give it; the
    /// tab's top-level origin is the one the user is asked about.
    pub(crate) fn place_local_network(&mut self, page: &mut Page) {
        let access = page.is_secure_context().then(|| {
            let tree = lock(&self.frame.tree);
            let allowed = tree.allowed(self.frame.id, page.origin());
            let origin = match self.frame.is_frame() {
                true => tree.top_origin(),
                false => page.origin().clone(),
            };
            net::LocalNetworkAccess {
                origin,
                local: allowed.local,
                loopback: allowed.loopback,
            }
        });
        self.network.set_local_network(page.id(), access.clone());
        page.set_local_network(access);
    }

    /// The flags of the `iframe` `node`'s `sandbox` attribute (M33.2):
    /// none without one.
    fn element_sandbox(&self, node: NodeId) -> Sandbox {
        self.shown
            .document
            .attr(node, "sandbox")
            .map_or(Sandbox::NONE, Sandbox::parse)
    }

    /// The sandboxing flags this engine's frame's next document is made
    /// with (M33.2): none at the top.
    pub(crate) fn creation_sandbox(&self) -> Sandbox {
        lock(&self.frame.tree).creation_sandbox(self.frame.id)
    }

    /// What the `iframe` `node`'s attributes navigate its frame to.
    fn frame_source(&self, node: NodeId) -> Source {
        let document = &self.shown.document;
        if let Some(srcdoc) = document.attr(node, "srcdoc") {
            return Source::Srcdoc(srcdoc.to_owned());
        }
        match document.attr(node, "src").filter(|src| !src.is_empty()) {
            Some(src) => match self.shown.page.resolve(document, src) {
                Some(url) if url.as_str() != "about:blank" => Source::Url(url),
                _ => Source::Blank,
            },
            None => Source::Blank,
        }
    }

    /// Makes, navigates and drops the frames of the document shown as
    /// its `iframe`s now stand (M33.2): one made for each connected
    /// `iframe` that has none, while the tab's limits allow, and
    /// navigated as its attributes say; one navigated again when they
    /// changed; one dropped when its element left.
    pub(crate) fn update_frames(&mut self) {
        let document = &self.shown.document;
        let iframes: Vec<NodeId> = if self.shown.made {
            Vec::new()
        } else {
            document
                .subtree(document.root())
                .filter(|&node| is_iframe(document, node))
                .collect()
        };
        let gone: Vec<NodeId> = self
            .shown
            .frames
            .keys()
            .filter(|node| !iframes.contains(node))
            .copied()
            .collect();
        for node in gone {
            self.drop_frame(node);
        }
        for node in iframes {
            let source = self.frame_source(node);
            // A change of its `sandbox` applies at the frame's next
            // navigation (M33.2).
            if let Some(frame) = self.shown.frames.get(&node) {
                let (sandbox, allow) = (self.element_sandbox(node), self.element_allow(node));
                lock(&self.frame.tree).set_element_policy(frame.id, sandbox, allow);
            }
            match self.shown.frames.get(&node) {
                Some(frame) if frame.source == source => continue,
                Some(_) => {}
                None => {
                    let Some(frame) = self.make_frame(node) else {
                        continue;
                    };
                    self.shown.frames.insert(node, frame);
                }
            }
            self.navigate_frame(node, source);
        }
        self.sync_frame_children();
    }

    /// Gives the tree this document's frames as its frame's children, in
    /// tree order, with their elements' names, and its realm the same.
    pub(crate) fn sync_frame_children(&mut self) {
        let document = &self.shown.document;
        let children: Vec<(FrameId, NodeId, String)> = self
            .frames_in_order()
            .into_iter()
            .filter_map(|node| {
                let frame = self.shown.frames.get(&node)?;
                let name = document.attr(node, "name").unwrap_or_default().to_owned();
                Some((frame.id, node, name))
            })
            .collect();
        lock(&self.frame.tree).set_children(self.frame.id, &children);
        let names = children.into_iter().map(|(.., name)| name).collect();
        self.shown.scripts.set_frame_list(names);
    }

    /// A frame for the `iframe` `node`, showing the initial `about:blank`
    /// of this document's origin, unless the tab's limits refuse it.
    fn make_frame(&mut self, node: NodeId) -> Option<Frame> {
        let id = if self.frame.depth >= MAX_FRAME_DEPTH {
            Err(format!("frames nest {MAX_FRAME_DEPTH} deep at most"))
        } else {
            // A number a script's question reserved for it, or a new one.
            lock(&self.frame.tree)
                .claim(self.frame.id, node)
                .ok_or_else(|| format!("a tab has {MAX_FRAMES} frames at most"))
        };
        let id = match id {
            Ok(id) => id,
            Err(why) => {
                let text = format!("An iframe was left empty: {why}");
                self.shown.scripts.say(js::ConsoleLevel::Error, text);
                return None;
            }
        };
        let (sandbox, allow) = (self.element_sandbox(node), self.element_allow(node));
        lock(&self.frame.tree).set_element_policy(id, sandbox, allow);
        let Some(engine) = self.child_engine(id) else {
            lock(&self.frame.tree).remove(id);
            let text = "An iframe was left empty: the browser gave its frame no number".to_owned();
            self.shown.scripts.say(js::ConsoleLevel::Error, text);
            return None;
        };
        let mut engine = Box::new(engine);
        let creator = self.shown.page.creator(&self.shown.document);
        engine.show_blank(&creator, true);
        Some(Frame {
            id,
            engine,
            remote: None,
            source: Source::Blank,
            told: None,
            drawn: None,
            tree: self.frame.tree.clone(),
        })
    }

    /// Navigates the frame of the `iframe` `node` to `source`.
    fn navigate_frame(&mut self, node: NodeId, source: Source) {
        let creator = self.shown.page.creator(&self.shown.document);
        let initiator = self.shown.page.initiator();
        // Its element's `referrerpolicy` (M33.3).
        let policy = self
            .shown
            .document
            .attr(node, "referrerpolicy")
            .and_then(net::ReferrerPolicy::parse);
        let current = self
            .shown
            .page
            .url()
            .map(|url| url.without_fragment().to_owned());
        let local_network = self.shown.page.local_network().cloned();
        let ancestors = &self.frame.ancestors;
        if let (Some(link), Some(frame)) = (&self.link, self.shown.frames.get(&node)) {
            let sandbox = frame.engine.creation_sandbox().bits();
            let _ = link.send(&crate::ipc::ToBrowser::FramePolicy {
                key: frame.engine.frame.key,
                sandbox,
            });
        }
        let Some(frame) = self.shown.frames.get_mut(&node) else {
            return;
        };
        frame.engine.frame.container.clone_from(&initiator);
        frame.engine.frame.container_csp = Some(creator.csp.clone());
        frame
            .engine
            .frame
            .container_local_network
            .clone_from(&local_network);
        frame.source = source.clone();
        // What its frame showed before is never told now: the initial
        // `about:blank` hears no `load` when a navigation follows (HTML's
        // "process the iframe attributes"); a `javascript:` URL brings no
        // document here, and it is still told.
        if !matches!(&source, Source::Url(url) if url.scheme() == "javascript") {
            frame.told = frame.engine.completed;
        }
        frame.drawn = None;
        if let Some(remote) = frame.remote
            && matches!(source, Source::Blank | Source::Srcdoc(_))
        {
            frame.remote = None;
            lock(&self.frame.tree).set_local(frame.id);
            self.dropped_remote.push(remote.key);
        }
        let Some(frame) = self.shown.frames.get_mut(&node) else {
            return;
        };
        match source {
            Source::Blank => frame.engine.show_blank(&creator, false),
            Source::Srcdoc(text) => frame.engine.show_srcdoc(text, creator),
            Source::Url(url) => {
                // No frame shows a document an ancestor shows (HTML 7.4.2,
                // "navigate", the recursion check).
                let key = url.without_fragment();
                if current.as_deref() == Some(key) || ancestors.iter().any(|a| a == key) {
                    frame.engine.show_blank(&creator, false);
                    return;
                }
                frame
                    .engine
                    .navigate_as_frame(url, creator, initiator, policy);
            }
        }
    }

    /// Drops the frame of the `iframe` `node`, its document unloaded:
    /// what it wrote to its console as it went is the tab's.
    pub(crate) fn drop_frame(&mut self, node: NodeId) {
        let Some(mut frame) = self.shown.frames.remove(&node) else {
            return;
        };
        if let Some(remote) = frame.remote {
            self.dropped_remote.push(remote.key);
        }
        frame.engine.unload_shown(false);
        frame.engine.stop();
        let lines = frame.engine.take_console();
        self.departed.extend(lines);
    }

    /// Unloads the document shown (M33.2): a frame's, dropped with it or
    /// left with its parent's.
    pub(crate) fn unload_shown(&mut self, kept: bool) {
        let placeholder = Shown::new(Document::new(), Page::given(0));
        let mut shown = std::mem::replace(&mut self.shown, placeholder);
        self.unload(&mut shown, kept);
        self.shown = shown;
    }

    /// Shows an `about:blank` of `creator`'s origin: a frame's initial
    /// one when `initial`, which its first navigation replaces.
    fn show_blank(&mut self, creator: &Creator, initial: bool) {
        self.cancel_navigation();
        let url = Url::parse("about:blank").expect("about:blank parses");
        let id = self.new_page_id();
        let mut page = Page::fetched(
            id,
            url.clone(),
            encoding::UTF_8,
            None,
            creator.address_space,
        )
        .inheriting(creator)
        .with_sandbox(self.creation_sandbox());
        let partition = self.partition_for(page.origin());
        self.network.set_partition(id, partition);
        self.place_local_network(&mut page);
        let document = parsed_or_empty("");
        let media = self.media(self.viewport);
        page.begin(&document, &mut self.network, &media);
        let mut shown = Shown::new(document, page);
        shown.initial = initial;
        self.commit(shown, Some(url), Handling::Replace);
    }

    /// Shows `text` as the frame's `about:srcdoc` document, of `creator`'s
    /// origin, base URL and policies, parsed and its scripts run as a
    /// response's would be.
    fn show_srcdoc(&mut self, text: String, creator: Creator) {
        self.cancel_navigation();
        let url = Url::parse("about:srcdoc").expect("about:srcdoc parses");
        let response = net::Response {
            url_list: vec![url.clone()],
            status: 200,
            status_message: b"OK".to_vec(),
            headers: net::HeaderList::default(),
            body: Vec::new(),
            mime_type: net::MimeType::parse("text/html;charset=utf-8"),
            address_space: None,
            response_type: net::ResponseType::Basic,
            cors_exposed: Vec::new(),
        };
        let id = self.new_page_id();
        self.navigation = Some(Navigation {
            url,
            incoming: Incoming::Receiving {
                id,
                head: Box::new(response),
                body: text.into_bytes(),
                sniffed: false,
            },
            handling: Handling::Replace,
            started: self.time_origin(),
            creator: Some(creator),
            sandbox: self.creation_sandbox(),
            referrer: String::new(),
        });
        self.load_state = LoadState::Loading;
        self.sniff_incoming(true);
    }

    /// Navigates the frame to `url`, as its element's `src` asks: a
    /// request of destination `iframe` its parent's document makes, as
    /// `initiator`, so that mixed content and the address spaces are
    /// checked for it. A local URL's document takes `creator`'s policies;
    /// `about:` URLs but `about:blank` are refused, and a parent with no
    /// URL fetches nothing.
    fn navigate_as_frame(
        &mut self,
        url: Url,
        creator: Creator,
        initiator: Option<net::Initiator>,
        policy: Option<net::ReferrerPolicy>,
    ) {
        // A fragment of the document shown is gone to, not fetched again
        // (HTML's "navigate to a fragment", M33.8).
        let shown = self.shown.page.url().map(Url::without_fragment);
        if url.fragment().is_some() && shown == Some(url.without_fragment()) {
            self.navigate_to_fragment(url, Handling::Push);
            return;
        }
        self.cancel_navigation();
        if url.scheme() == "javascript" {
            if self.scripts_run() {
                self.run_javascript_url(&url);
            }
            return;
        }
        if url.scheme() == "about" {
            let error = crate::LoadError::Network(net::NetworkError::BadScheme("about".to_owned()));
            let shown = self.made(crate::pages::error(&url, &error), Some(error));
            self.commit(shown, Some(url), Handling::Replace);
            return;
        }
        let Some(initiator) = initiator else {
            return;
        };
        let local = matches!(url.scheme(), "data" | "blob");
        let id = self.new_page_id();
        // Its container's referrer, by its element's policy or the
        // container's (M33.3).
        let referrer_policy = policy.unwrap_or(creator.referrer_policy);
        let referrer =
            net::referrer::determine(&initiator.url, &initiator.origin, referrer_policy, &url)
                .unwrap_or_default();
        let mut request =
            net::Request::get(url.clone(), net::Destination::Iframe).initiated_by(initiator);
        request.partition = self.frame_partition();
        request.referrer_policy = Some(referrer_policy);
        // Its container's `frame-src` decides (M33.1).
        request.csp = Some(creator.csp.clone());
        request.local_network = self.frame_local_network(&url);
        self.network.fetch(id, Target::Document, request);
        self.navigation = Some(Navigation {
            url,
            incoming: Incoming::Fetching(id),
            handling: Handling::Replace,
            started: self.time_origin(),
            creator: local.then_some(creator),
            sandbox: self.creation_sandbox(),
            referrer,
        });
        self.load_state = LoadState::Loading;
    }

    /// The navigable a link or form whose target is `name` goes to, when
    /// it is another frame of the tab (HTML 7.3.1, "the rules for
    /// choosing a navigable", M33.2): `_parent`, `_top`, or a frame named
    /// so that this one may navigate; `_blank`, or a name found nowhere,
    /// which ask for a new tab the browser has not (M37), go to the top
    /// from a frame. `None` for this frame's own. A sandboxed document's
    /// navigation to `url` its flags refuse, by the user's activation or
    /// not, goes nowhere: the console's line saying so.
    pub(crate) fn other_navigable(
        &self,
        name: &str,
        url: &Url,
        activated: bool,
    ) -> Result<Option<FrameId>, String> {
        self.navigable_for(&self.shown.page, name, url, activated)
    }

    /// [`Engine::other_navigable`] for the document of `page` in this
    /// engine's frame, the one shown or the one on its way.
    pub(crate) fn navigable_for(
        &self,
        page: &Page,
        name: &str,
        url: &Url,
        activated: bool,
    ) -> Result<Option<FrameId>, String> {
        let me = self.frame.id;
        let tree = lock(&self.frame.tree);
        let origin = page.origin();
        let sandbox = page.sandbox();
        let new_tab = || match sandbox.has(Sandbox::AUXILIARY_NAVIGATION) {
            true => Err(format!(
                "Blocked opening '{url}' in a new window because the request was made in a \
                 sandboxed frame whose 'allow-popups' permission is not set."
            )),
            false => Ok(tree.top(me)),
        };
        let chosen = match name.to_ascii_lowercase().as_str() {
            "" | "_self" => me,
            "_parent" => tree.parent(me).unwrap_or(me),
            "_top" => tree.top(me),
            "_blank" => new_tab()?,
            _ => match tree.find_named(name) {
                Some(id) if tree.may_navigate(me, id, (origin, sandbox), activated) => id,
                Some(id) if sandbox.any() => id,
                Some(_) => me,
                None => new_tab()?,
            },
        };
        if chosen == me {
            return Ok(None);
        }
        if !sandbox.any()
            || name.eq_ignore_ascii_case("_blank")
            || tree.may_navigate(me, chosen, (origin, sandbox), activated)
        {
            return Ok(Some(chosen));
        }
        let target = tree
            .nodes
            .get(&chosen)
            .and_then(|node| node.origin.as_ref())
            .map_or_else(|| "null".to_owned(), net::Origin::serialization);
        let from = page.url().map_or("", Url::as_str);
        let why = match chosen == tree.top(me) {
            true => {
                "The frame attempting navigation of the top-level window is sandboxed, but the \
                 flag of 'allow-top-navigation' or 'allow-top-navigation-by-user-activation' is \
                 not set."
            }
            false => {
                "The frame attempting navigation is sandboxed, and is therefore disallowed from \
                 navigating its ancestors."
            }
        };
        Err(format!(
            "Unsafe attempt to initiate navigation for frame with origin '{target}' from frame \
             with URL '{from}'. {why}"
        ))
    }

    /// Whether the frame's response `response` to the navigation of page
    /// `page` may not be shown in it (M33.2): then the navigation is
    /// over, the engine's error page shown in its place, of an opaque
    /// origin, and the console says why.
    pub(crate) fn refuse_framing(&mut self, page: PageId, response: &net::Response) -> bool {
        if !self.frame.is_frame() {
            return false;
        }
        let ancestors = lock(&self.frame.tree).ancestor_origins(self.frame.id);
        let at = response.url();
        let Some(line) = crate::sandbox::framing_refused(at, &response.headers, &ancestors) else {
            return false;
        };
        let Some(navigation) = self.navigation.take() else {
            return false;
        };
        self.network.cancel(page, None);
        let error = crate::LoadError::Framed;
        let shown = self.made(crate::pages::error(at, &error), Some(error));
        self.commit(shown, Some(navigation.url), navigation.handling);
        // The page made has no URL: the line names the one refused.
        let entry = scripts::ConsoleEntry::of(
            scripts::ConsoleSource::Policy,
            js::ConsoleLevel::Error,
            line,
        );
        let lines = self.frame_lines(vec![entry], Some(at));
        self.departed.extend(lines);
        true
    }

    /// Asks frame `frame` to navigate to `url` (M33.2): it does at its
    /// next task, as started by a document of `origin`.
    pub(crate) fn navigate_other(
        &mut self,
        frame: FrameId,
        url: Url,
        replace: bool,
        post: Option<Post>,
        origin: net::Origin,
    ) {
        let task = FrameTask::Navigate(Box::new(FrameNavigation {
            url,
            replace,
            post,
            origin,
        }));
        if lock(&self.frame.tree).post(frame, task) {
            self.wake();
        }
    }

    /// Takes what other frames asked of this one (M33.2): a message
    /// delivered to its window as a task when its document is of the
    /// origin it was posted for, scripts on; a navigation started.
    pub(crate) fn take_frame_tasks(&mut self) {
        let tasks = lock(&self.frame.tree).take_inbox(self.frame.id);
        for task in tasks {
            match task {
                FrameTask::Message {
                    data,
                    origin,
                    source,
                    target_origin,
                } => {
                    let own = self.shown.page.origin().serialization();
                    if self.scripts_run() && (target_origin == "*" || target_origin == own) {
                        let now = self.now();
                        self.shown.scripts.post(
                            now,
                            js::Task::Message {
                                target: js::MessageTarget::Frame(source),
                                data,
                                origin,
                            },
                        );
                    }
                }
                FrameTask::Navigate(navigation) => {
                    let FrameNavigation {
                        url,
                        replace,
                        post,
                        origin,
                    } = *navigation;
                    let source = crate::NavigationSource {
                        origin,
                        activated: false,
                        csp: None,
                        form: false,
                        referrer: None,
                        referrer_policy: net::ReferrerPolicy::default(),
                    };
                    match post {
                        Some(post) => self.navigate_with(url, Some(post), source),
                        None => self.navigate_replacing(url, replace, Some(source)),
                    }
                }
            }
        }
    }

    /// Where a document of `origin` in this engine's frame is in its tab
    /// (M33.2): its own origin may make its context a cross-site one; a
    /// frame's requests, cookies and storage are kept apart by it, none
    /// at the top.
    pub(crate) fn partition_for(&self, origin: &net::Origin) -> Option<net::Partition> {
        self.frame
            .is_frame()
            .then(|| lock(&self.frame.tree).partition(self.frame.id, origin))
    }

    /// Where this engine's frame is in its tab whatever its document: the
    /// top-level document's origin, and whether an ancestor is of another
    /// site; a run's host adds its document's own origin.
    pub(crate) fn frame_partition(&self) -> Option<net::Partition> {
        self.frame.is_frame().then(|| {
            let tree = lock(&self.frame.tree);
            let top = tree.top_origin();
            tree.partition(self.frame.id, &top)
        })
    }

    /// Whether other frames asked something of this one.
    pub(crate) fn frame_tasks_waiting(&self) -> bool {
        lock(&self.frame.tree).has_tasks(self.frame.id)
    }

    /// Whether the frame's document is loaded, or failed: what its
    /// element's `load` waits for.
    fn frame_settled(&self) -> bool {
        self.navigation.is_none() && self.load_state != LoadState::Loading
    }

    /// Whether every frame's element was told its document loaded: what
    /// the document's own load waits for (HTML's "delaying the load
    /// event"), and a runner waits for once a frame is put back.
    pub fn frames_settled(&self) -> bool {
        self.shown.frames.values().all(Frame::settled)
    }

    /// Fires `load` at each `iframe` whose frame's document has loaded
    /// since it was navigated, in tree order, and brings the load state
    /// up to date: the frames' `load`s come before the document's.
    pub(crate) fn tell_frame_loads(&mut self) {
        let mut loaded: Vec<NodeId> = self
            .shown
            .frames
            .iter_mut()
            .filter(|(_, frame)| frame.to_tell())
            .map(|(&node, frame)| {
                match &mut frame.remote {
                    Some(remote) => remote.told = remote.loads,
                    None => frame.told = frame.engine.completed,
                }
                node
            })
            .collect();
        if loaded.is_empty() {
            // A frame's navigation can end with no new document (a 204,
            // a download): its element hears nothing, but the document's
            // load may have waited for that navigation alone.
            if self.load_state == LoadState::Loading && self.frames_settled() {
                self.update_load_state();
            }
            return;
        }
        let document = &self.shown.document;
        let order: HashMap<NodeId, usize> = document
            .subtree(document.root())
            .enumerate()
            .map(|(at, node)| (node, at))
            .collect();
        loaded.sort_by_key(|node| order.get(node).copied().unwrap_or(usize::MAX));
        for node in loaded {
            if self.shown.document.contains(node) {
                self.fire(
                    js::Target::Node(scripts::handle(node)),
                    js::EventInit::new("Event", "load"),
                );
            }
        }
        self.update_load_state();
    }

    /// Takes the network's results of every frame (M33.2), then tells
    /// the elements of those that loaded. Whether one wants a frame.
    pub(crate) fn poll_frames(&mut self) -> bool {
        let mut wants = false;
        for node in self.frames_in_run_order() {
            if let Some(frame) = self.shown.frames.get_mut(&node) {
                wants |= frame.engine.poll_network();
            }
        }
        self.tell_frame_loads();
        wants
    }

    /// Runs the frames' tasks that are due, then tells the elements of
    /// those that loaded. Whether one wants a frame.
    pub(crate) fn run_frame_tasks(&mut self) -> bool {
        let mut wants = false;
        for node in self.frames_in_run_order() {
            if let Some(frame) = self.shown.frames.get_mut(&node) {
                wants |= frame.engine.run_tasks();
            }
        }
        self.tell_frame_loads();
        wants
    }

    /// When the frames' next task is due: now when an element is to be
    /// told its frame loaded.
    pub(crate) fn frames_next_task_at(&self) -> Option<Duration> {
        self.shown
            .frames
            .values()
            .filter_map(|frame| {
                if frame.to_tell() {
                    Some(Duration::ZERO)
                } else {
                    frame.engine.next_task_at()
                }
            })
            .min()
    }

    /// Whether a frame wants to be drawn again.
    pub(crate) fn frames_need_frame(&self) -> bool {
        self.shown
            .frames
            .values()
            .any(|frame| frame.engine.needs_frame())
    }

    /// When a frame next wants to be drawn, as [`Engine::next_frame_at`]
    /// says of each.
    pub(crate) fn frames_next_frame_at(&self) -> Option<Duration> {
        self.shown
            .frames
            .values()
            .filter_map(|frame| frame.engine.next_frame_at())
            .min()
    }

    /// Whether a frame fetches, decodes an image, or has a worker.
    pub(crate) fn frames_busy(&self, what: fn(&Engine) -> bool) -> bool {
        self.shown.frames.values().any(|frame| what(&frame.engine))
    }

    /// Applies `f` to every frame's engine, and theirs: a setting the tab
    /// changed.
    pub(crate) fn for_each_frame(&mut self, f: &mut dyn FnMut(&mut Engine)) {
        for frame in self.shown.frames.values_mut() {
            f(&mut frame.engine);
            frame.engine.for_each_frame(f);
        }
    }

    /// The frames' consoles since they were last taken, in tree order.
    pub(crate) fn take_frame_console(&mut self) -> Vec<scripts::ConsoleEntry> {
        let order = self.frames_in_order();
        let mut entries = Vec::new();
        for node in order {
            if let Some(frame) = self.shown.frames.get_mut(&node) {
                entries.extend(frame.engine.take_console());
            }
        }
        entries
    }

    /// `lines` a document at `url` wrote, each saying which frame's
    /// document it was when the engine shows a frame's (M33.2): the
    /// tab's console is all its frames'.
    pub(crate) fn frame_lines(
        &self,
        mut lines: Vec<scripts::ConsoleEntry>,
        url: Option<&Url>,
    ) -> Vec<scripts::ConsoleEntry> {
        if self.frame.is_frame() {
            let url = url.map_or_else(String::new, Url::to_string);
            for entry in &mut lines {
                entry.text = format!("[frame {url}] {}", entry.text);
            }
        }
        lines
    }

    /// A frame's notice, if one has one.
    pub(crate) fn take_frame_notice(&mut self) -> Option<String> {
        self.shown
            .frames
            .values_mut()
            .find_map(|frame| frame.engine.take_notice())
    }

    /// The `iframe`s with frames, in tree order.
    fn frames_in_order(&self) -> Vec<NodeId> {
        let document = &self.shown.document;
        document
            .subtree(document.root())
            .filter(|node| self.shown.frames.contains_key(node))
            .collect()
    }

    /// Every frame, in the order their work is done (M33): their
    /// elements' tree order, then any whose element has left, so that
    /// what they post one another comes in the same order every run.
    fn frames_in_run_order(&self) -> Vec<NodeId> {
        let mut order = self.frames_in_order();
        let mut rest: Vec<NodeId> = self
            .shown
            .frames
            .keys()
            .filter(|node| !order.contains(node))
            .copied()
            .collect();
        rest.sort_unstable();
        order.extend(rest);
        order
    }

    /// The `iframe` whose frame the viewport point `(x, y)` is over, and
    /// the point in that frame's viewport (M33.2): within its content
    /// box, the border and padding around it being the parent's.
    pub(crate) fn frame_under(&mut self, x: f32, y: f32) -> Option<(NodeId, f32, f32)> {
        let node = self.hit_test(x, y)?;
        if !self.shown.frames.contains_key(&node) {
            return None;
        }
        let (fx, fy) = self.frame_point(node, x, y)?;
        let (w, h) = self.frame_engine(node)?.viewport;
        (fx >= 0.0 && fy >= 0.0 && fx < w as f32 && fy < h as f32).then_some((node, fx, fy))
    }

    /// The viewport point `(x, y)` in the viewport of the frame of the
    /// `iframe` `node`: from its content box's top left, through the
    /// transforms around it.
    pub(crate) fn frame_point(&mut self, node: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let (px, py) = self.padding_offset(node, x, y);
        let layout = self.layout(self.viewport.0 as f32);
        let f = fragment_of(&layout.roots, node)?;
        let content = f.children.first()?;
        let border = f.style.border_width;
        // The content's offsets are from the frame's box (M35.9).
        Some((px - (content.x - border[3]), py - (content.y - border[0])))
    }

    /// An engine of a renderer for the frame root the browser numbers
    /// `key` (M34): a frame whose parent's document another renderer shows,
    /// its ancestors (the parent first, each the browser's number, origin
    /// and URL) frames of other renderers in its tree, its document made
    /// with the `sandbox` flags.
    pub(crate) fn frame_root(
        fonts: text::FontCollection,
        link: Arc<crate::renderer::link::Link>,
        key: u32,
        ancestors: &[(u32, net::Origin, Option<Url>)],
        sandbox: Sandbox,
    ) -> Engine {
        let mut engine = Engine::with_fonts(fonts);
        let tree = FrameTree::shared();
        let id = {
            let mut tree = lock(&tree);
            let mut parent = None;
            for (key, origin, _) in ancestors.iter().rev() {
                parent = Some(tree.add_stub(parent, *key, origin.clone()));
            }
            let id = tree.add_root(parent.unwrap_or(TOP), key);
            tree.set_element_policy(id, sandbox, Allowed::default());
            id
        };
        engine.frame = FrameContext {
            tree,
            id,
            key,
            depth: ancestors.len(),
            ancestors: ancestors
                .iter()
                .filter_map(|(_, _, url)| url.as_ref().map(|url| url.without_fragment().to_owned()))
                .collect(),
            container: None,
            container_csp: None,
            container_local_network: None,
        };
        engine.window_focused = false;
        engine.set_link(link, key);
        engine
    }

    /// Marks the frame the browser numbers `key`, this engine's or one
    /// of its frames', as shown by another renderer (M34); whether one is.
    pub(crate) fn mark_remote(&mut self, key: u32) -> bool {
        let tree = self.frame.tree.clone();
        for frame in self.shown.frames.values_mut() {
            if frame.engine.frame.key == key && frame.remote.is_none() {
                frame.remote = Some(RemoteFrame {
                    key,
                    loads: 0,
                    told: 0,
                });
                frame.drawn = None;
                lock(&tree).set_remote(frame.id, key);
                self.changed = true;
                return true;
            }
            if frame.remote.is_none() && frame.engine.mark_remote(key) {
                return true;
            }
        }
        false
    }

    /// The frame of another renderer the browser numbers `key` loaded a
    /// document (M34): its element hears `load`; whether it is here.
    pub(crate) fn remote_loaded(&mut self, key: u32) -> bool {
        let mut found = false;
        for frame in self.shown.frames.values_mut() {
            match &mut frame.remote {
                Some(remote) if remote.key == key => {
                    remote.loads += 1;
                    found = true;
                }
                Some(_) => {}
                None => found |= frame.engine.remote_loaded(key),
            }
            if found {
                break;
            }
        }
        if found {
            self.tell_frame_loads();
        }
        found
    }

    /// The frames of other renderers this page shows, and its frames',
    /// with their content boxes in its viewport, CSS pixels (M34): the
    /// holes the browser fills.
    pub(crate) fn remote_holes(&mut self) -> Vec<(u32, [f32; 4])> {
        let nodes: Vec<NodeId> = self.shown.frames.keys().copied().collect();
        let mut holes = Vec::new();
        for node in nodes {
            let Some(rect) = self.content_box(node) else {
                continue;
            };
            let Some(frame) = self.shown.frames.get_mut(&node) else {
                continue;
            };
            match frame.remote {
                Some(remote) => holes.push((remote.key, rect)),
                None => holes.extend(
                    frame
                        .engine
                        .remote_holes()
                        .into_iter()
                        .map(|(key, [x, y, w, h])| (key, [x + rect[0], y + rect[1], w, h])),
                ),
            }
        }
        holes
    }

    /// The content box of the `iframe` `node` in the viewport.
    fn content_box(&mut self, node: NodeId) -> Option<[f32; 4]> {
        let (border, _) = self.border_box(node)?;
        let layout = self.layout(self.viewport.0 as f32);
        let f = fragment_of(&layout.roots, node)?;
        let content = f.children.first()?;
        // The content's offsets are from the frame's box (M35.9).
        Some([
            border.x + content.x,
            border.y + content.y,
            content.w,
            content.h,
        ])
    }

    /// The engine, this or one of its frames', of the frame the browser
    /// numbers `key` (M34).
    pub(crate) fn engine_of_key(&mut self, key: u32) -> Option<&mut Engine> {
        if self.frame.key == key {
            return Some(self);
        }
        self.shown
            .frames
            .values_mut()
            .filter(|frame| frame.remote.is_none())
            .find_map(|frame| frame.engine.engine_of_key(key))
    }

    /// The frames of other renderers this page and its frames dropped
    /// since this was last asked (M34): their views go.
    pub(crate) fn take_remote_drops(&mut self) -> Vec<u32> {
        let mut dropped = std::mem::take(&mut self.dropped_remote);
        for frame in self.shown.frames.values_mut() {
            dropped.extend(frame.engine.take_remote_drops());
        }
        dropped
    }

    /// The engine of the frame of the `iframe` `node`, if it has one.
    pub(crate) fn frame_engine(&mut self, node: NodeId) -> Option<&mut Engine> {
        self.shown
            .frames
            .get_mut(&node)
            .map(|frame| &mut *frame.engine)
    }

    /// The `iframe` focused, when it has a frame: the frame the keys go
    /// to (M33.2).
    pub(crate) fn focused_frame(&self) -> Option<NodeId> {
        self.shown
            .document
            .focused()
            .filter(|node| self.shown.frames.contains_key(node))
    }

    /// The frame the pointer's events go to at `(x, y)`, and the point in
    /// it: the one a press began in while a button is held, as capture
    /// would; none while a press that began here is held; else the one
    /// under the pointer. The frame it was over before hears it leave.
    pub(crate) fn pointer_frame(&mut self, x: f32, y: f32) -> Option<(NodeId, f32, f32)> {
        let at = match self.shown.pressed_frame {
            Some(node) => self
                .frame_point(node, x, y)
                .map(|(fx, fy)| (node, fx, fy))
                .filter(|_| self.shown.frames.contains_key(&node)),
            None if self.buttons != 0 => None,
            None => self.frame_under(x, y),
        };
        let now = at.map(|(node, _, _)| node);
        if self.shown.pointer_frame != now {
            if let Some(old) = self.shown.pointer_frame
                && let Some(child) = self.frame_engine(old)
            {
                child.pointer_leave();
            }
            self.shown.pointer_frame = now;
        }
        at
    }

    /// Moves the focus into the frame of the `iframe` `node`, whose
    /// document's own focus is the child's to keep (M33.2): its window
    /// hears `focus`; this document's element focused and window lose it.
    pub(crate) fn focus_frame(&mut self, node: NodeId) {
        if self.shown.document.focused() == Some(node) {
            return;
        }
        self.focus(Some(node), false, false);
    }

    /// Moves the focus to the area after (before, `backward`) the
    /// `iframe` `node` in the sequential order, the frame's own areas
    /// passed: Tab past a frame's last.
    pub(crate) fn focus_after_frame(&mut self, node: NodeId, backward: bool) -> bool {
        self.focus_from(Some(node), backward)
    }

    /// The pixels the frame of the `iframe` `node` last drew, for a print
    /// (M33.2): a frame prints as it is on the screen.
    pub(crate) fn frame_drawing(&self, node: NodeId) -> Option<Arc<images::Image>> {
        let frame = self.shown.frames.get(&node)?;
        frame.drawn.as_ref().map(|drawn| drawn.pixels.clone())
    }

    /// The pixels of the frame of the `iframe` `node`, its content box
    /// `place` CSS pixels big landing on `size` raster pixels (M33.2):
    /// drawn again when the frame wants it or its size changed, at the
    /// time of the frame being drawn, at the tab's scale unless the box is
    /// transformed; the frame the image is shown as and its pixels.
    pub(crate) fn frame_pixels(
        &mut self,
        node: NodeId,
        place: (f32, f32),
        size: (f32, f32),
        time: Option<FrameTime>,
    ) -> Option<(u32, Arc<images::Image>)> {
        if place.0 <= 0.0 || place.1 <= 0.0 {
            return None;
        }
        let scale = self.scale;
        let frame = self.shown.frames.get_mut(&node)?;
        let raster = (
            size.0.round().max(1.0) as u32,
            size.1.round().max(1.0) as u32,
        );
        // Another renderer's frame (M34): a hole of its size, which the
        // browser fills.
        if frame.remote.is_some() {
            let stale = frame
                .drawn
                .as_ref()
                .is_none_or(|drawn| drawn.size != raster);
            if stale {
                let pixels: Vec<u8> = HOLE
                    .iter()
                    .copied()
                    .cycle()
                    .take(raster.0 as usize * raster.1 as usize * 4)
                    .collect();
                let image = images::Image {
                    width: raster.0,
                    height: raster.1,
                    frames: vec![images::Frame {
                        pixels: pixels.into(),
                        delay: 0,
                    }],
                    plays: images::Plays::Times(1),
                };
                frame.drawn = Some(Drawn {
                    size: raster,
                    frame: FRAME_DRAWINGS.fetch_add(1, Ordering::Relaxed),
                    pixels: Arc::new(image),
                });
            }
            return frame
                .drawn
                .as_ref()
                .map(|drawn| (drawn.frame, drawn.pixels.clone()));
        }
        // A frame under a transform that scales it lays out at its box's
        // size and draws at the size it lands on.
        let at = if (raster.0 as f32 - place.0 * scale).abs() > 1.0 {
            raster.0 as f32 / place.0
        } else {
            scale
        };
        frame.engine.set_scale_factor(at);
        let stale = frame
            .drawn
            .as_ref()
            .is_none_or(|drawn| drawn.size != raster)
            || frame.engine.needs_frame();
        if stale {
            let bytes = match time {
                Some(time) => frame.engine.frame_at(raster.0, raster.1, time).to_vec(),
                None => frame.engine.frame(raster.0, raster.1),
            };
            if bytes.is_empty() {
                frame.drawn = None;
                return None;
            }
            let image = images::Image {
                width: raster.0,
                height: raster.1,
                frames: vec![images::Frame {
                    pixels: bytes.into(),
                    delay: 0,
                }],
                plays: images::Plays::Times(1),
            };
            frame.drawn = Some(Drawn {
                size: raster,
                frame: FRAME_DRAWINGS.fetch_add(1, Ordering::Relaxed),
                pixels: Arc::new(image),
            });
        }
        frame
            .drawn
            .as_ref()
            .map(|drawn| (drawn.frame, drawn.pixels.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_numbers_its_frames_and_holds_them_to_the_limit() {
        let tree = FrameTree::shared();
        let mut tree = lock(&tree);
        let first = tree.add(TOP).expect("room");
        let under = tree.add(first).expect("room");
        assert_ne!(first, TOP);
        assert_ne!(under, first);
        assert_eq!(tree.frames(), 2);
        tree.remove(first);
        assert_eq!(tree.frames(), 0, "a frame leaves with its parent");
        for _ in 0..MAX_FRAMES {
            assert!(tree.add(TOP).is_some());
        }
        assert_eq!(tree.add(TOP), None);
        tree.remove(TOP);
        assert_eq!(tree.frames(), MAX_FRAMES, "the top stays");
    }
}
