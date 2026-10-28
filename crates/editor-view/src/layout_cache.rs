//! One bounded pair of derived, unselected/selected display galleys.

use super::layout_payload::retained_bytes;
use super::*;
#[cfg(test)]
use std::mem::size_of;

// Retained payload only: egui's own cache, allocator overhead, transient layout
// work, frame output references and native/GPU allocations are outside this cap.
const MAX_RETAINED_BYTES: usize = 256 * 1024 * 1024;

pub(super) struct LayoutCache {
    max_retained_bytes: usize,
    font_epoch: Option<Arc<Galley>>,
    base: Option<CachedLayout>,
    selected: Option<CachedSelection>,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self {
            max_retained_bytes: MAX_RETAINED_BYTES,
            font_epoch: None,
            base: None,
            selected: None,
        }
    }
}

struct CachedLayout {
    document_id: DocumentId,
    revision: Revision,
    galley: Arc<Galley>,
}

struct CachedSelection {
    range: CCursorRange,
    background: Color32,
    foreground: Color32,
    galley: Arc<Galley>,
}

impl LayoutCache {
    pub(super) fn clear(&mut self) {
        self.base = None;
        self.selected = None;
    }

    pub(super) fn get(
        &mut self,
        document: &Document,
        font_epoch: Arc<Galley>,
    ) -> Option<Arc<Galley>> {
        // In pinned egui 0.33.3 this tiny job is cached in the same Fonts cache
        // as document layouts. Font definitions, atlas resets, DPI, font ID and
        // text color all yield a new Arc. Keeping the old Arc prevents address
        // reuse from hiding a reset. No font-data scan or full-text hash is needed.
        if !self
            .font_epoch
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &font_epoch))
        {
            self.clear();
            self.font_epoch = (retained_bytes([font_epoch.as_ref()]) <= self.max_retained_bytes)
                .then_some(font_epoch);
        }
        if let Some(base) = &self.base
            && base.document_id == document.id()
            && base.revision == document.revision()
        {
            return Some(base.galley.clone());
        }
        self.clear();
        None
    }

    pub(super) fn insert(&mut self, document: &Document, galley: Arc<Galley>) {
        if let Some(base) = &self.base
            && base.document_id == document.id()
            && base.revision == document.revision()
            && Arc::ptr_eq(&base.galley, &galley)
        {
            return;
        }
        self.clear();
        let Some(epoch) = &self.font_epoch else {
            return;
        };
        if retained_bytes([epoch.as_ref(), galley.as_ref()]) <= self.max_retained_bytes {
            self.base = Some(CachedLayout {
                document_id: document.id(),
                revision: document.revision(),
                galley,
            });
        }
    }

    pub(super) fn paint_selection(
        &mut self,
        mut galley: Arc<Galley>,
        visuals: &egui::Visuals,
        range: &CCursorRange,
    ) -> Arc<Galley> {
        if range.is_empty() {
            self.selected = None;
            return galley;
        }
        if let Some(selected) = &self.selected
            && selected.range == *range
            && selected.background == visuals.selection.bg_fill
            && selected.foreground == visuals.selection.stroke.color
        {
            return selected.galley.clone();
        }
        self.selected = None;
        visuals::paint_text_selection(&mut galley, visuals, range, None);
        if let Some(base) = &self.base
            && let Some(epoch) = &self.font_epoch
            && retained_bytes([epoch.as_ref(), base.galley.as_ref(), galley.as_ref()])
                <= self.max_retained_bytes
        {
            self.selected = Some(CachedSelection {
                range: *range,
                background: visuals.selection.bg_fill,
                foreground: visuals.selection.stroke.color,
                galley: galley.clone(),
            });
        }
        galley
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layouts(text: &str) -> (Arc<Galley>, Arc<Galley>) {
        let ctx = egui::Context::default();
        let mut result = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            result = Some(ctx.fonts_mut(|fonts| {
                (
                    fonts.layout_no_wrap(text.into(), FontId::monospace(16.0), Color32::WHITE),
                    fonts.layout_no_wrap(String::new(), FontId::monospace(16.0), Color32::WHITE),
                )
            }));
        });
        result.unwrap()
    }

    #[test]
    fn retained_payload_deduplicates_aligned_sharing_and_conservatively_counts_other_rows() {
        let (base, _) = layouts("abc");
        let base_bytes = retained_bytes([base.as_ref()]);
        assert_eq!(base_bytes, retained_bytes([base.as_ref(), base.as_ref()]));

        let shared = base.as_ref().clone();
        let shared_container_bytes = size_of::<Galley>()
            + 2 * size_of::<usize>()
            + shared.rows.capacity() * size_of::<egui::epaint::text::PlacedRow>();
        assert_eq!(
            retained_bytes([base.as_ref(), &shared]),
            base_bytes + shared_container_bytes
        );

        let mut repeated_rows = base.as_ref().clone();
        repeated_rows.rows = vec![base.rows[0].clone(); 32];
        let mut one_row = repeated_rows.clone();
        one_row.rows.truncate(1);
        assert_eq!(one_row.rows.capacity(), repeated_rows.rows.capacity());
        assert!(retained_bytes([&one_row]) < retained_bytes([&repeated_rows]));
        assert_eq!(
            retained_bytes([&one_row, &repeated_rows]),
            retained_bytes([&repeated_rows])
                + size_of::<Galley>()
                + 2 * size_of::<usize>()
                + one_row.rows.capacity() * size_of::<egui::epaint::text::PlacedRow>()
        );
    }

    #[test]
    fn base_reuses_selection_only_changes_and_invalidates_identity_revision_and_fonts() {
        let (base, epoch) = layouts("abc");
        let mut document = Document::new(DocumentId(1), "abc").unwrap();
        let mut cache = LayoutCache::default();
        assert!(cache.get(&document, epoch.clone()).is_none());
        cache.insert(&document, base.clone());
        document
            .set_selection(Selection { anchor: 0, head: 2 })
            .unwrap();
        assert!(Arc::ptr_eq(
            &cache.get(&document, epoch.clone()).unwrap(),
            &base
        ));

        let mut other_document = Document::new(DocumentId(2), "abc").unwrap();
        assert!(cache.get(&other_document, epoch.clone()).is_none());
        cache.insert(&other_document, base.clone());
        other_document.replace(0..1, "z").unwrap();
        assert!(cache.get(&other_document, epoch.clone()).is_none());

        cache.insert(&document, base);
        let (_, other_epoch) = layouts("abc");
        assert!(!Arc::ptr_eq(&epoch, &other_epoch));
        assert!(cache.get(&document, other_epoch).is_none());
        assert!(cache.base.is_none());
        assert!(cache.selected.is_none());
    }

    #[test]
    fn exact_budget_admits_base_refuses_selection_and_evicts_previous_entries() {
        let (base, epoch) = layouts("abc");
        let document = Document::new(DocumentId(1), "abc").unwrap();
        let base_bytes = retained_bytes([epoch.as_ref(), base.as_ref()]);
        let mut cache = LayoutCache {
            max_retained_bytes: base_bytes,
            ..Default::default()
        };
        assert!(cache.get(&document, epoch.clone()).is_none());
        cache.insert(&document, base.clone());
        assert!(cache.base.is_some());
        let range = CCursorRange::two(CCursor::new(0), CCursor::new(2));
        let visuals = egui::Visuals::default();
        let original_rows = base.rows.clone();
        let painted = cache.paint_selection(base.clone(), &visuals, &range);
        assert_ne!(painted.rows, original_rows);
        assert_eq!(base.rows, original_rows);
        assert!(cache.selected.is_none());
        assert!(cache.base.is_some());

        cache.max_retained_bytes =
            retained_bytes([epoch.as_ref(), base.as_ref(), painted.as_ref()]);
        let selected = cache.paint_selection(base.clone(), &visuals, &range);
        assert!(cache.selected.is_some());
        assert!(Arc::ptr_eq(
            &selected,
            &cache.paint_selection(base.clone(), &visuals, &range)
        ));
        cache.paint_selection(base.clone(), &visuals, &CCursorRange::default());
        assert!(cache.selected.is_none());

        cache.max_retained_bytes = base_bytes - 1;
        let replacement = Document::new(DocumentId(2), "abc").unwrap();
        cache.insert(&replacement, base);
        assert!(cache.base.is_none());
        assert!(cache.selected.is_none());
        assert!(cache.get(&document, epoch).is_none());
    }

    #[test]
    fn a_budget_smaller_than_the_epoch_retains_no_entry() {
        let (base, epoch) = layouts("abc");
        let document = Document::new(DocumentId(1), "abc").unwrap();
        let mut cache = LayoutCache {
            max_retained_bytes: retained_bytes([epoch.as_ref()]) - 1,
            ..Default::default()
        };
        assert!(cache.get(&document, epoch).is_none());
        assert!(cache.font_epoch.is_none());
        cache.insert(&document, base);
        assert!(cache.base.is_none());
        assert!(cache.selected.is_none());
    }
}
