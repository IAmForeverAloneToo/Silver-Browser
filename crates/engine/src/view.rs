//! What the shell's session drives (M34.5, docs/design/processes.md,
//! "The session's proxy"): an [`Engine`] in one process, or a
//! [`RemoteTab`](crate::service::RemoteTab), the browser's proxy of the
//! engines its renderers run. The methods are the engine's, by its names;
//! what the session asks and waits for in one process comes back later
//! from a renderer, as the `take_` methods give it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{
    Button, Clock, ConsoleEntry, Cursor, Engine, FrameTime, Key, KeyOutcome, Link, LoadState,
    Modifiers, PermissionPrompt, Preferences, Profile, Scrollbar, Url,
};

/// What the session drives: a tab's page.
pub trait View {
    fn fonts(&self) -> &text::FontCollection;

    fn navigate(&mut self, url: Url);
    fn go_back(&mut self);
    fn go_forward(&mut self);
    fn reload(&mut self);
    fn stop(&mut self);
    fn can_go_back(&self) -> bool;
    fn can_go_forward(&self) -> bool;

    fn url(&self) -> Option<&Url>;
    fn title(&self) -> String;
    fn load_state(&self) -> &LoadState;
    fn undecodable_encoding(&self) -> Option<&'static str>;
    fn take_notice(&mut self) -> Option<String>;
    fn take_console(&mut self) -> Vec<ConsoleEntry>;
    fn permission_prompt(&self) -> Option<PermissionPrompt>;
    fn answer_prompt(&mut self, id: u64, answer: Option<bool>);

