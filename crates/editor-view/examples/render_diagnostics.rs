//! Internal CPU/allocation diagnostics; no native input or display presentation.
use egui::{Event, FontFamily, Id, ImeEvent, Key, Modifiers};
use rusteditor_editor_core::{Document, DocumentId, Selection};
use rusteditor_editor_view::EditorView;
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    time::Instant,
};

struct CountingAllocator;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);

fn added(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    if ACTIVE.load(Relaxed) {
        ALLOCATED.fetch_add(size, Relaxed);
        CALLS.fetch_add(1, Relaxed);
        PEAK.fetch_max(live, Relaxed);
    }
}

// SAFETY: This diagnostic allocator forwards each operation unchanged to System
// and touches only allocation-free atomic counters. The caller retains all
// GlobalAlloc pointer/layout obligations; no allocation ownership is changed.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller provides a valid allocation layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            added(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller provides a valid allocation layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            added(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        // SAFETY: System allocated ptr with this layout, as required by caller.
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a System allocation and a valid new size.
        let result = unsafe { System.realloc(ptr, layout, size) };
        if !result.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            added(size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measured<T>(f: impl FnOnce() -> T) -> (T, Value) {
    let before = LIVE.load(Relaxed);
    ALLOCATED.store(0, Relaxed);
    CALLS.store(0, Relaxed);
    PEAK.store(before, Relaxed);
    ACTIVE.store(true, Relaxed);
    let start = Instant::now();
    let value = f();
    let ns = start.elapsed().as_nanos() as u64;
    ACTIVE.store(false, Relaxed);
    let allocated = ALLOCATED.load(Relaxed);
    let calls = CALLS.load(Relaxed);
    let after = LIVE.load(Relaxed);
    let peak = PEAK.load(Relaxed);
    (
        value,
        json!({"cpu_ns": ns, "allocated_bytes": allocated,
        "allocation_calls": calls, "live_bytes_before": before,
        "live_bytes_after": after, "peak_live_bytes": peak,
        "peak_extra_live_bytes": peak.saturating_sub(before)}),
    )
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

struct Harness {
    ctx: egui::Context,
    view: EditorView,
    document: Document,
    time: f64,
}

impl Harness {
    fn new(text: &str, fonts: &egui::FontDefinitions) -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts.clone());
        let mut this = Self {
            ctx,
            view: EditorView::default(),
            document: Document::new(DocumentId(1), text).unwrap(),
            time: 0.0,
        };
        for _ in 0..3 {
            this.frame(Vec::new());
        }
        this
    }

    fn frame(&mut self, events: Vec<Event>) -> (usize, usize) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                time: Some(self.time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let output = self.view.show(
                                ui,
                                Id::new("render-diagnostic"),
                                &mut self.document,
                            );
                            assert!(output.error.is_none(), "{:?}", output.error);
                        });
                });
            },
        );
        let result = (
            output.shapes.len(),
            self.ctx.fonts(|f| f.num_galleys_in_cache()),
        );
        drop(output);
        result
    }

    fn selection(&mut self, anchor: usize, head: usize) {
        self.document
            .set_selection(Selection { anchor, head })
            .unwrap();
    }
}

