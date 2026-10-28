//! A bounded core-driven egui editor. The core owns text, selection and history;
//! only uncommitted IME preedit is retained as a presentation overlay.
#![forbid(unsafe_code)]

mod grapheme;
mod input;

use egui::{
    Color32, EventFilter, FontId, Galley, Id, Response, Sense, Ui,
    text::{CCursor, CCursorRange},
    text_selection::{TextCursorState, text_cursor_state::cursor_rect, visuals},
};
use rusteditor_editor_core::{
    Document, DocumentId, Revision, Selection, byte_to_scalar, scalar_to_byte,
};
use std::{ops::Range, sync::Arc};

struct Composition {
    document_id: DocumentId,
    revision: Revision,
    range: Range<usize>,
    text: String,
}

/// View state contains no independently editable document or undo history.
pub struct EditorView {
    composition: Option<Composition>,
    ignore_ime_commit: bool,
    h_pos: Option<f32>,
    focus_requested: bool,
    last_interaction: f64,
}

impl Default for EditorView {
    fn default() -> Self {
        Self {
            composition: None,
            ignore_ime_commit: false,
            h_pos: None,
            focus_requested: true,
            last_interaction: 0.0,
        }
    }
}

pub struct ViewOutput {
    pub response: Response,
    pub error: Option<String>,
}

impl EditorView {
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
        self.h_pos = None;
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// Suppress application commands throughout a composition frame, including
    /// its commit, so shortcuts cannot race the platform's IME events.
    pub fn blocks_shortcuts(&self, ctx: &egui::Context) -> bool {
        self.is_composing()
            || ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Ime(egui::ImeEvent::Preedit(_) | egui::ImeEvent::Commit(_))
                    )
                })
            })
    }

    pub fn show(&mut self, ui: &mut Ui, id: Id, document: &mut Document) -> ViewOutput {
        let mut error = None;
        let initial_revision = document.revision();
        if self
            .composition
            .as_ref()
            .is_some_and(|c| c.document_id != document.id() || c.revision != document.revision())
        {
            self.composition = None;
            self.ignore_ime_commit = true;
        }
        normalize(document);
        let font = FontId::monospace(16.0);
        let color = ui.visuals().text_color();
        let mut galley = self.layout(ui.ctx(), document, &font, color);
        let size = (galley.size() + egui::vec2(12.0, 12.0)).max(ui.available_size());
        let (_, rect) = ui.allocate_space(size);
        // Assign a stable focus ID separately from the allocation's automatic ID.
        let mut response = ui
            .interact(rect, id, Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Text);
        if self.focus_requested && ui.is_enabled() {
            response.request_focus();
            self.focus_requested = false;
        }
        let pos = rect.min + egui::vec2(6.0, 6.0);
        let mut interacted = false;
        if ui.is_enabled()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if self.composition.is_some() && ui.input(|i| i.pointer.any_pressed()) {
                self.composition = None;
                self.ignore_ime_commit = true;
                galley = self.layout(ui.ctx(), document, &font, color);
            }
            if !self.is_composing() {
                let scalar = galley.cursor_from_pos(pointer - pos);
                let pointer_cursor =
                    nearest_cursor(document.text(), &galley, scalar, pointer - pos);
                let mut state = TextCursorState::from(cursors(
                    document.text(),
                    document.selection(),
                    self.h_pos,
                ));
                if state.pointer_interaction(
                    ui,
                    &response,
                    pointer_cursor,
                    &galley,
                    response.dragged(),
                ) {
                    let range = state
                        .char_range()
                        .expect("pointer interaction sets a selection");
                    let selection =
                        grapheme::selection(document.text(), bytes(document.text(), range));
                    document
                        .set_selection(selection)
                        .expect("valid view selection");
                    self.h_pos = None;
                    response.request_focus();
                    interacted = true;
                }
            }
        }

        let filter = EventFilter {
            horizontal_arrows: true,
            vertical_arrows: true,
            tab: true,
            ..Default::default()
        };
        let focused = response.has_focus() && ui.is_enabled() && ui.input(|i| i.focused);
        if focused {
            ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
            let events = ui.input(|i| i.filtered_events(&filter));
            let mut ime_owns_input = self.is_composing();
            for event in events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Disabled) => ime_owns_input = false,
                    egui::Event::Ime(_) => ime_owns_input = true,
                    _ => {}
                }
                // Native key/text companions must not duplicate IME commit or
                // edit the original selection while the platform owns preedit.
                // egui-winit emits Disabled immediately after Commit. Preserve
                // ordinary input before Enabled and after Disabled in this frame.
                if ime_owns_input
                    && !matches!(
                        event,
                        egui::Event::Ime(_)
                            | egui::Event::WindowFocused(_)
                            | egui::Event::Key {
                                key: egui::Key::Escape,
                                pressed: true,
                                ..
                            }
                    )
                {
                    continue;
                }
                match self.handle_event(ui.ctx(), document, &event, &galley, id) {
                    Ok(changed) => interacted |= changed,
                    Err(e) => error = Some(e.to_string()),
                }
                // The next event in the same frame sees the current text and
                // cursor, including after a paste, deletion, undo or IME commit.
                galley = self.layout(ui.ctx(), document, &font, color);
            }
        } else {
            self.ignore_ime_commit |= self.composition.take().is_some();
            galley = self.layout(ui.ctx(), document, &font, color);
        }

        let displayed_selection = self.display_selection(document);
        let range = cursors(galley.text(), displayed_selection, self.h_pos);
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let caret = cursor_rect(&galley, &range.primary, row_height).translate(pos.to_vec2());
        let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        visuals::paint_text_selection(&mut galley, ui.visuals(), &range, None);
        painter.galley(pos, galley, color);
        if focused {
            if interacted {
                self.last_interaction = ui.input(|i| i.time);
                ui.scroll_to_rect(caret.expand(6.0), None);
            }
            visuals::paint_text_cursor(
                ui,
                &painter,
                caret,
                ui.input(|i| i.time) - self.last_interaction,
            );
            let transform = ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .unwrap_or_default();
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect: transform * rect,
                    cursor_rect: transform * caret,
                });
                o.mutable_text_under_cursor = response.hovered();
            });
        }
        if document.revision() != initial_revision {
            response.mark_changed();
        }
        ViewOutput { response, error }
    }

    fn layout(
        &self,
        ctx: &egui::Context,
        document: &Document,
        font: &FontId,
        color: Color32,
    ) -> Arc<Galley> {
        let text = if let Some(c) = &self.composition
            && !c.text.is_empty()
        {
            let mut text =
                String::with_capacity(document.text().len() - c.range.len() + c.text.len());
            text.push_str(&document.text()[..c.range.start]);
            text.push_str(&c.text);
            text.push_str(&document.text()[c.range.end..]);
            text
        } else {
            document.text().to_owned()
        };
        ctx.fonts_mut(|f| f.layout_no_wrap(text, font.clone(), color))
    }

    fn display_selection(&self, document: &Document) -> Selection {
        if let Some(c) = &self.composition
            && !c.text.is_empty()
        {
            Selection {
                anchor: c.range.start,
                head: c.range.start + c.text.len(),
            }
        } else {
            document.selection()
        }
    }
}

