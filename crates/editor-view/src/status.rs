//! Read-only presentation metrics. Columns count graphemes, not bytes or glyphs.
use rusteditor_editor_core::Document;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentStatus {
    pub lines: usize,
    pub line: usize,
    pub column: usize,
    pub line_endings: &'static str,
    pub selection_bytes: usize,
    pub selection_graphemes: usize,
}

impl DocumentStatus {
    pub(crate) fn new(document: &Document) -> Self {
        let text = document.text();
        let mut lf = 0;
        let mut crlf = 0;
        let mut cr = 0;
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                if index > 0 && text.as_bytes()[index - 1] == b'\r' {
                    crlf += 1;
                } else {
                    lf += 1;
                }
            } else if byte == b'\r' && text.as_bytes().get(index + 1) != Some(&b'\n') {
                cr += 1;
            }
        }
        let prefix = &text[..document.selection().head];
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        Self {
            lines: lf + crlf + 1,
            line: prefix.bytes().filter(|&b| b == b'\n').count() + 1,
            column: prefix[line_start..].graphemes(true).count() + 1,
            line_endings: match (lf > 0, crlf > 0, cr > 0) {
                (false, false, false) => "None",
                (true, false, false) => "LF",
                (false, true, false) => "CRLF",
                (false, false, true) => "CR",
                _ => "Mixed",
            },
            selection_bytes: document.selected_text().len(),
            selection_graphemes: document.selected_text().graphemes(true).count(),
        }
    }
}