fn load_fonts(args: &[String]) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (flag, name) in [("--font", "system-cjk"), ("--emoji-font", "system-emoji")] {
        if let Some(i) = args.iter().position(|arg| arg == flag) {
            let path = Path::new(args.get(i + 1).expect("font path"));
            assert!(path.metadata().unwrap().len() <= 64 * 1024 * 1024);
            fonts.font_data.insert(
                name.into(),
                egui::FontData::from_owned(std::fs::read(path).unwrap()).into(),
            );
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                let fallbacks = fonts.families.entry(family).or_default();
                if name == "system-emoji" {
                    let i = fallbacks
                        .iter()
                        .position(|n| n.contains("Emoji") || n.contains("emoji"))
                        .unwrap_or(fallbacks.len());
                    fallbacks.insert(i, name.into());
                } else {
                    fallbacks.push(name.into());
                }
            }
        }
    }
    fonts
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() >= 3,
        "render_diagnostics INPUT SAMPLES [--font PATH] [--emoji-font PATH]"
    );
    let text = std::fs::read_to_string(&args[1]).expect("bounded UTF-8 fixture");
    assert!(text.len() <= rusteditor_editor_core::MAX_DOCUMENT_BYTES);
    let count: usize = args[2].parse().unwrap();
    assert!((1..=1000).contains(&count));
    let fonts = load_fonts(&args);
    let mut rows = Vec::new();
    for scenario in [
        "idle",
        "caret",
        "selection-change",
        "selection-idle-all",
        "edit",
        "edit-after-idle",
        "ime-preedit",
        "ime-repeat",
        "ime-commit",
        "ime-cancel",
        "ime-native-clear-commit",
    ] {
        let mut h = Harness::new(&text, &fonts);
        if scenario == "selection-idle-all" {
            h.selection(0, text.len());
            h.frame(vec![]);
        }
        if scenario == "ime-repeat" {
            h.frame(vec![Event::Ime(ImeEvent::Preedit("ni".into()))]);
        }
        let mut samples = Vec::new();
        for i in 0..count {
            if scenario == "edit-after-idle" {
                for _ in 0..3 {
                    h.frame(vec![]);
                }
            }
            let events = match scenario {
                "caret" => vec![key(
                    if i % 2 == 0 {
                        Key::ArrowRight
                    } else {
                        Key::ArrowLeft
                    },
                    Modifiers::NONE,
                )],
                "selection-change" => vec![key(
                    if i % 2 == 0 {
                        Key::ArrowRight
                    } else {
                        Key::ArrowLeft
                    },
                    Modifiers::SHIFT,
                )],
                "edit" | "edit-after-idle" => {
                    if i % 2 == 0 {
                        vec![Event::Text("a".into())]
                    } else {
                        vec![key(Key::Backspace, Modifiers::NONE)]
                    }
                }
                "ime-preedit" => vec![Event::Ime(ImeEvent::Preedit(
                    if i % 2 == 0 { "ni" } else { "hao" }.into(),
                ))],
                "ime-repeat" => vec![Event::Ime(ImeEvent::Preedit("ni".into()))],
                "ime-commit" | "ime-cancel" | "ime-native-clear-commit" => {
                    h.selection(
                        0,
                        if h.document.text().starts_with('你') {
                            3
                        } else {
                            0
                        },
                    );
                    h.frame(vec![
                        Event::Ime(ImeEvent::Enabled),
                        Event::Ime(ImeEvent::Preedit("ni".into())),
                    ]);
                    if scenario == "ime-cancel" {
                        vec![key(Key::Escape, Modifiers::NONE)]
                    } else if scenario == "ime-native-clear-commit" {
                        vec![
                            Event::Ime(ImeEvent::Disabled),
                            Event::Ime(ImeEvent::Commit("你".into())),
                            Event::Ime(ImeEvent::Disabled),
                        ]
                    } else {
                        vec![
                            Event::Ime(ImeEvent::Commit("你".into())),
                            Event::Ime(ImeEvent::Disabled),
                        ]
                    }
                }
                _ => vec![],
            };
            let ((shapes, cached), mut sample) = measured(|| h.frame(events));
            sample["index"] = json!(i);
            sample["shape_count"] = json!(shapes);
            sample["egui_cached_galleys"] = json!(cached);
            sample["document_revision"] = json!(h.document.revision().0);
            samples.push(sample);
        }
        rows.push(json!({"scenario": scenario, "samples": samples}));
    }
    let ctx = egui::Context::default();
    ctx.set_fonts(fonts);
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    let font = egui::FontId::monospace(16.0);
    let color = egui::Color32::WHITE;
    let base = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), color));
    for scenario in [
        "text-copy-only",
        "egui-layout-cache-hit",
        "selection-mutate-first",
        "selection-mutate-all",
    ] {
        let mut samples = Vec::new();
        for i in 0..count {
            let mut shared = false;
            let (_, mut sample) = measured(|| match scenario {
                "text-copy-only" => {
                    drop(black_box(text.clone()));
                }
                "egui-layout-cache-hit" => {
                    let galley =
                        ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), color));
                    shared = Arc::ptr_eq(&base, &galley);
                    drop(black_box(galley));
                }
                _ => {
                    let mut galley = base.clone();
                    let scalars = if scenario == "selection-mutate-all" {
                        text.chars().count()
                    } else {
                        usize::from(!text.is_empty())
                    };
                    let range = egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(scalars),
                    );
                    egui::text_selection::visuals::paint_text_selection(
                        &mut galley,
                        &ctx.style().visuals,
                        &range,
                        None,
                    );
                    shared = Arc::ptr_eq(&base, &galley);
                    drop(black_box(galley));
                }
            });
            sample["index"] = json!(i);
            sample["galley_shared_with_base"] = json!(shared);
            samples.push(sample);
        }
        rows.push(json!({"scenario": scenario, "samples": samples}));
    }
    let probe = cache_probe(&ctx, &base, &font, color);
    println!(
        "{}",
        json!({"kind": "internal-cpu-allocation-diagnostic", "presentation_latency": false,
        "build": {"debug_assertions": cfg!(debug_assertions), "target_os":std::env::consts::OS,
            "target_arch":std::env::consts::ARCH},
        "frame_scope": "Context::run, EditorView::show, output drop; no tessellation, native input, GPU or display",
        "allocator_scope": "successful Rust System allocation/reallocation requested bytes; reallocation counts full new size; live/peak exclude native/GPU allocations and allocator overhead",
        "samples_per_scenario": count, "viewport_points": [900,600], "pixels_per_point": 1.0,
        "text_bytes": text.len(), "base_galley_rows": base.rows.len(), "base_galley_vertices": base.num_vertices,
        "egui_cache_probe": probe, "scenarios": rows})
    );
}

