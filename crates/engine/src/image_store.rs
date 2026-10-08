//! A document's images (M23.1, M23.5; `docs/design/images.md`,
//! "Fetching" and "Two decodes"): each URL fetched once, its header read
//! when it arrives and its pixels decoded when first drawn, both on
//! threads of their own.

use std::collections::HashMap;
use std::sync::Arc;

use dom::{Document, NodeId};
use images::{Header, Image};

use crate::loader::{JobId, Lost, Page, PageId, Pool, Waker};

/// The most images decoded at once, each on a thread of its own.
pub const MAX_DECODE_THREADS: usize = 4;

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
}

/// What a decode thread is given.
pub(crate) enum Work {
    Header(Arc<[u8]>),
    Pixels(Arc<[u8]>),
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
}

impl ImageStore {
    #[cfg(test)]
    pub(crate) fn get(&self, url: &str) -> Option<&ImageEntry> {
        self.entries.get(url)
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
                    },
                );
                *next += 1;
                wanted.push(url);
            }
            self.sources.insert(node, key);
        }
        wanted
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
    /// pixels. Says whether the image's size or brokenness changed.
    pub(crate) fn done(
        &mut self,
        url: &str,
        done: Result<Done, Lost>,
        bytes: Option<Arc<[u8]>>,
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
                false
            }
            (Err(_), ImageState::Ready(ready)) => {
                ready.pixels = Pixels::Failed;
                false
            }
            _ => false,
        }
    }

    /// The pixels of `url` if decoded; otherwise, the first time they are
    /// asked for, the work that decodes them.
    pub(crate) fn pixels(&mut self, url: &str) -> (Option<Arc<Image>>, Option<Work>) {
        let Some(ImageEntry {
            state: ImageState::Ready(ready),
            ..
        }) = self.entries.get_mut(url)
        else {
            return (None, None);
        };
        match &ready.pixels {
            Pixels::Decoded(image) => (Some(image.clone()), None),
            Pixels::NotDecoded => {
                ready.pixels = Pixels::Decoding;
                (None, Some(Work::Pixels(ready.bytes.clone())))
            }
            Pixels::Decoding | Pixels::Failed => (None, None),
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
            Work::Header(bytes) | Work::Pixels(bytes) => bytes.clone(),
        };
        self.jobs.insert(id, Job { page, url, bytes });
        self.pool.run(id, move || match work {
            Work::Header(bytes) => Done::Header(images::read_header(&bytes)),
            Work::Pixels(bytes) => Done::Pixels(images::decode(&bytes)),
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
