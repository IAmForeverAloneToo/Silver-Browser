//! Bytes into the natives at the speed of a copy (G255 step 7a, the
//! lead's ruling): a binding copies a buffer's bytes where its spec
//! copies them, by `bytesStash(buffer, offset, length)`, which copies
//! the range into a vector held for the realm under a token, and passes
//! the token to its native in place of a binary string; the native's
//! [`super::dom`] reading of bytes takes the vector by its token, once.
//!
//! One copy, made in the one `unsafe` block of this module
//! ([`copy_range`]): `rquickjs` lends a buffer's bytes only through
//! `unsafe`. A shared buffer never reaches it, nor a range past the
//! buffer's length as it is at the copy.
//!
//! A token is the realm's: the bootstrap holds `bytesStash` and
//! `bytesDrop` privately, no script reaches them, and a realm's natives
//! take only its own tokens. What is not taken goes when the method
//! that stashed it ends (`bytesDrop`), at the next microtask checkpoint,
//! or with the realm: a token is never read twice, and one not held is
//! an internal error, never a read.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::function::This;
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, Persistent, Value};

/// The most a realm holds at once: a call takes at most five.
const MOST_HELD: usize = 16;

/// A realm's copied bytes, each under its token until a native takes
/// it, and the getters its copies check buffers by, which go before the
/// realm's context does ([`Stash::close`]).
#[derive(Default)]
pub(crate) struct Stash {
    next: u32,
    held: Vec<(u32, Vec<u8>)>,
    getters: Option<Getters>,
}

/// A realm's stash, shared by its natives and its agent's checkpoint.
pub(crate) type Stashed = Rc<RefCell<Stash>>;

impl Stash {
    /// Holds `bytes`, under a token no other held bytes have; `None` when
    /// the realm holds [`MOST_HELD`] already.
    pub(crate) fn put(&mut self, bytes: Vec<u8>) -> Option<u32> {
        if self.held.len() >= MOST_HELD {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        while self.held.iter().any(|(token, _)| *token == self.next) {
            self.next = self.next.wrapping_add(1);
        }
        self.held.push((self.next, bytes));
        Some(self.next)
    }

    /// The bytes under `token`, which it no longer holds.
    pub(crate) fn take(&mut self, token: u32) -> Option<Vec<u8>> {
        let at = self.held.iter().position(|(held, _)| *held == token)?;
        Some(self.held.swap_remove(at).1)
    }

    /// Lets go of everything held: at a microtask checkpoint, and as the
    /// realm goes.
    pub(crate) fn clear(&mut self) {
        self.held.clear();
    }

    /// Lets go of everything, the getters too, as the realm goes: no value
    /// of its context outlives it.
    pub(crate) fn close(&mut self) {
        self.held.clear();
        self.getters = None;
    }

    /// How many are held, for the tests.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether the getters went, for the tests.
    #[cfg(test)]
    pub(crate) fn closed(&self) -> bool {
        self.getters.is_none()
    }
}

/// A token as a script passes it: a whole number of a `u32`.
pub(crate) fn token_of(number: f64) -> Option<u32> {
    (number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&number))
        .then_some(number as u32)
}

/// A whole number of bytes, as `bytesStash` is given them.
fn count_of(number: f64) -> Option<usize> {
    (number.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&number))
        .then(|| usize::try_from(number as u64).ok())
        .flatten()
}

/// The getters of `ArrayBuffer.prototype` as the realm began, before any
/// script could replace them: `byteLength`, which throws for anything
/// but an ArrayBuffer (a SharedArrayBuffer among them), and `detached`.
#[derive(Clone)]
struct Getters {
    byte_length: Persistent<Function<'static>>,
    detached: Option<Persistent<Function<'static>>>,
}

