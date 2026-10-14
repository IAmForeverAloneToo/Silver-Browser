//! A pipe in memory (M34.6): what a renderer run as a thread speaks
//! over in the engine's tests, as a process speaks over its standard
//! input and output. Writes never wait; a read waits for bytes, and
//! answers none once the writer is gone and all is read. A write after
//! the reader is gone fails as a broken pipe's does.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

#[derive(Default)]
struct State {
    bytes: VecDeque<u8>,
    writer_gone: bool,
    reader_gone: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The writing end of a [`pipe`].
pub struct PipeWriter(Arc<Shared>);

/// The reading end of a [`pipe`].
pub struct PipeReader(Arc<Shared>);

/// A new pipe's two ends.
pub fn pipe() -> (PipeWriter, PipeReader) {
    let shared = Arc::new(Shared::default());
    (PipeWriter(shared.clone()), PipeReader(shared))
}

/// Two pipes, one each way: each end's reader reads what the other's
/// writer writes.
pub fn duplex() -> ((PipeReader, PipeWriter), (PipeReader, PipeWriter)) {
    let (to_b, from_a) = pipe();
    let (to_a, from_b) = pipe();
    ((from_b, to_b), (from_a, to_a))
}

/// What breaks a [`pipe`] from outside, as a process's death does: its
/// reader reads its end once what was written is read, and its writer
/// fails.
pub struct PipeBreaker(Arc<Shared>);

impl PipeBreaker {
    pub fn break_pipe(&self) {
        let mut state = self.0.lock();
        state.writer_gone = true;
        state.reader_gone = true;
        state.bytes.clear();
        self.0.ready.notify_all();
    }
}

impl PipeWriter {
    /// What breaks this pipe.
    pub fn breaker(&self) -> PipeBreaker {
        PipeBreaker(self.0.clone())
    }
}

impl Write for PipeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock();
        if state.reader_gone {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        state.bytes.extend(bytes);
        self.0.ready.notify_all();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.0.lock().writer_gone = true;
        self.0.ready.notify_all();
    }
}

impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut state = self.0.lock();
        while state.bytes.is_empty() && !state.writer_gone {
            state = self
                .0
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let n = buffer.len().min(state.bytes.len());
        for (to, from) in buffer.iter_mut().zip(state.bytes.drain(..n)) {
            *to = from;
        }
        Ok(n)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.reader_gone = true;
        state.bytes.clear();
    }
}