fn payload_bytes(galley: &egui::Galley, unique_rows: bool) -> usize {
    use std::{collections::HashSet, mem::size_of};
    let mut seen = HashSet::new();
    let mut bytes = size_of::<egui::Galley>()
        + size_of::<egui::text::LayoutJob>()
        + galley.job.text.capacity()
        + galley.job.sections.capacity() * size_of::<egui::text::LayoutSection>()
        + galley.rows.capacity() * size_of::<egui::epaint::text::PlacedRow>();
    for row in &galley.rows {
        if !unique_rows || seen.insert(Arc::as_ptr(&row.row)) {
            bytes += size_of::<egui::epaint::text::Row>()
                + row.glyphs.capacity() * size_of::<egui::epaint::text::Glyph>()
                + row.visuals.mesh.vertices.capacity() * size_of::<egui::epaint::Vertex>()
                + row.visuals.mesh.indices.capacity() * size_of::<u32>();
        }
    }
    bytes
}

fn cache_probe(
    ctx: &egui::Context,
    base: &Arc<egui::Galley>,
    font: &egui::FontId,
    color: egui::Color32,
) -> Value {
    let mut selected = base.clone();
    let range = egui::text::CCursorRange::two(
        egui::text::CCursor::new(0),
        egui::text::CCursor::new(usize::from(!base.text().is_empty())),
    );
    egui::text_selection::visuals::paint_text_selection(
        &mut selected,
        &ctx.style().visuals,
        &range,
        None,
    );
    let row_sharing: Vec<_> = base.rows.iter().zip(&selected.rows).enumerate().take(3)
        .map(|(index,(a,b))| json!({"index":index,"shared_after_first_scalar_selection":Arc::ptr_eq(&a.row,&b.row)})).collect();
    let base_payload = json!({"unique_row_payload_bytes":payload_bytes(base,true),
        "conservative_row_payload_bytes":payload_bytes(base,false)});
    let selected_payload = json!({"unique_row_payload_bytes":payload_bytes(&selected,true),
        "conservative_row_payload_bytes":payload_bytes(&selected,false)});
    let mut all_selected = base.clone();
    let all = egui::text::CCursorRange::two(
        egui::text::CCursor::new(0),
        egui::text::CCursor::new(base.text().chars().count()),
    );
    egui::text_selection::visuals::paint_text_selection(
        &mut all_selected,
        &ctx.style().visuals,
        &all,
        None,
    );
    let all_selected_payload = json!({"unique_row_payload_bytes":payload_bytes(&all_selected,true),
        "conservative_row_payload_bytes":payload_bytes(&all_selected,false)});
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |_| {});
    }
    let cache_after_idle = ctx.fonts(|f| f.num_galleys_in_cache());
    let next = ctx.fonts_mut(|f| f.layout_no_wrap(base.text().to_owned(), font.clone(), color));
    json!({"base_payload":base_payload,"first_scalar_selected_payload":selected_payload,
        "all_selected_payload":all_selected_payload,"row_sharing":row_sharing,
        "layout_job_shared_after_selection":Arc::ptr_eq(&base.job,&selected.job),
        "galley_shared_after_selection":Arc::ptr_eq(base,&selected),
        "egui_entries_after_three_unreferenced_passes":cache_after_idle,
        "same_galley_after_three_unreferenced_passes":Arc::ptr_eq(base,&next),
        "payload_scope":"public Galley/LayoutJob/row/glyph/vertex/index sizes and capacities; excludes Arc headers, font-name heap storage, allocator overhead and egui cache bookkeeping; unique rows deduplicate row Arcs"})
}
