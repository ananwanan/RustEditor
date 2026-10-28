//! Allocation-free retained-payload accounting shared with the diagnostic example.

use egui::Galley;
use std::{mem::size_of, sync::Arc};

// Painted galleys preserve row indices, sharing their LayoutJob and untouched
// rows with the base. Deduplicate that sharing without allocating. Any sharing
// at different indices or within one galley is conservatively counted again.
// This capacity walk runs only on admission, over at most three galleys.
pub(super) fn retained_bytes<const N: usize>(galleys: [&Galley; N]) -> usize {
    let mut bytes = 0_usize;
    for (index, galley) in galleys.iter().copied().enumerate() {
        let previous = &galleys[..index];
        if previous.iter().any(|old| std::ptr::eq(*old, galley)) {
            continue;
        }
        bytes += size_of::<Galley>() + 2 * size_of::<usize>();
        bytes += galley.rows.capacity() * size_of::<egui::epaint::text::PlacedRow>();
        if !previous
            .iter()
            .any(|old| Arc::ptr_eq(&old.job, &galley.job))
        {
            let job = &galley.job;
            bytes += size_of::<egui::text::LayoutJob>() + 2 * size_of::<usize>();
            bytes += job.text.capacity();
            bytes += job.sections.capacity() * size_of::<egui::text::LayoutSection>();
            for section in &job.sections {
                if let egui::FontFamily::Name(name) = &section.format.font_id.family {
                    bytes += name.len() + 2 * size_of::<usize>();
                }
            }
        }
        for (row_index, row) in galley.rows.iter().enumerate() {
            if !previous.iter().any(|old| {
                old.rows
                    .get(row_index)
                    .is_some_and(|old_row| Arc::ptr_eq(&old_row.row, &row.row))
            }) {
                bytes += size_of::<egui::epaint::text::Row>() + 2 * size_of::<usize>();
                bytes += row.glyphs.capacity() * size_of::<egui::epaint::text::Glyph>();
                bytes += row.visuals.mesh.vertices.capacity() * size_of::<egui::epaint::Vertex>();
                bytes += row.visuals.mesh.indices.capacity() * size_of::<u32>();
            }
        }
    }
    bytes
}
