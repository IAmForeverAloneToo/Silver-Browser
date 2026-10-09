//! A document's images (M23.1, M23.5; `docs/design/images.md`,
//! "Fetching" and "Two decodes"): each URL fetched once, its header read
//! when it arrives and its pixels decoded when first drawn, both on
//! threads of their own.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dom::{Document, NodeId};
use images::{Header, Image};

use crate::loader::{JobId, Lost, Page, PageId, Pool, Waker};

/// The most images decoded at once, each on a thread of its own.
pub const MAX_DECODE_THREADS: usize = 4;

/// The bytes of rendered gradients a document keeps (M23.3): past it the
/// ones drawn longest ago are dropped, and rendered again when drawn.
pub const GRADIENT_BUDGET: usize = 64 << 20;

/// The bytes of decoded pixels a document keeps (M23.5): past it the
/// images drawn longest ago, none drawn in the frame, are dropped, and
/// decoded again when next drawn.
pub const MAX_DECODED_BYTES: usize = 512 << 20;

/// What became of an image's URL.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImageState {
    /// Being fetched.
    Fetching,
    /// Fetched, its header being read.
    Reading,
    /// Its header read: its size is known.
    Ready(Ready),
    /// Not fetched, or not an image this engine reads.
    Broken,
}

/// An image whose header was read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ready {
    pub(crate) header: Header,
    bytes: Arc<[u8]>,
    pub(crate) pixels: Pixels,
    /// When on the document's timeline, in seconds, the header was read:
    /// where an animation's timeline starts, which every box showing it
    /// shares (HTML section 15.4.2, M23.1).
    pub(crate) started: f64,
    /// The number of the last frame that drew it, and the largest it was
    /// drawn at, in raster pixels each way (M23.5).
    drawn: u64,
    largest: (f32, f32),
    /// The largest drawn size the pixels kept are a downscaled copy for,
    /// `None` for the natural size; and while a decode is under way, the
    /// one it is for.
    kept_for: Option<(f32, f32)>,
    asked: Option<Option<(f32, f32)>>,
}

impl Ready {
    /// What the pixels are decoded for, drawn as large as they have been
    /// (M23.5): a copy for that size where it is under half the natural
    /// size both ways, else the natural size.
    fn wanted(&self) -> Option<(f32, f32)> {
        let (w, h) = self.largest;
        (w > 0.0
            && h > 0.0
            && w * 2.0 < self.header.width as f32
            && h * 2.0 < self.header.height as f32)
            .then_some((w, h))
    }

    /// The work that decodes the pixels for `wanted`: at twice that
    /// size for a copy.
    fn work(&mut self, wanted: Option<(f32, f32)>) -> Work {
        self.asked = Some(wanted);
        let size = wanted.map(|(w, h)| ((w * 2.0).ceil() as u32, (h * 2.0).ceil() as u32));
        Work::Pixels(self.bytes.clone(), size)
    }
}

/// An image's pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Pixels {
    NotDecoded,
    Decoding,
    Decoded(Arc<Image>),
    /// The header read but the image did not decode: drawn as broken.
    Failed,
}

/// One URL's image.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageEntry {
    /// Its number, unique in the engine: what display items compare.
    pub(crate) id: u64,
    pub(crate) state: ImageState,
    /// Whether its URL is same origin with the document's (M23.2).
    same_origin: bool,
}

/// Whether `url` is same origin with the document `page` shows: never
/// for a page with no URL, nor for a `file:` one, whose origin is
/// opaque.
fn same_origin(page: &Page, url: &net::Url) -> bool {
    page.url()
        .is_some_and(|document| document.origin() == url.origin())
}

impl ImageEntry {
    /// The header the boxes showing it are given (M23.2): of an image of
    /// another origin, its orientation as applied for good, which
    /// `image-orientation: none` does not undo (CSS Images 3 section 5.3).
    fn shown_header(&self, header: Header) -> Header {
        if self.same_origin {
            return header;
        }
        Header {
            orientation: images::Orientation::Normal,
            ..header
        }
    }
}

