//! A bounded core-driven egui editor. The core owns text, selection and history;
//! only uncommitted IME preedit is retained as a presentation overlay.
#![forbid(unsafe_code)]

mod grapheme;
mod gutter;
mod input;
mod status;
pub use status::DocumentStatus;

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
    ime_session_active: bool,
    ignore_ime_commit: bool,
    h_pos: Option<f32>,
    focus_requested: bool,
    last_interaction: f64,
    status_cache: Option<(DocumentId, Revision, Selection, DocumentStatus)>,
}

impl Default for EditorView {
    fn default() -> Self {
        Self {
            composition: None,
            ime_session_active: false,
            ignore_ime_commit: false,
            h_pos: None,
            focus_requested: true,
            last_interaction: 0.0,
            status_cache: None,
        }
    }
}

pub struct ViewOutput {
    pub response: Response,
    pub error: Option<String>,
}

impl EditorView {
    /// Cached by identity/revision/selection; never retains editable text.
    pub fn status(&mut self, document: &Document) -> DocumentStatus {
        let key = (document.id(), document.revision(), document.selection());
        if let Some((id, revision, selection, status)) = self.status_cache
            && (id, revision, selection) == key
        {
            return status;
        }
        let status = DocumentStatus::new(document);
        self.status_cache = Some((key.0, key.1, key.2, status));
        status
    }

    fn cancel_composition(&mut self) {
        // A platform may deliver a bare Commit without Enabled or Preedit.
        // Ignore delayed commits only when an observed session was cancelled;
        // ordinary focus changes must not disable that native input path.
        self.ignore_ime_commit |= self.ime_session_active || self.composition.is_some();
        self.ime_session_active = false;
        self.composition = None;
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
        self.h_pos = None;
    }

    /// End a tab's native IME session. Both departing and arriving views must
    /// reject delayed commits until a fresh Enabled or nonempty Preedit event.
    pub fn reset_ime(&mut self) {
        self.cancel_composition();
        self.ignore_ime_commit = true;
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// Suppress application commands throughout a composition frame, including
    /// its commit, so shortcuts cannot race the platform's IME events.
    pub fn blocks_shortcuts(&self, ctx: &egui::Context) -> bool {
        self.ime_session_active
            || self.is_composing()
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
            self.cancel_composition();
        }
        normalize(document);
        let font = FontId::monospace(16.0);
        let color = ui.visuals().text_color();
        let mut galley = self.layout(ui.ctx(), document, &font, color);
        let gutter_width = gutter::width(ui, &galley, &font);
        let size = (galley.size() + egui::vec2(gutter_width + 12.0, 12.0)).max(ui.available_size());
        let (_, rect) = ui.allocate_space(size);
        // Assign a stable focus ID separately from the allocation's automatic ID.
        let mut response = ui
            .interact(rect, id, Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Text);
        if self.focus_requested && ui.is_enabled() {
            response.request_focus();
            self.focus_requested = false;
        }
        let pos = rect.min + egui::vec2(gutter_width + 6.0, 6.0);
        // Keep line numbers fixed when horizontally scrolling the text.
        // ScrollArea expands its clip by this margin. Anchor at the viewport's
        // content edge so the gutter does not jump when the offset leaves zero.
        let gutter_left = rect
            .left()
            .max(ui.clip_rect().left() + ui.visuals().clip_rect_margin);
        let gutter_right = gutter_left + gutter_width;
        let mut interacted = false;
        if ui.is_enabled()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if (self.ime_session_active || self.composition.is_some())
                && ui.input(|i| i.pointer.any_pressed())
            {
                self.cancel_composition();
                galley = self.layout(ui.ctx(), document, &font, color);
            }
            if !self.is_composing() {
                let mut relative = pointer - pos;
                if pointer.x < gutter_right {
                    relative.x = 0.0;
                }
                let scalar = galley.cursor_from_pos(relative);
                let pointer_cursor = nearest_cursor(document.text(), &galley, scalar, relative);
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
            let mut events = ui
                .input(|i| i.filtered_events(&filter))
                .into_iter()
                .peekable();
            let mut ime_owns_input = self.ime_session_active || self.is_composing();
            while let Some(event) = events.next() {
                // Windows clears preedit immediately before delivering its
                // result. egui-winit maps that clear to Disabled, so an adjacent
                // Disabled/Commit pair is a commit boundary, not cancellation.
                // Keep the original composition range until Commit consumes it.
                if matches!(event, egui::Event::Ime(egui::ImeEvent::Disabled))
                    && matches!(
                        events.peek(),
                        Some(egui::Event::Ime(egui::ImeEvent::Commit(_)))
                    )
                {
                    continue;
                }
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
            self.cancel_composition();
            galley = self.layout(ui.ctx(), document, &font, color);
        }

        // Input can change the digit count in this very frame.
        let gutter_width = gutter::width(ui, &galley, &font);
        let pos = rect.min + egui::vec2(gutter_width + 6.0, 6.0);
        let gutter_rect = egui::Rect::from_min_max(
            egui::pos2(gutter_left, ui.clip_rect().top().max(rect.top())),
            egui::pos2(
                gutter_left + gutter_width,
                ui.clip_rect().bottom().min(rect.bottom()),
            ),
        );
        let text_clip = egui::Rect::from_min_max(
            egui::pos2(gutter_rect.right(), ui.clip_rect().top()),
            ui.clip_rect().max,
        )
        .intersect(rect);
        let displayed_selection = self.display_selection(document);
        let range = cursors(galley.text(), displayed_selection, self.h_pos);
        let row_height = ui.fonts_mut(|f| f.row_height(&font));
        let caret = cursor_rect(&galley, &range.primary, row_height).translate(pos.to_vec2());
        ui.painter_at(rect.intersect(ui.clip_rect())).rect_filled(
            rect,
            0.0,
            ui.visuals().extreme_bg_color,
        );
        let painter = ui.painter_at(text_clip);
        visuals::paint_text_selection(&mut galley, ui.visuals(), &range, None);
        gutter::paint(ui, &galley, pos, gutter_rect, &font);
        painter.galley(pos, galley, color);
        if focused {
            if interacted {
                self.last_interaction = ui.input(|i| i.time);
                let mut target = caret.expand(6.0);
                target.min.x -= gutter_width;
                ui.scroll_to_rect(target, None);
                ui.ctx().request_repaint();
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
                    // egui-winit 0.33 uses rect for set_ime_cursor_area. On
                    // Windows this excludes the supplied area from candidates,
                    // so the full document rect would anchor them far below the
                    // caret. Supply the caret line as the native exclusion area.
                    rect: transform * caret,
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
