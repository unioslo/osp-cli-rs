//! Message sink traits and ergonomic helpers.

use crate::ui::messages::{MessageBuffer, MessageLevel, UiMessage};

/// Object-safe sink for user-facing messages.
pub trait MessageSink {
    /// Pushes one message into the sink.
    fn push_message(&mut self, message: UiMessage);
}

impl MessageSink for MessageBuffer {
    fn push_message(&mut self, message: UiMessage) {
        MessageBuffer::push_message(self, message);
    }
}

/// Convenience methods layered over [`MessageSink`].
#[cfg(test)]
pub trait MessageSinkExt: MessageSink {
    /// Pushes a message by level and raw text.
    fn msg(&mut self, level: MessageLevel, text: impl Into<String>) {
        self.push_message(UiMessage::new(level, text));
    }

    /// Pushes a warning message.
    fn warning_msg(&mut self, text: impl Into<String>) {
        self.msg(MessageLevel::Warning, text);
    }

    /// Pushes an informational message.
    fn info_msg(&mut self, text: impl Into<String>) {
        self.msg(MessageLevel::Info, text);
    }
}

#[cfg(test)]
impl<T: MessageSink + ?Sized> MessageSinkExt for T {}

#[cfg(test)]
mod tests {
    use super::MessageSinkExt;
    use crate::ui::messages::{MessageBuffer, MessageLevel};

    #[test]
    fn sink_extension_methods_write_into_message_buffer() {
        let mut buffer = MessageBuffer::default();
        buffer.info_msg("hello");
        buffer.warning_msg("careful");

        assert_eq!(buffer.entries().len(), 2);
        assert_eq!(buffer.entries()[0].level, MessageLevel::Info);
        assert_eq!(buffer.entries()[1].text, "careful");
    }
}