/// What a decode thread is given: bytes whose header to read, or whose
/// pixels to decode, downscaled to a size where one is given (M23.5).
pub(crate) enum Work {
    Header(Arc<[u8]>),
    Pixels(Arc<[u8]>, Option<(u32, u32)>),
}

/// What a decode thread gives back.
pub(crate) enum Done {
    Header(Result<Header, images::Error>),
    Pixels(Result<Image, images::Error>),
}

/// A document's images: HTML's list of available images, by URL, and
/// the elements showing each.
#[derive(Default)]
pub(crate) struct ImageStore {
    entries: HashMap<String, ImageEntry>,
    /// Each `img` element's URL, as last scanned.
    sources: HashMap<NodeId, String>,
    /// Each entry's URL by its number, which display items name it by.
    ids: HashMap<u64, String>,
    /// Each URL a style's `<image>` names, as the style has it, and the
    /// URL of its entry (M23.3): what paint asks for an image by. `None`
    /// for one that does not resolve.
    css: HashMap<Arc<str>, Option<String>>,
    /// The entries a marker or a `content` shows, whose size is laid out
    /// (M23.3), by URL: their arrival lays the document out again.
    laid_out: HashSet<String>,
    /// The gradients rendered (M23.3), by their numbers, with when each
    /// was last asked for; and the bytes they hold.
    gradients: RefCell<Gradients>,
    /// The frames drawn, which number them (M23.5).
    frames: u64,
    /// The decoded bytes kept, when not [`MAX_DECODED_BYTES`]: a test's.
    budget: Option<usize>,
}

/// Rendered gradients, kept under [`GRADIENT_BUDGET`].
#[derive(Default)]
struct Gradients {
    kept: HashMap<u64, (Arc<Image>, u64)>,
    bytes: usize,
    asked: u64,
}

impl ImageStore {
    #[cfg(test)]
    pub(crate) fn get(&self, url: &str) -> Option<&ImageEntry> {
        self.entries.get(url)
    }

    /// Whether `url`'s image is known broken, `Some(true)`, or ready with
    /// its size, `Some(false)`, or neither yet (M26: what its `img`
    /// elements hear).
    pub(crate) fn outcome(&self, url: &str) -> Option<bool> {
        match &self.entries.get(url)?.state {
            ImageState::Ready(ready) => Some(ready.pixels == Pixels::Failed),
            ImageState::Broken => Some(true),
            ImageState::Fetching | ImageState::Reading => None,
        }
    }

    /// The URLs whose images are known ready or broken (M26).
    pub(crate) fn known(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| matches!(entry.state, ImageState::Ready(_) | ImageState::Broken))
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Whether no image is still being fetched or read: what the load
    /// waits for.
    pub(crate) fn settled(&self) -> bool {
        self.entries
            .values()
            .all(|entry| !matches!(entry.state, ImageState::Fetching | ImageState::Reading))
    }

    /// Scans `document` for the images its `img` elements show, against
    /// `page`'s base URL, and returns the URLs not yet wanted, which are
    /// now being fetched. `next` numbers the new entries.
    pub(crate) fn scan(
        &mut self,
        document: &Document,
        page: &Page,
        next: &mut u64,
    ) -> Vec<net::Url> {
        self.sources.clear();
        let mut wanted = Vec::new();
        for node in document.subtree(document.root()) {
            if document.tag(node) != Some("img") {
                continue;
            }
            let Some(url) = document
                .attr(node, "src")
                .filter(|src| !src.is_empty())
                .and_then(|src| page.resolve(document, src))
            else {
                continue;
            };
            let key = url.as_str().to_owned();
            if !self.entries.contains_key(&key) {
                self.entries.insert(
                    key.clone(),
                    ImageEntry {
                        id: *next,
                        state: ImageState::Fetching,
                        same_origin: same_origin(page, &url),
                    },
                );
                self.ids.insert(*next, key.clone());
                *next += 1;
                wanted.push(url);
            }
            self.sources.insert(node, key);
        }
        wanted
    }

