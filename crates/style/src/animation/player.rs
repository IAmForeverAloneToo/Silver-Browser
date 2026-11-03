//! An animation's playback (Web Animations 1 section 4.4; D75): its
//! start time or hold time on its timeline, its playback rate, its
//! pending play and pause tasks, and what its promises and its finish
//! notification are owed, which every animation a document runs has,
//! CSS's and scripts' alike.
//!
//! The procedures are the draft's, step by step: setting the current
//! time and the start time, playing, pausing, finishing, cancelling,
//! reversing, updating the playback rate seamlessly, the pending tasks
//! run when the animation is ready, and updating the finished state. A
//! procedure's effect on the promises a script holds is told as
//! [`Notice`]s, which the bindings carry out on their objects: a new
//! promise, one resolved or rejected, and the finish notification steps
//! due now or in a microtask. Times are milliseconds; a time a script
//! cannot see is `None`.

/// What a procedure owes the promises of the animation, and its finish
/// notification, in the order they were owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// A new pending ready promise replaces the current one.
    ReadyNew,
    /// The current ready promise is resolved with the animation.
    ReadyResolved,
    /// The current ready promise is rejected with an `AbortError`, and a
    /// new resolved one replaces it (section 4.4.14, "reset an
    /// animation's pending tasks").
    ReadyRejected,
    /// A new pending finished promise replaces the current one.
    FinishedNew,
    /// The current finished promise is rejected with an `AbortError`,
    /// and a new pending one replaces it (section 4.4.15, cancel).
    FinishedRejected,
    /// The finish notification steps are due: at once when `sync`, else
    /// in a microtask, unless one is queued already.
    FinishNotify { sync: bool },
}

/// An animation's play state (section 4.4.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Idle,
    Running,
    Paused,
    Finished,
}

impl PlayState {
    /// Its name, as `playState` gives it.
    pub fn name(self) -> &'static str {
        match self {
            PlayState::Idle => "idle",
            PlayState::Running => "running",
            PlayState::Paused => "paused",
            PlayState::Finished => "finished",
        }
    }
}

/// A pending task (section 4.4.4): played or paused once the animation is
/// ready, at the next animation frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Play,
    Pause,
}

/// Why a procedure refused: the `DOMException` a script gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `InvalidStateError`: playing backwards, or finishing, an effect
    /// that never ends; finishing at a playback rate of zero.
    InvalidState,
    /// `TypeError`: an unresolved current time set on an animation whose
    /// current time is resolved.
    Type,
}

/// What a procedure needs of the animation's surroundings: its timeline's
/// current time, `None` when it has no timeline or the timeline is
/// inactive; its associated effect's end time, zero for none; and, on a
/// progress-based timeline, the range of it the animation is attached
/// to, its start and end as the timeline's times (Web Animations 2
/// section 4.4.1.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Context {
    pub timeline: Option<f64>,
    pub end: f64,
    pub range: Option<(f64, f64)>,
}

/// An animation's playback.
#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub start_time: Option<f64>,
    pub hold_time: Option<f64>,
    pub playback_rate: f64,
    pub pending_playback_rate: Option<f64>,
    pub pending: Option<Pending>,
    /// The current time the last update of the finished state saw.
    previous_current_time: Option<f64>,
    /// Whether the current ready promise is still pending.
    ready_pending: bool,
    /// Whether the current finished promise is resolved.
    finished_resolved: bool,
    /// Whether its start time follows its progress-based timeline's
    /// range (Web Animations 2's auto-align start time flag): set by
    /// playing, cleared by a time a script sets.
    pub auto_align: bool,
}

impl Default for Player {
    fn default() -> Self {
        Player::new()
    }
}

impl Player {
    /// An idle animation: no times, a playback rate of one, its ready
    /// promise resolved and its finished promise pending.
    pub fn new() -> Self {
        Player {
            start_time: None,
            hold_time: None,
            playback_rate: 1.0,
            pending_playback_rate: None,
            pending: None,
            previous_current_time: None,
            ready_pending: false,
            finished_resolved: false,
            auto_align: false,
        }
    }

