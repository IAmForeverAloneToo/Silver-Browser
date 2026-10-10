//! What the browsing keeps (M28, docs/design/storage.md): the profile
//! the engine writes to, and when. The cookies with an expiry join the
//! jar when the engine is given a profile, and what changes is written
//! in one transaction no more than once in [`FLUSH_INTERVAL`].

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use net::cookies::Jar;
use profile::{Changes, Profile};

#[cfg(test)]
mod tests;

/// The least time between two writes of the profile: a change waits at
/// most this long to be written, and a crash loses at most this much.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// The engine's profile and what of it waits to be written.
pub(crate) struct Storage {
    /// `None` keeps nothing: an engine of tests and tools.
    profile: Option<Profile>,
    /// The jar's count of changes when its cookies were last written;
    /// `None` when they must be.
    cookies_written: Option<u64>,
    /// When the profile was last written, or tried, by the engine's
    /// clock.
    flushed: Option<Duration>,
}

impl Storage {
    pub(crate) fn new() -> Storage {
        Storage {
            profile: None,
            cookies_written: None,
            flushed: None,
        }
    }

    /// Keeps what changes in `profile` from now on, and takes its
    /// cookies into `jar`.
    pub(crate) fn set_profile(
        &mut self,
        profile: Profile,
        jar: &Mutex<Jar>,
    ) -> Result<(), profile::Error> {
        let records = profile.cookies()?;
        let mut jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
        let other = jar.restore(&records, SystemTime::now());
        self.cookies_written = (!other).then(|| jar.kept_changes());
        self.profile = Some(profile);
        Ok(())
    }

    /// When the profile is next to be written, if something waits:
    /// [`FLUSH_INTERVAL`] after the last write.
    pub(crate) fn flush_at(&self, jar: &Mutex<Jar>) -> Option<Duration> {
        self.profile.as_ref()?;
        let count = jar
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kept_changes();
        (self.cookies_written != Some(count)).then(|| {
            self.flushed
                .map_or(Duration::ZERO, |at| at + FLUSH_INTERVAL)
        })
    }

    /// Writes what changed since the last write, at `now`. A write that
    /// fails leaves it waiting, to be tried again an interval on.
    pub(crate) fn flush(&mut self, jar: &Mutex<Jar>, now: Duration) -> Result<(), profile::Error> {
        let Some(profile) = &self.profile else {
            return Ok(());
        };
        let mut changes = Changes::new();
        let count = {
            let jar = jar.lock().unwrap_or_else(PoisonError::into_inner);
            let count = jar.kept_changes();
            if self.cookies_written != Some(count) {
                changes.cookies(jar.export());
            }
            count
        };
        if changes.is_empty() {
            return Ok(());
        }
        self.flushed = Some(now);
        profile.commit(&changes)?;
        self.cookies_written = Some(count);
        Ok(())
    }
}
