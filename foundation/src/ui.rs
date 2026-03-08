//! Rendering, layout, document, and message formatting surfaces.

pub mod chrome {
    pub use crate::osp_ui::chrome::*;
}

pub mod clipboard {
    pub use crate::osp_ui::clipboard::*;
}

pub mod document {
    pub use crate::osp_ui::document::*;
}

pub mod format {
    pub use crate::osp_ui::format::{
        MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules,
        build_help_document,
    };
}

pub mod interactive {
    pub use crate::osp_ui::interactive::*;
}

pub mod messages {
    pub use crate::osp_ui::messages::*;
}

pub mod style {
    pub use crate::osp_ui::style::*;
}

pub mod theme {
    pub use crate::osp_ui::theme::*;
}

pub use crate::osp_ui::{
    CodeBlock, Document, Interactive, InteractiveResult, InteractiveRuntime, JsonBlock,
    LineBlock, LinePart, MregBlock, MregEntry, MregRow, MregValue, PanelBlock,
    PanelRules, RenderBackend, RenderRuntime, RenderSettings,
    ResolvedRenderSettings, Spinner, StyleOverrides, TableAlign, TableBlock,
    TableBorderStyle, TableOverflow, TableStyle, ValueBlock, copy_output_to_clipboard,
    copy_rows_to_clipboard, line_from_inline, parts_from_inline, render_document,
    render_document_for_copy, render_inline, render_output, render_output_for_copy,
    render_rows, render_rows_for_copy,
};