    /// The start time its range gives it (Web Animations 2 section
    /// 4.4.11.1): the range's start played forwards, its end backwards.
    fn aligned_start(&self, cx: &Context) -> Option<f64> {
        let (start, end) = cx.range?;
        Some(if self.effective_playback_rate() >= 0.0 {
            start
        } else {
            end
        })
    }

    /// Aligns the start time with the range again, where it follows it
    /// and runs: the range moves as its timeline is measured.
    pub fn align(&mut self, cx: &Context) {
        if self.auto_align
            && self.pending.is_none()
            && self.hold_time.is_none()
            && self.start_time.is_some()
            && let Some(start) = self.aligned_start(cx)
        {
            self.start_time = Some(start);
        }
    }

    /// The current time (section 4.4.8).
    pub fn current_time(&self, cx: &Context) -> Option<f64> {
        if let Some(hold) = self.hold_time {
            return Some(hold);
        }
        let (timeline, start) = (cx.timeline?, self.start_time?);
        Some((timeline - start) * self.playback_rate)
    }

    /// The current time with an unresolved hold time, as updating the
    /// finished state reads it when the animation did not seek.
    fn unconstrained_time(&self, cx: &Context) -> Option<f64> {
        let (timeline, start) = (cx.timeline?, self.start_time?);
        Some((timeline - start) * self.playback_rate)
    }

    /// The pending playback rate, else the playback rate.
    pub fn effective_playback_rate(&self) -> f64 {
        self.pending_playback_rate.unwrap_or(self.playback_rate)
    }

    /// The play state (section 4.4.15).
    pub fn play_state(&self, cx: &Context) -> PlayState {
        let current = self.current_time(cx);
        if current.is_none() && self.start_time.is_none() && self.pending.is_none() {
            return PlayState::Idle;
        }
        if self.pending == Some(Pending::Pause)
            || (self.start_time.is_none() && self.pending != Some(Pending::Play))
        {
            return PlayState::Paused;
        }
        let rate = self.effective_playback_rate();
        if let Some(current) = current
            && ((rate > 0.0 && current >= cx.end) || (rate < 0.0 && current <= 0.0))
        {
            return PlayState::Finished;
        }
        PlayState::Running
    }

    /// Whether a pending task waits for the animation to be ready.
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the current finished promise is resolved.
    pub fn finished_resolved(&self) -> bool {
        self.finished_resolved
    }

    /// Applies the pending playback rate (section 4.4.17).
    fn apply_pending_playback_rate(&mut self) {
        if let Some(rate) = self.pending_playback_rate.take() {
            self.playback_rate = rate;
        }
    }

    /// Resolves the current ready promise, if it is pending.
    fn resolve_ready(&mut self, notices: &mut Vec<Notice>) {
        if self.ready_pending {
            self.ready_pending = false;
            notices.push(Notice::ReadyResolved);
        }
    }

    /// A new pending ready promise.
    fn new_ready(&mut self, notices: &mut Vec<Notice>) {
        self.ready_pending = true;
        notices.push(Notice::ReadyNew);
    }

    /// Silently sets the current time (section 4.4.9).
    fn silently_set_current_time(
        &mut self,
        seek: Option<f64>,
        cx: &Context,
    ) -> Result<(), Refusal> {
        let Some(seek) = seek else {
            if self.current_time(cx).is_some() {
                return Err(Refusal::Type);
            }
            return Ok(());
        };
        self.auto_align = false;
        if self.hold_time.is_some()
            || self.start_time.is_none()
            || cx.timeline.is_none()
            || self.playback_rate == 0.0
        {
            self.hold_time = Some(seek);
        } else if let Some(timeline) = cx.timeline {
            self.start_time = Some(timeline - seek / self.playback_rate);
        }
        if cx.timeline.is_none() {
            self.start_time = None;
        }
        self.previous_current_time = None;
        Ok(())
    }

