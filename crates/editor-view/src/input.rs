use super::*;
use egui::{
    Event, ImeEvent, Key, Modifiers,
    text_selection::text_cursor_state::{ccursor_next_word, ccursor_previous_word},
};
use rusteditor_editor_core::{EditError, MAX_DOCUMENT_BYTES, Transaction};

impl EditorView {
    pub(super) fn handle_event(
        &mut self,
        ctx: &egui::Context,
        document: &mut Document,
        event: &Event,
        galley: &Galley,
        id: Id,
    ) -> Result<bool, EditError> {
        if let Event::Ime(ime) = event {
            match ime {
                ImeEvent::Enabled => {
                    self.ime_session_active = true;
                    self.ignore_ime_commit = false;
                }
                ImeEvent::Preedit(text) => {
                    if text == "\r" || text == "\n" {
                        return Ok(false);
                    }
                    self.ime_session_active = true;
                    if !text.is_empty() {
                        self.ignore_ime_commit = false;
                    }
                    if self.composition.is_none() && !text.is_empty() {
                        self.composition = Some(Composition {
                            document_id: document.id(),
                            revision: document.revision(),
                            range: document.selection().range(),
                            text: String::new(),
                        });
                    }
                    if let Some(c) = &mut self.composition {
                        if document.text().len() - c.range.len() + text.len() > MAX_DOCUMENT_BYTES {
                            return Err(EditError::TooLarge);
                        }
                        c.text.clone_from(text);
                    }
                }
                ImeEvent::Commit(text) => {
                    self.ime_session_active = false;
                    if self.ignore_ime_commit {
                        return Ok(false);
                    }
                    let composition = self.composition.take();
                    if !text.is_empty() && text != "\r" && text != "\n" {
                        let range = if let Some(c) = composition {
                            if c.document_id != document.id() || c.revision != document.revision() {
                                return Err(EditError::StaleRevision);
                            }
                            c.range
                        } else {
                            document.selection().range()
                        };
                        replace(document, range, text)?;
                    }
                }
                ImeEvent::Disabled => {
                    self.cancel_composition();
                }
            }
            self.h_pos = None;
            return Ok(true);
        }
        if matches!(
            event,
            Event::WindowFocused(false)
                | Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                }
        ) {
            self.cancel_composition();
            return Ok(true);
        }
        if self.is_composing() {
            return Ok(false);
        }
        normalize(document);

        if let Event::Key {
            key: key @ (Key::ArrowLeft | Key::ArrowRight),
            pressed: true,
            modifiers,
            ..
        } = event
            && !modifiers.ctrl
            && !modifiers.alt
            && !modifiers.mac_cmd
        {
            let selection = document.selection();
            let left = *key == Key::ArrowLeft;
            let head = if !modifiers.shift && selection.anchor != selection.head {
                if left {
                    selection.range().start
                } else {
                    selection.range().end
                }
            } else if left {
                grapheme::previous(document.text(), selection.head)
            } else {
                grapheme::next(document.text(), selection.head)
            };
            document.set_selection(Selection {
                anchor: if modifiers.shift {
                    selection.anchor
                } else {
                    head
                },
                head,
            })?;
            self.h_pos = None;
            return Ok(true);
        }

        let mut cursor = cursors(document.text(), document.selection(), self.h_pos);
        if cursor.on_event(ctx.os(), event, galley, id) {
            let mut selection = bytes(document.text(), cursor);
            let position = galley.pos_from_cursor(cursor.primary).center().to_vec2();
            let nearest = nearest_cursor(document.text(), galley, cursor.primary, position);
            selection.head = scalar_to_byte(document.text(), nearest.index);
            if cursor.is_empty() {
                selection.anchor = selection.head;
            }
            document.set_selection(grapheme::selection(document.text(), selection))?;
            self.h_pos = cursor.h_pos;
            return Ok(true);
        }