fn cursors(text: &str, selection: Selection, h_pos: Option<f32>) -> CCursorRange {
    let mut cursors = CCursorRange::two(
        CCursor::new(byte_to_scalar(text, selection.anchor).expect("valid core selection")),
        CCursor::new(byte_to_scalar(text, selection.head).expect("valid core selection")),
    );
    cursors.h_pos = h_pos;
    cursors
}

fn bytes(text: &str, cursors: CCursorRange) -> Selection {
    Selection {
        anchor: scalar_to_byte(text, cursors.secondary.index),
        head: scalar_to_byte(text, cursors.primary.index),
    }
}

fn normalize(document: &mut Document) {
    let selection = grapheme::selection(document.text(), document.selection());
    document
        .set_selection(selection)
        .expect("valid grapheme selection");
}

fn nearest_cursor(text: &str, galley: &Galley, cursor: CCursor, pos: egui::Vec2) -> CCursor {
    let byte = scalar_to_byte(text, cursor.index);
    let lower =
        CCursor::new(byte_to_scalar(text, grapheme::floor(text, byte)).expect("grapheme boundary"));
    let upper =
        CCursor::new(byte_to_scalar(text, grapheme::ceil(text, byte)).expect("grapheme boundary"));
    let distance = |c| {
        galley
            .pos_from_cursor(c)
            .center()
            .distance_sq(pos.to_pos2())
    };
    if distance(lower) < distance(upper) {
        lower
    } else {
        upper
    }
}

#[cfg(test)]
mod tests;
