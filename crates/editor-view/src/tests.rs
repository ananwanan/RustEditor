use super::*;
use egui::{Event, ImeEvent, Key, Modifiers};
use rusteditor_editor_core::{DocumentId, MAX_DOCUMENT_BYTES};
use unicode_segmentation::UnicodeSegmentation;

struct Harness {
    ctx: egui::Context,
    view: EditorView,
    document: Document,
    time: f64,
    error: Option<String>,
}

impl Harness {
    fn new(text: &str) -> Self {
        let mut harness = Self {
            ctx: egui::Context::default(),
            view: EditorView::default(),
            document: Document::new(DocumentId(1), text).unwrap(),
            time: 0.0,
            error: None,
        };
        harness.frame(Vec::new());
        harness
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.error = None;
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                focused: true,
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let output =
                                self.view
                                    .show(ui, Id::new("test-editor"), &mut self.document);
                            self.error = output.error;
                        });
                });
            },
        )
    }

    fn caret(&mut self, byte: usize) {
        self.document
            .set_selection(Selection {
                anchor: byte,
                head: byte,
            })
            .unwrap();
    }
}

fn key_with(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}
fn key(key: Key) -> Event {
    key_with(key, Modifiers::NONE)
}
fn ime(event: ImeEvent) -> Event {
    Event::Ime(event)
}

#[test]
fn horizontal_navigation_and_directed_selection_use_whole_graphemes() {
    let text = "中e\u{301}👩🏽‍💻🇨🇳1️⃣\r\n尾";
    let mut h = Harness::new(text);
    let boundaries: Vec<_> = text
        .grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    for &byte in boundaries.iter().skip(1) {
        h.frame(vec![key(Key::ArrowRight)]);
        assert_eq!(
            h.document.selection(),
            Selection {
                anchor: byte,
                head: byte
            }
        );
    }
    for &byte in boundaries.iter().rev().skip(1) {
        h.frame(vec![key_with(Key::ArrowLeft, Modifiers::SHIFT)]);
        assert_eq!(
            h.document.selection(),
            Selection {
                anchor: text.len(),
                head: byte
            }
        );
    }
    let output = h.frame(vec![Event::Copy]);
    assert!(
        output
            .platform_output
            .commands
            .contains(&egui::OutputCommand::CopyText(text.into()))
    );
    h.frame(vec![key(Key::ArrowLeft)]);
    assert_eq!(h.document.selection(), Selection::default());
    assert_eq!(h.document.text(), text);
    assert!(!h.document.can_undo());
}

#[test]
fn backspace_and_delete_remove_one_cluster_and_are_one_undo_step() {
    for cluster in ["e\u{301}", "👩🏽‍💻", "🇨🇳", "1️⃣", "\r\n", "한", "क्‍ष"]
    {
        for deletion in [Key::Backspace, Key::Delete] {
            let text = format!("L{cluster}R");
            let mut h = Harness::new(&text);
            let caret = if deletion == Key::Backspace {
                1 + cluster.len()
            } else {
                1
            };
            h.caret(caret);
            h.frame(vec![key(deletion)]);
            assert_eq!(h.document.text(), "LR", "{cluster:?} {deletion:?}");
            h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
            assert_eq!(h.document.text(), text);
            assert_eq!(
                h.document.selection(),
                Selection {
                    anchor: caret,
                    head: caret
                }
            );
            assert!(!h.document.can_undo());
            h.frame(vec![key_with(Key::Y, Modifiers::COMMAND)]);
            assert_eq!(h.document.text(), "LR");
        }
    }
}

#[test]
fn same_frame_paste_navigation_and_deletion_observe_each_previous_event() {
    let mut h = Harness::new("");
    h.frame(vec![
        Event::Paste("Ae\u{301}👩‍💻🇨🇳Z".into()),
        key(Key::ArrowLeft),
        key(Key::ArrowLeft),
        key(Key::Backspace),
        key(Key::Delete),
        Event::Text("中".into()),
    ]);
    assert_eq!(h.document.text(), "Ae\u{301}中Z");
    assert_eq!(h.document.selection().head, "Ae\u{301}中".len());
    h.frame(vec![
        key(Key::Home),
        key(Key::ArrowRight),
        key(Key::ArrowRight),
        key(Key::ArrowRight),
    ]);
    assert_eq!(h.document.selection().head, "Ae\u{301}中".len());
}

