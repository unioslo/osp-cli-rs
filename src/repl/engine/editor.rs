//! Internal prompt and line-editor adapter mechanics.
//!
//! Host-facing REPL configuration lives in [`super::config`]. This module
//! translates that semantic surface into reedline-specific behavior such as
//! prompt rendering, menu reopening, and terminal capability probing.

use std::borrow::Cow;
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::time::{Duration, Instant};

use reedline::{
    EditCommand, EditMode, Emacs, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, ReedlineEvent, ReedlineRawEvent,
};

use super::{PromptRightRenderer, ReplInputMode};

pub(crate) struct AutoCompleteEmacs {
    inner: Emacs,
    menu_name: String,
}

impl AutoCompleteEmacs {
    pub(crate) fn new(inner: Emacs, menu_name: impl Into<String>) -> Self {
        Self {
            inner,
            menu_name: menu_name.into(),
        }
    }

    pub(crate) fn should_reopen_menu(commands: &[EditCommand]) -> bool {
        // reedline closes menus on ordinary edits. Reopen after text-changing
        // edits so completion keeps behaving like an interactive shell menu
        // instead of forcing the user to press Tab again after every keystroke.
        commands.iter().any(|cmd| {
            matches!(
                cmd,
                EditCommand::InsertChar(_)
                    | EditCommand::InsertString(_)
                    | EditCommand::ReplaceChar(_)
                    | EditCommand::ReplaceChars(_, _)
                    | EditCommand::Backspace
                    | EditCommand::Delete
                    | EditCommand::CutChar
                    | EditCommand::BackspaceWord
                    | EditCommand::DeleteWord
                    | EditCommand::Clear
                    | EditCommand::ClearToLineEnd
                    | EditCommand::CutCurrentLine
                    | EditCommand::CutFromStart
                    | EditCommand::CutFromLineStart
                    | EditCommand::CutToEnd
                    | EditCommand::CutToLineEnd
                    | EditCommand::CutWordLeft
                    | EditCommand::CutBigWordLeft
                    | EditCommand::CutWordRight
                    | EditCommand::CutBigWordRight
                    | EditCommand::CutWordRightToNext
                    | EditCommand::CutBigWordRightToNext
                    | EditCommand::PasteCutBufferBefore
                    | EditCommand::PasteCutBufferAfter
                    | EditCommand::Undo
                    | EditCommand::Redo
            )
        })
    }
}

impl EditMode for AutoCompleteEmacs {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let parsed = self.inner.parse_event(event);
        match parsed {
            ReedlineEvent::Edit(commands) if Self::should_reopen_menu(&commands) => {
                ReedlineEvent::Multiple(vec![
                    ReedlineEvent::Edit(commands),
                    ReedlineEvent::Menu(self.menu_name.clone()),
                ])
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

    if matches!(input_mode, ReplInputMode::Auto) && !cursor_position_reports_supported() {
        return Some(BasicInputReason::CursorProbeUnsupported);
    }

    None
}

#[cfg(not(unix))]
fn cursor_position_reports_supported() -> bool {
    true
}

#[cfg(unix)]
fn cursor_position_reports_supported() -> bool {
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

pub(crate) fn parse_cursor_position_report(bytes: &[u8]) -> Option<(u16, u16)> {
    let rest = bytes.strip_prefix(b"\x1b[")?;
    let row_end = rest.iter().position(|byte| !byte.is_ascii_digit())?;
    if row_end == 0 || *rest.get(row_end)? != b';' {
        return None;
    }
    let row = std::str::from_utf8(&rest[..row_end])
        .ok()?
        .parse::<u16>()
        .ok()?;
    let col_rest = &rest[row_end + 1..];
    let col_end = col_rest.iter().position(|byte| !byte.is_ascii_digit())?;
    if col_end == 0 || *col_rest.get(col_end)? != b'R' {
        return None;
    }
    let col = std::str::from_utf8(&col_rest[..col_end])
        .ok()?
        .parse::<u16>()
        .ok()?;
    Some((col, row))
}

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
