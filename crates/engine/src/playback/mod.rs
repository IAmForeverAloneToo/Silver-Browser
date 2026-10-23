//! Media elements (M40 step 6, HTML 4.8.11, `docs/design/audio-video.md`,
//! "The media element"): each `<video>` and `<audio>` element's state in
//! its document's host, and the standard's algorithms over it: load,
//! resource selection and fetch, the ready states, play and pause with
//! their promises, seeking, the end and `loop`, the autoplay policy. Its
//! resource is read by its pipeline ([`pipeline`]); what the pipeline
//! tells comes back as a task, and what a script's call queues goes as
//! one: each a list of *actions* (an event at the element or one of its
//! `<source>` children, a promise settled), which the bootstrap's
//! `media` entry carries out in order.

pub(crate) mod pipeline;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use devtools::Json;
use dom::{Document, NodeId};
use net::{Destination, Request};

use crate::image_store::CrossOrigin;
use crate::loader::{Page, media::MediaResource};
use crate::scripts::workers::{Inbox, WorkerMessage};
pub(crate) use pipeline::ElementSound;
use pipeline::{Failure, Meta, Pipeline, Report};

/// The most pipelines a process runs at once (the reference's limit of
/// players a renderer); past it, an element's load fails with a format
/// error until one ends.
pub(crate) const MAX_MEDIA_PIPELINES: usize = 75;

static PIPELINES: AtomicUsize = AtomicUsize::new(0);

const NETWORK_EMPTY: u16 = 0;
const NETWORK_IDLE: u16 = 1;
const NETWORK_LOADING: u16 = 2;
const NETWORK_NO_SOURCE: u16 = 3;

const HAVE_NOTHING: u16 = 0;
const HAVE_METADATA: u16 = 1;
const HAVE_CURRENT_DATA: u16 = 2;
const HAVE_ENOUGH_DATA: u16 = 4;

const NOT_ALLOWED: &str =
    "play() failed because the user didn't interact with the document first. https://goo.gl/xX8pDD";
const NO_SOURCES: &str = "The element has no supported sources.";
const PAUSED: &str =
    "The play() request was interrupted by a call to pause(). https://goo.gl/LdLk22";
const NEW_LOAD: &str =
    "The play() request was interrupted by a new load request. https://goo.gl/LdLk22";

/// What the bootstrap does for an element: fire an event at it (or at a
/// `<source>` of its), or settle a `play()` promise.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    Event(NodeId, &'static str),
    Settle(u64, Option<(&'static str, &'static str)>),
}

fn actions_json(actions: &[Action]) -> Json {
    Json::Array(
        actions
            .iter()
            .map(|action| match action {
                Action::Event(node, kind) => Json::object([
                    ("node", Json::from(crate::scripts::handle(*node).0)),
                    ("event", Json::from(*kind)),
                ]),
                Action::Settle(promise, error) => match error {
                    None => Json::object([("promise", Json::from(*promise))]),
                    Some((name, message)) => Json::object([
                        ("promise", Json::from(*promise)),
                        ("name", Json::from(*name)),
                        ("message", Json::from(*message)),
                    ]),
                },
            })
            .collect(),
    )
}

/// A number as the bootstrap reads it: `Number(text)`, so that NaN and
/// the infinities cross.
fn number(value: f64) -> Json {
    Json::from(if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        value.to_string()
    })
}

/// What a call needs of its document.
pub(crate) struct Env<'a> {
    pub(crate) document: &'a Document,
    pub(crate) page: Option<&'a Page>,
    pub(crate) inbox: Option<Inbox>,
    /// Whether the user has activated the tab (sticky activation).
    pub(crate) activated: bool,
}