#[test]
fn inserting_next_to_combining_text_records_a_valid_redo_caret() {
    let mut h = Harness::new("\u{301}!");
    h.frame(vec![Event::Text("e".into())]);
    assert_eq!(h.document.text(), "e\u{301}!");
    assert_eq!(h.document.selection().head, "e\u{301}".len());
    h.frame(vec![
        key_with(Key::Z, Modifiers::COMMAND),
        key_with(Key::Y, Modifiers::COMMAND),
    ]);
    assert_eq!(h.document.selection().head, "e\u{301}".len());
}

#[test]
fn oversized_paste_preserves_original_selection_revision_and_history() {
    let mut h = Harness::new("keep 你好");
    let selection = Selection {
        anchor: 0,
        head: h.document.text().len(),
    };
    h.document.set_selection(selection).unwrap();
    h.frame(vec![Event::Paste("x".repeat(MAX_DOCUMENT_BYTES + 1))]);
    assert_eq!(h.error.as_deref(), Some("TooLarge"));
    assert_eq!(h.document.text(), "keep 你好");
    assert_eq!(h.document.selection(), selection);
    assert_eq!(h.document.revision(), Revision(1));
    assert!(!h.document.can_undo());
}

#[test]
fn ime_preedit_is_an_overlay_and_commit_is_one_atomic_transaction() {
    let mut h = Harness::new("before 原 after");
    let selection = Selection {
        anchor: 7,
        head: 10,
    };
    h.document.set_selection(selection).unwrap();
    for preedit in ["ni", "你"] {
        let output = h.frame(vec![ime(ImeEvent::Preedit(preedit.into()))]);
        assert!(h.view.is_composing());
        assert_eq!(h.document.text(), "before 原 after");
        assert_eq!(h.document.revision(), Revision(1));
        assert_eq!(h.document.selection(), selection);
        assert!(output.platform_output.ime.is_some());
    }
    h.frame(vec![
        ime(ImeEvent::Commit("你".into())),
        key(Key::Enter),
        Event::Text("你".into()),
    ]);
    assert_eq!(h.document.text(), "before 你 after");
    assert_eq!(h.document.revision(), Revision(2));
    assert!(!h.view.is_composing());
    h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(h.document.text(), "before 原 after");
    assert_eq!(h.document.selection(), selection);
    assert!(!h.document.can_undo());
}

#[test]
fn cancelled_or_stale_ime_never_deletes_original_text_or_replays_a_late_commit() {
    for cancellation in [ime(ImeEvent::Disabled), key(Key::Escape)] {
        let mut h = Harness::new("原文");
        h.document
            .set_selection(Selection { anchor: 0, head: 3 })
            .unwrap();
        h.frame(vec![ime(ImeEvent::Preedit("ni".into()))]);
        h.frame(vec![cancellation, ime(ImeEvent::Commit("你".into()))]);
        assert_eq!(h.document.text(), "原文");
        assert!(!h.document.can_undo());
    }
    let mut h = Harness::new("原文");
    h.frame(vec![ime(ImeEvent::Preedit("ni".into()))]);
    h.document.replace(0..0, "new ").unwrap();
    h.frame(vec![ime(ImeEvent::Commit("你".into()))]);
    assert_eq!(h.document.text(), "new 原文");
    h.frame(vec![
        ime(ImeEvent::Preedit("hao".into())),
        ime(ImeEvent::Commit("好".into())),
    ]);
    assert_eq!(h.document.text(), "new 好原文");
}

