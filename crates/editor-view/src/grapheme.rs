//! Extended grapheme boundaries, always calculated with the full text context.
use rusteditor_editor_core::Selection;
use unicode_segmentation::GraphemeCursor;

pub(crate) fn floor(text: &str, byte: usize) -> usize {
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    if cursor.is_boundary(text, 0).expect("complete text chunk") {
        byte
    } else {
        cursor
            .prev_boundary(text, 0)
            .expect("complete text chunk")
            .unwrap_or(0)
    }
}

pub(crate) fn ceil(text: &str, byte: usize) -> usize {
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    if cursor.is_boundary(text, 0).expect("complete text chunk") {
        byte
    } else {
        cursor
            .next_boundary(text, 0)
            .expect("complete text chunk")
            .unwrap_or(text.len())
    }
}

pub(crate) fn previous(text: &str, byte: usize) -> usize {
    GraphemeCursor::new(byte, text.len(), true)
        .prev_boundary(text, 0)
        .expect("complete text chunk")
        .unwrap_or(0)
}

pub(crate) fn next(text: &str, byte: usize) -> usize {
    GraphemeCursor::new(byte, text.len(), true)
        .next_boundary(text, 0)
        .expect("complete text chunk")
        .unwrap_or(text.len())
}

/// A selection expands outwards; a caret advances past a newly joined cluster.
/// Plugin transactions may still address any valid UTF-8 scalar boundary.
pub(crate) fn selection(text: &str, selection: Selection) -> Selection {
    let range = selection.range();
    if range.is_empty() {
        let head = ceil(text, selection.head);
        Selection { anchor: head, head }
    } else if selection.anchor < selection.head {
        Selection {
            anchor: floor(text, selection.anchor),
            head: ceil(text, selection.head),
        }
    } else {
        Selection {
            anchor: ceil(text, selection.anchor),
            head: floor(text, selection.head),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn boundaries_match_forward_segmentation_at_every_scalar_in_context() {
        // Regional indicators need context before the current cluster; slicing
        // the suffix before segmentation would incorrectly pair these flags.
        let text = "中e\u{301}👩🏽‍💻🇨🇳🇺🇸🇫🇷1️⃣\r\nक्‍ष한";
        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        for byte in text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
        {
            assert_eq!(
                floor(text, byte),
                *boundaries
                    .iter()
                    .filter(|&&i| i <= byte)
                    .next_back()
                    .unwrap()
            );
            assert_eq!(
                ceil(text, byte),
                *boundaries.iter().find(|&&i| i >= byte).unwrap()
            );
            assert_eq!(
                previous(text, byte),
                boundaries
                    .iter()
                    .copied()
                    .filter(|&i| i < byte)
                    .next_back()
                    .unwrap_or(0)
            );
            assert_eq!(
                next(text, byte),
                boundaries
                    .iter()
                    .copied()
                    .find(|&i| i > byte)
                    .unwrap_or(text.len())
            );
        }
        assert_eq!(next("", 0), 0);
        assert_eq!(previous("", 0), 0);
    }
}
