//! Messages over a pair of byte streams (M34.2): each message a frame,
//! its length in 32 bits and then its bytes, at most [`MAX_MESSAGE`]; a
//! [`Sender`] that any thread may write through; a reader thread that
//! decodes each frame and hands it on; and [`Calls`], the questions a
//! thread asks and waits on, matched to their answers by number.

use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::{self, JoinHandle};

use super::{Encoder, MAX_MESSAGE, Malformed, Wire, decode_all};

/// The channel is closed: the other side is gone, or this side closed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the channel is closed")
    }
}

/// Why a channel's reading stopped other than at its end.
#[derive(Debug)]
pub enum ReadError {
    /// The stream failed, or ended inside a frame.
    Io(io::Error),
    /// A frame said it was longer than [`MAX_MESSAGE`].
    TooLong(usize),
    /// A frame's bytes were no message.
    Malformed(Malformed),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(error) => write!(f, "{error}"),
            ReadError::TooLong(len) => write!(f, "a message of {len} bytes"),
            ReadError::Malformed(malformed) => write!(f, "a malformed message: {malformed}"),
        }
    }
}

/// `message`'s frame: its length and its bytes. `None` for a message
/// longer than [`MAX_MESSAGE`], which this side never makes: bodies and
/// pixels go in pieces.
pub fn frame<M: Wire>(message: &M) -> Option<Vec<u8>> {
    let mut out = Encoder::after(&[0; 4]);
    message.encode(&mut out);
    let mut bytes = out.into_bytes();
    let len = bytes.len() - 4;
    if len > MAX_MESSAGE {
        return None;
    }
    bytes[..4].copy_from_slice(&(len as u32).to_le_bytes());
    Some(bytes)
}

/// The next message of `reader`; `None` at its end between two frames.
pub fn read_message<M: Wire>(reader: &mut impl Read) -> Result<Option<M>, ReadError> {
    let mut head = [0; 4];
    let mut got = 0;
    while got < head.len() {
        match reader.read(&mut head[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(ReadError::Io(io::ErrorKind::UnexpectedEof.into())),
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ReadError::Io(error)),
        }
    }
    let len = u32::from_le_bytes(head) as usize;
    if len > MAX_MESSAGE {
        return Err(ReadError::TooLong(len));
    }
    // Read as the bytes come, so that a length alone allocates nothing.
    let mut bytes = Vec::new();
    reader
        .take(len as u64)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() != len {
        return Err(ReadError::Io(io::ErrorKind::UnexpectedEof.into()));
    }
    decode_all(&bytes).map(Some).map_err(ReadError::Malformed)
}

/// Where messages of type `M` are written: any thread may send through
/// a clone; a message is written whole or not at all.
pub struct Sender<M> {
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    message: PhantomData<fn(&M)>,
}

impl<M> Clone for Sender<M> {
    fn clone(&self) -> Self {
        Sender {
            writer: self.writer.clone(),
            message: PhantomData,
        }
    }
}

impl<M: Wire> Sender<M> {
    pub fn new(writer: impl Write + Send + 'static) -> Sender<M> {
        Sender {
            writer: Arc::new(Mutex::new(Some(Box::new(writer)))),
            message: PhantomData,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Box<dyn Write + Send>>> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes `message`; the channel closes if the stream fails.
    pub fn send(&self, message: &M) -> Result<(), Closed> {
        let frame = frame(message).ok_or(Closed)?;
        let mut writer = self.lock();
        let stream = writer.as_mut().ok_or(Closed)?;
        if stream
            .write_all(&frame)
            .and_then(|()| stream.flush())
            .is_err()
        {
            *writer = None;
            return Err(Closed);
        }
        Ok(())
    }

    /// Closes the stream: the other side reads its end.
    pub fn close(&self) {
        *self.lock() = None;
    }

    /// Whether the channel is closed on this side.
    pub fn is_closed(&self) -> bool {
        self.lock().is_none()
    }
}

/// Starts a thread named `name` that reads `reader`'s messages and gives
/// each to `deliver`, then, at the stream's end or at what stops it,
/// calls `closed` with the reason (none at a clean end).
pub fn spawn_reader<M, R>(
    name: &str,
    reader: R,
    mut deliver: impl FnMut(M) + Send + 'static,
    closed: impl FnOnce(Option<ReadError>) + Send + 'static,
) -> io::Result<JoinHandle<()>>
where
    M: Wire + Send + 'static,
    R: Read + Send + 'static,
{
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        let mut reader = BufReader::with_capacity(64 << 10, reader);
        let reason = loop {
            match read_message::<M>(&mut reader) {
                Ok(Some(message)) => deliver(message),
                Ok(None) => break None,
                Err(error) => break Some(error),
            }
        };
        closed(reason);
    })
}

/// The questions a side asks and waits on, each with its number, and
/// their answers of type `A`.
pub struct Calls<A> {
    next: AtomicU64,
    /// `None` once the channel closed: no question waits any more.
    waiting: Mutex<Option<HashMap<u64, mpsc::Sender<A>>>>,
}

impl<A> Default for Calls<A> {
    fn default() -> Self {
        Calls {
            next: AtomicU64::new(1),
            waiting: Mutex::new(Some(HashMap::new())),
        }
    }
}

impl<A> Calls<A> {
    pub fn new() -> Calls<A> {
        Calls::default()
    }

    fn lock(&self) -> MutexGuard<'_, Option<HashMap<u64, mpsc::Sender<A>>>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new question's number, and where its answer will come.
    pub fn begin(&self) -> Result<(u64, mpsc::Receiver<A>), Closed> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::channel();
        self.lock().as_mut().ok_or(Closed)?.insert(id, answer);
        Ok((id, answered))
    }

    /// Gives question `id` its answer; false when no question waits by
    /// that number.
    pub fn answer(&self, id: u64, answer: A) -> bool {
        let waiting = self.lock().as_mut().and_then(|waiting| waiting.remove(&id));
        waiting.is_some_and(|waiting| waiting.send(answer).is_ok())
    }

    /// How many questions wait.
    pub fn waiting(&self) -> usize {
        self.lock().as_ref().map_or(0, HashMap::len)
    }

    /// Every question waiting ends unanswered, and none is asked again.
    pub fn close(&self) {
        *self.lock() = None;
    }

    /// Asks through `sender` the message `ask` makes of the question's
    /// number, and waits for its answer.
    pub fn call<M: Wire>(
        &self,
        sender: &Sender<M>,
        ask: impl FnOnce(u64) -> M,
    ) -> Result<A, Closed> {
        let (id, answered) = self.begin()?;
        if sender.send(&ask(id)).is_err() {
            if let Some(waiting) = self.lock().as_mut() {
                waiting.remove(&id);
            }
            return Err(Closed);
        }
        answered.recv().map_err(|_| Closed)
    }
}
