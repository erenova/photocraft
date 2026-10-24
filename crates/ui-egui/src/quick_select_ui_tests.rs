//! Quick Selection grows while dragging (#9145): harness tests through the tool events the canvas
//! and the control channel send.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::*;
use crate::canvas::{ToolEvent, tool_event};

const RED: [f32; 4] = [0.85, 0.15, 0.1, 1.0];
const BLUE: [f32; 4] = [0.1, 0.25, 0.8, 1.0];
const W: i32 = 1600;
const H: i32 = 600;
const SPLIT: i32 = 1200;

/// A `W` × `H` document, red left of x = `SPLIT` and blue right of it, with the Quick Selection
/// tool and a 20 px brush. The red part is far too big for a stroke to take whole, so a drag
/// grows the selection along its path.
fn quick_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": W, "height": H})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, SPLIT, H), &RED);
            s.fill_rect(Rect::new(SPLIT, 0, W, H), &BLUE);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::QuickSelection;
    app.session.tools.brush.size = 20.0;
    app
}

/// Selected pixels (coverage ≥ ½) of `sel`.
fn area(sel: Option<&Surface>) -> usize {
    let Some(s) = sel else { return 0 };
    let mut v = Vec::new();
    s.read_region_into(Rect::new(0, 0, W, H), &mut v);
    v.iter().filter(|c| **c >= 0.5).count()
}

/// The selection as 8-bit coverage, to compare exactly.
fn bytes(sel: Option<&Surface>) -> Vec<u8> {
    let Some(s) = sel else { return vec![0; (W * H) as usize] };
    let mut v = Vec::new();
    s.read_region_into(Rect::new(0, 0, W, H), &mut v);
    v.iter().map(|c| (c * 255.0).round() as u8).collect()
}

fn doc_selection(app: &PhotocraftApp) -> Option<Surface> {
    app.session.active().unwrap().doc.selection.clone()
}

fn preview(app: &PhotocraftApp) -> Option<Surface> {
    let doc = app.session.active().unwrap().doc.id;
    shown(app, doc).expect("a stroke is under way").0
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
}

fn drag(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}

fn up(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Up { x, y }, m);
}

/// The last journal entry, which must be a `select.quick`.
fn last_quick(app: &PhotocraftApp) -> Value {
    let (id, p) = app.session.journal.last().expect("a journal entry").clone();
    assert_eq!(id, "select.quick");
    p
}

/// The selection `params` make on a fresh copy of the document with `before` selected.
fn replay(before: Option<&Value>, params: &Value) -> Vec<u8> {
    let mut a = quick_app();
    if let Some(p) = before {
        a.session.execute("select.rect", p.clone()).unwrap();
    }
    a.session.execute("select.quick", params.clone()).unwrap();
    bytes(doc_selection(&a).as_ref())
}

#[test]
fn the_selection_grows_while_dragging_and_release_commits_what_was_shown() {
    for threaded in [false, true] {
        let mut app = quick_app();
        app.background_jobs = threaded;
        let steps = app.session.active().unwrap().history.past_len();
        down(&mut app, 100.0, 300.0, Modifiers::NONE);
        if threaded {
            settle(&mut app);
        }
        let first = area(preview(&app).as_ref());
        assert!(first > 0, "threaded {threaded}: the press selects at once, before any release");
        assert_eq!(doc_selection(&app), None, "threaded {threaded}: nothing is committed while the button is held");
        let mut grown = Vec::new();
        for k in 1..=8 {
            drag(&mut app, 100.0 + 40.0 * f64::from(k), 300.0, Modifiers::NONE);
            if threaded {
                assert_eq!(progress(&app).map(|p| p.1), Some(true), "the segment grows on a worker, not in the event");
                settle(&mut app);
            }
            grown.push(area(preview(&app).as_ref()));
        }
        assert!(grown[3] > first && grown[7] > grown[3], "threaded {threaded}: the selection grows with the drag: {first} {grown:?}");
        assert!(
            bytes(preview(&app).as_ref()).chunks(W as usize).all(|row| row[SPLIT as usize..].iter().all(|c| *c == 0)),
            "threaded {threaded}: the edge holds"
        );
        assert!(preview(&app).is_some_and(|s| s.sample_channel(100, 20, 0) < 0.5), "threaded {threaded}: the red part is not taken whole");
        let last = bytes(preview(&app).as_ref());
        // Released where the last move was: the release adds no point, so nothing grows after it.
        up(&mut app, 420.0, 300.0, Modifiers::NONE);
        if threaded {
            settle(&mut app);
        }
        assert!(app.quick_live.is_none(), "threaded {threaded}: the stroke is committed once caught up");
        let p = last_quick(&app);
        assert!(p["segments"].as_array().is_some_and(|s| s.len() > 1), "threaded {threaded}: {p}");
        let committed = bytes(doc_selection(&app).as_ref());
        assert_eq!(committed, last, "threaded {threaded}: released == last preview");
        assert_eq!(replay(None, &p), committed, "threaded {threaded}: the journal replays to the same selection");
        // One history step for the whole stroke.
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Quick Selection"));
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "threaded {threaded}: one step for the stroke");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(doc_selection(&app).map(|s| area(Some(&s))).unwrap_or(0), 0, "threaded {threaded}: undo removes the whole stroke");
    }
}