/// An element's state.
struct Player {
    node: NodeId,
    video: bool,
    network: u16,
    ready: u16,
    paused: bool,
    seeking: bool,
    ended: bool,
    /// The official playback position, while no pipeline gives one.
    position: f64,
    /// Where playback starts once the resource's head is read, a seek
    /// asked for before.
    start_at: f64,
    duration: f64,
    size: (u32, u32),
    default_rate: f64,
    rate: f64,
    volume: f64,
    muted: bool,
    error: Option<Failure>,
    current_src: String,
    can_autoplay: bool,
    pending: Vec<u64>,
    pipeline: Option<Pipeline>,
    /// Its sound as its pipelines give it, the one a graph may take.
    sound: Arc<ElementSound>,
    /// The request a load made, which the engine starts (its network's
    /// policies on it), and what its URL says the resource is.
    pending_fetch: Option<(Request, media::Hint)>,
    /// The load's number: a report of an older one is dropped.
    generation: u64,
    seek_id: u64,
    /// The `<source>` children not tried yet, in order.
    candidates: Vec<NodeId>,
    /// The `<source>` being tried.
    trying: Option<NodeId>,
    /// The `src` value the last load went by.
    src_seen: Option<String>,
    played: Vec<(f64, f64)>,
    played_from: Option<f64>,
    whole: bool,
}

impl Player {
    fn new(node: NodeId, video: bool, document: &Document) -> Player {
        Player {
            node,
            video,
            network: NETWORK_EMPTY,
            ready: HAVE_NOTHING,
            paused: true,
            seeking: false,
            ended: false,
            position: 0.0,
            start_at: 0.0,
            duration: f64::NAN,
            size: (0, 0),
            default_rate: 1.0,
            rate: 1.0,
            volume: 1.0,
            muted: document.attr(node, "muted").is_some(),
            error: None,
            current_src: String::new(),
            can_autoplay: true,
            pending: Vec::new(),
            pipeline: None,
            sound: {
                let sound = Arc::new(ElementSound::default());
                sound.set_gain(1.0);
                sound
            },
            pending_fetch: None,
            generation: 0,
            seek_id: 0,
            candidates: Vec::new(),
            trying: None,
            src_seen: None,
            played: Vec::new(),
            played_from: None,
            whole: false,
        }
    }

    fn position(&self) -> f64 {
        match &self.pipeline {
            Some(pipeline) if self.ready >= HAVE_METADATA && !self.seeking => pipeline.position(),
            _ => self.position,
        }
    }

