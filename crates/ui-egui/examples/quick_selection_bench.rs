//! Quick Selection drags on a 24 MP document (#9145): how often the outline updates while the
//! button is held, and the longest frame the UI thread spends, with the real `PhotocraftApp` in an
//! offscreen egui_kittest harness on wgpu. Pointer moves arrive at 60 Hz in real time (segments
//! grow on their worker meanwhile); each frame runs the app's logic (`--render` also paints it).
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example quick_selection_bench -- [--size 6000x4000]
//!     [--brush 30] [--frames 120] [--step 8] [--zoom 0.5] [--render] [--verbose] [--json out.json]
//! ```
//!
//! Scenarios, one after another on the same document:
//! - `drag`: a stroke inside an object, released, until committed;
//! - `select all, subtract`: the same over Select All with the Subtract button (the whole image
//!   outlined, the "refine Select Subject" workflow);
//! - `press after release`: a new stroke pressed the frame after a release, while the first still
//!   grows;
//! - `command after release`: a command that edits the document (Deselect, cheap in itself) and
//!   view commands the frame after a release;
//! - `edit mid-drag`: ⌘D while the button is down, then the rest of the drag and the release.
//!
//! "Before" is what the release used to cost: the whole stroke as one `select.quick` on the UI
//! thread (no outline while dragging), timed on the same document and stroke.

use std::time::{Duration, Instant};

use egui::{Event, Modifiers, PointerButton, Pos2};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{PhotocraftApp, Services, quick_select_ui};
use rayon::prelude::*;
use serde_json::{Value, json};

type H = egui_kittest::Harness<'static, PhotocraftApp>;

fn arg<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

static RENDER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn frame(h: &mut H) -> f64 {
    let t = Instant::now();
    h.step();
    if RENDER.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = h.render();
    }
    let f = ms(t);
    if f > 16.0 && VERBOSE.load(std::sync::atomic::Ordering::Relaxed) {
        let p = &h.state().perf;
        println!(
            "    slow frame {f:.1} ms: refresh {} {} px, composite {:.1} ms, upload {:.1} ms, last command {} {:.1} ms, spans {:?}",
            p.last_refresh, p.last_refresh_px, p.composite_ms, p.upload_ms, p.last_command, p.command_ms, p.spans
        );
    }
    f
}

const PERIOD: Duration = Duration::from_micros(16_667);

/// Sleep out the rest of a 60 Hz frame that began at `started`.
fn pace(started: Instant) {
    if let Some(rest) = PERIOD.checked_sub(started.elapsed()) {
        std::thread::sleep(rest);
    }
}

fn hash(x: i32, y: i32) -> f32 {
    ((x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)) as u32 % 997) as f32 / 997.0 - 0.5
}

