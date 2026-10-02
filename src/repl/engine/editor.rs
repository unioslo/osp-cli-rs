//! Internal prompt and line-editor adapter mechanics.
//!
//! Host-facing REPL configuration lives in [`super::config`]. This module
//! translates that semantic surface into reedline-specific behavior such as
//! prompt rendering, menu reopening, and terminal capability probing.

use std::borrow::Cow;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
#[cfg(unix)]
use std::time::{Duration, Instant};

use reedline::{
    EditCommand, EditMode, Emacs, Menu, MenuEvent, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, ReedlineEvent, ReedlineRawEvent,
};

use super::{PromptRightRenderer, ReplInputMode, ReplTabMode};
use crate::repl::menu::SharedCompletionMenu;

/// The input line and cursor as reedline last painted them.
///
/// An edit mode only sees key events, never the buffer. The hinter runs on
/// every paint and records the line here, so key handling can measure the
/// word being typed.
#[derive(Clone, Default)]
pub(crate) struct PaintedLine(Arc<Mutex<(String, usize)>>);

impl PaintedLine {
    pub(crate) fn set(&self, line: &str, cursor: usize) {
        let mut painted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        painted.0.clear();
        painted.0.push_str(line);
        painted.1 = cursor;
    }

    /// Characters between the start of the current word and the cursor.
    fn word_len_before_cursor(&self) -> usize {
        let painted = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let before = painted.0.get(..painted.1).unwrap_or(&painted.0);
        before
            .chars()
            .rev()
            .take_while(|ch| !ch.is_whitespace())
            .count()
    }
}

pub(crate) struct AutoCompleteEmacs {
    inner: Emacs,
    menu: SharedCompletionMenu,
    tab_mode: ReplTabMode,
    painted: PaintedLine,
}

impl AutoCompleteEmacs {
    pub(crate) fn new(
        inner: Emacs,
        menu: SharedCompletionMenu,
        tab_mode: ReplTabMode,
        painted: PaintedLine,
    ) -> Self {
        Self {
            inner,
            menu,
            tab_mode,
            painted,
        }
    }

    /// Whether typing these commands should open a closed menu.
    ///
    /// Starting a flag always does; a `-` inside a word such as a hostname
    /// does not. Otherwise `tab_mode` decides. A space only opens in `Always`
    /// mode and is handled with the token commit below.
    pub(crate) fn opens_menu(&self, commands: &[EditCommand]) -> bool {
        let [EditCommand::InsertChar(ch)] = commands else {
            return false;
        };
        if *ch == '-' && self.painted.word_len_before_cursor() == 0 {
            return true;
        }
        match self.tab_mode {
            ReplTabMode::Tab => false,
            ReplTabMode::Always => true,
            ReplTabMode::AfterLetters(count) => {
                !ch.is_whitespace() && self.painted.word_len_before_cursor() + 1 >= count
            }
        }
    }

    fn auto_open(&self) -> ReedlineEvent {
        ReedlineEvent::Menu(self.menu.name().to_string())
    }

    /// Whether `event` is Tab opening a closed menu.
    fn is_tab_open(&self, event: &ReedlineEvent) -> bool {
        matches!(
            event,
            ReedlineEvent::UntilFound(events)
                if matches!(
                    events.as_slice(),
                    [ReedlineEvent::Menu(name), ReedlineEvent::MenuNext] if name == self.menu.name()
                )
        )
    }
}

/// The menu move a key binding asks for once the completion menu is open.
///
/// With the menu open, reedline resolves `UntilFound` to its first menu move:
/// `Menu(name)` only applies to a closed menu. Menu navigation takes precedence
/// over accepting an inline history hint.
pub(crate) fn menu_navigation(event: &ReedlineEvent) -> Option<MenuEvent> {
    match event {
        ReedlineEvent::MenuNext => Some(MenuEvent::NextElement),
        ReedlineEvent::MenuPrevious => Some(MenuEvent::PreviousElement),
        ReedlineEvent::MenuUp => Some(MenuEvent::MoveUp),
        ReedlineEvent::MenuDown => Some(MenuEvent::MoveDown),
        ReedlineEvent::MenuLeft => Some(MenuEvent::MoveLeft),
        ReedlineEvent::MenuRight => Some(MenuEvent::MoveRight),
        ReedlineEvent::MenuPageNext => Some(MenuEvent::NextPage),
        ReedlineEvent::MenuPagePrevious => Some(MenuEvent::PreviousPage),
        ReedlineEvent::UntilFound(events) => events.iter().find_map(menu_navigation),
        _ => None,
    }
}

