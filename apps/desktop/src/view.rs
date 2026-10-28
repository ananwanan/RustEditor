#![forbid(unsafe_code)]
use eframe::egui::{
    self, Color32, FontFamily, FontId, Key, Modifiers, TextBuffer,
    text::{CCursor, CCursorRange},
};
use rusteditor_editor_core::{
    Document, MAX_DOCUMENT_BYTES, Selection, byte_to_scalar, scalar_to_byte,
};
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use std::{any::TypeId, ops::Range, path::PathBuf};

/// A frame-scoped transaction adapter. The draft exists only during TextEdit's
/// synchronous call and commits once; Document remains the persistent authority.
struct CoreBuffer<'a> {
    document: &'a mut Document,
    draft: Option<String>,
    error: Option<String>,
}

impl<'a> CoreBuffer<'a> {
    fn new(document: &'a mut Document) -> Self {
        Self {
            document,
            draft: None,
            error: None,
        }
    }
    fn finish(self, selection: Selection) -> Result<(), String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if let Some(draft) = self.draft {
            self.document
                .commit_draft(&draft, selection)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            self.document
                .set_selection(selection)
                .map_err(|e| e.to_string())
        }
    }
}

impl TextBuffer for CoreBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.draft
            .as_deref()
            .unwrap_or_else(|| self.document.text())
    }
    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        if self.error.is_some() {
            return 0;
        }
        if self.as_str().len().saturating_add(text.len()) > MAX_DOCUMENT_BYTES {
            self.error = Some("M0 document limit is 4 MiB; edit was rejected".into());
            return 0;
        }
        let byte = scalar_to_byte(self.as_str(), char_index);
        self.draft
            .get_or_insert_with(|| self.document.text().to_owned())
            .insert_str(byte, text);
        text.chars().count()
    }
    fn delete_char_range(&mut self, chars: Range<usize>) {
        if self.error.is_some() {
            return;
        }
        let start = scalar_to_byte(self.as_str(), chars.start);
        let end = scalar_to_byte(self.as_str(), chars.end);
        if start > end {
            self.error = Some("Invalid character range".into());
            return;
        }
        if start == end {
            return;
        }
        self.draft
            .get_or_insert_with(|| self.document.text().to_owned())
            .replace_range(start..end, "");
    }
    fn replace_with(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        if text.len() > MAX_DOCUMENT_BYTES {
            self.error = Some("M0 document limit is 4 MiB; edit was rejected".into());
        } else {
            self.draft = Some(text.to_owned());
        }
    }
    fn type_id(&self) -> TypeId {
        TypeId::of::<CoreBuffer<'static>>()
    }
}

pub struct EditorApp {
    document: SharedDocument,
    host: PluginHost,
    diagnostics: Vec<String>,
    status: String,
    sync_cursor: bool,
    composing: bool,
    confirm_close: bool,
    allow_close: bool,
    font_status: String,
}

