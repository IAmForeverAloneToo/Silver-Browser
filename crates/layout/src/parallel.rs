//! Layout's work on a pool of threads (M35.3, `docs/design/performance.md`,
//! "Threads"): a flex container's items, a grid's and a table's cells,
//! each laid out with the inputs its container gave it, have no order
//! between them, and when there are at least [`PARALLEL_MIN_ITEMS`] of
//! them they are laid out on a pool of at most [`MAX_WORKER_THREADS`]
//! threads. What a pass shares across them, its memo of items and
//! widths and the layout cache, is behind locks ([`Locked`], [`Count`]);
//! the font measures are sharded (`text`). The fragments made are the
//! same whichever thread made them, in the same order.
//!
//! The pool is the process's own, made the first time a container asks
//! for it; a machine of one core, or one that refuses a thread, lays
//! everything out in order.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use rayon::prelude::*;

/// The most threads the pool has: fewer on a machine with fewer cores.
pub const MAX_WORKER_THREADS: usize = 8;

/// The fewest items or cells laid out on the pool rather than in order:
/// below it, handing them out costs more than it saves.
pub const PARALLEL_MIN_ITEMS: usize = 32;

/// A worker's stack, as large as the thread layout starts on: layout
/// recurses once a level of nesting.
const WORKER_STACK: usize = 8 << 20;

/// The pool, if this machine has more than one core and gives it its
/// threads.
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(MAX_WORKER_THREADS);
        if threads < 2 {
            return None;
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .stack_size(WORKER_STACK)
            .thread_name(|i| format!("layout-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// `f` of each of `items`, in their order: on the pool when there are
/// [`PARALLEL_MIN_ITEMS`] of them or more, on the one the caller is on
/// when it is a pool's (a nested container's, or a test's of one
/// thread).
pub(crate) fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    if items.len() < PARALLEL_MIN_ITEMS {
        return items.iter().map(f).collect();
    }
    let f = |item: &T| {
        #[cfg(test)]
        seen();
        f(item)
    };
    if rayon::current_thread_index().is_some() {
        return items.par_iter().map(f).collect();
    }
    match pool() {
        Some(pool) => pool.install(|| items.par_iter().map(f).collect()),
        None => items.iter().map(f).collect(),
    }
}

/// The threads that laid items out on the pool, for the tests.
#[cfg(test)]
static SEEN: Mutex<Vec<std::thread::ThreadId>> = Mutex::new(Vec::new());

#[cfg(test)]
fn seen() {
    let id = std::thread::current().id();
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if !seen.contains(&id) {
        seen.push(id);
    }
}

/// `f` on each of `items`, as [`map`] runs it.
pub(crate) fn for_each_mut<T: Send>(items: &mut [T], f: impl Fn(&mut T) + Sync + Send) {
    if items.len() < PARALLEL_MIN_ITEMS {
        items.iter_mut().for_each(f);
        return;
    }
    if rayon::current_thread_index().is_some() {
        items.par_iter_mut().for_each(f);
        return;
    }
    match pool() {
        Some(pool) => pool.install(|| items.par_iter_mut().for_each(&f)),
        None => items.iter_mut().for_each(f),
    }
}

/// A value behind a lock, with a `RefCell`'s names: what a pass and the
/// layout cache share between the threads laying out a container's items.
/// Borrowing locks, until the borrow ends.
#[derive(Default)]
pub(crate) struct Locked<T>(Mutex<T>);

impl<T> Locked<T> {
    pub(crate) fn new(value: T) -> Self {
        Locked(Mutex::new(value))
    }

    pub(crate) fn borrow(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.borrow()
    }

    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.0.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T: fmt::Debug> fmt::Debug for Locked<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow().fmt(f)
    }
}

/// A count, with a `Cell`'s names, that threads add to.
#[derive(Debug, Default)]
pub(crate) struct Count(AtomicUsize);

impl Count {
    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    pub(crate) fn set(&self, value: usize) {
        self.0.store(value, Ordering::Relaxed);
    }

    /// Adds one.
    pub(crate) fn add(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use crate::block::layout_document;
    use crate::testing::boxes_of;

    #[test]
    fn many_items_and_cells_lay_out_the_same_on_one_thread_and_on_the_pool() {
        // M35.3: a flex container's, a grid's and a table's hundred items
        // and cells, of texts of every length, and flex containers nested
        // in the items; laid out on a pool of one thread and on the pool.
        let fonts = text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses");
        // The first the longest: an item whose own size its line's
        // depends on.
        let words = |i: usize| "word ".repeat(if i == 0 { 12 } else { 1 + i % 7 });
        let items: String = (0..100)
            .map(|i| {
                format!(
                    "<div class=i><p>{}</p><div class=n><p>{i}</p></div></div>",
                    words(i)
                )
            })
            .collect();
        let cells: String = (0..10)
            .map(|r| {
                let row: String = (0..10)
                    .map(|c| format!("<td>{}</td>", words(r * 10 + c)))
                    .collect();
                format!("<tr>{row}</tr>")
            })
            .collect();
        let html =
            format!("<div class=f>{items}</div><div class=g>{items}</div><table>{cells}</table>");
        let doc = html::parse_document(&html).expect("html parsing never fails");
        let sheet = css::parse_stylesheet(
            ".f { display: flex; flex-wrap: wrap; align-items: flex-start } .g { display: grid; \
             grid-template-columns: repeat(7, auto) } .n { display: flex } p { margin: 0 } \
             .f .i { width: 60px }",
        )
        .expect("test css is valid");
        let (_, boxes) = boxes_of(&doc, &[sheet]);
        let one = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("a pool of one");
        let alone = one.install(|| format!("{:?}", layout_document(&boxes, 800.0, &fonts)));
        let pooled = format!("{:?}", layout_document(&boxes, 800.0, &fonts));
        assert_eq!(alone, pooled);
        // On a machine of more than one core, the pool's threads did it:
        // besides the one of the pool of one, more than one.
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = super::SEEN.lock().map_or(0, |seen| seen.len());
        assert!(
            cores < 2 || threads > 2,
            "{threads} threads on {cores} cores"
        );
    }
}