    /// Scans the computed `styles` of `document`'s elements, and of their
    /// `::before`, `::after` and markers, for the images their
    /// backgrounds, list markers and `content` name (M23.3), against
    /// `page`'s base URL, which leaves one a fetched sheet made absolute
    /// as it is; returns the URLs not yet wanted, which are now being
    /// fetched. `next` numbers the new entries.
    pub(crate) fn scan_styles(
        &mut self,
        document: &Document,
        styles: &style::Styles,
        page: &Page,
        next: &mut u64,
    ) -> Vec<net::Url> {
        let mut wanted = Vec::new();
        for node in document.subtree(document.root()) {
            let pseudo = styles.pseudo_styles(node);
            let all = styles.get(node).into_iter().chain(
                pseudo
                    .into_iter()
                    .flat_map(|p| [p.before.as_ref(), p.after.as_ref(), p.marker.as_ref()])
                    .flatten(),
            );
            for style in all {
                // The images a style names, and whether its box's size
                // depends on each.
                let backgrounds = style
                    .background
                    .iter()
                    .flat_map(|b| b.image.iter().flatten())
                    .chain(style.border_image.iter().filter_map(|b| b.source.as_ref()))
                    .map(|image| (image, false));
                let marker = style
                    .lists
                    .iter()
                    .filter_map(|l| l.list_style_image.as_ref())
                    .map(|image| (image, true));
                let content = match &style.content {
                    style::computed::Content::Items(items) => items.as_ref(),
                    _ => &[],
                }
                .iter()
                .filter_map(|item| match item {
                    style::computed::ContentItem::Image(image) => Some((image, true)),
                    _ => None,
                });
                for (image, lays_out) in backgrounds.chain(marker).chain(content) {
                    let style::computed::Image::Url { url: written, .. } = image else {
                        continue;
                    };
                    if lays_out && let Some(Some(key)) = self.css.get(written) {
                        self.laid_out.insert(key.clone());
                    }
                    if written.is_empty() || self.css.contains_key(written) {
                        continue;
                    }
                    let url = page.resolve(document, written);
                    let key = url.as_ref().map(|url| url.as_str().to_owned());
                    self.css.insert(Arc::clone(written), key.clone());
                    if lays_out && let Some(key) = &key {
                        self.laid_out.insert(key.clone());
                    }
                    if let (Some(url), Some(key)) = (url, key)
                        && !self.entries.contains_key(&key)
                    {
                        self.entries.insert(
                            key.clone(),
                            ImageEntry {
                                id: *next,
                                state: ImageState::Fetching,
                                same_origin: same_origin(page, &url),
                            },
                        );
                        self.ids.insert(*next, key);
                        *next += 1;
                        wanted.push(url);
                    }
                }
            }
        }
        wanted
    }

    /// Whether a marker or a `content` shows the image at `url`, whose
    /// arrival changes a box's size (M23.3).
    pub(crate) fn lays_out(&self, url: &str) -> bool {
        self.laid_out.contains(url)
    }

    /// The elements showing `url`.
    pub(crate) fn users(&self, url: &str) -> Vec<NodeId> {
        self.sources
            .iter()
            .filter(|(_, source)| source.as_str() == url)
            .map(|(&node, _)| node)
            .collect()
    }

    /// Takes the body fetched for `url`, or its failure: what to read,
    /// if anything. The HTTP status is not looked at (HTML section
    /// 4.8.4.3): a body that is no image, an empty one too, fails to read.
    pub(crate) fn fetched(&mut self, url: &str, body: Option<Vec<u8>>) -> Option<Work> {
        let entry = self.entries.get_mut(url)?;
        match body {
            Some(body) => {
                entry.state = ImageState::Reading;
                Some(Work::Header(Arc::from(body)))
            }
            None => {
                entry.state = ImageState::Broken;
                None
            }
        }
    }

