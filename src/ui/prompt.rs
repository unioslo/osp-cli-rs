//! Line input shared by command prompts and the REPL terminal protocol.

use std::io;

/// Read a response, excluding terminal cursor reports left by an editor query.
///
/// Only a well-formed CPR sequence is consumed; arbitrary escape sequences and
/// user text remain intact. Returns zero on EOF, like `Stdin::read_line`.
pub fn read_line(answer: &mut String) -> io::Result<usize> {
    let mut line = String::new();
    let count = io::stdin().read_line(&mut line)?;
    let mut rest = line.as_str();
    while let Some(start) = rest.find("\x1b[") {
        answer.push_str(&rest[..start]);
        rest = &rest[start..];
        if parse_cursor_position_report(rest.as_bytes()).is_some() {
            let end = rest.find('R').expect("validated cursor report");
            rest = &rest[end + 1..];
        } else {
            answer.push('\x1b');
            rest = &rest[1..];
        }
    }
    answer.push_str(rest);
    Ok(count)
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
