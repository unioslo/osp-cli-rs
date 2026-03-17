use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionTitleStyle {
    Plain,
    Ruled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FooterRuleStyle {
    None,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedTitle {
    pub prefix: String,
    pub title: String,
    pub suffix: String,
}

impl RenderedTitle {
    pub(crate) fn plain_text(&self) -> String {
        format!("{}{}{}", self.prefix, self.title, self.suffix)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectionChrome {
    title: SectionTitleStyle,
    footer: FooterRuleStyle,
}

pub(crate) const PLAIN_SECTION_CHROME: SectionChrome =
    SectionChrome::new(SectionTitleStyle::Plain, FooterRuleStyle::None);
pub(crate) const GUIDE_SECTION_CHROME: SectionChrome =
    SectionChrome::new(SectionTitleStyle::Ruled, FooterRuleStyle::None);
pub(crate) const FULL_HELP_LAYOUT_CHROME: SectionChrome =
    SectionChrome::new(SectionTitleStyle::Ruled, FooterRuleStyle::Rule);

impl SectionChrome {
    const fn new(title: SectionTitleStyle, footer: FooterRuleStyle) -> Self {
        Self { title, footer }
    }

    pub(crate) fn render_title_line(
        self,
        title: &str,
        width: Option<usize>,
        unicode: bool,
    ) -> RenderedTitle {
        match self.title {
            SectionTitleStyle::Plain => plain_section_title(title),
            SectionTitleStyle::Ruled => ruled_section_title(title, width, unicode),
        }
    }

    pub(crate) fn render_title(self, title: &str, width: Option<usize>, unicode: bool) -> String {
        self.render_title_line(title, width, unicode).plain_text()
    }

    pub(crate) fn render_footer_rule(self, width: Option<usize>, unicode: bool) -> Option<String> {
        match self.footer {
            FooterRuleStyle::None => None,
            FooterRuleStyle::Rule => ruled_line(width, unicode),
        }
    }
}

fn plain_section_title(title: &str) -> RenderedTitle {
    RenderedTitle {
        prefix: String::new(),
        title: title.trim_end_matches(':').to_string(),
        suffix: ":".to_string(),
    }
}

fn ruled_line(width: Option<usize>, unicode: bool) -> Option<String> {
    let fill = if unicode { '─' } else { '-' };
    width
        .filter(|width| *width > 0)
        .map(|width| fill.to_string().repeat(width))
}

fn ruled_section_title(title: &str, width: Option<usize>, unicode: bool) -> RenderedTitle {
    let title = title.trim_end_matches(':').to_string();
    let fill = if unicode { '─' } else { '-' };
    let prefix = format!("{fill} ");
    let target_width = match width {
        Some(width) => width.max(12),
        None => 24,
    };
    let used = UnicodeWidthStr::width(prefix.as_str()) + UnicodeWidthStr::width(title.as_str()) + 1;
    if used >= target_width {
        return RenderedTitle {
            prefix,
            title,
            suffix: String::new(),
        };
    }
    RenderedTitle {
        prefix,
        title,
        suffix: format!(" {}", fill.to_string().repeat(target_width - used)),
    }
}

#[cfg(test)]
mod tests {
    use super::{FULL_HELP_LAYOUT_CHROME, GUIDE_SECTION_CHROME, PLAIN_SECTION_CHROME};

    #[test]
    fn plain_section_titles_add_trailing_colon_unit() {
        assert_eq!(
            PLAIN_SECTION_CHROME.render_title("Options", None, false),
            "Options:"
        );
        assert_eq!(
            PLAIN_SECTION_CHROME.render_title("Options:", None, true),
            "Options:"
        );
    }

    #[test]
    fn guide_rule_titles_respect_target_width_unit() {
        assert_eq!(
            GUIDE_SECTION_CHROME.render_title("Usage", Some(12), false),
            "- Usage ----"
        );
        assert_eq!(
            FULL_HELP_LAYOUT_CHROME.render_title("Usage", Some(12), false),
            "- Usage ----"
        );
        assert_eq!(
            GUIDE_SECTION_CHROME.render_title("Usage", Some(12), true),
            "─ Usage ────"
        );
    }

    #[test]
    fn footer_rule_requires_positive_width_and_explicit_profile_unit() {
        assert_eq!(
            GUIDE_SECTION_CHROME.render_footer_rule(Some(4), false),
            None
        );
        assert_eq!(
            FULL_HELP_LAYOUT_CHROME.render_footer_rule(None, false),
            None
        );
        assert_eq!(
            FULL_HELP_LAYOUT_CHROME.render_footer_rule(Some(0), false),
            None
        );
        assert_eq!(
            FULL_HELP_LAYOUT_CHROME.render_footer_rule(Some(4), false),
            Some("----".to_string())
        );
        assert_eq!(
            FULL_HELP_LAYOUT_CHROME.render_footer_rule(Some(4), true),
            Some("────".to_string())
        );
    }
}
