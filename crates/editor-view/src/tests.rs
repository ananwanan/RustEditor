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
    scroll_offset: Option<egui::Vec2>,
    focused: bool,
    max_texture_side: Option<usize>,
}

impl Harness {
    fn new(text: &str) -> Self {
        let mut harness = Self {
            ctx: egui::Context::default(),
            view: EditorView::default(),
            document: Document::new(DocumentId(1), text).unwrap(),
            time: 0.0,
            error: None,
            scroll_offset: None,
            focused: true,
            max_texture_side: None,
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
                focused: self.focused,
                time: Some(self.time),
                events,
                max_texture_side: self.max_texture_side,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]);
                    if let Some(offset) = self.scroll_offset {
                        scroll = scroll.scroll_offset(offset);
                    }
                    scroll.show(ui, |ui| {
                        let output = self
                            .view
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
        h.frame(vec![cancellation]);
        h.frame(vec![ime(ImeEvent::Commit("你".into()))]);
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
fn cancelling_before_nonempty_preedit_rejects_late_commits() {
    for cancellation in [
        key(Key::Escape),
        Event::WindowFocused(false),
        ime(ImeEvent::Disabled),
    ] {
        let mut h = Harness::new("原文");
        let selection = Selection { anchor: 3, head: 0 };
        h.document.set_selection(selection).unwrap();
        h.frame(vec![
            ime(ImeEvent::Enabled),
            ime(ImeEvent::Preedit(String::new())),
        ]);
        h.frame(vec![cancellation]);
        h.view.request_focus();
        h.frame(vec![ime(ImeEvent::Commit("你".into()))]);
        assert_eq!(h.document.text(), "原文");
        assert_eq!(h.document.selection(), selection);
        assert_eq!(h.document.revision(), Revision(1));
        assert!(!h.document.can_undo());

        // A new platform session may commit directly without a visible preedit.
        h.view.request_focus();
        h.frame(vec![
            ime(ImeEvent::Enabled),
            ime(ImeEvent::Commit("好".into())),
        ]);
        assert_eq!(h.document.text(), "好文");
        assert_eq!(h.document.revision(), Revision(2));
        h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
        assert_eq!(h.document.text(), "原文");
        assert_eq!(h.document.selection(), selection);
    }
}

#[test]
fn bare_ime_commits_work_after_unfocused_frames_and_normal_focus_changes() {
    let mut h = Harness::new("原文");
    h.focused = false;
    h.frame(vec![]);
    h.focused = true;
    h.view.request_focus();
    h.frame(vec![ime(ImeEvent::Commit("你".into()))]);
    assert_eq!(h.document.text(), "你原文");
    assert_eq!(h.document.revision(), Revision(2));

    // Disabled follows a successful native commit; it is not cancellation.
    h.frame(vec![ime(ImeEvent::Disabled)]);
    h.frame(vec![Event::WindowFocused(false)]);
    h.view.request_focus();
    h.frame(vec![ime(ImeEvent::Commit("好".into()))]);
    assert_eq!(h.document.text(), "你好原文");
    assert_eq!(h.document.revision(), Revision(3));
}

#[test]
fn native_preedit_clear_before_commit_preserves_the_composition_range() {
    for preedit in [false, true] {
        let mut h = Harness::new("原文");
        h.document
            .set_selection(Selection { anchor: 3, head: 0 })
            .unwrap();
        if preedit {
            h.frame(vec![ime(ImeEvent::Preedit("ni".into()))]);
            // Selection changes do not invalidate the original preedit range.
            h.document
                .set_selection(Selection { anchor: 3, head: 6 })
                .unwrap();
        }
        h.frame(vec![
            ime(ImeEvent::Disabled),
            ime(ImeEvent::Commit("你".into())),
            ime(ImeEvent::Disabled),
        ]);
        assert_eq!(h.document.text(), "你文");
        assert_eq!(h.document.revision(), Revision(2));
        assert!(!h.view.is_composing());
        assert!(!h.view.ime_session_active);
        h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
        assert_eq!(h.document.text(), "原文");
        assert!(!h.document.can_undo());
    }
}

#[test]
fn native_preedit_clear_does_not_revive_a_cancelled_session() {
    let mut h = Harness::new("原文");
    h.frame(vec![ime(ImeEvent::Preedit("ni".into()))]);
    h.frame(vec![Event::WindowFocused(false)]);
    h.frame(vec![
        ime(ImeEvent::Disabled),
        ime(ImeEvent::Commit("你".into())),
        ime(ImeEvent::Disabled),
    ]);
    assert_eq!(h.document.text(), "原文");
    assert_eq!(h.document.revision(), Revision(1));
    assert!(!h.document.can_undo());
}

#[test]
fn pointer_cancels_an_observed_session_before_nonempty_preedit() {
    let mut h = Harness::new("原文");
    let output = h.frame(vec![
        ime(ImeEvent::Enabled),
        ime(ImeEvent::Preedit(String::new())),
    ]);
    assert!(h.view.ime_session_active);
    let caret = output.platform_output.ime.unwrap().cursor_rect;
    let pos = egui::pos2(caret.min.x + 120.0, caret.center().y);
    h.frame(vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]);
    assert_eq!(h.document.selection().head, h.document.text().len());
    h.frame(vec![
        Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
        ime(ImeEvent::Commit("你".into())),
    ]);
    assert_eq!(h.document.text(), "原文");
    assert_eq!(h.document.revision(), Revision(1));
    assert!(!h.document.can_undo());
    assert!(!h.view.ime_session_active);
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
    let caret = output.platform_output.ime.unwrap().cursor_rect;
    // Probe many x positions through the compound glyph sequence. All clicks
    // must land on one of the extended cluster boundaries, including drag start.
    let mut heads = std::collections::BTreeSet::new();
    for x in (2..150).step_by(3) {
        h.time += 0.6;
        let position = egui::pos2(caret.min.x + x as f32, caret.center().y);
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
fn native_ime_area_tracks_the_caret_after_scrolling_at_multiple_scales() {
    let line = format!("{}中e\u{301}\n", "abcdef".repeat(30));
    for scale in [1.0, 1.5, 2.0] {
        let mut h = Harness::new(&line.repeat(80));
        h.ctx.set_pixels_per_point(scale);
        h.caret(20 * line.len() + 60);
        let before = h.frame(vec![]).platform_output.ime.unwrap();
        assert_eq!(before.rect, before.cursor_rect);
        assert!(before.rect.height() < 40.0);

        h.scroll_offset = Some(egui::vec2(48.0, 128.0));
        let after = h.frame(vec![]).platform_output.ime.unwrap();
        assert_eq!(after.rect, after.cursor_rect);
        let movement = before.rect.min - after.rect.min;
        assert!((movement.x - 48.0).abs() < 0.1, "{movement:?}");
        assert!((movement.y - 128.0).abs() < 0.1, "{movement:?}");
        assert!(after.rect.is_finite());
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

fn text_shape(output: &egui::FullOutput) -> &egui::epaint::TextShape {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        })
        .expect("the nonempty editor paints a text shape")
}

fn assert_candidate_matches_display(
    ctx: &egui::Context,
    output: &egui::FullOutput,
    selection: Selection,
) {
    let text = text_shape(output);
    let range = cursors(text.galley.text(), selection, None);
    let font = &text.galley.job.sections[0].format.font_id;
    let row_height = ctx.fonts_mut(|fonts| fonts.row_height(font));
    let expected =
        cursor_rect(&text.galley, &range.primary, row_height).translate(text.pos.to_vec2());
    let ime = output.platform_output.ime.expect("focused candidate area");
    assert_eq!(ime.rect, ime.cursor_rect);
    assert_eq!(ime.cursor_rect, expected);
}

#[test]
fn rendered_text_and_navigation_follow_each_edit_in_a_batch() {
    let mut h = Harness::new("start");
    h.caret(5);
    let output = h.frame(vec![
        Event::Paste("\n中e\u{301}TAIL".into()),
        key(Key::Home),
        key(Key::ArrowRight),
        key(Key::Delete),
        key(Key::End),
        Event::Text("!".into()),
    ]);
    assert_eq!(h.document.text(), "start\n中TAIL!");
    assert_eq!(text_shape(&output).galley.text(), h.document.text());
    assert_eq!(h.document.selection().head, h.document.text().len());
    assert_candidate_matches_display(&h.ctx, &output, h.document.selection());

    let output = h.frame(vec![
        key_with(Key::Z, Modifiers::COMMAND),
        key(Key::Home),
        Event::Text("前".into()),
        key(Key::ArrowDown),
        key_with(Key::Y, Modifiers::COMMAND),
    ]);
    assert_eq!(h.document.text(), "start\n前中TAIL");
    assert_eq!(text_shape(&output).galley.text(), h.document.text());
    assert_candidate_matches_display(&h.ctx, &output, h.document.selection());
}

#[test]
fn alternating_edits_keep_egui_document_and_paragraph_cache_entries_warm() {
    let text = "hello 中e\u{301}\nsecond line\n尾";
    let mut h = Harness::new(text);
    h.caret(text.len());
    let initial = h.frame(vec![]);
    let original = text_shape(&initial).galley.clone();
    let mut edited = None;
    for cycle in 0..3 {
        let inserted = h.frame(vec![Event::Text("x".into())]);
        let inserted_galley = text_shape(&inserted).galley.clone();
        assert_eq!(inserted_galley.text(), format!("{text}x"));
        if let Some(previous) = &edited {
            assert!(Arc::ptr_eq(previous, &inserted_galley), "cycle {cycle}");
        }
        edited = Some(inserted_galley);

        let deleted = h.frame(vec![key(Key::Backspace)]);
        assert_eq!(h.document.text(), text);
        assert_eq!(text_shape(&deleted).galley.text(), text);
        assert!(Arc::ptr_eq(&original, &text_shape(&deleted).galley));
        assert_candidate_matches_display(&h.ctx, &deleted, h.document.selection());
    }
}

#[test]
fn selection_paint_does_not_contaminate_later_cursor_or_document_frames() {
    let mut h = Harness::new("Ae\u{301}中Z");
    let mut visuals = h.ctx.style().visuals.clone();
    let highlight = Color32::from_rgb(23, 45, 67);
    visuals.selection.bg_fill = highlight;
    h.ctx.set_visuals(visuals);
    let baseline = h.frame(vec![]);
    let original = text_shape(&baseline).galley.clone();
    let original_rows = original.rows.clone();
    h.document
        .set_selection(Selection { anchor: 1, head: 7 })
        .unwrap();
    let selected = h.frame(vec![]);
    let stable_selected = h.frame(vec![]);
    assert!(Arc::ptr_eq(
        &text_shape(&selected).galley,
        &text_shape(&stable_selected).galley,
    ));
    let context = h.ctx.clone();
    let contains_highlight = |output: &egui::FullOutput| {
        context
            .tessellate(output.shapes.clone(), output.pixels_per_point)
            .iter()
            .any(|primitive| match &primitive.primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    mesh.vertices.iter().any(|vertex| vertex.color == highlight)
                }
                egui::epaint::Primitive::Callback(_) => false,
            })
    };
    assert!(contains_highlight(&selected));
    assert_eq!(original.rows, original_rows);
    assert_eq!(text_shape(&selected).galley.text(), h.document.text());
    assert_candidate_matches_display(&h.ctx, &selected, h.document.selection());

    h.caret(1);
    let unselected = h.frame(vec![]);
    assert!(!contains_highlight(&unselected));
    assert_eq!(text_shape(&unselected).galley.rows, original_rows);
    assert_candidate_matches_display(&h.ctx, &unselected, h.document.selection());

    h.document = Document::new(DocumentId(2), "different").unwrap();
    let replaced = h.frame(vec![]);
    assert_eq!(h.document.revision(), Revision(1));
    assert_eq!(text_shape(&replaced).galley.text(), "different");
    assert!(!contains_highlight(&replaced));
    h.document.replace(0..9, "revised").unwrap();
    let revised = h.frame(vec![]);
    assert_eq!(text_shape(&revised).galley.text(), "revised");
}

#[test]
fn rendered_ime_overlay_observes_repeated_preedit_clear_commit_and_cancel() {
    for native_clear in [
        ime(ImeEvent::Disabled),
        ime(ImeEvent::Preedit(String::new())),
    ] {
        let mut h = Harness::new("L原R");
        let selection = Selection { anchor: 4, head: 1 };
        h.document.set_selection(selection).unwrap();
        for preedit in ["ni", "ni", "nih"] {
            let output = h.frame(vec![ime(ImeEvent::Preedit(preedit.into()))]);
            assert_eq!(text_shape(&output).galley.text(), format!("L{preedit}R"));
            assert_eq!(h.document.text(), "L原R");
            assert_eq!(h.document.revision(), Revision(1));
            assert_candidate_matches_display(
                &h.ctx,
                &output,
                Selection {
                    anchor: 1,
                    head: 1 + preedit.len(),
                },
            );
        }
        let output = h.frame(vec![
            native_clear,
            ime(ImeEvent::Commit("你".into())),
            ime(ImeEvent::Disabled),
        ]);
        assert_eq!(text_shape(&output).galley.text(), "L你R");
        assert_eq!(h.document.revision(), Revision(2));
        assert_candidate_matches_display(&h.ctx, &output, h.document.selection());
        let output = h.frame(vec![key_with(Key::Z, Modifiers::COMMAND)]);
        assert_eq!(text_shape(&output).galley.text(), "L原R");
        assert_eq!(h.document.selection(), selection);
        assert!(!h.document.can_undo());
        h.frame(vec![ime(ImeEvent::Preedit("hao".into()))]);
        let output = h.frame(vec![key(Key::Escape)]);
        assert_eq!(text_shape(&output).galley.text(), "L原R");
        let output = h.frame(vec![
            ime(ImeEvent::Disabled),
            ime(ImeEvent::Commit("好".into())),
            ime(ImeEvent::Disabled),
        ]);
        assert_eq!(text_shape(&output).galley.text(), "L原R");
        assert_eq!(h.document.selection(), selection);
        assert_eq!(h.document.revision(), Revision(3));
    }
}

#[test]
fn display_and_candidate_geometry_refresh_after_font_scale_theme_and_atlas_changes() {
    let mut h = Harness::new("MMMMiiiW");
    h.caret(h.document.text().len());
    let initial = h.frame(vec![]);
    let initial_width = text_shape(&initial).galley.size().x;

    let mut style = (*h.ctx.style()).clone();
    style.override_font_id = Some(FontId::monospace(26.0));
    h.ctx.set_style(style);
    let larger = h.frame(vec![]);
    assert_eq!(
        text_shape(&larger).galley.job.sections[0]
            .format
            .font_id
            .size,
        26.0
    );
    assert!(text_shape(&larger).galley.size().x > initial_width);
    assert_candidate_matches_display(&h.ctx, &larger, h.document.selection());

    let mut definitions = egui::FontDefinitions::default();
    definitions.families.insert(
        egui::FontFamily::Monospace,
        definitions.families[&egui::FontFamily::Proportional].clone(),
    );
    h.ctx.set_fonts(definitions);
    let redefined = h.frame(vec![]);
    assert_ne!(
        text_shape(&redefined).galley.rows,
        text_shape(&larger).galley.rows
    );
    assert_candidate_matches_display(&h.ctx, &redefined, h.document.selection());

    for scale in [1.0, 1.5, 2.0] {
        h.ctx.set_pixels_per_point(scale);
        let output = h.frame(vec![]);
        assert_eq!(output.pixels_per_point, scale);
        assert_eq!(text_shape(&output).galley.pixels_per_point, scale);
        assert_candidate_matches_display(&h.ctx, &output, h.document.selection());
    }

    let color = Color32::from_rgb(11, 133, 211);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(color);
    h.ctx.set_visuals(visuals);
    let themed = h.frame(vec![]);
    assert_eq!(
        text_shape(&themed).galley.job.sections[0].format.color,
        color
    );
    assert_candidate_matches_display(&h.ctx, &themed, h.document.selection());

    // Changing the atlas limit rebuilds Fonts without changing its definitions.
    let before_reset = text_shape(&themed).galley.clone();
    h.max_texture_side = Some(4096);
    let atlas_reset = h.frame(vec![]);
    assert!(!Arc::ptr_eq(
        &before_reset,
        &text_shape(&atlas_reset).galley
    ));
    assert_candidate_matches_display(&h.ctx, &atlas_reset, h.document.selection());

    let before_alpha = text_shape(&atlas_reset).galley.clone();
    h.ctx.style_mut(|style| {
        style.visuals.text_alpha_from_coverage = egui::epaint::AlphaFromCoverage::Gamma(0.7);
    });
    let alpha_reset = h.frame(vec![]);
    assert!(!Arc::ptr_eq(
        &before_alpha,
        &text_shape(&alpha_reset).galley
    ));
    assert_candidate_matches_display(&h.ctx, &alpha_reset, h.document.selection());

    h.ctx = egui::Context::default();
    h.view.request_focus();
    let other_context = h.frame(vec![]);
    assert_eq!(text_shape(&other_context).galley.pixels_per_point, 1.0);
    assert_eq!(
        text_shape(&other_context).galley.job.sections[0]
            .format
            .font_id
            .size,
        16.0
    );
    assert_candidate_matches_display(&h.ctx, &other_context, h.document.selection());
}

#[test]
fn selected_text_refreshes_when_only_selection_theme_changes() {
    let mut h = Harness::new("selected");
    h.document
        .set_selection(Selection { anchor: 0, head: 8 })
        .unwrap();
    let before = h.frame(vec![]);
    let old_color = text_shape(&before).galley.job.sections[0].format.color;
    let background = Color32::from_rgb(31, 47, 61);
    let foreground = Color32::from_rgb(71, 89, 103);
    h.ctx.style_mut(|style| {
        style.visuals.selection.bg_fill = background;
        style.visuals.selection.stroke.color = foreground;
    });
    let after = h.frame(vec![]);
    assert_eq!(
        text_shape(&after).galley.job.sections[0].format.color,
        old_color
    );
    let primitives = h
        .ctx
        .tessellate(after.shapes.clone(), after.pixels_per_point);
    for color in [background, foreground] {
        assert!(
            primitives
                .iter()
                .any(|primitive| match &primitive.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => {
                        mesh.vertices.iter().any(|vertex| vertex.color == color)
                    }
                    egui::epaint::Primitive::Callback(_) => false,
                })
        );
    }
    assert_candidate_matches_display(&h.ctx, &after, h.document.selection());
}