    /// Sets the current time (section 4.4.9).
    pub fn set_current_time(
        &mut self,
        seek: Option<f64>,
        cx: &Context,
        notices: &mut Vec<Notice>,
    ) -> Result<(), Refusal> {
        self.silently_set_current_time(seek, cx)?;
        if self.pending == Some(Pending::Pause) {
            self.hold_time = seek;
            self.apply_pending_playback_rate();
            self.start_time = None;
            self.pending = None;
            self.resolve_ready(notices);
        }
        self.update_finished_state(true, false, cx, notices);
        Ok(())
    }

    /// What becomes of the playback when its animation moves to another
    /// timeline (Web Animations 2 section 4.4.1), `cx` the new one's:
    /// `previous` its play state and progress through its effect on the
    /// old one, `from` and `to` whether the old and the new timelines
    /// are progress-based. Onto a progress-based timeline its start time
    /// follows the range, a playing one plays again there and a paused
    /// one holds the same progress; off one, it takes the same progress
    /// in the new timeline's time. Between two document timelines the
    /// start time is kept.
    pub fn timeline_changed(
        &mut self,
        previous: (PlayState, Option<f64>),
        (from, to): (bool, bool),
        cx: &Context,
        notices: &mut Vec<Notice>,
    ) {
        let (state, progress) = previous;
        if to {
            self.auto_align = true;
            self.start_time = None;
            self.hold_time = None;
            match state {
                PlayState::Running | PlayState::Finished => {
                    if self.pending.is_none() {
                        self.new_ready(notices);
                    }
                    self.pending = Some(Pending::Play);
                }
                PlayState::Paused => {
                    self.hold_time = progress.map(|p| p * cx.end);
                }
                PlayState::Idle => {}
            }
        } else if from && let Some(progress) = progress {
            let seek = progress * cx.end;
            if seek.is_finite() {
                let _ = self.silently_set_current_time(Some(seek), cx);
            }
        } else if self.start_time.is_some() {
            self.hold_time = None;
        }
        self.update_finished_state(false, false, cx, notices);
    }

    /// Sets the start time (section 4.4.10).
    pub fn set_start_time(
        &mut self,
        new_start: Option<f64>,
        cx: &Context,
        notices: &mut Vec<Notice>,
    ) {
        self.auto_align = false;
        if cx.timeline.is_none() && new_start.is_some() {
            self.hold_time = None;
        }
        let previous = self.current_time(cx);
        self.apply_pending_playback_rate();
        self.start_time = new_start;
        if new_start.is_some() {
            if self.playback_rate != 0.0 {
                self.hold_time = None;
            }
        } else {
            self.hold_time = previous;
        }
        if self.pending.take().is_some() {
            self.resolve_ready(notices);
        }
        self.update_finished_state(true, false, cx, notices);
    }

    /// Plays the animation (section 4.4.11), rewinding it to its start or
    /// end when `auto_rewind` says so and it is past them.
    pub fn play(
        &mut self,
        auto_rewind: bool,
        cx: &Context,
        notices: &mut Vec<Notice>,
    ) -> Result<(), Refusal> {
        let aborted_pause = self.pending == Some(Pending::Pause);
        // On a progress-based timeline the start time follows the range
        // (Web Animations 2 section 4.4.11): set once the animation is
        // ready, a time held let go.
        if cx.range.is_some() && auto_rewind {
            if cx.end == f64::INFINITY && self.effective_playback_rate() < 0.0 {
                return Err(Refusal::InvalidState);
            }
            let has_pending_ready_promise = self.pending.take().is_some();
            self.auto_align = true;
            self.hold_time = None;
            self.start_time = None;
            if !has_pending_ready_promise {
                self.new_ready(notices);
            }
            self.pending = Some(Pending::Play);
            self.update_finished_state(false, false, cx, notices);
            return Ok(());
        }
        let mut has_pending_ready_promise = false;
        let mut seek = None;
        let rate = self.effective_playback_rate();
        let current = self.current_time(cx);
        if rate > 0.0 && auto_rewind && current.is_none_or(|c| c < 0.0 || c >= cx.end) {
            seek = Some(0.0);
        } else if rate < 0.0 && auto_rewind && current.is_none_or(|c| c <= 0.0 || c > cx.end) {
            if cx.end == f64::INFINITY {
                return Err(Refusal::InvalidState);
            }
            seek = Some(cx.end);
        } else if rate == 0.0 && current.is_none() {
            seek = Some(0.0);
        }
        if seek.is_some() {
            self.hold_time = seek;
        }
        if self.hold_time.is_some() {
            self.start_time = None;
        }
        if self.pending.take().is_some() {
            has_pending_ready_promise = true;
        }
        if self.hold_time.is_none()
            && seek.is_none()
            && !aborted_pause
            && self.pending_playback_rate.is_none()
        {
            return Ok(());
        }
        if !has_pending_ready_promise {
            self.new_ready(notices);
        }
        self.pending = Some(Pending::Play);
        self.update_finished_state(false, false, cx, notices);
        Ok(())
    }