    /// Takes a decode thread's result for `url`, its bytes kept for the
    /// pixels, at `now` on the document's timeline. Says whether the
    /// image's size or brokenness changed.
    pub(crate) fn done(
        &mut self,
        url: &str,
        done: Result<Done, Lost>,
        bytes: Option<Arc<[u8]>>,
        now: f64,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(url) else {
            return false;
        };
        match (done, &mut entry.state) {
            (Ok(Done::Header(Ok(header))), ImageState::Reading) => {
                let bytes = bytes.expect("a header's bytes come back with it");
                entry.state = ImageState::Ready(Ready {
                    header,
                    bytes,
                    pixels: Pixels::NotDecoded,
                    started: now,
                    drawn: 0,
                    largest: (0.0, 0.0),
                    kept_for: None,
                    asked: None,
                });
                true
            }
            (Ok(Done::Header(Err(_))) | Err(_), ImageState::Reading) => {
                entry.state = ImageState::Broken;
                true
            }
            (Ok(Done::Pixels(result)), ImageState::Ready(ready)) => {
                ready.pixels = match result {
                    Ok(image) => Pixels::Decoded(Arc::new(image)),
                    Err(_) => Pixels::Failed,
                };
                ready.kept_for = ready.asked.take().flatten();
                false
            }
            (Err(_), ImageState::Ready(ready)) => {
                ready.pixels = Pixels::Failed;
                ready.asked = None;
                false
            }
            _ => false,
        }
    }

    /// When the timeline of the image numbered `id` starts (M23.1).
    pub(crate) fn started(&self, id: u64) -> Option<f64> {
        match &self.entries.get(self.url_of(id)?)?.state {
            ImageState::Ready(ready) => Some(ready.started),
            _ => None,
        }
    }

    /// The URL of the image numbered `id`.
    pub(crate) fn url_of(&self, id: u64) -> Option<&str> {
        self.ids.get(&id).map(String::as_str)
    }

    /// The pixels of the image numbered `id`, decoded now on this thread
    /// if they are not yet: what a print draws, which waits for nothing.
    pub(crate) fn decoded_now(&self, id: u64) -> Option<Arc<Image>> {
        let ImageState::Ready(ready) = &self.entries.get(self.url_of(id)?)?.state else {
            return None;
        };
        match &ready.pixels {
            Pixels::Decoded(image) if ready.kept_for.is_none() => Some(image.clone()),
            Pixels::Failed => None,
            // A downscaled copy is for the screen: the page has it whole.
            Pixels::Decoded(_) | Pixels::NotDecoded | Pixels::Decoding => {
                images::decode(&ready.bytes).ok().map(Arc::new)
            }
        }
    }

    /// The pixels of `url` if decoded; otherwise, the first time they are
    /// asked for, the work that decodes them, for the largest it was
    /// drawn at (M23.5). A downscaled copy since drawn larger than it was
    /// made for is decoded again, and drawn until then.
    pub(crate) fn pixels(&mut self, url: &str) -> (Option<Arc<Image>>, Option<Work>) {
        let Some(ImageEntry {
            state: ImageState::Ready(ready),
            ..
        }) = self.entries.get_mut(url)
        else {
            return (None, None);
        };
        match &ready.pixels {
            Pixels::Decoded(image) => {
                let image = image.clone();
                let (w, h) = ready.largest;
                let small = ready.kept_for.is_some_and(|k| w > k.0 || h > k.1);
                if small && ready.asked.is_none() {
                    let wanted = ready.wanted();
                    return (Some(image), Some(ready.work(wanted)));
                }
                (Some(image), None)
            }
            Pixels::NotDecoded => {
                ready.pixels = Pixels::Decoding;
                let wanted = ready.wanted();
                (None, Some(ready.work(wanted)))
            }
            Pixels::Decoding | Pixels::Failed => (None, None),
        }
    }