impl EditMode for AutoCompleteEmacs {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let parsed = self.inner.parse_event(event);
        // Selection moves become buffer edits here, before reedline paints;
        // see `OspCompletionMenu::navigate`.
        if let Some(commands) =
            menu_navigation(&parsed).and_then(|nav| self.menu.navigate_painted_line(nav))
        {
            return ReedlineEvent::Edit(commands);
        }
        // Only Tab completes like a shell; menus opened by typing just list.
        if self.is_tab_open(&parsed) {
            self.menu.mark_tab_open();
        }
        match parsed {
            ReedlineEvent::Edit(commands) if commands == [EditCommand::InsertChar(' ')] => {
                // Space commits the token: close the menu, keeping any cycled
                // selection already in the buffer, instead of letting reedline
                // refresh it with the next token's candidates. `Always` then
                // opens a fresh menu for the next token.
                let mut events = vec![ReedlineEvent::Esc, ReedlineEvent::Edit(commands)];
                if self.tab_mode == ReplTabMode::Always {
                    events.push(self.auto_open());
                }
                ReedlineEvent::Multiple(events)
            }
            // An open menu already refilters on every edit.
            ReedlineEvent::Edit(commands)
                if !self.menu.is_active() && self.opens_menu(&commands) =>
            {
                ReedlineEvent::Multiple(vec![ReedlineEvent::Edit(commands), self.auto_open()])
            }
            other => other,
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

pub(crate) fn is_cursor_position_error(err: &io::Error) -> bool {
    if matches!(err.raw_os_error(), Some(6 | 25)) {
        return true;
    }
    let message = err.to_string().to_ascii_lowercase();
    message.contains("cursor position could not be read")
        || message.contains("no such device or address")
        || message.contains("inappropriate ioctl")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BasicInputReason {
    Explicit,
    NotATerminal,
    CursorProbeUnsupported,
}

pub(crate) fn basic_input_reason(input_mode: ReplInputMode) -> Option<BasicInputReason> {
    if matches!(input_mode, ReplInputMode::Basic) {
        return Some(BasicInputReason::Explicit);
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Some(BasicInputReason::NotATerminal);
    }

    if matches!(input_mode, ReplInputMode::Auto)
        && (CURSOR_POSITION_FAILED.load(Ordering::Relaxed)
            || !*CURSOR_POSITION_SUPPORTED.get_or_init(probe_cursor_position_reports))
    {
        return Some(BasicInputReason::CursorProbeUnsupported);
    }

    None
}

static CURSOR_POSITION_SUPPORTED: OnceLock<bool> = OnceLock::new();
static CURSOR_POSITION_FAILED: AtomicBool = AtomicBool::new(false);

pub(crate) fn mark_cursor_position_unsupported() {
    CURSOR_POSITION_FAILED.store(true, Ordering::Relaxed);
}

#[cfg(not(unix))]
fn probe_cursor_position_reports() -> bool {
    true
}

#[cfg(unix)]
fn probe_cursor_position_reports() -> bool {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    const CURSOR_PROBE_TIMEOUT: Duration = Duration::from_millis(75);

    struct RawModeGuard {
        fd: i32,
        original: libc::termios,
        active: bool,
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            if self.active {
                unsafe {
                    libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
                }
            }
        }
    }

    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut original = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
        return true;
    }
    let original = unsafe { original.assume_init() };
    let mut raw = original;
    unsafe {
        libc::cfmakeraw(&mut raw);
    }
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return true;
    }
    let _guard = RawModeGuard {
        fd,
        original,
        active: true,
    };

    let mut stdout = io::stdout();
    if stdout.write_all(b"\x1b[6n").is_err() || stdout.flush().is_err() {
        return true;
    }

    // Probe early so we can choose basic input before reedline owns the
    // terminal, instead of surfacing a cursor-request failure mid-session.
    let start = Instant::now();
    let mut buffer = Vec::with_capacity(32);
    while start.elapsed() < CURSOR_PROBE_TIMEOUT {
        let remaining = CURSOR_PROBE_TIMEOUT
            .saturating_sub(start.elapsed())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, remaining) };
        if ready <= 0 {
            break;
        }
        let mut chunk = [0u8; 64];
        let read = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if read <= 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read as usize]);
        if contains_cursor_position_report(&buffer) {
            return true;
        }
        if buffer.len() >= 256 {
            break;
        }
    }

    false
}

pub(crate) fn contains_cursor_position_report(bytes: &[u8]) -> bool {
    bytes.windows(2).enumerate().any(|(start, window)| {
        window == b"\x1b[" && parse_cursor_position_report(&bytes[start..]).is_some()
    })
}

pub(crate) use crate::ui::prompt::parse_cursor_position_report;

pub(crate) struct OspPrompt {
    left: String,
    indicator: String,
    right: Option<PromptRightRenderer>,
}

impl OspPrompt {
    pub(crate) fn new(left: String, indicator: String, right: Option<PromptRightRenderer>) -> Self {
        Self {
            left,
            indicator,
            right,
        }
    }

    pub(crate) fn left(&self) -> &str {
        &self.left
    }

    pub(crate) fn indicator(&self) -> &str {
        &self.indicator
    }
}

impl Prompt for OspPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.left.as_str())
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        match &self.right {
            Some(render) => Cow::Owned(render()),
            None => Cow::Borrowed(""),
        }
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator.as_str())
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }

    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Reset
    }

    fn get_indicator_color(&self) -> reedline::Color {
        reedline::Color::Reset
    }
}