    fn gain(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume as f32 }
    }

    /// The played ranges, with the stretch now playing.
    fn played(&self) -> Vec<(f64, f64)> {
        let mut ranges = self.played.clone();
        if let Some(from) = self.played_from {
            ranges.push((from, self.position()));
        }
        normalized(ranges)
    }

    fn mark_played(&mut self) {
        if let Some(from) = self.played_from.take() {
            let to = self.position();
            self.played.push((from, to));
            self.played = normalized(std::mem::take(&mut self.played));
        }
    }

    /// Rejects the pending `play()` promises with `name` and `message`.
    fn reject_pending(&mut self, name: &'static str, message: &'static str, out: &mut Vec<Action>) {
        for promise in self.pending.drain(..) {
            out.push(Action::Settle(promise, Some((name, message))));
        }
    }

    fn resolve_pending(&mut self, out: &mut Vec<Action>) {
        for promise in self.pending.drain(..) {
            out.push(Action::Settle(promise, None));
        }
    }

    fn drop_pipeline(&mut self) {
        self.pending_fetch = None;
        if self.pipeline.take().is_some() {
            PIPELINES.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn snapshot(&self, document: &Document) -> Json {
        let position = self.position();
        let finite = self.duration.is_finite();
        let ranges = |list: Vec<(f64, f64)>| {
            Json::Array(
                list.into_iter()
                    .map(|(a, b)| Json::Array(vec![number(a), number(b)]))
                    .collect(),
            )
        };
        let buffered = if self.ready >= HAVE_CURRENT_DATA && finite {
            let to = if self.whole {
                self.duration
            } else {
                position.max(0.0)
            };
            vec![(0.0, to)]
        } else {
            Vec::new()
        };
        let seekable = if self.ready >= HAVE_METADATA && finite {
            vec![(0.0, self.duration)]
        } else {
            Vec::new()
        };
        let ended = self.ended
            && finite
            && position >= self.duration - 1e-6
            && document.attr(self.node, "loop").is_none();
        Json::object([
            ("networkState", Json::from(u64::from(self.network))),
            ("readyState", Json::from(u64::from(self.ready))),
            ("paused", Json::Bool(self.paused)),
            ("seeking", Json::Bool(self.seeking)),
            ("ended", Json::Bool(ended)),
            ("currentTime", number(position)),
            ("duration", number(self.duration)),
            ("volume", Json::from(self.volume)),
            ("muted", Json::Bool(self.muted)),
            ("playbackRate", Json::from(self.rate)),
            ("defaultPlaybackRate", Json::from(self.default_rate)),
            ("currentSrc", Json::from(self.current_src.as_str())),
            ("videoWidth", Json::from(u64::from(self.size.0))),
            ("videoHeight", Json::from(u64::from(self.size.1))),
            (
                "error",
                match &self.error {
                    None => Json::Null,
                    Some(failure) => Json::object([
                        ("code", Json::from(u64::from(failure.code))),
                        ("message", Json::from(failure.message.as_str())),
                    ]),
                },
            ),
            ("buffered", ranges(buffered)),
            ("seekable", ranges(seekable)),
            ("played", ranges(self.played())),
        ])
    }
}

/// Ranges in order, those that overlap or touch joined.
fn normalized(mut ranges: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    ranges.retain(|(a, b)| b > a);
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1e-6 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// A document's media elements.
#[derive(Default)]
pub(crate) struct Players {
    players: HashMap<NodeId, Player>,
}

impl Players {
    /// Starts the fetches the loads asked for, each with `start` (the
    /// engine's network, its page's policies on the request), and their
    /// pipelines, which tell through `inbox`.
    pub(crate) fn start_pending(
        &mut self,
        mut start: impl FnMut(Request) -> MediaResource,
        inbox: Option<&Inbox>,
    ) {
        for player in self.players.values_mut() {
            let Some((request, hint)) = player.pending_fetch.take() else {
                continue;
            };
            let mut out = Vec::new();
            if PIPELINES.load(Ordering::Relaxed) >= MAX_MEDIA_PIPELINES {
                fail(player, format_error(), &mut out);
                if let Some(inbox) = inbox {
                    send(inbox, &out);
                }
                continue;
            }
            let resource = start(request);
            let generation = player.generation;
            let inbox = inbox.cloned();
            let handle = crate::scripts::handle(player.node).0;
            let tell = Box::new(move |report: Report| {
                if let Some(inbox) = &inbox {
                    let message = Json::object([
                        ("node", Json::from(handle)),
                        ("generation", Json::from(generation)),
                        ("report", report.to_json()),
                    ]);
                    let _ = inbox.send(WorkerMessage::Media(message.write()));
                }
            });
            PIPELINES.fetch_add(1, Ordering::Relaxed);
            let pipeline = Pipeline::start(resource, hint, tell, player.sound.clone());
            pipeline.set_gain(player.gain());
            pipeline.set_rate(player.rate.abs().clamp(0.0625, 16.0));
            if player.start_at > 0.0 {
                pipeline.seek(0, player.start_at);
                player.position = player.start_at;
                player.start_at = 0.0;
            }
            if !player.paused {
                pipeline.set_playing(true);
            }
            player.whole = false;
            player.pipeline = Some(pipeline);
        }
    }

    fn player<'a>(&'a mut self, node: NodeId, document: &Document) -> Option<&'a mut Player> {
        let tag = document.tag(node)?;
        let video = match tag {
            "video" => true,
            "audio" => false,
            _ => return None,
        };
        Some(
            self.players
                .entry(node)
                .or_insert_with(|| Player::new(node, video, document)),
        )
    }

    /// The element `node`'s pipeline, for its picture.
    // Video on screen (step 7) is its user.
    #[allow(dead_code)]
    pub(crate) fn picture(&self, node: NodeId) -> Option<media::Picture> {
        self.players.get(&node)?.pipeline.as_ref()?.picture()
    }

    /// The element `node`'s sound, for a `MediaElementAudioSourceNode`.
    pub(crate) fn sound(&mut self, node: NodeId, document: &Document) -> Option<Arc<ElementSound>> {
        Some(self.player(node, document)?.sound.clone())
    }

    /// Whether an element plays, which keeps its document from the
    /// back-forward cache.
    pub(crate) fn playing(&self) -> bool {
        self.players.values().any(|p| !p.paused)
    }

    /// One call of `native.media`: its answer, or the error the bootstrap
    /// throws (`name`, `message`).
    pub(crate) fn call(
        &mut self,
        op: &Json,
        env: &Env<'_>,
        node: Option<NodeId>,
    ) -> Result<Json, (&'static str, String)> {
        let what = op.get("op").and_then(Json::as_str).unwrap_or("");
        if what == "canPlayType" {
            let kind = op.get("type").and_then(Json::as_str).unwrap_or("");
            return Ok(Json::from(media::can_play_type(kind).as_str()));
        }
        let node = node.ok_or(("TypeError", "Illegal invocation".to_owned()))?;
        let mut queued = Vec::new();
        let player = self
            .player(node, env.document)
            .ok_or(("TypeError", "Illegal invocation".to_owned()))?;
        let answer = match what {
            "state" => player.snapshot(env.document),
            "load" => {
                load(player, env, &mut queued);
                Json::Null
            }
            "play" => {
                let promise = op.get("promise").and_then(Json::as_f64).unwrap_or(0.0) as u64;
                play(player, promise, env, &mut queued)?;
                Json::Null
            }
            "pause" => {
                pause(player, env, &mut queued);
                Json::Null
            }
            "seek" => {
                let time = op.get("time").and_then(Json::as_f64).unwrap_or(0.0);
                seek(player, time, &mut queued);
                Json::Null
            }
            "volume" => {
                let volume = op.get("value").and_then(Json::as_f64).unwrap_or(1.0);
                if player.volume != volume {
                    player.volume = volume;
                    queued.push(Action::Event(node, "volumechange"));
                }
                set_gain(player);
                Json::Null
            }
            "muted" => {
                let muted = op.get("value").and_then(Json::as_bool).unwrap_or(false);
                if player.muted != muted {
                    player.muted = muted;
                    queued.push(Action::Event(node, "volumechange"));
                }
                set_gain(player);
                Json::Null
            }
            "rate" | "defaultRate" => {
                let rate = op.get("value").and_then(Json::as_f64).unwrap_or(1.0);
                let field = if what == "rate" {
                    &mut player.rate
                } else {
                    &mut player.default_rate
                };
                if *field != rate {
                    *field = rate;
                    queued.push(Action::Event(node, "ratechange"));
                }
                if what == "rate"
                    && let Some(pipeline) = &player.pipeline
                {
                    pipeline.set_rate(rate.abs().clamp(0.0625, 16.0));
                }
                Json::Null
            }
            other => return Err(("TypeError", format!("no media call '{other}'"))),
        };
        queue(env, queued);
        Ok(answer)
    }

    /// What the pipeline of load `generation` of element `node` told: the
    /// actions to carry out now, in the task that heard it.
    pub(crate) fn report(
        &mut self,
        node: NodeId,
        generation: u64,
        report: &Json,
        env: &Env<'_>,
    ) -> Json {
        let mut out = Vec::new();
        let Some(player) = self.players.get_mut(&node) else {
            return actions_json(&out);
        };
        if player.generation != generation || player.pipeline.is_none() {
            return actions_json(&out);
        }
        let Some(report) = pipeline::Report::from_json(report) else {
            return actions_json(&out);
        };
        reported(player, report, env, &mut out);
        actions_json(&out)
    }

    /// What `changes` to the document mean to its media elements: a
    /// `src` set or taken off loads again; an element put in, or given
    /// a `<source>`, with nothing loaded selects a resource; one taken
    /// out of the document pauses.
    pub(crate) fn changed(&mut self, changes: &[dom::Change], env: &Env<'_>) {
        let document = env.document;
        let mut queued = Vec::new();
        let is_media = |node: NodeId| matches!(document.tag(node), Some("video" | "audio"));
        let mut loads = Vec::new();
        let mut selects = Vec::new();
        let mut pauses = Vec::new();
        for change in changes {
            match change {
                dom::Change::AttributeChanged { element, name, .. }
                    if name == "src" && is_media(*element) =>
                {
                    loads.push(*element);
                }
                dom::Change::Inserted { parent, child } => {
                    for node in document.subtree(*child) {
                        if is_media(node) {
                            selects.push(node);
                        }
                    }
                    if document.tag(*child) == Some("source") && is_media(*parent) {
                        selects.push(*parent);
                    }
                }
                dom::Change::Removed { child, .. } => {
                    if !document.contains(*child) {
                        continue;
                    }
                    for node in document.subtree(*child) {
                        if is_media(node) {
                            pauses.push(node);
                        }
                    }
                }
                dom::Change::Everything => {
                    for node in document.subtree(document.root()) {
                        if is_media(node) {
                            let src = document.attr(node, "src").map(str::to_owned);
                            match self.players.get(&node) {
                                Some(player) if player.src_seen != src => loads.push(node),
                                None => selects.push(node),
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for node in loads {
            if let Some(player) = self.player(node, document) {
                // A load in the run that set it (play() of an element with
                // nothing loaded) took this value up already.
                let taken = player.network != NETWORK_EMPTY
                    && player.src_seen.as_deref() == document.attr(node, "src");
                if !taken {
                    load(player, env, &mut queued);
                }
            }
        }
        for node in selects {
            let has_source = document.attr(node, "src").is_some()
                || document
                    .children(node)
                    .any(|child| document.tag(child) == Some("source"));
            if let Some(player) = self.player(node, document)
                && player.network == NETWORK_EMPTY
                && has_source
            {
                select(player, env, &mut queued);
            }
        }
        for node in pauses {
            let connected = document.is_inclusive_ancestor(document.root(), node);
            if !connected
                && let Some(player) = self.players.get_mut(&node)
                && !player.paused
            {
                internal_pause(player, &mut queued);
            }
        }
        // The players of elements gone.
        self.players.retain(|node, player| {
            let live = document.contains(*node);
            if !live {
                player.drop_pipeline();
            }
            live
        });
        queue(env, queued);
    }
}

impl Drop for Players {
    fn drop(&mut self) {
        for player in self.players.values_mut() {
            player.drop_pipeline();
        }
    }
}

fn set_gain(player: &Player) {
    if let Some(pipeline) = &player.pipeline {
        pipeline.set_gain(player.gain());
    }
}

/// Sends `actions` as a task of the document's own.
fn queue(env: &Env<'_>, actions: Vec<Action>) {
    if let Some(inbox) = &env.inbox {
        send(inbox, &actions);
    }
}

fn send(inbox: &Inbox, actions: &[Action]) {
    if actions.is_empty() {
        return;
    }
    let message = Json::object([("actions", actions_json(actions))]);
    let _ = inbox.send(WorkerMessage::Media(message.write()));
}

fn format_error() -> Failure {
    Failure {
        code: 4,
        message: "MEDIA_ELEMENT_ERROR: Format error".to_owned(),
    }
}

/// HTML's media element load algorithm.
fn load(player: &mut Player, env: &Env<'_>, out: &mut Vec<Action>) {
    let node = player.node;
    player.drop_pipeline();
    player.generation += 1;
    if matches!(player.network, NETWORK_LOADING | NETWORK_IDLE) {
        out.push(Action::Event(node, "abort"));
    }
    if player.network != NETWORK_EMPTY {
        out.push(Action::Event(node, "emptied"));
        player.network = NETWORK_EMPTY;
        player.ready = HAVE_NOTHING;
        if !player.paused {
            player.paused = true;
            player.reject_pending("AbortError", NEW_LOAD, out);
        }
        player.seeking = false;
        player.position = 0.0;
        player.duration = f64::NAN;
        player.size = (0, 0);
        player.played.clear();
        player.played_from = None;
    }
    player.ended = false;
    player.rate = player.default_rate;
    player.error = None;
    player.can_autoplay = true;
    select(player, env, out);
}

/// HTML's resource selection algorithm: the `src` attribute, else the
/// `<source>` children in order.
fn select(player: &mut Player, env: &Env<'_>, out: &mut Vec<Action>) {
    let document = env.document;
    let node = player.node;
    player.network = NETWORK_NO_SOURCE;
    player.src_seen = document.attr(node, "src").map(str::to_owned);
    if let Some(src) = document.attr(node, "src") {
        if src.is_empty() {
            fail(
                player,
                Failure {
                    code: 4,
                    message: "MEDIA_ELEMENT_ERROR: Empty src attribute".to_owned(),
                },
                out,
            );
            return;
        }
        let url = env.page.and_then(|page| page.resolve(document, src));
        match url {
            Some(url) => {
                player.candidates.clear();
                player.trying = None;
                fetch(player, url, env, out, true);
            }
            None => fail(player, format_error(), out),
        }
        return;
    }
    let sources: Vec<NodeId> = document
        .children(node)
        .filter(|child| document.tag(*child) == Some("source"))
        .collect();
    if sources.is_empty() {
        player.network = NETWORK_EMPTY;
        return;
    }
    player.candidates = sources;
    next_source(player, env, out, true);
}

/// Tries the next `<source>`: one with no `src`, an unresolvable one, or
/// one whose `type` cannot be played, fails at once.
fn next_source(player: &mut Player, env: &Env<'_>, out: &mut Vec<Action>, first: bool) {
    let document = env.document;
    let mut first = first;
    while !player.candidates.is_empty() {
        let source = player.candidates.remove(0);
        let src = document.attr(source, "src").filter(|src| !src.is_empty());
        let playable = document
            .attr(source, "type")
            .is_none_or(|kind| media::can_play_type(kind) != media::CanPlay::No);
        let url = src.and_then(|src| env.page.and_then(|page| page.resolve(document, src)));
        match url {
            Some(url) if playable => {
                player.trying = Some(source);
                fetch(player, url, env, out, first);
                return;
            }
            _ => {
                out.push(Action::Event(source, "error"));
            }
        }
        first = false;
    }
    // None left: the element waits for another `<source>`.
    player.trying = None;
    player.network = NETWORK_NO_SOURCE;
    if first {
        // HTML fires `loadstart` before it looks at the children.
        out.insert(0, Action::Event(player.node, "loadstart"));
    }
}

/// HTML's resource fetch algorithm: `url` fetched into its buffer, by
/// the engine once the call is done, and read by a pipeline.
fn fetch(
    player: &mut Player,
    url: net::Url,
    env: &Env<'_>,
    out: &mut Vec<Action>,
    loadstart: bool,
) {
    let document = env.document;
    let node = player.node;
    player.current_src = url.to_string();
    player.network = NETWORK_LOADING;
    if loadstart {
        out.push(Action::Event(node, "loadstart"));
    }
    let Some(page) = env.page else {
        fail(
            player,
            Failure {
                code: 2,
                message: "MEDIA_ELEMENT_ERROR: Network error".to_owned(),
            },
            out,
        );
        return;
    };
    let destination = if player.video {
        Destination::Video
    } else {
        Destination::Audio
    };
    let mut request = Request::get(url.clone(), destination);
    if let Some(initiator) = page.initiator() {
        request = request.initiated_by(initiator);
    }
    match CrossOrigin::of(document.attr(node, "crossorigin")) {
        CrossOrigin::None => {}
        CrossOrigin::Anonymous => request = request.with_cors(net::Credentials::SameOrigin),
        CrossOrigin::UseCredentials => request = request.with_cors(net::Credentials::Include),
    }
    request.referrer_policy = Some(page.referrer_policy(document));
    let hint = media::Hint {
        mime: None,
        extension: url
            .path_str()
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, extension)| extension.to_ascii_lowercase()),
    };
    player.pending_fetch = Some((request, hint));
}

/// The resource failed: the element's error, or the next `<source>`.
fn fail(player: &mut Player, failure: Failure, out: &mut Vec<Action>) {
    player.drop_pipeline();
    if let Some(source) = player.trying.take() {
        out.push(Action::Event(source, "error"));
        return;
    }
    player.error = Some(failure);
    player.network = NETWORK_NO_SOURCE;
    out.push(Action::Event(player.node, "error"));
    player.reject_pending("NotSupportedError", NO_SOURCES, out);
}

/// Whether `play()` may start sound now (the autoplay policy): with the
/// user's activation, or muted.
fn allowed(player: &Player, env: &Env<'_>) -> bool {
    env.activated || player.muted || player.volume == 0.0
}

fn play(
    player: &mut Player,
    promise: u64,
    env: &Env<'_>,
    out: &mut Vec<Action>,
) -> Result<(), (&'static str, String)> {
    if !allowed(player, env) {
        return Err(("NotAllowedError", NOT_ALLOWED.to_owned()));
    }
    if player.error.as_ref().is_some_and(|e| e.code == 4) {
        return Err(("NotSupportedError", NO_SOURCES.to_owned()));
    }
    player.pending.push(promise);
    if player.network == NETWORK_EMPTY {
        select(player, env, out);
    }
    let node = player.node;
    // At the end, from the start again.
    if player.ended
        || (player.duration.is_finite()
            && player.position() >= player.duration - 1e-6
            && player.ready >= HAVE_METADATA)
    {
        player.ended = false;
        seek(player, 0.0, out);
    }
    if player.paused {
        player.paused = false;
        out.push(Action::Event(node, "play"));
        if player.ready < HAVE_ENOUGH_DATA || player.seeking {
            out.push(Action::Event(node, "waiting"));
        } else {
            out.push(Action::Event(node, "playing"));
            player.resolve_pending(out);
        }
        player.played_from = Some(player.position());
        if let Some(pipeline) = &player.pipeline {
            pipeline.set_playing(true);
        }
    } else if player.ready >= HAVE_ENOUGH_DATA {
        player.resolve_pending(out);
    }
    player.can_autoplay = false;
    Ok(())
}

fn pause(player: &mut Player, env: &Env<'_>, out: &mut Vec<Action>) {
    if player.network == NETWORK_EMPTY {
        select(player, env, out);
    }
    player.can_autoplay = false;
    if !player.paused {
        internal_pause(player, out);
    }
}

/// HTML's internal pause steps.
fn internal_pause(player: &mut Player, out: &mut Vec<Action>) {
    player.mark_played();
    player.position = player.position();
    player.paused = true;
    if let Some(pipeline) = &player.pipeline {
        pipeline.set_playing(false);
    }
    out.push(Action::Event(player.node, "timeupdate"));
    out.push(Action::Event(player.node, "pause"));
    player.reject_pending("AbortError", PAUSED, out);
}

/// HTML's seek, to `time`.
fn seek(player: &mut Player, time: f64, out: &mut Vec<Action>) {
    if player.ready == HAVE_NOTHING {
        player.start_at = time.max(0.0);
        return;
    }
    let mut time = time.max(0.0);
    if player.duration.is_finite() {
        time = time.min(player.duration);
    }
    player.mark_played();
    player.ended = false;
    player.seeking = true;
    player.position = time;
    player.ready = HAVE_METADATA;
    player.seek_id += 1;
    out.push(Action::Event(player.node, "seeking"));
    if let Some(pipeline) = &player.pipeline {
        pipeline.seek(player.seek_id, time);
    }
}

/// What a report means.
fn reported(player: &mut Player, report: Report, env: &Env<'_>, out: &mut Vec<Action>) {
    let node = player.node;
    match report {
        Report::Fetched => {
            if player.network == NETWORK_LOADING {
                player.network = NETWORK_IDLE;
            }
            player.whole = true;
            out.push(Action::Event(node, "progress"));
            out.push(Action::Event(node, "suspend"));
        }
        Report::Meta(meta) => metadata(player, &meta, out),
        Report::Ready(meta) => {
            if player.ready < HAVE_METADATA {
                // Its head and first picture together: every state at
                // once, as the reference's events see it.
                player.ready = HAVE_ENOUGH_DATA;
                metadata(player, &meta, out);
            }
            player.ready = HAVE_ENOUGH_DATA;
            out.push(Action::Event(node, "loadeddata"));
            out.push(Action::Event(node, "canplay"));
            out.push(Action::Event(node, "canplaythrough"));
            if !player.paused {
                out.push(Action::Event(node, "playing"));
                player.resolve_pending(out);
            } else if player.can_autoplay
                && env.document.attr(node, "autoplay").is_some()
                && allowed(player, env)
            {
                player.paused = false;
                player.can_autoplay = false;
                player.played_from = Some(player.position());
                if let Some(pipeline) = &player.pipeline {
                    pipeline.set_playing(true);
                }
                out.push(Action::Event(node, "play"));
                out.push(Action::Event(node, "playing"));
            }
        }
        Report::Seeked(id, time) => {
            if id != player.seek_id || !player.seeking {
                return;
            }
            player.seeking = false;
            player.position = time;
            player.ready = HAVE_ENOUGH_DATA;
            if !player.paused {
                player.played_from = Some(time);
            }
            out.push(Action::Event(node, "timeupdate"));
            out.push(Action::Event(node, "seeked"));
            out.push(Action::Event(node, "canplay"));
            out.push(Action::Event(node, "canplaythrough"));
            if !player.paused {
                out.push(Action::Event(node, "playing"));
            }
        }
        Report::Time(time) => {
            if !player.paused && !player.seeking {
                player.position = time;
                out.push(Action::Event(node, "timeupdate"));
            }
        }
        Report::Ended(time) => {
            if player.paused || player.seeking {
                return;
            }
            player.position = time;
            if env.document.attr(node, "loop").is_some() {
                seek(player, 0.0, out);
                if let Some(pipeline) = &player.pipeline {
                    pipeline.set_playing(true);
                }
                return;
            }
            player.mark_played();
            player.ended = true;
            out.push(Action::Event(node, "timeupdate"));
            player.paused = true;
            out.push(Action::Event(node, "pause"));
            out.push(Action::Event(node, "ended"));
        }
        Report::Failed(failure) => {
            let failing_source = player.trying.is_some();
            fail(player, failure, out);
            if failing_source {
                next_source(player, env, out, false);
            }
        }
    }
}

/// The resource's head read: its duration, its size, `HAVE_METADATA`.
fn metadata(player: &mut Player, meta: &Meta, out: &mut Vec<Action>) {
    let node = player.node;
    player.ready = player.ready.max(HAVE_METADATA);
    player.duration = meta.duration;
    out.push(Action::Event(node, "durationchange"));
    if let Some(size) = meta.size {
        player.size = size;
        out.push(Action::Event(node, "resize"));
    }
    out.push(Action::Event(node, "loadedmetadata"));
}