#[test]
fn empty_preedit_and_focus_loss_preserve_core_text() {
    let mut h = Harness::new("原文");
    h.document
        .set_selection(Selection { anchor: 0, head: 3 })
        .unwrap();
    h.frame(vec![
        ime(ImeEvent::Preedit("ni".into())),
        ime(ImeEvent::Preedit(String::new())),
    ]);
    assert_eq!(h.document.text(), "原文");
    h.frame(vec![Event::WindowFocused(false)]);
    assert!(!h.view.is_composing());
    h.frame(vec![ime(ImeEvent::Commit("你".into()))]);
    assert_eq!(h.document.text(), "原文");
    assert_eq!(h.document.revision(), Revision(1));
}

#[test]
fn ordinary_text_surrounding_an_ime_session_in_one_frame_is_not_lost() {
    let mut h = Harness::new("");
    h.frame(vec![
        Event::Text("prefix ".into()),
        ime(ImeEvent::Enabled),
        ime(ImeEvent::Preedit("ni".into())),
        key(Key::Enter),
        ime(ImeEvent::Commit("你".into())),
        ime(ImeEvent::Disabled),
        Event::Text("好".into()),
        Event::Paste(" suffix".into()),
    ]);
    assert_eq!(h.document.text(), "prefix 你好 suffix");
    assert!(!h.view.is_composing());
    assert_eq!(h.document.selection().head, h.document.text().len());
}