    /// Pauses the animation (section 4.4.13).
    pub fn pause(&mut self, cx: &Context, notices: &mut Vec<Notice>) -> Result<(), Refusal> {
        if self.pending == Some(Pending::Pause) || self.play_state(cx) == PlayState::Paused {
            return Ok(());
        }
        let mut seek = None;
        // On a progress-based timeline nothing is sought: the pause
        // holds the progress the range gives at the frame it runs, which
        // waits while the timeline is inactive (Web Animations 2 section
        // 4.4.13).
        let finite = cx.range.is_some();
        if finite && self.start_time.is_none() && self.hold_time.is_none() {
            self.auto_align = true;
        }
        if self.current_time(cx).is_none() && !finite {
            if self.playback_rate >= 0.0 {
                seek = Some(0.0);
            } else {
                if cx.end == f64::INFINITY {
                    return Err(Refusal::InvalidState);
                }
                seek = Some(cx.end);
            }
        }
        if seek.is_some() {
            self.hold_time = seek;
        }
        let mut has_pending_ready_promise = false;
        if self.pending == Some(Pending::Play) {
            self.pending = None;
            has_pending_ready_promise = true;
        }
        if !has_pending_ready_promise {
            self.new_ready(notices);
        }
        self.pending = Some(Pending::Pause);
        self.update_finished_state(false, false, cx, notices);
        Ok(())
    }

    /// Runs the pending task, the animation being ready at the timeline's
    /// time `ready` (sections 4.4.11 and 4.4.13).
    pub fn ready(&mut self, ready: f64, cx: &Context, notices: &mut Vec<Notice>) {
        match self.pending.take() {
            Some(Pending::Play) if self.auto_align && self.hold_time.is_none() => {
                self.apply_pending_playback_rate();
                self.start_time = self.aligned_start(cx).or(Some(ready));
                self.resolve_ready(notices);
                self.update_finished_state(false, false, cx, notices);
            }
            Some(Pending::Play) => {
                if let Some(hold) = self.hold_time {
                    self.apply_pending_playback_rate();
                    let start = if self.playback_rate == 0.0 {
                        ready
                    } else {
                        ready - hold / self.playback_rate
                    };
                    self.start_time = Some(start);
                    if self.playback_rate != 0.0 {
                        self.hold_time = None;
                    }
                } else if let (Some(start), Some(_)) = (self.start_time, self.pending_playback_rate)
                {
                    let to_match = (ready - start) * self.playback_rate;
                    self.apply_pending_playback_rate();
                    if self.playback_rate == 0.0 {
                        self.hold_time = Some(to_match);
                        self.start_time = Some(ready);
                    } else {
                        self.start_time = Some(ready - to_match / self.playback_rate);
                    }
                }
                self.resolve_ready(notices);
                self.update_finished_state(false, false, cx, notices);
            }
            Some(Pending::Pause) => {
                if self.auto_align && self.start_time.is_none() && self.hold_time.is_none() {
                    self.start_time = self.aligned_start(cx);
                }
                if let (Some(start), None) = (self.start_time, self.hold_time) {
                    self.hold_time = Some((ready - start) * self.playback_rate);
                }
                self.apply_pending_playback_rate();
                self.start_time = None;
                self.resolve_ready(notices);
                self.update_finished_state(false, false, cx, notices);
            }
            None => {}
        }
    }

