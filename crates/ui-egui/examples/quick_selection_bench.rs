//! Quick Selection drag on a 24 MP document (#9145): how often the outline updates while the
//! button is held, and the longest frame the UI thread spends, with the real `PhotocraftApp` in an
//! offscreen egui_kittest harness on wgpu. Pointer moves arrive at 60 Hz in real time (the
//! segments grow on their worker meanwhile); each frame runs the app's logic (`--render` also
//! paints it on the GPU).
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example quick_selection_bench -- [--size 6000x4000]
//!     [--brush 30] [--frames 120] [--step 8] [--zoom 0.5] [--render]
//! ```
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
use photocraft_ui_egui::{PhotocraftApp, Services};
use rayon::prelude::*;
use serde_json::json;

type H = egui_kittest::Harness<'static, PhotocraftApp>;

fn arg<T: std::str::FromStr>(args: &[String], name: &str) -> Option<T> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

static RENDER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn frame(h: &mut H) -> f64 {
    let t = Instant::now();
    h.step();
    if RENDER.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = h.render();
    }
    ms(t)
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

fn stats(name: &str, mut v: Vec<f64>) {
    v.sort_by(f64::total_cmp);
    let Some(&max) = v.last() else { return };
    let (med, p90) = (v[v.len() / 2], v[(v.len() * 9 / 10).min(v.len() - 1)]);
    println!("{name:<34} median {med:>7.2} ms  p90 {p90:>7.2}  max {max:>7.2}  (n = {})", v.len());
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
    RENDER.store(args.iter().any(|a| a == "--render"), std::sync::atomic::Ordering::Relaxed);
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
    // The stroke: inside the big red disc, a wave left to right.
    let (cx, cy) = (0.3 * w as f64, 0.5 * hh as f64);
    let at = |k: usize| (cx - 0.6 * step * frames as f64 / 2.0 + 0.6 * step * k as f64, cy + 0.3 * step * frames as f64 / 4.0 * (k as f64 / 15.0).sin());
    {
        let app = h.state_mut();
        app.ui.tool = Tool::QuickSelection;
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
    let period = Duration::from_micros(16_667);
    let pace = |started: Instant| {
        if let Some(rest) = period.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    };
    let (x0, y0) = at(0);
    let p0 = screen(&h, x0, y0);
    h.event(Event::PointerMoved(p0));
    frame(&mut h);
    let drag_start = Instant::now();
    let started = Instant::now();
    h.event(Event::PointerButton { pos: p0, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    let press = frame(&mut h);
    pace(started);
    let mut times = Vec::with_capacity(frames);
    let mut first_outline = None;
    let mut seen = 0;
    let mut gaps = Vec::new();
    let mut last_update = drag_start;
    for k in 1..=frames {
        let started = Instant::now();
        let (x, y) = at(k);
        h.event(Event::PointerMoved(screen(&h, x, y)));
        times.push(frame(&mut h));
        let updates = photocraft_ui_egui::quick_select_ui::progress(h.state()).map_or(0, |p| p.0);
        if updates > seen {
            if first_outline.is_none() {
                first_outline = Some(ms(drag_start));
            }
            gaps.push(last_update.elapsed().as_secs_f64() * 1000.0);
            last_update = Instant::now();
            seen = updates;
        }
        pace(started);
    }
    let drag_s = drag_start.elapsed().as_secs_f64();
    let (x1, y1) = at(frames);
    let released = Instant::now();
    let p1 = screen(&h, x1, y1);
    h.event(Event::PointerButton { pos: p1, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    let release = frame(&mut h);
    let mut after = Vec::new();
    while h.state().session.journal.last().is_none_or(|(id, _)| id != "select.quick") && released.elapsed() < Duration::from_secs(30) {
        let started = Instant::now();
        after.push(frame(&mut h));
        pace(started);
    }
    let caught_up = ms(released);
    let (_, params) = h.state().session.journal.last().cloned().unwrap_or_default();
    let segments = params["segments"].as_array().map_or(1, Vec::len);
    println!("after: {seen} outline updates while dragging over {drag_s:.2} s = {:.1} per second", seen as f64 / drag_s);
    println!("after: first outline {:.1} ms after the press; {segments} segments committed", first_outline.unwrap_or(f64::NAN));
    println!("after: press frame {press:.2} ms, release frame {release:.2} ms, committed {caught_up:.1} ms after the release");
    stats("after: drag frames (UI thread)", times.clone());
    stats("after: time between outline updates", gaps);
    stats("after: frames until committed", after.clone());
    let longest = times.iter().chain(&after).copied().fold(press.max(release), f64::max);
    println!("after: longest UI-thread frame {longest:.2} ms");

    // Before: no outline while dragging, the whole stroke selected at once on the release.
    let r = h.state_mut().run("edit.undo", json!({}));
    assert!(r.is_ok(), "undo: {r:?}");
    let mut whole = params.clone();
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
    stats("before: release (select.quick, whole stroke)", once);
}