    /// Whether a decode is under way (M23.5).
    pub(crate) fn decoding(&self) -> bool {
        self.entries
            .values()
            .any(|entry| matches!(&entry.state, ImageState::Ready(ready) if ready.asked.is_some()))
    }

    /// Starts frame drawing: its number.
    pub(crate) fn next_frame(&mut self) -> u64 {
        self.frames += 1;
        self.frames
    }

    /// The image numbered `id` was drawn in frame `frame` at `size`, in
    /// raster pixels each way (M23.5).
    pub(crate) fn drew(&mut self, id: u64, frame: u64, size: (f32, f32)) {
        let Some(url) = self.ids.get(&id) else {
            return;
        };
        if let Some(ImageEntry {
            state: ImageState::Ready(ready),
            ..
        }) = self.entries.get_mut(url)
        {
            ready.drawn = frame;
            ready.largest.0 = ready.largest.0.max(size.0);
            ready.largest.1 = ready.largest.1.max(size.1);
        }
    }

    /// Drops the decoded pixels of the images drawn longest ago, none
    /// drawn in frame `frame`, while all kept hold more than the budget
    /// (M23.5); they are decoded again when next drawn.
    pub(crate) fn keep_within_budget(&mut self, frame: u64) {
        let budget = self.budget.unwrap_or(MAX_DECODED_BYTES);
        let mut kept: Vec<(u64, usize, &String)> = Vec::new();
        let mut total = 0;
        for (url, entry) in &self.entries {
            if let ImageState::Ready(ready) = &entry.state
                && let Pixels::Decoded(image) = &ready.pixels
            {
                total += image.bytes();
                if ready.drawn < frame {
                    kept.push((ready.drawn, image.bytes(), url));
                }
            }
        }
        if total <= budget {
            return;
        }
        kept.sort_unstable();
        let dropped: Vec<String> = kept
            .into_iter()
            .take_while(|(_, bytes, _)| {
                let over = total > budget;
                total = total.saturating_sub(*bytes);
                over
            })
            .map(|(_, _, url)| url.clone())
            .collect();
        for url in dropped {
            if let Some(ImageEntry {
                state: ImageState::Ready(ready),
                ..
            }) = self.entries.get_mut(&url)
            {
                ready.pixels = Pixels::NotDecoded;
                ready.kept_for = None;
            }
        }
    }

    /// Keeps `bytes` of decoded pixels in place of [`MAX_DECODED_BYTES`].
    #[cfg(test)]
    pub(crate) fn set_budget(&mut self, bytes: usize) {
        self.budget = Some(bytes);
    }
}

/// What each `img` shows, as the box tree builder asks (M23.2): the
/// image its URL names once its header is read, unless its pixels did
/// not decode; nothing yet while it is fetched or read; broken when it
/// was not fetched, did not read, or has a URL that does not parse or
/// was never fetched.
impl layout::Images for ImageStore {
    fn image(&self, node: NodeId) -> layout::ImageState {
        let entry = self
            .sources
            .get(&node)
            .and_then(|url| self.entries.get(url));
        match entry.map(|entry| (entry, &entry.state)) {
            Some((entry, ImageState::Ready(ready))) if ready.pixels != Pixels::Failed => {
                layout::ImageState::Ready {
                    id: entry.id,
                    header: entry.shown_header(ready.header),
                }
            }
            Some((_, ImageState::Fetching | ImageState::Reading)) => layout::ImageState::Coming,
            _ => layout::ImageState::Broken,
        }
    }