#[test]
fn new_replaces_once_then_the_tool_switches_to_add() {
    let mut app = quick_app();
    let rect = json!({"x": 1300, "y": 20, "width": 40, "height": 40});
    app.run("select.rect", rect.clone()).unwrap();
    assert_eq!(app.ui.quick_selection_mode, NEW, "New is the default");
    down(&mut app, 40.0, 300.0, Modifiers::NONE);
    drag(&mut app, 80.0, 300.0, Modifiers::NONE);
    up(&mut app, 100.0, 300.0, Modifiers::NONE);
    let p = last_quick(&app);
    assert_eq!(p["mode"], "replace");
    let sel = doc_selection(&app);
    assert!(sel.as_ref().is_some_and(|s| s.sample_channel(1320, 40, 0) < 0.5), "New replaced the rectangle");
    assert!(sel.as_ref().is_some_and(|s| s.sample_channel(80, 300, 0) >= 0.5), "and kept every segment of its stroke");
    assert_eq!(replay(Some(&rect), &p), bytes(sel.as_ref()));
    assert_eq!(app.ui.quick_selection_mode, ADD, "after the first stroke the tool is on Add");
    // The next stroke adds.
    down(&mut app, 40.0, 100.0, Modifiers::NONE);
    up(&mut app, 60.0, 100.0, Modifiers::NONE);
    assert_eq!(last_quick(&app)["mode"], "add");
    let sel = doc_selection(&app);
    assert!(sel.as_ref().is_some_and(|s| s.sample_channel(80, 300, 0) >= 0.5 && s.sample_channel(50, 100, 0) >= 0.5));
}

#[test]
fn alt_subtracts_pressed_before_or_during_the_drag_and_shift_adds() {
    let rect = json!({"x": 0, "y": 0, "width": SPLIT, "height": H});
    // ⌥ held at the press.
    let mut app = quick_app();
    app.ui.quick_selection_mode = ADD;
    app.run("select.rect", rect.clone()).unwrap();
    down(&mut app, 60.0, 300.0, Modifiers::ALT);
    drag(&mut app, 100.0, 300.0, Modifiers::ALT);
    up(&mut app, 120.0, 300.0, Modifiers::ALT);
    assert_eq!(last_quick(&app)["mode"], "subtract");
    assert!(doc_selection(&app).is_some_and(|s| s.sample_channel(100, 300, 0) < 0.5 && s.sample_channel(10, 10, 0) >= 0.5));
    assert_eq!(app.ui.quick_selection_mode, ADD, "⌥ was temporary");
    // ⌥ pressed after the button went down (a window manager takes ⌥-press-drag, #9145).
    let mut app = quick_app();
    app.ui.quick_selection_mode = ADD;
    app.run("select.rect", rect.clone()).unwrap();
    down(&mut app, 60.0, 300.0, Modifiers::NONE);
    drag(&mut app, 80.0, 300.0, Modifiers::NONE);
    drag(&mut app, 120.0, 300.0, Modifiers::ALT);
    drag(&mut app, 160.0, 300.0, Modifiers::ALT);
    let shown = preview(&app);
    assert!(shown.as_ref().is_some_and(|s| s.sample_channel(150, 300, 0) < 0.5), "the outline shows the subtraction during the drag");
    up(&mut app, 170.0, 300.0, Modifiers::ALT);
    let p = last_quick(&app);
    assert_eq!(p["mode"], "add");
    assert!(p["segments"].as_array().unwrap().iter().any(|s| s["mode"] == "subtract"), "{p}");
    let sel = doc_selection(&app);
    assert!(sel.as_ref().is_some_and(|s| s.sample_channel(150, 300, 0) < 0.5 && s.sample_channel(10, 10, 0) >= 0.5));
    assert_eq!(replay(Some(&rect), &p), bytes(sel.as_ref()));
    // The Subtract button with no key held.
    let mut app = quick_app();
    app.run("select.rect", rect.clone()).unwrap();
    app.ui.quick_selection_mode = SUBTRACT;
    down(&mut app, 60.0, 300.0, Modifiers::NONE);
    up(&mut app, 120.0, 300.0, Modifiers::NONE);
    assert_eq!(last_quick(&app)["mode"], "subtract");
    assert!(doc_selection(&app).is_some_and(|s| s.sample_channel(90, 300, 0) < 0.5 && s.sample_channel(10, 10, 0) >= 0.5));
    assert_eq!(app.ui.quick_selection_mode, SUBTRACT, "the Subtract button stays chosen");
    // ⇧ adds, even on New.
    let mut app = quick_app();
    app.run("select.rect", json!({"x": 1300, "y": 20, "width": 40, "height": 40})).unwrap();
    down(&mut app, 60.0, 300.0, Modifiers::SHIFT);
    up(&mut app, 100.0, 300.0, Modifiers::SHIFT);
    assert_eq!(last_quick(&app)["mode"], "add");
    assert!(doc_selection(&app).is_some_and(|s| s.sample_channel(1320, 40, 0) >= 0.5 && s.sample_channel(80, 300, 0) >= 0.5));
}