    /// Finishes the animation (section 4.4.14).
    pub fn finish(&mut self, cx: &Context, notices: &mut Vec<Notice>) -> Result<(), Refusal> {
        let rate = self.effective_playback_rate();
        if rate == 0.0 || (rate > 0.0 && cx.end == f64::INFINITY) {
            return Err(Refusal::InvalidState);
        }
        self.apply_pending_playback_rate();
        self.auto_align = false;
        let limit = if self.playback_rate > 0.0 {
            cx.end
        } else {
            0.0
        };
        let _ = self.silently_set_current_time(Some(limit), cx);
        if self.start_time.is_none()
            && let Some(timeline) = cx.timeline
        {
            self.start_time = Some(timeline - limit / self.playback_rate);
        }
        if self.pending == Some(Pending::Pause) && self.start_time.is_some() {
            self.hold_time = None;
            self.pending = None;
            self.resolve_ready(notices);
        }
        if self.pending == Some(Pending::Play) && self.start_time.is_some() {
            self.pending = None;
            self.resolve_ready(notices);
        }
        self.update_finished_state(true, true, cx, notices);
        Ok(())
    }

    /// Cancels the animation (section 4.4.16): its pending tasks reset,
    /// its finished promise rejected, its times unresolved. Whether it was
    /// not idle, so that its cancel event is due.
    pub fn cancel(&mut self, cx: &Context, notices: &mut Vec<Notice>) -> bool {
        if self.play_state(cx) == PlayState::Idle {
            return false;
        }
        // Reset the pending tasks: the ready promise rejected, if one
        // waited on them.
        if self.pending.take().is_some() {
            self.apply_pending_playback_rate();
            self.ready_pending = false;
            notices.push(Notice::ReadyRejected);
        }
        notices.push(Notice::FinishedRejected);
        self.finished_resolved = false;
        self.hold_time = None;
        self.start_time = None;
        true
    }

    /// Reverses the animation (section 4.4.18).
    pub fn reverse(&mut self, cx: &Context, notices: &mut Vec<Notice>) -> Result<(), Refusal> {
        if cx.timeline.is_none() {
            return Err(Refusal::InvalidState);
        }
        let original = self.pending_playback_rate;
        self.pending_playback_rate = Some(-self.effective_playback_rate());
        let result = self.play(true, cx, notices);
        if result.is_err() {
            self.pending_playback_rate = original;
        }
        result
    }

    /// Sets the playback rate at once (section 4.4.17), the current time
    /// kept.
    pub fn set_playback_rate(&mut self, rate: f64, cx: &Context, notices: &mut Vec<Notice>) {
        self.pending_playback_rate = None;
        let previous = self.current_time(cx);
        self.playback_rate = rate;
        // On a progress-based timeline a running animation keeps its start
        // time, or follows the range where its start time does, its
        // current time scaled (Web Animations 2 section 4.4.17).
        if cx.range.is_some() && self.start_time.is_some() && self.hold_time.is_none() {
            self.align(cx);
            self.update_finished_state(false, false, cx, notices);
            return;
        }
        if let Some(previous) = previous {
            let _ = self.set_current_time(Some(previous), cx, notices);
        }
    }