    fn set_preferences(&mut self, preferences: Preferences);
    fn set_scripting(&mut self, on: bool);
    fn set_compat_user_agent(&mut self, on: bool);
    fn set_hidden(&mut self, hidden: bool);
    fn set_window_focus(&mut self, focused: bool);
    fn set_modifiers(&mut self, modifiers: Modifiers);
    fn set_scale_factor(&mut self, scale: f32);
    fn scale_factor(&self) -> f32;
    fn set_viewport(&mut self, width: u32, height: u32);
    fn set_layer_budget(&mut self, bytes: usize);
    fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>);
    fn set_clock(&mut self, clock: Clock);

    fn run_tasks(&mut self) -> bool;
    fn poll_network(&mut self) -> bool;
    fn next_task_at(&self) -> Option<Duration>;
    fn needs_frame(&self) -> bool;
    fn next_frame_at(&self) -> Option<Duration>;
    fn frame_at(&mut self, width: u32, height: u32, time: FrameTime) -> &[u8];
    fn draw_ahead(&mut self, until: Instant) -> u32;

    fn pointer_move(&mut self, x: f32, y: f32);
    fn pointer_leave(&mut self);
    fn pointer_down(&mut self, x: f32, y: f32, button: Button, count: u32);
    fn pointer_up(&mut self, x: f32, y: f32, button: Button, count: u32);
    fn wheel(&mut self, x: f32, y: f32, dy: f32);
    fn cursor_at(&mut self, x: f32, y: f32) -> Cursor;
    fn link_at(&mut self, x: f32, y: f32) -> Option<Link>;
    fn key_down(&mut self, key: &Key) -> KeyOutcome;
    fn key_up(&mut self, key: &Key);
    fn focus_next(&mut self, backward: bool) -> bool;

    fn insert_text(&mut self, text: &str) -> bool;
    fn copy(&self) -> Option<String>;
    fn cut(&mut self) -> Option<String>;
    fn paste(&mut self, text: &str) -> bool;
    fn can_paste(&self) -> bool;
    fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)>;
    fn popup_open(&self) -> bool;

    fn scrollbar(&mut self) -> Option<Scrollbar>;
    fn scroll_by(&mut self, dy: f32);
    fn scroll_to_fraction(&mut self, fraction: f32);

    fn flush_profile(&mut self) -> Result<(), profile::Error>;
    /// Keeps what the browsing keeps in `profile` from now on (M28).
    fn set_profile(&mut self, profile: Profile) -> Result<(), profile::Error>;

    /// The selection's text, to be cut once the clipboard has it: a
    /// remote page gives it later, through [`View::take_copied`].
    fn copy_to_cut(&mut self) -> Option<String> {
        self.copy()
    }

    /// Whether the keyboard's focus left the page (sequential navigation
    /// past its last element) since this was last asked: an answer that
    /// a remote page gives after [`View::key_down`] returned.
    fn take_focus_left(&mut self) -> bool {
        false
    }

    /// The text a remote page copied, and whether it was asked to cut
    /// it, since this was last asked: the answer to [`View::copy`], which
    /// a remote page gives later.
    fn take_copied(&mut self) -> Option<(String, bool)> {
        None
    }

    /// Asks a remote page to cut what it copied for a cut, once the
    /// clipboard has it.
    fn cut_copied(&mut self) {}

    /// Whether the frame drawn holds the page's own pixels (M35.6): a
    /// remote page's not before its renderer has drawn one.
    fn has_page_frame(&self) -> bool {
        true
    }

    /// The tabs its documents asked the browser to open since this was
    /// last asked (M37).
    fn take_opened(&mut self) -> Vec<crate::frames::OpenTab> {
        Vec::new()
    }

    /// Whether its top-level document asked to close the tab since this
    /// was last asked, and a script opened the tab, which lets it (M37).
    fn take_close_request(&mut self) -> bool {
        false
    }

    /// Whether a script opened the tab the view shows (M37).
    fn set_opened_by_script(&mut self, by_script: bool) {
        let _ = by_script;
    }

    /// The user's languages from now on (M37): its documents'
    /// `navigator.languages`.
    fn set_languages(&mut self, languages: Vec<String>) {
        let _ = languages;
    }

    /// The downloads of the view's tab (M37), its context's, if it saves
    /// any.
    fn downloads(&self) -> Option<crate::downloads::Downloads> {
        None
    }

    /// How many renderer processes the view keeps alive (M37): the
    /// browser's cap across its tabs counts them.
    fn renderer_count(&self) -> usize {
        0
    }

    /// The tab the view shows is closed (M37): what it kept of its own,
    /// its `sessionStorage`, is let go.
    fn closed(&mut self) {}

    /// A view of the same kind showing the browser's DevTools panel
    /// (M36), on this one's fonts, clock, waker and preferences; `None`
    /// where there is none.
    fn devtools_panel(&self) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }

    /// The panel's page hears `message` from the browser.
    fn devtools_send(&mut self, message: String) {
        let _ = message;
    }

    /// What the panel's page sent the browser since this was last asked.
    fn take_devtools(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// The page shown is asked the panel's request `message`.
    fn inspect(&mut self, message: String) {
        let _ = message;
    }

    /// What the page shown answered the panel, and told it, since this
    /// was last asked.
    fn take_inspected(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// The tab's network log (M36.3), if it keeps one.
    fn network_log(&self) -> Option<Arc<crate::network_log::NetworkLog>> {
        None
    }

    /// The panel's request `message` of the storage panel's (M36.8), of
    /// the page shown, which the browser answers from its own records.
    fn storage_inspect(&mut self, message: &str) -> String {
        let _ = message;
        String::new()
    }
}

impl View for Engine {
    fn fonts(&self) -> &text::FontCollection {
        Engine::fonts(self)
    }
    fn navigate(&mut self, url: Url) {
        Engine::navigate(self, url)
    }
    fn go_back(&mut self) {
        Engine::go_back(self)
    }
    fn go_forward(&mut self) {
        Engine::go_forward(self)
    }
    fn reload(&mut self) {
        Engine::reload(self)
    }
    fn stop(&mut self) {
        Engine::stop(self)
    }
    fn can_go_back(&self) -> bool {
        Engine::can_go_back(self)
    }
    fn can_go_forward(&self) -> bool {
        Engine::can_go_forward(self)
    }
    fn url(&self) -> Option<&Url> {
        Engine::url(self)
    }
    fn title(&self) -> String {
        Engine::title(self)
    }
    fn load_state(&self) -> &LoadState {
        Engine::load_state(self)
    }
    fn undecodable_encoding(&self) -> Option<&'static str> {
        Engine::undecodable_encoding(self)
    }
    fn take_notice(&mut self) -> Option<String> {
        Engine::take_notice(self)
    }
    fn take_console(&mut self) -> Vec<ConsoleEntry> {
        Engine::take_console(self)
    }
    fn permission_prompt(&self) -> Option<PermissionPrompt> {
        Engine::permission_prompt(self)
    }
    fn answer_prompt(&mut self, id: u64, answer: Option<bool>) {
        Engine::answer_prompt(self, id, answer)
    }
    fn set_preferences(&mut self, preferences: Preferences) {
        Engine::set_preferences(self, preferences)
    }
    fn set_scripting(&mut self, on: bool) {
        Engine::set_scripting(self, on)
    }
    fn set_compat_user_agent(&mut self, on: bool) {
        Engine::set_compat_user_agent(self, on)
    }
    fn set_hidden(&mut self, hidden: bool) {
        Engine::set_hidden(self, hidden)
    }
    fn set_window_focus(&mut self, focused: bool) {
        Engine::set_window_focus(self, focused)
    }
    fn set_modifiers(&mut self, modifiers: Modifiers) {
        Engine::set_modifiers(self, modifiers)
    }
    fn set_scale_factor(&mut self, scale: f32) {
        Engine::set_scale_factor(self, scale)
    }
    fn scale_factor(&self) -> f32 {
        Engine::scale_factor(self)
    }
    fn set_viewport(&mut self, width: u32, height: u32) {
        Engine::set_viewport(self, width, height)
    }
    fn set_layer_budget(&mut self, bytes: usize) {
        Engine::set_layer_budget(self, bytes)
    }
    fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        Engine::set_waker(self, move || waker())
    }
    fn set_clock(&mut self, clock: Clock) {
        Engine::set_clock(self, clock)
    }
    fn run_tasks(&mut self) -> bool {
        Engine::run_tasks(self)
    }
    fn poll_network(&mut self) -> bool {
        Engine::poll_network(self)
    }
    fn next_task_at(&self) -> Option<Duration> {
        Engine::next_task_at(self)
    }
    fn needs_frame(&self) -> bool {
        Engine::needs_frame(self)
    }
    fn next_frame_at(&self) -> Option<Duration> {
        Engine::next_frame_at(self)
    }
    fn frame_at(&mut self, width: u32, height: u32, time: FrameTime) -> &[u8] {
        Engine::frame_at(self, width, height, time)
    }
    fn draw_ahead(&mut self, until: Instant) -> u32 {
        Engine::draw_ahead(self, until)
    }
    fn pointer_move(&mut self, x: f32, y: f32) {
        Engine::pointer_move(self, x, y)
    }
    fn pointer_leave(&mut self) {
        Engine::pointer_leave(self)
    }
    fn pointer_down(&mut self, x: f32, y: f32, button: Button, count: u32) {
        Engine::pointer_down(self, x, y, button, count)
    }
    fn pointer_up(&mut self, x: f32, y: f32, button: Button, count: u32) {
        Engine::pointer_up(self, x, y, button, count)
    }
    fn wheel(&mut self, x: f32, y: f32, dy: f32) {
        Engine::wheel(self, x, y, dy)
    }
    fn cursor_at(&mut self, x: f32, y: f32) -> Cursor {
        Engine::cursor_at(self, x, y)
    }
    fn link_at(&mut self, x: f32, y: f32) -> Option<Link> {
        Engine::link_at(self, x, y)
    }
    fn key_down(&mut self, key: &Key) -> KeyOutcome {
        Engine::key_down(self, key)
    }
    fn key_up(&mut self, key: &Key) {
        Engine::key_up(self, key)
    }
    fn focus_next(&mut self, backward: bool) -> bool {
        Engine::focus_next(self, backward)
    }
    fn insert_text(&mut self, text: &str) -> bool {
        Engine::insert_text(self, text)
    }
    fn copy(&self) -> Option<String> {
        Engine::copy(self)
    }
    fn cut(&mut self) -> Option<String> {
        Engine::cut(self)
    }
    fn paste(&mut self, text: &str) -> bool {
        Engine::paste(self, text)
    }
    fn can_paste(&self) -> bool {
        Engine::can_paste(self)
    }
    fn caret_area(&mut self) -> Option<(f32, f32, f32, f32)> {
        Engine::caret_area(self)
    }
    fn popup_open(&self) -> bool {
        Engine::popup_open(self).is_some()
    }
    fn scrollbar(&mut self) -> Option<Scrollbar> {
        Engine::scrollbar(self)
    }
    fn scroll_by(&mut self, dy: f32) {
        Engine::scroll_by(self, dy)
    }
    fn scroll_to_fraction(&mut self, fraction: f32) {
        Engine::scroll_to_fraction(self, fraction)
    }
    fn flush_profile(&mut self) -> Result<(), profile::Error> {
        Engine::flush_profile(self)
    }
    fn set_profile(&mut self, profile: Profile) -> Result<(), profile::Error> {
        Engine::set_profile(self, profile)
    }
    fn closed(&mut self) {
        let tab = self.tab;
        self.storage().forget_tab(tab);
    }

    fn take_opened(&mut self) -> Vec<crate::frames::OpenTab> {
        Engine::take_opened(self)
    }

    fn downloads(&self) -> Option<crate::downloads::Downloads> {
        Some(self.downloads.clone())
    }

    fn set_languages(&mut self, languages: Vec<String>) {
        Engine::set_languages(self, languages);
    }

    fn take_close_request(&mut self) -> bool {
        self.take_close_asked() && self.opened_by_script
    }

    fn set_opened_by_script(&mut self, by_script: bool) {
        Engine::set_opened_by_script(self, by_script);
    }

    fn devtools_panel(&self) -> Option<Engine> {
        Some(Engine::devtools_panel(self))
    }
    fn devtools_send(&mut self, message: String) {
        Engine::devtools_send(self, message)
    }
    fn take_devtools(&mut self) -> Vec<String> {
        Engine::take_devtools(self)
    }
    fn inspect(&mut self, message: String) {
        Engine::inspect(self, &message)
    }
    fn take_inspected(&mut self) -> Vec<String> {
        Engine::take_inspected(self)
    }
    fn network_log(&self) -> Option<Arc<crate::network_log::NetworkLog>> {
        Some(self.network.log().clone())
    }
    fn storage_inspect(&mut self, message: &str) -> String {
        let jar = self.network.cookies();
        let url = Engine::url(self).cloned();
        crate::storage::devtools::inspect(&self.storage, &jar, self.tab, url.as_ref(), message)
    }
}