/// A photo-like test image: a noisy gradient with textured discs of other colours on it.
fn image(w: u32, h: u32) -> Layer {
    let discs = [(0.3f32, 0.5f32, 0.22f32, [0.85f32, 0.25, 0.15]), (0.68, 0.42, 0.16, [0.2, 0.7, 0.3]), (0.55, 0.8, 0.1, [0.9, 0.85, 0.2])];
    let mut l = Layer::raster("photo", PixelFormat::RGBA8);
    let s = l.surface_mut().expect("raster");
    let (wf, hf) = (w as f32, h as f32);
    let mut y = 0;
    while y < h as i32 {
        let band = Rect::new(0, y, w as i32, (y + 256).min(h as i32));
        let mut bytes = vec![0u8; w as usize * band.height() as usize * 4];
        bytes.par_chunks_mut(w as usize * 4).enumerate().for_each(|(row, px)| {
            let yy = band.y0 + row as i32;
            for (x, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let (fx, fy) = (x as f32 / wf, yy as f32 / hf);
                let n = hash(x as i32, yy) * 0.08;
                let mut c = [0.35 + 0.3 * fx + n, 0.4 + 0.2 * fy + n, 0.6 - 0.2 * fx + n];
                for (cx, cy, r, col) in discs {
                    let (dx, dy) = ((fx - cx) * wf, (fy - cy) * hf);
                    if dx * dx + dy * dy <= (r * hf) * (r * hf) {
                        let t = ((x as f32 + yy as f32) * 0.02).sin() * 0.05;
                        c = [col[0] + n + t, col[1] + n + t, col[2] + n];
                    }
                }
                for (k, v) in c.iter().enumerate() {
                    p[k] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                p[3] = 255;
            }
        });
        s.write_interleaved(band, &bytes);
        y = band.y1;
    }
    l
}

fn screen(h: &H, x: f64, y: f64) -> Pos2 {
    let app = h.state();
    let idx = app.session.active_index().unwrap_or(0);
    let v = &app.ui.views[idx];
    let rect = photocraft_ui_egui::rulers::content_rect(app, app.last_canvas_rect);
    ViewXform { rect, zoom: v.zoom, center: v.center, flip: app.ui.view.flip_horizontal, rotation: v.rotation, aspect: 1.0 }.to_screen(x as f32, y as f32)
}

fn stats(name: &str, v: &[f64]) {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    let Some(&max) = v.last() else { return };
    let (med, p90) = (v[v.len() / 2], v[(v.len() * 9 / 10).min(v.len() - 1)]);
    println!("  {name:<38} median {med:>7.2} ms  p90 {p90:>7.2}  max {max:>7.2}  (n = {})", v.len());
}

/// What happens around a drag.
#[derive(Clone, Copy, PartialEq)]
enum Extra {
    None,
    /// ⌘D halfway through the drag, the button still down.
    EditMidDrag,
    /// A second stroke pressed the frame after the release.
    PressAfter,
    /// A document command and a view command the frame after the release.
    CommandAfter,
}

#[derive(Default)]
struct Run {
    drag: Vec<f64>,
    gaps: Vec<f64>,
    press: f64,
    release: f64,
    /// The frames after the release until the stroke (and any follow-up) was committed, the frame
    /// it was committed in, and a few frames after.
    after: Vec<f64>,
    commit: f64,
    settle: Vec<f64>,
    /// Commands ran in the commit frame (they waited for it): its time is theirs too.
    deferred: bool,
    /// The frame (or call) of the extra action.
    extra: Vec<f64>,
    updates: usize,
    drag_s: f64,
    first_outline: Option<f64>,
    committed_ms: f64,
    params: Value,
}

impl Run {
    /// The longest UI-thread frame the stroke itself cost (a command that waited for the commit
    /// runs in the commit frame, and costs what it costs at any time: not counted here).
    fn longest(&self) -> f64 {
        let commit = if self.deferred { 0.0 } else { self.commit };
        self.drag.iter().chain(&self.after).chain(&self.extra).chain(&self.settle).copied().fold(self.press.max(self.release).max(commit), f64::max)
    }

    /// Every UI-thread frame of the stroke (see [`Run::longest`]).
    fn frames(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.drag.iter().chain(&self.after).chain(&self.settle).copied().collect();
        v.extend([self.press, self.release]);
        if !self.deferred {
            v.push(self.commit);
        }
        v
    }
}

/// One stroke of `frames` moves along `at`, released, then frames until every stroke is committed.
fn drag(h: &mut H, frames: usize, at: &dyn Fn(usize) -> (f64, f64), extra: Extra) -> Run {
    let mut run = Run::default();
    let journal = h.state().session.journal.len();
    let (x0, y0) = at(0);
    let p0 = screen(h, x0, y0);
    h.event(Event::PointerMoved(p0));
    frame(h);
    let start = Instant::now();
    h.event(Event::PointerButton { pos: p0, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    run.press = frame(h);
    pace(start);
    let (mut seen, mut last_update) = (0, start);
    for k in 1..=frames {
        let started = Instant::now();
        let (x, y) = at(k);
        h.event(Event::PointerMoved(screen(h, x, y)));
        if extra == Extra::EditMidDrag && k == frames / 2 {
            let t = Instant::now();
            let r = h.state_mut().run("select.deselect", json!({}));
            assert!(r.is_ok(), "select.deselect: {r:?}");
            run.extra.push(ms(t) + frame(h));
        } else {
            run.drag.push(frame(h));
        }
        let updates = quick_select_ui::progress(h.state()).map_or(0, |p| p.0);
        if updates > seen {
            run.first_outline.get_or_insert(ms(start));
            run.gaps.push(ms(last_update));
            last_update = Instant::now();
            seen = updates;
        }
        pace(started);
    }
    run.updates = seen;
    run.drag_s = start.elapsed().as_secs_f64();
    let (x1, y1) = at(frames);
    let released = Instant::now();
    let p1 = screen(h, x1, y1);
    h.event(Event::PointerButton { pos: p1, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    run.release = frame(h);
    match extra {
        Extra::PressAfter => {
            // The next stroke, 300 px lower, pressed at once and dragged a little.
            let started = Instant::now();
            let p2 = screen(h, x1, y1 - 300.0);
            h.event(Event::PointerMoved(p2));
            h.event(Event::PointerButton { pos: p2, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            run.extra.push(frame(h));
            pace(started);
            for k in 1..=10 {
                let started = Instant::now();
                h.event(Event::PointerMoved(screen(h, x1 - 8.0 * f64::from(k), y1 - 300.0)));
                run.extra.push(frame(h));
                pace(started);
            }
            let p3 = screen(h, x1 - 80.0, y1 - 300.0);
            h.event(Event::PointerButton { pos: p3, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            run.extra.push(frame(h));
        }
        Extra::CommandAfter => {
            // A document edit (cheap in itself: Deselect) waits for the commit; view commands run.
            for id in ["select.deselect", "view.zoomIn", "view.zoomOut"] {
                let t = Instant::now();
                let r = h.state_mut().run(id, json!({}));
                assert!(r.is_ok(), "{id}: {r:?}");
                run.extra.push(ms(t));
            }
        }
        _ => {}
    }
    run.deferred = extra == Extra::CommandAfter;
    while !quick_select_ui::idle(h.state()) && released.elapsed() < Duration::from_secs(60) {
        let started = Instant::now();
        let f = frame(h);
        if quick_select_ui::idle(h.state()) {
            run.commit = f;
        } else {
            run.after.push(f);
        }
        pace(started);
    }
    run.committed_ms = ms(released);
    run.params = h.state().session.journal.iter().skip(journal).find(|(id, _)| id == "select.quick").map(|(_, p)| p.clone()).unwrap_or_default();
    // A couple of frames for the canvas to settle on the committed selection.
    for _ in 0..3 {
        run.settle.push(frame(h));
    }
    run
}

fn report(name: &str, r: &Run) {
    let segments = r.params["segments"].as_array().map_or(1, Vec::len);
    println!("{name}:");
    println!(
        "  {} outline updates over {:.2} s = {:.1} per second; first {:.0} ms after the press; {segments} segments",
        r.updates,
        r.drag_s,
        r.updates as f64 / r.drag_s.max(1e-9),
        r.first_outline.unwrap_or(f64::NAN)
    );
    println!(
        "  press frame {:.2} ms, release frame {:.2} ms, committed {:.0} ms after the release, in a frame of {:.2} ms{}",
        r.press,
        r.release,
        r.committed_ms,
        r.commit,
        if r.deferred { " (with the commands that waited for it)" } else { "" }
    );
    stats("drag frames (UI thread)", &r.drag);
    stats("time between outline updates", &r.gaps);
    stats("frames until committed", &r.after);
    stats("frames after the commit", &r.settle);
    if !r.extra.is_empty() {
        stats("the extra action's frames / calls", &r.extra);
    }
    println!("  longest UI-thread frame {:.2} ms", r.longest());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (w, hh) = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
        .unwrap_or((6000u32, 4000u32));
    let brush: f32 = arg(&args, "--brush").unwrap_or(30.0);
    let frames: usize = arg(&args, "--frames").unwrap_or(120);
    let step: f64 = arg(&args, "--step").unwrap_or(8.0);
    let zoom: f32 = arg(&args, "--zoom").unwrap_or(0.5);
    let json_out: Option<String> = arg(&args, "--json");
    RENDER.store(args.iter().any(|a| a == "--render"), std::sync::atomic::Ordering::Relaxed);
    VERBOSE.store(args.iter().any(|a| a == "--verbose"), std::sync::atomic::Ordering::Relaxed);
    let t = Instant::now();
    let mut doc = Document::new("photo", Size::new(w, hh), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(image(w, hh));
    println!("built {w}×{hh} in {:.0} ms; brush {brush} px, {frames} frames at 60 Hz, {step} px per frame, zoom {zoom}", ms(t));

    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0));
    let mut h: H = builder.wgpu_setup(photocraft_ui_egui::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            app.set_wgpu(rs.clone());
        }
        // The desktop app's setting: commands and segments run off the UI thread.
        app.background_jobs = true;
        app.session.open_document(doc, Some("photo.psd".into()));
        app.sync_views();
        app
    });
    for _ in 0..6 {
        frame(&mut h);
    }
    // The strokes: inside the big red disc, a wave left to right.
    let (cx, cy) = (0.3 * w as f64, 0.5 * hh as f64);
    let at = move |k: usize| (cx - 0.6 * step * frames as f64 / 2.0 + 0.6 * step * k as f64, cy + 0.3 * step * frames as f64 / 4.0 * (k as f64 / 15.0).sin());
    {
        let app = h.state_mut();
        app.ui.tool = Tool::QuickSelection;
        app.ui.quick_selection_mode = quick_select_ui::ADD;
        app.session.tools.brush.size = brush;
        let idx = app.session.active_index().unwrap_or(0);
        if let Some(v) = app.ui.views.get_mut(idx) {
            v.zoom = zoom;
            v.center = [cx as f32, cy as f32];
        }
    }
    for _ in 0..6 {
        frame(&mut h);
    }
    // Between scenarios: no selection, Add, and half a second of frames for the canvas to settle.
    let settle = |h: &mut H| {
        for _ in 0..30 {
            let started = Instant::now();
            frame(h);
            pace(started);
        }
    };
    let reset = |h: &mut H| {
        let _ = h.state_mut().run("select.deselect", json!({}));
        h.state_mut().ui.quick_selection_mode = quick_select_ui::ADD;
        settle(h);
    };

    let plain = drag(&mut h, frames, &at, Extra::None);
    report("drag", &plain);
    reset(&mut h);
    let r = h.state_mut().run("select.all", json!({}));
    assert!(r.is_ok(), "select.all: {r:?}");
    h.state_mut().ui.quick_selection_mode = quick_select_ui::SUBTRACT;
    settle(&mut h);
    let subtract = drag(&mut h, frames, &at, Extra::None);
    report("select all, subtract", &subtract);
    reset(&mut h);
    let repress = drag(&mut h, frames, &at, Extra::PressAfter);
    report("press after release", &repress);
    reset(&mut h);
    let command = drag(&mut h, frames, &at, Extra::CommandAfter);
    report("command after release", &command);
    reset(&mut h);
    // A selection made before the stroke, which ⌘D removes mid-drag.
    let r = h.state_mut().run("select.rect", json!({"x": 10, "y": 10, "width": 200, "height": 200}));
    assert!(r.is_ok(), "select.rect: {r:?}");
    settle(&mut h);
    let edit = drag(&mut h, frames, &at, Extra::EditMidDrag);
    report("edit mid-drag", &edit);
    reset(&mut h);

    // Before: no outline while dragging, the whole stroke selected at once on the release.
    let mut whole = plain.params.clone();
    if let Some(o) = whole.as_object_mut() {
        o.remove("segments");
        o.insert("mode".into(), json!("add"));
    }
    let mut once = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        let r = h.state_mut().session.execute("select.quick", whole.clone());
        once.push(ms(t));
        assert!(r.is_ok(), "select.quick: {r:?}");
        let _ = h.state_mut().session.execute("edit.undo", json!({}));
    }
    println!("before: 0 outline updates while dragging; the release ran the whole stroke on the UI thread:");
    stats("release (select.quick, whole stroke)", &once);
    let longest = [&plain, &subtract, &repress, &command, &edit].iter().map(|r| r.longest()).fold(0.0, f64::max);
    println!("after: longest UI-thread frame in every scenario {longest:.2} ms");

    if let Some(out) = json_out {
        let row = photocraft_testkit::perf::row;
        let mut after_edit = edit.frames();
        after_edit.extend(&edit.extra);
        let rows = vec![
            row("drag frame", &plain.frames(), None, None),
            row("outline update interval", &plain.gaps, None, None),
            row("drag frame, select all subtract", &subtract.frames(), None, None),
            row("press after release", &repress.extra, None, None),
            row("command after release", &command.extra, None, None),
            row("frames after a mid-drag edit", &after_edit, None, None),
            row("before: release of the whole stroke", &once, None, None),
        ];
        let context = json!({"width": w, "height": hh, "brush": brush, "frames": frames});
        let report = photocraft_testkit::perf::report("quick_selection_bench", context, rows, None);
        if let Err(e) = photocraft_testkit::perf::write_report(&out, &report) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