impl Getters {
    fn of(ctx: &Ctx<'_>) -> rquickjs::Result<Getters> {
        let prototype: Object<'_> = ctx
            .globals()
            .get::<_, Object<'_>>("ArrayBuffer")?
            .get("prototype")?;
        let getter = |name: &str| -> rquickjs::Result<Option<Function<'_>>> {
            let reflect: Object<'_> = ctx.globals().get("Object")?;
            let describe: Function<'_> = reflect.get("getOwnPropertyDescriptor")?;
            let descriptor: Value<'_> = describe.call((prototype.clone(), name))?;
            Ok(descriptor
                .into_object()
                .and_then(|descriptor| descriptor.get::<_, Function<'_>>("get").ok()))
        };
        let byte_length = getter("byteLength")?
            .ok_or_else(|| Exception::throw_internal(ctx, "no ArrayBuffer byteLength"))?;
        Ok(Getters {
            byte_length: Persistent::save(ctx, byte_length),
            detached: getter("detached")?.map(|get| Persistent::save(ctx, get)),
        })
    }
}

/// Installs `bytesStash` and `bytesDrop` on the bootstrap's natives, for
/// the realm whose stash `stash` is. Run before any script of the realm.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    natives: &Object<'js>,
    stash: &Stashed,
) -> rquickjs::Result<()> {
    stash.borrow_mut().getters = Some(Getters::of(ctx)?);
    let held = Rc::clone(stash);
    natives.set(
        "bytesStash",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  buffer: Value<'js>,
                  offset: f64,
                  length: f64|
                  -> rquickjs::Result<u32> {
                // Cloned out, so that no borrow is held while they run.
                let getters = held.borrow().getters.clone();
                let Some(getters) = getters else {
                    return Err(Exception::throw_internal(&ctx, "the realm is gone"));
                };
                let bytes = copy_range(&ctx, &getters, buffer, offset, length)?;
                held.borrow_mut()
                    .put(bytes)
                    .ok_or_else(|| Exception::throw_internal(&ctx, "too many bytes held"))
            },
        )?
        .with_name("bytesStash")?,
    )?;
    let held = Rc::clone(stash);
    natives.set(
        "bytesDrop",
        Function::new(ctx.clone(), move |tokens: rquickjs::function::Rest<f64>| {
            let mut held = held.borrow_mut();
            for token in tokens.0.into_iter().filter_map(token_of) {
                held.take(token);
            }
        })?
        .with_name("bytesDrop")?,
    )?;
    Ok(())
}

/// The bytes `buffer` holds from `offset` for `length`, copied now.
///
/// `buffer` must be an ArrayBuffer, not shared (a TypeError
/// otherwise); a detached one holds none; a range past its length as it
/// is now, a resizable one's shrunk among them, is refused (a
/// RangeError), never cut short.
#[allow(unsafe_code)]
fn copy_range<'js>(
    ctx: &Ctx<'js>,
    getters: &Getters,
    buffer: Value<'js>,
    offset: f64,
    length: f64,
) -> rquickjs::Result<Vec<u8>> {
    let not_bytes = || Exception::throw_type(ctx, "not an ArrayBuffer's bytes");
    // The brand check first, by the realm's own getter: it throws for a
    // SharedArrayBuffer, whose bytes another thread may write as they
    // are copied, and for anything else not an ArrayBuffer.
    let byte_length = getters.byte_length.clone().restore(ctx)?;
    if byte_length.call::<_, f64>((This(buffer.clone()),)).is_err() {
        let _ = ctx.catch();
        return Err(not_bytes());
    }
    let detached = match &getters.detached {
        Some(get) => {
            let get = get.clone().restore(ctx)?;
            get.call::<_, bool>((This(buffer.clone()),)).unwrap_or(true)
        }
        None => false,
    };
    let (Some(offset), Some(length)) = (count_of(offset), count_of(length)) else {
        return Err(Exception::throw_range(ctx, "not a range of bytes"));
    };
    if detached {
        return if offset == 0 && length == 0 {
            Ok(Vec::new())
        } else {
            Err(Exception::throw_range(
                ctx,
                "a detached buffer has no bytes",
            ))
        };
    }
    let Some(buffer) = ArrayBuffer::from_value(buffer) else {
        return Err(not_bytes());
    };
    let copied = {
        // SAFETY: `as_bytes`' slice aliases the ArrayBuffer's backing
        // store, which QuickJS-ng owns and a script could write, detach
        // or (resizable) reallocate. The buffer is not shared: the
        // realm's own `ArrayBuffer.prototype.byteLength` getter took it
        // above, which throws for a SharedArrayBuffer, so no other
        // thread writes it. From `as_bytes` to the end of `to_vec` no
        // JavaScript runs and nothing reaches QuickJS: no value is made
        // or freed, no allocation of its heap, no collection, no
        // callback; `to_vec` allocates in Rust's heap alone. The length
        // is the slice's own, read here after the detached check, and
        // the range is checked against it with checked arithmetic. The
        // buffer is kept alive by the caller's argument.
        let bytes = unsafe { buffer.as_bytes() };
        bytes.and_then(|bytes| {
            offset
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .map(|end| bytes[offset..end].to_vec())
        })
    };
    match copied {
        Some(bytes) => Ok(bytes),
        None => {
            // A buffer detached after all leaves QuickJS-ng's TypeError.
            let _ = ctx.catch();
            Err(Exception::throw_range(
                ctx,
                "a range past the buffer's bytes",
            ))
        }
    }
}

