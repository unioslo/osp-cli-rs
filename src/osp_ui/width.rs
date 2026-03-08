use unicode_width::UnicodeWidthStr;

pub(crate) fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

pub(crate) fn mreg_alignment_key_width(key: &str) -> usize {
    display_width(strip_count_suffix(key))
}

pub(crate) fn strip_count_suffix(key: &str) -> &str {
    if let Some(prefix_end) = key.rfind(" (") {
        let suffix = &key[prefix_end + 2..];
        if let Some(count) = suffix.strip_suffix(')')
            && !count.is_empty()
            && count.bytes().all(|byte| byte.is_ascii_digit())
        {
            return &key[..prefix_end];
        }
    }
    key
}