        match event {
            Event::Copy | Event::Cut if !document.selected_text().is_empty() => {
                ctx.copy_text(document.selected_text().to_owned());
                if matches!(event, Event::Cut) {
                    replace(document, document.selection().range(), "")?;
                }
            }
            Event::Text(text) if !text.is_empty() && text != "\r" && text != "\n" => {
                replace(document, document.selection().range(), text)?
            }
            Event::Paste(text) if !text.is_empty() => {
                replace(document, document.selection().range(), text)?
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if modifiers.matches_logically(Modifiers::COMMAND | Modifiers::SHIFT)
                    && *key == Key::Z
                    || modifiers.matches_logically(Modifiers::COMMAND) && *key == Key::Y
                {
                    document.redo()?;
                    normalize(document);
                } else if modifiers.matches_logically(Modifiers::COMMAND) && *key == Key::Z {
                    document.undo()?;
                    normalize(document);
                } else {
                    let selection = document.selection();
                    match key {
                        Key::Backspace | Key::Delete | Key::H
                            if *key != Key::H || modifiers.ctrl =>
                        {
                            if *key == Key::Delete
                                && modifiers.shift
                                && ctx.os() == egui::os::OperatingSystem::Windows
                            {
                                return Ok(false);
                            }
                            let mut range = selection.range();
                            let left = *key != Key::Delete;
                            if modifiers.mac_cmd {
                                let cursor =
                                    CCursor::new(byte_to_scalar(document.text(), selection.head)?);
                                let target = if left {
                                    galley.cursor_begin_of_paragraph(&cursor)
                                } else {
                                    galley.cursor_end_of_paragraph(&cursor)
                                };
                                let target = scalar_to_byte(document.text(), target.index);
                                range = range.start.min(target)..range.end.max(target);
                            }
                            if range.is_empty() {
                                let target = if (modifiers.ctrl || modifiers.alt) && *key != Key::H
                                {
                                    let cursor = CCursor::new(byte_to_scalar(
                                        document.text(),
                                        selection.head,
                                    )?);
                                    scalar_to_byte(
                                        document.text(),
                                        if left {
                                            ccursor_previous_word(document.text(), cursor)
                                        } else {
                                            ccursor_next_word(document.text(), cursor)
                                        }
                                        .index,
                                    )
                                } else if left {
                                    grapheme::previous(document.text(), selection.head)
                                } else {
                                    grapheme::next(document.text(), selection.head)
                                };
                                range = selection.head.min(target)..selection.head.max(target);
                            }
                            replace(
                                document,
                                grapheme::floor(document.text(), range.start)
                                    ..grapheme::ceil(document.text(), range.end),
                                "",
                            )?;
                        }
                        Key::Enter if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
                            replace(document, selection.range(), "\n")?
                        }
                        Key::Tab if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
                            if !modifiers.shift {
                                replace(document, selection.range(), "\t")?;
                            } else if selection.range().is_empty()
                                && document.text()[..selection.head].ends_with('\t')
                            {
                                replace(document, selection.head - 1..selection.head, "")?;
                            }
                        }
                        _ => return Ok(false),
                    }
                }
            }
            _ => return Ok(false),
        }
        self.h_pos = None;
        Ok(true)
    }
}

/// Validate the entire result and its grapheme caret before one core transaction.
/// A rejected paste never deletes the original selection or advances history.
fn replace(
    document: &mut Document,
    range: Range<usize>,
    replacement: &str,
) -> Result<(), EditError> {
    let len = document.text().len() - range.len();
    if len.saturating_add(replacement.len()) > MAX_DOCUMENT_BYTES {
        return Err(EditError::TooLarge);
    }
    let mut result = String::with_capacity(len + replacement.len());
    result.push_str(&document.text()[..range.start]);
    result.push_str(replacement);
    result.push_str(&document.text()[range.end..]);
    let head = grapheme::ceil(&result, range.start + replacement.len());
    document.apply(Transaction {
        document_id: document.id(),
        expected_revision: document.revision(),
        range,
        replacement: replacement.to_owned(),
        selection_after: Selection { anchor: head, head },
    })?;
    Ok(())
}