/// The copy of `length` bytes at `offset` of the buffer that script
/// `source` ends with, in a context of its own (one with
/// SharedArrayBuffer, which a page's realm has not), for the tests and
/// the fuzz target `bytes_in`: the bytes, or the error's name and
/// message; `None` when `source` throws. Nothing is left pending.
#[cfg(any(test, feature = "fuzzing"))]
pub fn copy_of_buffer(source: &str, offset: f64, length: f64) -> Option<Result<Vec<u8>, String>> {
    let runtime = rquickjs::Runtime::new().ok()?;
    let context = rquickjs::Context::full(&runtime).ok()?;
    context.with(|ctx| {
        let getters = Getters::of(&ctx).ok()?;
        let Ok(buffer) = ctx.eval::<Value<'_>, _>(source) else {
            let _ = ctx.catch();
            return None;
        };
        let result = copy_range(&ctx, &getters, buffer, offset, length).map_err(|_| {
            let thrown = ctx.catch();
            let thrown = thrown.as_object();
            let member = |name: &str| {
                thrown
                    .and_then(|thrown| thrown.get::<_, String>(name).ok())
                    .unwrap_or_default()
            };
            format!("{}: {}", member("name"), member("message"))
        });
        assert!(!ctx.has_exception(), "an exception left behind");
        Some(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`copy_of_buffer`], its source made.
    fn copied(source: &str, offset: f64, length: f64) -> Result<Vec<u8>, String> {
        copy_of_buffer(source, offset, length).unwrap_or_else(|| panic!("{source} throws"))
    }

    const BYTES: &str = "new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7]).buffer";

    #[test]
    fn a_range_of_a_buffer_is_copied_exactly() {
        assert_eq!(copied(BYTES, 0.0, 8.0), Ok((0..8).collect()));
        assert_eq!(copied(BYTES, 3.0, 2.0), Ok(vec![3, 4]));
        assert_eq!(copied(BYTES, 8.0, 0.0), Ok(Vec::new()));
        assert_eq!(copied("new ArrayBuffer(0)", 0.0, 0.0), Ok(Vec::new()));
    }

    #[test]
    fn a_range_past_the_buffer_is_refused_never_cut_short() {
        let past = Err("RangeError: a range past the buffer's bytes".to_owned());
        assert_eq!(copied(BYTES, 0.0, 9.0), past);
        assert_eq!(copied(BYTES, 9.0, 0.0), past);
        assert_eq!(copied(BYTES, 7.0, 2.0), past);
        // Their sum past a usize: checked, not wrapped.
        assert_eq!(
            copied(BYTES, 9_007_199_254_740_991.0, 9_007_199_254_740_991.0),
            past
        );
        let malformed = Err("RangeError: not a range of bytes".to_owned());
        assert_eq!(copied(BYTES, -1.0, 1.0), malformed);
        assert_eq!(copied(BYTES, 0.5, 1.0), malformed);
        assert_eq!(copied(BYTES, 0.0, f64::NAN), malformed);
        assert_eq!(copied(BYTES, 0.0, f64::INFINITY), malformed);
    }

    #[test]
    fn a_shrunk_resizable_buffer_is_read_at_its_length_now() {
        let shrunk = "const b = new ArrayBuffer(8, {maxByteLength: 16}); \
                      new Uint8Array(b).set([9, 9, 9, 9, 9, 9, 9, 9]); b.resize(4); b";
        assert_eq!(copied(shrunk, 0.0, 4.0), Ok(vec![9; 4]));
        assert_eq!(
            copied(shrunk, 0.0, 8.0),
            Err("RangeError: a range past the buffer's bytes".to_owned())
        );
        let grown = "const b = new ArrayBuffer(2, {maxByteLength: 16}); b.resize(6); b";
        assert_eq!(copied(grown, 0.0, 6.0), Ok(vec![0; 6]));
    }

    #[test]
    fn a_detached_buffer_holds_no_bytes() {
        let detached = "const b = new ArrayBuffer(8); b.transfer(); b";
        assert_eq!(copied(detached, 0.0, 0.0), Ok(Vec::new()));
        assert_eq!(
            copied(detached, 0.0, 1.0),
            Err("RangeError: a detached buffer has no bytes".to_owned())
        );
    }

    #[test]
    fn a_shared_buffer_or_anything_else_is_refused() {
        let refused = Err("TypeError: not an ArrayBuffer's bytes".to_owned());
        assert_eq!(copied("new SharedArrayBuffer(8)", 0.0, 8.0), refused);
        // A growable one QuickJS-ng makes only with allocator hooks, which
        // no realm here has.
        assert_eq!(copied("new Uint8Array(8)", 0.0, 8.0), refused);
        assert_eq!(copied("({byteLength: 8})", 0.0, 8.0), refused);
        assert_eq!(copied("'bytes'", 0.0, 0.0), refused);
        // A script's own byteLength getter changes nothing: the realm's
        // first one is asked.
        let spoofed = "Object.defineProperty(ArrayBuffer.prototype, 'byteLength', \
                       {get() { return 8; }}); new SharedArrayBuffer(8)";
        assert_eq!(copied(spoofed, 0.0, 8.0), refused);
    }

    #[test]
    fn a_token_is_taken_once_and_the_stash_is_bounded() {
        let mut stash = Stash::default();
        let a = stash.put(vec![1]).unwrap();
        let b = stash.put(vec![2]).unwrap();
        assert_ne!(a, b);
        assert_eq!(stash.take(a), Some(vec![1]));
        assert_eq!(stash.take(a), None);
        for _ in 0..MOST_HELD - 1 {
            assert!(stash.put(Vec::new()).is_some());
        }
        assert_eq!(stash.put(Vec::new()), None);
        stash.clear();
        assert_eq!(stash.len(), 0);
        assert_eq!(stash.take(b), None);
    }

    #[test]
    fn tokens_and_counts_are_whole_numbers() {
        assert_eq!(token_of(3.0), Some(3));
        assert_eq!(token_of(3.5), None);
        assert_eq!(token_of(-1.0), None);
        assert_eq!(token_of(f64::NAN), None);
        assert_eq!(count_of(0.0), Some(0));
        assert_eq!(count_of(-0.5), None);
        assert_eq!(count_of(f64::INFINITY), None);
    }
}