#[test]
fn a_command_during_the_catch_up_commits_the_stroke_first() {
    let mut app = quick_app();
    app.background_jobs = true;
    down(&mut app, 40.0, 300.0, Modifiers::NONE);
    for k in 1..=4 {
        drag(&mut app, 40.0 + 30.0 * f64::from(k), 300.0, Modifiers::NONE);
    }
    up(&mut app, 180.0, 300.0, Modifiers::NONE);
    // Whatever is still growing, the next command comes after the stroke.
    app.run("select.inverse", json!({})).unwrap();
    let ids: Vec<&str> = app.session.journal.iter().rev().take(2).map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["select.inverse", "select.quick"]);
    assert!(app.quick_live.is_none());
}

#[test]
fn hostile_pointer_input_never_panics() {
    for threaded in [false, true] {
        for size in [0.0, 1.0, 1e9, f32::NAN, f32::INFINITY, -3.0] {
            let mut app = quick_app();
            app.background_jobs = threaded;
            app.session.tools.brush.size = size;
            down(&mut app, f64::NAN, 5.0, Modifiers::NONE);
            for (x, y) in [(f64::INFINITY, 1.0), (-1e300, 1e300), (1e9, -1e9), (f64::NAN, f64::NAN), (10.0, 10.0), (-5.0, -5.0)] {
                drag(&mut app, x, y, Modifiers::ALT);
            }
            up(&mut app, f64::NEG_INFINITY, 0.0, Modifiers::NONE);
            if threaded {
                settle(&mut app);
            }
            flush(&mut app);
        }
    }
    // 100 000 points in one drag: one segment, grown once.
    let mut app = quick_app();
    down(&mut app, 10.0, 10.0, Modifiers::NONE);
    if let Some(d) = app.drag.as_mut() {
        d.points.extend((0..100_000).map(|i| [f64::from(i % 400), f64::from((i / 400) % 240), 1.0]));
    }
    pump(&mut app);
    up(&mut app, 30.0, 30.0, Modifiers::NONE);
    assert!(app.quick_live.is_none());
    assert_eq!(last_quick(&app)["points"].as_array().map(Vec::len), Some(100_002));
    // No document: the press starts nothing and the release selects nothing.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::QuickSelection;
    down(&mut app, 10.0, 10.0, Modifiers::NONE);
    drag(&mut app, 20.0, 10.0, Modifiers::NONE);
    up(&mut app, 30.0, 10.0, Modifiers::NONE);
    assert!(app.quick_live.is_none());
}

#[test]
fn the_options_bar_has_new_add_and_subtract_but_no_intersect() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        quick_app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    assert!(h.query_by_label_contains("Intersect with selection").is_none());
    for (label, mode) in [("Subtract from selection", SUBTRACT), ("Add to selection", ADD), ("New selection", NEW)] {
        h.get_by_label_contains(label).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.quick_selection_mode, mode, "{label}");
    }
    // The other selection tools' bar mode is their own.
    assert_eq!(h.state().ui.selection_mode, 0);
}