    /// Updates the playback rate seamlessly (section 4.4.17.1).
    pub fn update_playback_rate(&mut self, rate: f64, cx: &Context, notices: &mut Vec<Notice>) {
        let previous_state = self.play_state(cx);
        self.pending_playback_rate = Some(rate);
        if self.pending.is_some() {
            return;
        }
        if self.current_time(cx).is_none() {
            self.apply_pending_playback_rate();
            return;
        }
        match previous_state {
            PlayState::Idle | PlayState::Paused => self.apply_pending_playback_rate(),
            PlayState::Finished => {
                let unconstrained = self.unconstrained_time(cx);
                if rate == 0.0 {
                    self.start_time = cx.timeline;
                } else if let (Some(timeline), Some(unconstrained)) = (cx.timeline, unconstrained) {
                    self.start_time = Some(timeline - unconstrained / rate);
                } else {
                    self.start_time = None;
                }
                self.apply_pending_playback_rate();
                self.update_finished_state(false, false, cx, notices);
            }
            PlayState::Running => {
                let _ = self.play(false, cx, notices);
            }
        }
    }

    /// Updates the finished state (section 4.4.15), `did_seek` when the
    /// current time was set, and runs the finish notification steps at
    /// once when `sync`, else in a microtask.
    pub fn update_finished_state(
        &mut self,
        did_seek: bool,
        sync: bool,
        cx: &Context,
        notices: &mut Vec<Notice>,
    ) {
        let unconstrained = if did_seek {
            self.current_time(cx)
        } else {
            self.unconstrained_time(cx)
        };
        // One whose start time follows its progress-based timeline's
        // range holds no time past its ends: its current time follows the
        // scroll back into its range.
        if let Some(unconstrained) = unconstrained
            && self.start_time.is_some()
            && self.pending.is_none()
            && !(self.auto_align && cx.range.is_some())
        {
            if self.playback_rate > 0.0 && unconstrained >= cx.end {
                self.hold_time = Some(if did_seek {
                    unconstrained
                } else {
                    self.previous_current_time
                        .map_or(cx.end, |previous| previous.max(cx.end))
                });
            } else if self.playback_rate < 0.0 && unconstrained <= 0.0 {
                self.hold_time = Some(if did_seek {
                    unconstrained
                } else {
                    self.previous_current_time
                        .map_or(0.0, |previous| previous.min(0.0))
                });
            } else if self.playback_rate != 0.0 && cx.timeline.is_some() {
                if did_seek && let (Some(hold), Some(timeline)) = (self.hold_time, cx.timeline) {
                    self.start_time = Some(timeline - hold / self.playback_rate);
                }
                self.hold_time = None;
            }
        }
        self.previous_current_time = self.current_time(cx);
        let finished = self.play_state(cx) == PlayState::Finished;
        if finished && !self.finished_resolved {
            notices.push(Notice::FinishNotify { sync });
        }
        if !finished && self.finished_resolved {
            self.finished_resolved = false;
            notices.push(Notice::FinishedNew);
        }
    }

    /// The finish notification steps (section 4.4.15): when the animation
    /// is still finished, its finished promise is resolved and a `finish`
    /// event is due. Whether it was.
    pub fn notify_finished(&mut self, cx: &Context) -> bool {
        if self.play_state(cx) != PlayState::Finished || self.finished_resolved {
            return false;
        }
        self.finished_resolved = true;
        true
    }

