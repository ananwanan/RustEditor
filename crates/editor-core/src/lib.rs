//! Bounded, GUI-independent M0 document model. Public offsets are UTF-8 bytes.
#![forbid(unsafe_code)]

use std::{collections::VecDeque, fmt, ops::Range};

pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_HISTORY_BYTES: usize = 8 * 1024 * 1024;
const MAX_HISTORY_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);

/// Direction is preserved; both endpoints must be UTF-8 character boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub document_id: DocumentId,
    pub expected_revision: Revision,
    pub range: Range<usize>,
    pub replacement: String,
    pub selection_after: Selection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    WrongDocument,
    StaleRevision,
    InvalidBoundary,
    TooLarge,
    RevisionExhausted,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EditError {}

#[derive(Clone, Debug)]
struct Change {
    start: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
}

impl Change {
    fn bytes(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }
}

#[derive(Debug)]
pub struct Document {
    id: DocumentId,
    revision: Revision,
    text: String,
    selection: Selection,
    undo: VecDeque<Change>,
    redo: Vec<Change>,
    history_bytes: usize,
}

impl Document {
    pub fn new(id: DocumentId, text: impl Into<String>) -> Result<Self, EditError> {
        let text = text.into();
        if id.0 == 0 {
            return Err(EditError::WrongDocument);
        }
        if text.len() > MAX_DOCUMENT_BYTES {
            return Err(EditError::TooLarge);
        }
        Ok(Self {
            id,
            revision: Revision(1),
            text,
            selection: Selection::default(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            history_bytes: 0,
        })
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn selection(&self) -> Selection {
        self.selection
    }
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.range()]
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn set_selection(&mut self, selection: Selection) -> Result<(), EditError> {
        validate_selection(&self.text, selection)?;
        self.selection = selection;
        Ok(())
    }

    /// Validates the complete result before changing any authoritative state.
    pub fn apply(&mut self, transaction: Transaction) -> Result<Revision, EditError> {
        if transaction.document_id != self.id {
            return Err(EditError::WrongDocument);
        }
        if transaction.expected_revision != self.revision {
            return Err(EditError::StaleRevision);
        }
        let range = transaction.range;
        if range.start > range.end
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return Err(EditError::InvalidBoundary);
        }
        let result_len = (self.text.len() - range.len())
            .checked_add(transaction.replacement.len())
            .filter(|len| *len <= MAX_DOCUMENT_BYTES)
            .ok_or(EditError::TooLarge)?;
        // M0 intentionally uses a bounded String. M1 may replace the storage independently.
        let mut result = String::with_capacity(result_len);
        result.push_str(&self.text[..range.start]);
        result.push_str(&transaction.replacement);
        result.push_str(&self.text[range.end..]);
        validate_selection(&result, transaction.selection_after)?;
        if result == self.text {
            self.selection = transaction.selection_after;
            return Ok(self.revision);
        }
        let next_revision = self.next_revision()?;
        let change = Change {
            start: range.start,
            removed: self.text[range].to_owned(),
            inserted: transaction.replacement,
            before: self.selection,
            after: transaction.selection_after,
        };
        self.history_bytes -= self.redo.iter().map(Change::bytes).sum::<usize>();
        self.redo.clear();
        self.history_bytes += change.bytes();
        self.undo.push_back(change);
        while self.history_bytes > MAX_HISTORY_BYTES || self.undo.len() > MAX_HISTORY_ENTRIES {
            if let Some(old) = self.undo.pop_front() {
                self.history_bytes -= old.bytes();
            }
        }
        self.text = result;
        self.selection = transaction.selection_after;
        self.revision = next_revision;
        Ok(self.revision)
    }

    pub fn replace(
        &mut self,
        range: Range<usize>,
        replacement: &str,
    ) -> Result<Revision, EditError> {
        let end = range
            .start
            .checked_add(replacement.len())
            .ok_or(EditError::TooLarge)?;
        self.apply(Transaction {
            document_id: self.id,
            expected_revision: self.revision,
            range,
            replacement: replacement.to_owned(),
            selection_after: Selection {
                anchor: end,
                head: end,
            },
        })
    }

