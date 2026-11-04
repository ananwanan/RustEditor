//! Gutter and text share the galley's row geometry, in logical egui points.
use egui::{Align2, Color32, FontId, Galley, Pos2, Rect, Ui};

pub(super) fn width(ui: &Ui, galley: &Galley, font: &FontId) -> f32 {
    let digits = galley.rows.len().max(1).ilog10().max(1) as usize + 1;
    ui.fonts_mut(|f| f.glyph_width(font, '0')) * digits as f32 + 18.0
}

pub(super) fn paint(ui: &Ui, galley: &Galley, origin: Pos2, gutter: Rect, font: &FontId) {
    let painter = ui.painter_at(gutter.intersect(ui.clip_rect()));
    painter.rect_filled(gutter, 0.0, ui.visuals().panel_fill);
    painter.vline(
        gutter.right() - 1.0,
        gutter.y_range(),
        (1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
    // Skip rows above/below the viewport; do not allocate a line-number buffer.
    let first = galley
        .rows
        .partition_point(|row| origin.y + row.rect().bottom() < gutter.top());
    for (index, row) in galley.rows.iter().enumerate().skip(first) {
        let top = origin.y + row.rect().top();
        if top > gutter.bottom() {
            break;
        }
        painter.text(
            egui::pos2(gutter.right() - 9.0, top),
            Align2::RIGHT_TOP,
            (index + 1).to_string(),
            font.clone(),
            Color32::GRAY,
        );
    }
}