impl EditorApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        document: SharedDocument,
        host: PluginHost,
        diagnostics: Vec<String>,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = Color32::from_rgb(27, 94, 122);
        style.visuals.panel_fill = Color32::from_rgb(20, 25, 33);
        style.visuals.extreme_bg_color = Color32::from_rgb(15, 19, 26);
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        cc.egui_ctx.set_style(style);
        let font_status = install_cjk_font(&cc.egui_ctx);
        Self {
            document,
            host,
            diagnostics,
            status: "Ready".into(),
            sync_cursor: true,
            composing: false,
            confirm_close: false,
            allow_close: false,
            font_status,
        }
    }

    fn core_action(&mut self, redo: bool) {
        let result = if redo {
            self.document.borrow_mut().redo()
        } else {
            self.document.borrow_mut().undo()
        };
        match result {
            Ok(changed) => {
                self.sync_cursor |= changed;
                self.status = if redo { "Redo" } else { "Undo" }.into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit(text)) if !text.is_empty() => {
                        self.composing = true
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled) => {
                        self.composing = false
                    }
                    _ => {}
                }
            }
        });
        let mut action = None;
        let mut undo = false;
        let mut redo = false;
        if !self.composing {
            ctx.input_mut(|input| {
                redo = input.consume_shortcut(&egui::KeyboardShortcut::new(
                    Modifiers::COMMAND | Modifiers::SHIFT,
                    Key::Z,
                )) || input
                    .consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Y));
                undo = input
                    .consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Z));
                if input.consume_shortcut(&egui::KeyboardShortcut::new(
                    Modifiers::COMMAND | Modifiers::SHIFT,
                    Key::U,
                )) {
                    action = Some(crate::UPPERCASE.to_owned());
                }
            });
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("RustEditor")
                        .size(23.0)
                        .strong()
                        .color(Color32::from_rgb(119, 217, 234)),
                );
                ui.label(
                    egui::RichText::new("M0 / TECHNICAL PREVIEW")
                        .small()
                        .color(Color32::GRAY),
                );
            });
            ui.separator();
            ui.horizontal(|ui| {
                let doc = self.document.borrow();
                undo |= ui
                    .add_enabled(!self.composing && doc.can_undo(), egui::Button::new("Undo"))
                    .clicked();
                redo |= ui
                    .add_enabled(!self.composing && doc.can_redo(), egui::Button::new("Redo"))
                    .clicked();
                drop(doc);
                ui.menu_button("Plugins", |ui| {
                    for command in self.host.commands() {
                        if ui
                            .add_enabled(!self.composing, egui::Button::new(&command.title))
                            .clicked()
                        {
                            action = Some(command.id);
                            ui.close();
                        }
                    }
                    if self.host.commands().is_empty() {
                        ui.label("No plugin commands loaded");
                    }
                });
                ui.separator();
                ui.label(
                    egui::RichText::new("Scratch buffer · not saved to disk")
                        .color(Color32::from_rgb(236, 186, 114)),
                );
            });
            ui.add_space(3.0);
        });

        if undo {
            self.core_action(false);
        }
        if redo {
            self.core_action(true);
        }

        egui::SidePanel::right("plugin_panel").default_width(230.0).resizable(true).show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Extensions");
            ui.label(format!("{} native plugin(s) loaded", self.host.loaded_count()));
            ui.label("Updates apply after restart.");
            ui.separator();
            for command in self.host.commands() {
                ui.label(egui::RichText::new(&command.title).strong());
                ui.label(egui::RichText::new(&command.plugin_id).small());
                if ui.add_enabled(!self.composing, egui::Button::new("Run command")).clicked() { action = Some(command.id); }
            }
            if self.host.commands().is_empty() {
                ui.label("Run cargo xtask stage, then restart to enable text-tools.");
            }
            ui.separator();
            ui.label("Ctrl+Shift+U  Uppercase\nCtrl+Z  Undo\nCtrl+Y / Ctrl+Shift+Z  Redo\nCtrl+C / X / V  Clipboard");
            ui.separator();
            ui.label(egui::RichText::new(&self.font_status).small());
            for error in &self.diagnostics { ui.colored_label(Color32::LIGHT_RED, error); }
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let doc = self.document.borrow();
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                ui.label(format!(
                    "UTF-8 · {} bytes · revision {} · selection {} bytes",
                    doc.text().len(),
                    doc.revision().0,
                    doc.selected_text().len()
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}% scale", ctx.pixels_per_point() * 100.0));
                    if self.composing {
                        ui.label("IME composing");
                    }
                });
            });
        });

        let editor_id = egui::Id::new("scratch_editor");
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut doc = self.document.borrow_mut();
            let mut state = egui::TextEdit::load_state(ctx, editor_id).unwrap_or_default();
            // Core owns history. Never retain TextEdit's independent String undo snapshots.
            state.clear_undoer();
            if self.sync_cursor {
                let selection = doc.selection();
                state.cursor.set_char_range(Some(CCursorRange::two(
                    CCursor::new(
                        byte_to_scalar(doc.text(), selection.anchor).expect("valid core selection"),
                    ),
                    CCursor::new(
                        byte_to_scalar(doc.text(), selection.head).expect("valid core selection"),
                    ),
                )));
            }
            state.store(ctx, editor_id);
            let mut selection = doc.selection();
            let mut buffer = CoreBuffer::new(&mut doc);
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut output = egui::TextEdit::multiline(&mut buffer)
                        .id(editor_id)
                        .font(FontId::monospace(16.0))
                        .code_editor()
                        .desired_width(ui.available_width().max(640.0))
                        .desired_rows(32)
                        .lock_focus(true)
                        .show(ui);
                    if self.sync_cursor {
                        output.response.request_focus();
                    }
                    if let Some(cursor) = output.state.cursor.char_range() {
                        selection = Selection {
                            anchor: scalar_to_byte(buffer.as_str(), cursor.secondary.index),
                            head: scalar_to_byte(buffer.as_str(), cursor.primary.index),
                        };
                    }
                    output.state.clear_undoer();
                    output.state.store(ctx, editor_id);
                });
            self.sync_cursor = false;
            if let Err(error) = buffer.finish(selection) {
                self.status = error;
                self.sync_cursor = true;
                ctx.request_repaint();
            }
        });

        if let Some(command) = action {
            match self.host.invoke(&command) {
                Ok(()) => {
                    self.status = "Plugin transaction applied".into();
                    self.sync_cursor = true;
                }
                Err(error) => self.status = error.to_string(),
            }
            ctx.request_repaint();
        }
        if ctx.input(|input| input.viewport().close_requested())
            && !self.allow_close
            && self.document.borrow().revision().0 > 1
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if self.confirm_close {
            egui::Modal::new(egui::Id::new("close_scratch")).show(ctx, |ui| {
                ui.heading("Discard scratch text?");
                ui.label("This preview does not save files. Closing discards the scratch buffer.");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() {
                        self.confirm_close = false;
                    }
                    if ui.button("Discard and close").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        }
    }
}