#[test]
fn pointer_hit_testing_snaps_to_a_grapheme_boundary() {
    let mut h = Harness::new("👩🏽‍💻e\u{301}尾");
    let output = h.frame(vec![]);
    let rect = output.platform_output.ime.unwrap().rect;
    // Probe many x positions through the compound glyph sequence. All clicks
    // must land on one of the extended cluster boundaries, including drag start.
    let mut heads = std::collections::BTreeSet::new();
    for x in (2..150).step_by(3) {
        h.time += 0.6;
        let position = rect.min + egui::vec2(x as f32, 14.0);
        h.frame(vec![
            Event::PointerMoved(position),
            Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        h.frame(vec![Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        let selection = h.document.selection();
        heads.insert(selection.head);
        assert_eq!(grapheme::selection(h.document.text(), selection), selection);
    }
    assert!(heads.contains(&0));
    assert!(heads.contains(&"👩🏽‍💻".len()));
    assert!(heads.contains(&h.document.text().len()));
}

#[test]
fn vertical_navigation_retains_the_preferred_column_across_short_lines() {
    let mut h = Harness::new("Ae\u{301}Z\nx\nBe\u{301}Q");
    h.caret("Ae\u{301}".len());
    h.frame(vec![key(Key::ArrowDown)]);
    assert_eq!(h.document.selection().head, "Ae\u{301}Z\nx".len());
    h.frame(vec![key(Key::ArrowDown)]);
    assert_eq!(
        h.document.selection().head,
        "Ae\u{301}Z\nx\nBe\u{301}".len()
    );
    h.frame(vec![key(Key::ArrowUp), key(Key::ArrowUp)]);
    assert_eq!(h.document.selection().head, "Ae\u{301}".len());
}

#[test]
fn word_navigation_deletion_and_plugin_selections_keep_whole_clusters() {
    let mut h = Harness::new("one e\u{301} 👩🏽‍💻 🇨🇳 尾");
    let ctrl = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    for direction in [Key::ArrowRight, Key::ArrowLeft] {
        for _ in 0..8 {
            h.frame(vec![key_with(direction, ctrl)]);
            assert_eq!(
                grapheme::selection(h.document.text(), h.document.selection()),
                h.document.selection()
            );
        }
    }
    h.caret(h.document.text().len());
    for _ in 0..40 {
        if h.document.text().is_empty() {
            break;
        }
        let previous = h.document.selection().head;
        h.frame(vec![key_with(Key::Backspace, ctrl)]);
        assert!(h.document.selection().head < previous);
        assert_eq!(
            grapheme::selection(h.document.text(), h.document.selection()),
            h.document.selection()
        );
    }
    assert!(h.document.text().is_empty());
    // ABI byte endpoints are allowed inside a grapheme; the UI expands that
    // selection before exposing it to clipboard or user edits.
    let mut h = Harness::new("Ae\u{301}Z");
    h.document
        .set_selection(Selection { anchor: 4, head: 2 })
        .unwrap();
    let output = h.frame(vec![Event::Cut]);
    assert!(
        output
            .platform_output
            .commands
            .contains(&egui::OutputCommand::CopyText("e\u{301}".into()))
    );
    assert_eq!(h.document.text(), "AZ");
    h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(h.document.selection(), Selection { anchor: 4, head: 1 });
}

#[test]
fn caret_and_hit_testing_use_logical_coordinates_at_multiple_scales() {
    for scale in [1.0, 1.5, 2.0] {
        let mut h = Harness::new("Ae\u{301}Z");
        h.ctx.set_pixels_per_point(scale);
        h.caret(1);
        let before = h.frame(vec![]).platform_output.ime.unwrap().cursor_rect;
        let output = h.frame(vec![key(Key::ArrowRight)]);
        assert_eq!(output.pixels_per_point, scale);
        let after = output.platform_output.ime.unwrap().cursor_rect;
        assert_eq!(h.document.selection().head, "Ae\u{301}".len());
        assert!(after.min.x > before.min.x);
        assert!(after.is_finite());
        let pos = after.center();
        h.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        assert_eq!(h.document.selection().head, "Ae\u{301}".len());
    }
}

#[test]
fn oversized_ime_commit_is_atomic_and_does_not_leave_preedit_in_history() {
    let mut h = Harness::new("keep 你好");
    let selection = Selection {
        anchor: 0,
        head: h.document.text().len(),
    };
    h.document.set_selection(selection).unwrap();
    h.frame(vec![ime(ImeEvent::Preedit("ni".into()))]);
    h.frame(vec![ime(ImeEvent::Commit(
        "x".repeat(MAX_DOCUMENT_BYTES + 1),
    ))]);
    assert_eq!(h.error.as_deref(), Some("TooLarge"));
    assert!(!h.view.is_composing());
    assert_eq!(h.document.text(), "keep 你好");
    assert_eq!(h.document.selection(), selection);
    assert!(!h.document.can_undo());
}

#[test]
fn randomized_input_batches_match_an_independent_grapheme_reference() {
    let mut h = Harness::new("");
    let mut reference = String::new();
    let mut caret = 0;
    let mut seed = 29_u64;
    for _ in 0..40 {
        let mut events = Vec::new();
        for _ in 0..13 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let bounds: Vec<_> = reference
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(reference.len()))
                .collect();
            let previous = bounds
                .iter()
                .copied()
                .filter(|&i| i < caret)
                .next_back()
                .unwrap_or(0);
            let next = bounds
                .iter()
                .copied()
                .find(|&i| i > caret)
                .unwrap_or(reference.len());
            match seed % 7 {
                0 => {
                    events.push(key(Key::ArrowLeft));
                    caret = previous;
                }
                1 => {
                    events.push(key(Key::ArrowRight));
                    caret = next;
                }
                2 => {
                    events.push(key(Key::Backspace));
                    reference.replace_range(previous..caret, "");
                    caret = previous;
                }
                3 => {
                    events.push(key(Key::Delete));
                    reference.replace_range(caret..next, "");
                }
                _ => {
                    let text = ["中", "e\u{301}", "👩🏽‍💻", "🇨🇳", "\r\n", "\u{301}", "a"]
                        [(seed as usize >> 24) % 7];
                    events.push(Event::Paste(text.into()));
                    reference.insert_str(caret, text);
                    caret += text.len();
                }
            }
            caret = reference
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain(std::iter::once(reference.len()))
                .find(|&i| i >= caret)
                .unwrap();
        }
        h.frame(events);
        assert!(h.error.is_none());
        assert_eq!(h.document.text(), reference);
        assert_eq!(
            h.document.selection(),
            Selection {
                anchor: caret,
                head: caret
            }
        );
    }
}