    /// The image a style's URL names, as [`ImageStore::scan_styles`]
    /// found it (M23.3): coming until scanned.
    fn url_image(&self, url: &str) -> layout::ImageState {
        let Some(key) = self.css.get(url) else {
            return layout::ImageState::Coming;
        };
        let entry = key.as_ref().and_then(|key| self.entries.get(key));
        match entry.map(|entry| (entry, &entry.state)) {
            Some((entry, ImageState::Ready(ready))) if ready.pixels != Pixels::Failed => {
                layout::ImageState::Ready {
                    id: entry.id,
                    header: entry.shown_header(ready.header),
                }
            }
            Some((_, ImageState::Fetching | ImageState::Reading)) => layout::ImageState::Coming,
            _ => layout::ImageState::Broken,
        }
    }
}

/// What paint asks of a style's images (M23.3): the image a URL names,
/// as the style has it, once its header is read, unless its pixels did
/// not decode.
impl paint::Pictures for ImageStore {
    /// A gradient rendered once for its size and kept, the least
    /// recently asked for dropped past the budget.
    fn gradient(&self, gradient: &style::computed::Gradient, w: f32, h: f32) -> Arc<Image> {
        let number = paint::gradient::number(gradient, w, h);
        let mut gradients = self.gradients.borrow_mut();
        gradients.asked += 1;
        let asked = gradients.asked;
        if let Some((image, when)) = gradients.kept.get_mut(&number) {
            *when = asked;
            return Arc::clone(image);
        }
        let image = Arc::new(paint::gradient::render(gradient, w, h));
        gradients.bytes += image.bytes();
        while gradients.bytes > GRADIENT_BUDGET {
            let Some(oldest) = gradients
                .kept
                .iter()
                .min_by_key(|(_, (_, when))| *when)
                .map(|(number, _)| *number)
            else {
                break;
            };
            if let Some((dropped, _)) = gradients.kept.remove(&oldest) {
                gradients.bytes -= dropped.bytes();
            }
        }
        gradients.kept.insert(number, (Arc::clone(&image), asked));
        image
    }

    /// Coming until scanned, and while fetched or read.
    fn coming(&self, url: &str) -> bool {
        let Some(key) = self.css.get(url) else {
            return true;
        };
        let entry = key.as_ref().and_then(|key| self.entries.get(key));
        matches!(
            entry.map(|entry| &entry.state),
            Some(ImageState::Fetching | ImageState::Reading)
        )
    }

    fn picture(&self, url: &str) -> Option<(u64, images::Header)> {
        let key = self.css.get(url)?.as_ref()?;
        let entry = self.entries.get(key)?;
        match &entry.state {
            ImageState::Ready(ready) if ready.pixels != Pixels::Failed => {
                Some((entry.id, entry.shown_header(ready.header)))
            }
            _ => None,
        }
    }
}

/// The threads images are read and decoded on, and what each is for.
pub(crate) struct Decoder {
    pool: Pool<Done>,
    jobs: HashMap<JobId, Job>,
    next: JobId,
}

struct Job {
    page: PageId,
    url: String,
    bytes: Arc<[u8]>,
}

/// A finished job: its page, its URL, what it gave, and the bytes it
/// was given.
pub(crate) struct Finished {
    pub(crate) page: PageId,
    pub(crate) url: String,
    pub(crate) done: Result<Done, Lost>,
    pub(crate) bytes: Arc<[u8]>,
}

impl Decoder {
    pub(crate) fn new() -> Decoder {
        Decoder {
            pool: Pool::new(MAX_DECODE_THREADS),
            jobs: HashMap::new(),
            next: 0,
        }
    }

    pub(crate) fn set_waker(&self, waker: Waker) {
        self.pool.set_waker(waker);
    }

    /// Starts `work` for `url` of `page`.
    pub(crate) fn start(&mut self, page: PageId, url: String, work: Work) {
        let id = self.next;
        self.next += 1;
        let bytes = match &work {
            Work::Header(bytes) | Work::Pixels(bytes, _) => bytes.clone(),
        };
        self.jobs.insert(id, Job { page, url, bytes });
        self.pool.run(id, move |_| match work {
            Work::Header(bytes) => Done::Header(images::read_header(&bytes)),
            Work::Pixels(bytes, size) => {
                Done::Pixels(images::decode(&bytes).map(|image| match size {
                    Some((w, h)) if w < image.width && h < image.height => {
                        images::downscale(&image, w, h)
                    }
                    _ => image,
                }))
            }
        });
    }