    /// Whether the animation's ready promise waits.
    pub fn ready_pending(&self) -> bool {
        self.ready_pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(timeline: f64, end: f64) -> Context {
        Context {
            timeline: Some(timeline),
            end,
            range: None,
        }
    }

    #[test]
    fn on_a_progress_based_timeline_the_start_time_follows_the_range() {
        let range = |timeline: f64, start: f64, end: f64| Context {
            timeline: Some(timeline),
            end: end - start,
            range: Some((start, end)),
        };
        let mut player = Player::new();
        let mut notices = Vec::new();
        player
            .play(true, &range(50.0, 0.0, 100.0), &mut notices)
            .unwrap();
        assert!(player.auto_align);
        assert_eq!(player.start_time, None);
        player.ready(50.0, &range(50.0, 0.0, 100.0), &mut notices);
        // Aligned with the range's start, wherever the timeline is.
        assert_eq!(player.start_time, Some(0.0));
        assert_eq!(player.current_time(&range(50.0, 0.0, 100.0)), Some(50.0));
        // The range moved: so does the start time.
        player.align(&range(50.0, 20.0, 80.0));
        assert_eq!(player.start_time, Some(20.0));
        // A time a script sets makes it stick.
        player
            .set_current_time(Some(10.0), &range(50.0, 20.0, 80.0), &mut notices)
            .unwrap();
        assert!(!player.auto_align);
        player.align(&range(50.0, 0.0, 100.0));
        assert_eq!(player.start_time, Some(40.0));
        // Backwards, aligned with its end.
        let mut back = Player::new();
        back.playback_rate = -1.0;
        back.play(true, &range(0.0, 0.0, 100.0), &mut notices)
            .unwrap();
        back.ready(0.0, &range(0.0, 0.0, 100.0), &mut notices);
        assert_eq!(back.start_time, Some(100.0));
        assert_eq!(back.current_time(&range(0.0, 0.0, 100.0)), Some(100.0));
    }

    #[test]
    fn playing_waits_for_the_ready_time_and_then_runs_from_the_hold_time() {
        let mut player = Player::new();
        let mut notices = Vec::new();
        assert_eq!(player.play_state(&cx(0.0, 1000.0)), PlayState::Idle);
        player.play(true, &cx(100.0, 1000.0), &mut notices).unwrap();
        assert_eq!(notices, vec![Notice::ReadyNew]);
        assert!(player.pending());
        assert_eq!(player.current_time(&cx(100.0, 1000.0)), Some(0.0));
        assert_eq!(player.start_time, None);
        assert_eq!(player.play_state(&cx(100.0, 1000.0)), PlayState::Running);
        notices.clear();
        player.ready(150.0, &cx(150.0, 1000.0), &mut notices);
        assert_eq!(notices, vec![Notice::ReadyResolved]);
        assert_eq!(player.start_time, Some(150.0));
        assert_eq!(player.current_time(&cx(400.0, 1000.0)), Some(250.0));
        // Past its end: finished, held there by the next update.
        notices.clear();
        player.update_finished_state(false, false, &cx(1300.0, 1000.0), &mut notices);
        assert_eq!(notices, vec![Notice::FinishNotify { sync: false }]);
        assert_eq!(player.hold_time, Some(1000.0));
        assert!(player.notify_finished(&cx(1300.0, 1000.0)));
        assert!(!player.notify_finished(&cx(1300.0, 1000.0)));
        // Played again: rewound, its finished promise new.
        notices.clear();
        player
            .play(true, &cx(1400.0, 1000.0), &mut notices)
            .unwrap();
        assert_eq!(notices, vec![Notice::ReadyNew, Notice::FinishedNew]);
        assert_eq!(player.current_time(&cx(1400.0, 1000.0)), Some(0.0));
    }

    #[test]
    fn pausing_holds_the_time_the_animation_is_ready_at() {
        let mut player = Player::new();
        let mut notices = Vec::new();
        player.play(true, &cx(0.0, 1000.0), &mut notices).unwrap();
        player.ready(0.0, &cx(0.0, 1000.0), &mut notices);
        notices.clear();
        player.pause(&cx(200.0, 1000.0), &mut notices).unwrap();
        assert_eq!(notices, vec![Notice::ReadyNew]);
        assert_eq!(player.play_state(&cx(200.0, 1000.0)), PlayState::Paused);
        // Its time runs on until the pause task holds it.
        assert_eq!(player.current_time(&cx(300.0, 1000.0)), Some(300.0));
        player.ready(300.0, &cx(300.0, 1000.0), &mut notices);
        assert_eq!(player.current_time(&cx(900.0, 1000.0)), Some(300.0));
        assert_eq!(player.start_time, None);
        // A pause while play-pending takes over the play's promise.
        let mut other = Player::new();
        notices.clear();
        other.play(true, &cx(0.0, 1000.0), &mut notices).unwrap();
        other.pause(&cx(0.0, 1000.0), &mut notices).unwrap();
        assert_eq!(notices, vec![Notice::ReadyNew]);
    }

    #[test]
    fn finishing_reversing_and_cancelling_follow_the_draft() {
        let mut notices = Vec::new();
        let mut player = Player::new();
        player.play(true, &cx(0.0, 1000.0), &mut notices).unwrap();
        notices.clear();
        player.finish(&cx(0.0, 1000.0), &mut notices).unwrap();
        // Finishing a play-pending animation resolves its ready promise
        // and notifies at once.
        assert_eq!(
            notices,
            vec![Notice::ReadyResolved, Notice::FinishNotify { sync: true }]
        );
        assert_eq!(player.current_time(&cx(0.0, 1000.0)), Some(1000.0));
        assert_eq!(player.start_time, Some(-1000.0));
        // An infinite effect neither finishes nor reverses.
        let mut forever = Player::new();
        assert_eq!(
            forever.finish(&cx(0.0, f64::INFINITY), &mut notices),
            Err(Refusal::InvalidState)
        );
        assert_eq!(
            forever.reverse(&cx(0.0, f64::INFINITY), &mut notices),
            Err(Refusal::InvalidState)
        );
        assert_eq!(forever.pending_playback_rate, None);
        // Reversed: from the end, its rate pending until ready.
        let mut back = Player::new();
        back.reverse(&cx(0.0, 1000.0), &mut notices).unwrap();
        // Cancelled while pending, its ready promise is rejected too.
        let mut pending = back.clone();
        let mut rejected = Vec::new();
        assert!(pending.cancel(&cx(0.0, 1000.0), &mut rejected));
        assert_eq!(
            rejected,
            vec![Notice::ReadyRejected, Notice::FinishedRejected]
        );
        assert_eq!(back.playback_rate, 1.0);
        assert_eq!(back.effective_playback_rate(), -1.0);
        assert_eq!(back.current_time(&cx(0.0, 1000.0)), Some(1000.0));
        back.ready(50.0, &cx(50.0, 1000.0), &mut notices);
        assert_eq!(back.playback_rate, -1.0);
        assert_eq!(back.current_time(&cx(150.0, 1000.0)), Some(900.0));
        // Cancelled: idle, its promises rejected.
        notices.clear();
        assert!(back.cancel(&cx(150.0, 1000.0), &mut notices));
        assert_eq!(notices, vec![Notice::FinishedRejected]);
        assert_eq!(back.play_state(&cx(150.0, 1000.0)), PlayState::Idle);
        assert!(!back.cancel(&cx(150.0, 1000.0), &mut notices));
    }

    #[test]
    fn the_current_time_and_the_start_time_set_as_the_draft_says() {
        let mut notices = Vec::new();
        let mut player = Player::new();
        // An idle animation seeked is paused there.
        player
            .set_current_time(Some(500.0), &cx(100.0, 1000.0), &mut notices)
            .unwrap();
        assert_eq!(player.play_state(&cx(100.0, 1000.0)), PlayState::Paused);
        assert_eq!(
            player.set_current_time(None, &cx(100.0, 1000.0), &mut notices),
            Err(Refusal::Type)
        );
        // A start time set runs it from there.
        player.set_start_time(Some(0.0), &cx(100.0, 1000.0), &mut notices);
        assert_eq!(player.current_time(&cx(300.0, 1000.0)), Some(300.0));
        assert_eq!(player.play_state(&cx(300.0, 1000.0)), PlayState::Running);
        // Seeking a running animation moves its start time.
        player
            .set_current_time(Some(100.0), &cx(300.0, 1000.0), &mut notices)
            .unwrap();
        assert_eq!(player.start_time, Some(200.0));
        // The playback rate changed seamlessly keeps the current time.
        player.update_playback_rate(2.0, &cx(300.0, 1000.0), &mut notices);
        assert!(player.pending());
        player.ready(300.0, &cx(300.0, 1000.0), &mut notices);
        assert_eq!(player.current_time(&cx(350.0, 1000.0)), Some(200.0));
        player.set_playback_rate(-1.0, &cx(350.0, 1000.0), &mut notices);
        assert_eq!(player.current_time(&cx(400.0, 1000.0)), Some(150.0));
    }
}