    /// Commit a bounded, temporary GUI draft as one minimal replacement.
    pub fn commit_draft(
        &mut self,
        draft: &str,
        selection: Selection,
    ) -> Result<Revision, EditError> {
        if draft.len() > MAX_DOCUMENT_BYTES {
            return Err(EditError::TooLarge);
        }
        let mut prefix = self
            .text
            .bytes()
            .zip(draft.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(prefix) || !draft.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let mut suffix = self.text[prefix..]
            .bytes()
            .rev()
            .zip(draft[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(self.text.len() - suffix)
            || !draft.is_char_boundary(draft.len() - suffix)
        {
            suffix -= 1;
        }
        self.apply(Transaction {
            document_id: self.id,
            expected_revision: self.revision,
            range: prefix..self.text.len() - suffix,
            replacement: draft[prefix..draft.len() - suffix].to_owned(),
            selection_after: selection,
        })
    }

    pub fn undo(&mut self) -> Result<bool, EditError> {
        if self.undo.is_empty() {
            return Ok(false);
        }
        let revision = self.next_revision()?;
        let change = self.undo.pop_back().expect("checked nonempty history");
        self.text.replace_range(
            change.start..change.start + change.inserted.len(),
            &change.removed,
        );
        self.selection = change.before;
        self.redo.push(change);
        self.revision = revision;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool, EditError> {
        if self.redo.is_empty() {
            return Ok(false);
        }
        let revision = self.next_revision()?;
        let change = self.redo.pop().expect("checked nonempty history");
        self.text.replace_range(
            change.start..change.start + change.removed.len(),
            &change.inserted,
        );
        self.selection = change.after;
        self.undo.push_back(change);
        self.revision = revision;
        Ok(true)
    }

    fn next_revision(&self) -> Result<Revision, EditError> {
        self.revision
            .0
            .checked_add(1)
            .map(Revision)
            .ok_or(EditError::RevisionExhausted)
    }
}

fn validate_selection(text: &str, selection: Selection) -> Result<(), EditError> {
    if text.is_char_boundary(selection.anchor) && text.is_char_boundary(selection.head) {
        Ok(())
    } else {
        Err(EditError::InvalidBoundary)
    }
}

/// GUI scalar indices are distinct from the byte offsets used by transactions.
pub fn scalar_to_byte(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(byte, _)| byte)
}

pub fn byte_to_scalar(text: &str, byte: usize) -> Result<usize, EditError> {
    if !text.is_char_boundary(byte) {
        return Err(EditError::InvalidBoundary);
    }
    Ok(text[..byte].chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_edit_undo_and_redo_keep_monotonic_revisions_and_selection() {
        let mut doc = Document::new(DocumentId(1), "你好 ß👩‍💻 e\u{301}\r\n").unwrap();
        let original = doc.text().to_owned();
        doc.set_selection(Selection { anchor: 9, head: 7 }).unwrap();
        doc.replace(7..9, "SS").unwrap();
        assert!(doc.text().contains("SS👩‍💻"));
        assert_eq!(doc.revision(), Revision(2));
        assert!(doc.undo().unwrap());
        assert_eq!(doc.text(), original);
        assert_eq!(doc.selection(), Selection { anchor: 9, head: 7 });
        assert!(doc.redo().unwrap());
        assert_eq!(doc.revision(), Revision(4));
        assert_eq!(doc.selection(), Selection { anchor: 9, head: 9 });
    }

    #[test]
    fn invalid_edits_are_atomic_and_stale_edits_cannot_apply_after_undo() {
        let mut doc = Document::new(DocumentId(7), "中🙂").unwrap();
        assert_eq!(doc.replace(1..3, "x"), Err(EditError::InvalidBoundary));
        assert_eq!(doc.replace(9..10, "x"), Err(EditError::InvalidBoundary));
        let transaction = Transaction {
            document_id: doc.id(),
            expected_revision: doc.revision(),
            range: 0..3,
            replacement: "a".into(),
            selection_after: Selection::default(),
        };
        let mut wrong_doc = transaction.clone();
        wrong_doc.document_id = DocumentId(8);
        assert_eq!(doc.apply(wrong_doc), Err(EditError::WrongDocument));
        let mut bad_selection = transaction.clone();
        bad_selection.selection_after = Selection { anchor: 2, head: 2 };
        assert_eq!(doc.apply(bad_selection), Err(EditError::InvalidBoundary));
        assert_eq!(doc.text(), "中🙂");
        assert!(!doc.can_undo());
        doc.apply(transaction.clone()).unwrap();
        doc.undo().unwrap();
        assert_eq!(doc.apply(transaction), Err(EditError::StaleRevision));
        doc.replace(0..0, "x").unwrap();
        assert!(!doc.can_redo());
    }

    #[test]
    fn limits_and_revision_overflow_preserve_the_document() {
        let mut doc = Document::new(DocumentId(1), "a").unwrap();
        assert_eq!(
            doc.replace(0..0, &"a".repeat(MAX_DOCUMENT_BYTES)),
            Err(EditError::TooLarge)
        );
        doc.revision = Revision(u64::MAX);
        assert_eq!(doc.replace(0..1, "b"), Err(EditError::RevisionExhausted));
        assert_eq!(doc.text(), "a");
    }

    #[test]
    fn draft_diff_never_splits_multibyte_prefixes_or_suffixes() {
        for (before, after) in [
            ("🙂中🙂", "🙃中🙃"),
            ("a中", "a丰"),
            ("", "你好"),
            ("a", ""),
            ("same", "same"),
        ] {
            let mut doc = Document::new(DocumentId(1), before).unwrap();
            doc.commit_draft(after, Selection::default()).unwrap();
            assert_eq!(doc.text(), after);
            if before != after {
                assert_eq!(doc.revision(), Revision(2));
                doc.undo().unwrap();
                assert_eq!(doc.text(), before);
                assert!(!doc.can_undo());
            }
        }
    }

    #[test]
    fn randomized_operations_match_a_reference_string_and_history() {
        let mut doc = Document::new(DocumentId(1), "").unwrap();
        let mut reference = String::new();
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        let mut seed = 19_u64;
        for _ in 0..2000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            match seed % 5 {
                0 if !undo.is_empty() => {
                    redo.push(reference.clone());
                    reference = undo.pop().unwrap();
                    doc.undo().unwrap();
                }
                1 if !redo.is_empty() => {
                    undo.push(reference.clone());
                    reference = redo.pop().unwrap();
                    doc.redo().unwrap();
                }
                _ => {
                    let bounds: Vec<_> = reference
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain(std::iter::once(reference.len()))
                        .collect();
                    let a = bounds[(seed as usize >> 8) % bounds.len()];
                    let b = bounds[(seed as usize >> 24) % bounds.len()];
                    let range = a.min(b)..a.max(b);
                    let replacement =
                        ["中", "🙂", "e\u{301}", "", "\r\n", "ß"][(seed as usize >> 40) % 6];
                    let mut next = reference.clone();
                    next.replace_range(range.clone(), replacement);
                    if next != reference {
                        undo.push(reference);
                        redo.clear();
                        if undo.len() > MAX_HISTORY_ENTRIES {
                            undo.remove(0);
                        }
                    }
                    reference = next;
                    doc.replace(range, replacement).unwrap();
                }
            }
            assert_eq!(doc.text(), reference);
            assert_eq!(doc.can_undo(), !undo.is_empty());
            assert_eq!(doc.can_redo(), !redo.is_empty());
            validate_selection(doc.text(), doc.selection()).unwrap();
        }
    }
}