    /// A finished job, if one has finished.
    pub(crate) fn take(&mut self) -> Option<Finished> {
        while let Some((id, done)) = self.pool.try_take() {
            if let Some(job) = self.jobs.remove(&id) {
                return Some(Finished {
                    page: job.page,
                    url: job.url,
                    done,
                    bytes: job.bytes,
                });
            }
        }
        None
    }

    /// Whether `page` has work running or waiting.
    pub(crate) fn busy(&self, page: PageId) -> bool {
        self.jobs.values().any(|job| job.page == page)
    }

    /// Forgets `page`'s work: its results are not taken.
    pub(crate) fn forget(&mut self, page: PageId) {
        self.jobs.retain(|_, job| job.page != page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paint::Pictures;

    #[test]
    fn pixels_are_wanted_at_twice_the_largest_drawn_size_when_under_half() {
        let mut ready = Ready {
            header: Header {
                format: images::Format::Png,
                width: 100,
                height: 50,
                orientation: images::Orientation::Normal,
            },
            bytes: Arc::from(Vec::new()),
            pixels: Pixels::NotDecoded,
            started: 0.0,
            drawn: 0,
            largest: (0.0, 0.0),
            kept_for: None,
            asked: None,
        };
        let size = |ready: &mut Ready, largest| {
            ready.largest = largest;
            let wanted = ready.wanted();
            match ready.work(wanted) {
                Work::Pixels(_, size) => size,
                Work::Header(_) => unreachable!("pixels"),
            }
        };
        assert_eq!(size(&mut ready, (0.0, 0.0)), None, "never drawn: whole");
        assert_eq!(size(&mut ready, (10.5, 20.0)), Some((21, 40)));
        assert_eq!(ready.asked, Some(Some((10.5, 20.0))));
        assert_eq!(size(&mut ready, (49.0, 25.0)), None, "half the height");
        assert_eq!(size(&mut ready, (50.0, 10.0)), None, "half the width");
        assert_eq!(ready.asked, Some(None));
    }

    fn gradient(colour: &str) -> style::computed::Gradient {
        let html =
            format!("<div style=\"background: linear-gradient(45deg, {colour}, blue)\"></div>");
        let document = Document::from_node(&html::parse_document(&html).expect("never fails"));
        let styles = style::Stylesheets::new(Vec::new())
            .cascade(&document, &style::CascadeContext::default());
        let div = document
            .subtree(document.root())
            .find(|&n| document.tag(n) == Some("div"))
            .expect("the div");
        match &styles.get(div).expect("styled").background().image[0] {
            Some(style::computed::Image::Gradient(g)) => (**g).clone(),
            other => panic!("not a gradient: {other:?}"),
        }
    }

    #[test]
    fn a_gradient_is_rendered_once_for_its_size_and_kept_under_the_budget() {
        let store = ImageStore::default();
        let red = gradient("red");
        let a = store.gradient(&red, 40.0, 30.0);
        assert!(Arc::ptr_eq(&a, &store.gradient(&red, 40.0, 30.0)));
        assert!(!Arc::ptr_eq(&a, &store.gradient(&red, 40.0, 31.0)));
        // Large ones push out the least recently asked for.
        for i in 0..5 {
            store.gradient(&red, 2000.0 - i as f32, 2000.0);
        }
        let gradients = store.gradients.borrow();
        assert!(gradients.bytes <= GRADIENT_BUDGET);
        assert!(gradients.kept.len() < 10);
        assert_eq!(
            gradients.bytes,
            gradients
                .kept
                .values()
                .map(|(i, _)| i.bytes())
                .sum::<usize>()
        );
    }
}
