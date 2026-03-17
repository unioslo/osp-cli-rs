pub(crate) fn visible_inline_text(value: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }

        if ch == '`' {
            let fence = if i + 1 < chars.len() && chars[i + 1] == '`' {
                2
            } else {
                1
            };
            let mut end = i + fence;
            while end + fence - 1 < chars.len() {
                if chars[end..end + fence]
                    .iter()
                    .all(|candidate| *candidate == '`')
                {
                    out.extend(chars[i + fence..end].iter());
                    i = end + fence;
                    break;
                }
                end += 1;
            }
            if i != end + fence {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        if ch == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut end = i + 2;
            while end + 1 < chars.len() {
                if chars[end] == '*' && chars[end + 1] == '*' {
                    out.extend(chars[i + 2..end].iter());
                    i = end + 2;
                    break;
                }
                end += 1;
            }
            if i != end + 2 {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        if ch == '*' {
            let mut end = i + 1;
            while end < chars.len() {
                if chars[end] == '*' {
                    out.extend(chars[i + 1..end].iter());
                    i = end + 1;
                    break;
                }
                end += 1;
            }
            if i != end + 1 {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}