fn install_cjk_font(ctx: &egui::Context) -> String {
    let mut paths: Vec<PathBuf> = std::env::var_os("RUSTEDITOR_FONT")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(windows) = std::env::var_os("WINDIR") {
        paths.push(PathBuf::from(windows).join("Fonts/msyh.ttc"));
    }
    paths.extend([
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
    ]);
    for path in paths {
        if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024 * 1024)
            && let Ok(bytes) = std::fs::read(&path)
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "system-cjk".into(),
                egui::FontData::from_owned(bytes).into(),
            );
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("system-cjk".into());
            }
            ctx.set_fonts(fonts);
            return "System CJK font loaded".into();
        }
    }
    "CJK font missing: set RUSTEDITOR_FONT to a TTF/TTC/OTF file".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusteditor_editor_core::DocumentId;

    #[test]
    fn text_buffer_uses_core_transactions_and_scalar_byte_conversion() {
        let mut document = Document::new(DocumentId(1), "中🙂e\u{301}").unwrap();
        {
            let mut buffer = CoreBuffer::new(&mut document);
            assert_eq!(buffer.insert_text("你好", 1), 2);
            buffer.delete_char_range(3..4);
            assert_eq!(buffer.as_str(), "中你好e\u{301}");
            buffer.finish(Selection::default()).unwrap();
        }
        document.undo().unwrap();
        assert_eq!(document.text(), "中🙂e\u{301}");
        assert!(!document.can_undo());
    }

    #[test]
    fn oversized_paste_after_deleting_a_selection_keeps_original_text_and_history() {
        let mut document = Document::new(DocumentId(1), "do not lose 你好").unwrap();
        let original = document.text().to_owned();
        let mut buffer = CoreBuffer::new(&mut document);
        buffer.delete_char_range(0..6);
        assert_eq!(
            buffer.insert_text(&"x".repeat(MAX_DOCUMENT_BYTES + 1), 0),
            0
        );
        assert!(buffer.finish(Selection::default()).is_err());
        assert_eq!(document.text(), original);
        assert_eq!(document.revision().0, 1);
        assert!(!document.can_undo());
    }
}
