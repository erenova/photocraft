use photocraft_algo::segment::{Rng, SurfaceSampler, quick};
use photocraft_algo::selection::{SelectionMode, combine_region};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use super::*;

/// A `w × h` document at `depth` whose left part (x < `split`) is warm and textured and whose
/// right part is cool.
fn two_regions_at(w: u32, h: u32, split: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    let mut rng = Rng::new(7);
    let noise: Vec<f32> = (0..w * h).map(|_| 0.04 * rng.normal()).collect();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        let mut data = Vec::with_capacity((w * h) as usize * n);
        for y in 0..h {
            for x in 0..w {
                let e = noise[(y * w + x) as usize];
                let c = if x < split { [0.8 + e, 0.5 + e, 0.3] } else { [0.2, 0.35 + e, 0.6 + e] };
                data.extend(c.iter().take(n.min(3)).map(|v| v.clamp(0.0, 1.0)));
                if n == 4 {
                    data.push(1.0);
                }
            }
        }
        bg.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
        Ok(())
    })
    .unwrap();
    s
}

fn two_regions(depth: u32) -> Session {
    two_regions_at(140, 100, 60, depth)
}

/// The selection as 8-bit coverage over the document, to compare two selections exactly.
fn sel_bytes(s: &Session) -> Vec<u8> {
    let st = s.active().unwrap();
    let (w, h) = (st.doc.size.width as i32, st.doc.size.height as i32);
    let sel = st.doc.selection.as_ref();
    (0..h).flat_map(|y| (0..w).map(move |x| sel.map_or(0, |m| (m.sample_channel(x, y, 0) * 255.0).round() as u8))).collect()
}

fn surface_bytes(sel: Option<&Surface>, w: i32, h: i32) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| sel.map_or(0, |m| (m.sample_channel(x, y, 0) * 255.0).round() as u8))).collect()
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

#[test]
fn segments_default_to_one_and_must_cover_the_stroke() {
    assert_eq!(segments(&json!({}), 5).unwrap(), vec![(5, SelectionMode::Add)]);
    assert_eq!(segments(&json!({"mode": "replace", "segments": null}), 3).unwrap(), vec![(3, SelectionMode::Replace)]);
    assert_eq!(
        segments(&json!({"mode": "new", "segments": [1, {"points": 2, "mode": "subtract"}, {"points": 1}]}), 4).unwrap(),
        vec![(1, SelectionMode::Replace), (2, SelectionMode::Subtract), (1, SelectionMode::Replace)]
    );
    // Quick Selection has no Intersect: it adds, as the stroke's mode always did.
    assert_eq!(segments(&json!({"segments": [{"points": 2, "mode": "intersect"}]}), 2).unwrap(), vec![(2, SelectionMode::Add)]);
    for bad in [
        json!({"segments": []}),
        json!({"segments": [2]}),
        json!({"segments": [2, 2]}),
        json!({"segments": [0, 3]}),
        json!({"segments": [-1, 4]}),
        json!({"segments": [1.5, 1.5]}),
        json!({"segments": ["3"]}),
        json!({"segments": [{"mode": "add"}]}),
        json!({"segments": [{"points": 3, "mode": 7}]}),
        json!({"segments": 3}),
        json!({"segments": {"points": 3}}),
        json!({"segments": [u64::MAX, u64::MAX]}),
        json!({"segments": [1, 1, 1, 1]}),
    ] {
        assert!(segments(&bad, 3).is_err(), "{bad}");
    }
}

#[test]
fn one_segment_is_the_whole_stroke_as_before() {
    // Without `segments` the stroke is one cut over all its points, combined with the selection
    // as `select.quick` always did: journals recorded before segments replay unchanged.
    for depth in [8, 16, 32] {
        for mode in ["add", "subtract", "replace"] {
            let mut s = two_regions(depth);
            s.execute("select.rect", json!({"x": 90, "y": 10, "width": 30, "height": 30})).unwrap();
            let before = s.active().unwrap().doc.selection.clone();
            let pts = [(20.0f32, 50.0f32), (35.0, 50.0), (100.0, 20.0)];
            let region = {
                let st = s.active().unwrap();
                quick::quick_select(&SurfaceSampler(st.doc.layers[0].surface().unwrap()), st.doc.bounds(), &pts, 8.0)
            };
            let want = combine_region(before.as_ref(), region.as_ref(), mode_named(mode));
            s.execute("select.quick", json!({"points": [[20, 50], [35, 50], [100, 20]], "size": 8, "mode": mode})).unwrap();
            assert_eq!(sel_bytes(&s), surface_bytes(want.as_ref(), 140, 100), "depth {depth}, {mode}");
        }
    }
}

/// The selection `select.quick` makes on a fresh copy of `two_regions` from `params`.
fn replayed(depth: u32, setup: &[(&str, Value)], params: &Value) -> Vec<u8> {
    let mut s = two_regions(depth);
    for (id, p) in setup {
        s.execute(id, p.clone()).unwrap();
    }
    s.execute("select.quick", params.clone()).unwrap();
    sel_bytes(&s)
}

#[test]
fn a_live_stroke_commits_what_it_showed_and_replays_the_same() {
    let setup = [("select.rect", json!({"x": 100, "y": 70, "width": 20, "height": 20}))];
    for depth in [8, 16] {
        let mut s = two_regions(depth);
        for (id, p) in &setup {
            s.execute(id, p.clone()).unwrap();
        }
        let settings = json!({"size": 8, "mode": "replace", "sampleAllLayers": false, "enhanceEdge": false});
        let src = QuickSource::new(&s, &settings).unwrap();
        let mut live = QuickStroke::new(src.selection().cloned());
        // Pushed as a drag delivers them: the press, then a few moves at a time; ⌥ arrives
        // during the drag and is let go again.
        let chunks: [(&[(f32, f32)], SelectionMode); 4] = [
            (&[(20.0, 50.0)], SelectionMode::Replace),
            (&[(24.0, 50.0), (30.0, 52.0)], SelectionMode::Replace),
            (&[(110.0, 30.0)], SelectionMode::Subtract),
            (&[(112.0, 40.0), (115.0, 45.0)], SelectionMode::Replace),
        ];
        let mut shown = Vec::new();
        for (pts, m) in chunks {
            live.push(&src, pts, m);
            shown.push(surface_bytes(live.selection(), 140, 100));
        }
        assert!(shown[0] != shown[2] && shown[2] != shown[3], "depth {depth}: each segment changes the selection");
        let mut params = live.params();
        for (k, v) in settings.as_object().unwrap() {
            if k != "mode" {
                params[k] = v.clone();
            }
        }
        assert_eq!(params["mode"], "replace");
        assert_eq!(params["segments"], json!([1, 2, {"points": 1, "mode": "subtract"}, 2]));
        s.offer_live_quick(QuickCommit::new(src, live));
        let r = s.execute("select.quick", params.clone()).unwrap();
        assert_eq!(r["fromLive"], true, "depth {depth}: the commit takes the live stroke");
        assert_eq!(sel_bytes(&s), *shown.last().unwrap(), "depth {depth}: released == last preview");
        // The journal entry replays to the same selection, segment by segment.
        let (id, journaled) = s.journal.last().unwrap().clone();
        assert_eq!(id, "select.quick");
        assert_eq!(replayed(depth, &setup, &journaled), *shown.last().unwrap(), "depth {depth}: replay == commit");
        // One history step: undo brings back the rectangle.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(cov(&s, 110, 80) >= 0.99 && cov(&s, 20, 50) < 0.01, "depth {depth}");
    }
}

#[test]
fn a_new_stroke_replaces_once_and_then_grows() {
    let mut s = two_regions(8);
    s.execute("select.rect", json!({"x": 100, "y": 70, "width": 20, "height": 20})).unwrap();
    // The press replaces the rectangle; the segments after it add, they don't replace each other.
    s.execute("select.quick", json!({"points": [[20, 50], [110, 30]], "size": 8, "mode": "replace", "segments": [1, 1]})).unwrap();
    assert!(cov(&s, 10, 10) >= 0.5, "the first segment's region stays");
    assert!(cov(&s, 110, 30) >= 0.5, "the second segment adds");
    assert!(cov(&s, 110, 80) < 0.5 || cov(&s, 130, 95) >= 0.5, "the old rectangle was replaced (or the right region grew over it)");
    // A first segment that finds nothing doesn't replace: the next one does.
    let mut s = two_regions(8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.quick", json!({"points": [[-500, -500], [20, 50]], "size": 8, "mode": "replace", "segments": [1, 1]})).unwrap();
    assert!(cov(&s, 20, 50) >= 0.5 && cov(&s, 130, 90) < 0.5, "replaced by the segment that found something");
}

#[test]
fn a_stroke_subtracts_from_where_its_segments_say() {
    let mut s = two_regions(8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("select.quick", json!({"points": [[100, 30], [110, 70]], "size": 10, "mode": "add", "segments": [1, {"points": 1, "mode": "subtract"}]}))
        .unwrap();
    assert!(cov(&s, 10, 50) >= 0.99, "the left part stays");
    assert!(cov(&s, 120, 20) < 0.01, "the subtracting segment took out the right part");
}

#[test]
fn an_offer_for_another_stroke_is_not_taken() {
    let mut s = two_regions(8);
    let settings = json!({"size": 8});
    let src = QuickSource::new(&s, &settings).unwrap();
    let mut live = QuickStroke::new(None);
    live.push(&src, &[(100.0, 50.0)], SelectionMode::Add);
    s.offer_live_quick(QuickCommit::new(src.clone(), live.clone()));
    // Other points: the stroke is grown again from its params, not taken from the offer.
    let r = s.execute("select.quick", json!({"points": [[20, 50]], "size": 8})).unwrap();
    assert_eq!(r.get("fromLive"), None);
    assert!(cov(&s, 10, 10) >= 0.5 && cov(&s, 130, 90) < 0.5);
    // Another brush size, or a document edited since the stroke began: not taken either.
    for edit in [false, true] {
        let mut s = two_regions(8);
        let src = QuickSource::new(&s, &settings).unwrap();
        let mut live = QuickStroke::new(None);
        live.push(&src, &[(100.0, 50.0)], SelectionMode::Add);
        let mut p = live.params();
        p["size"] = json!(8);
        if edit {
            s.execute("select.all", json!({})).unwrap();
        } else {
            p["size"] = json!(9);
        }
        s.offer_live_quick(QuickCommit::new(src, live));
        let r = s.execute("select.quick", p).unwrap();
        assert_eq!(r.get("fromLive"), None, "edit {edit}");
    }
}

#[test]
fn a_stroke_that_finds_nothing_keeps_the_selection_and_history() {
    for m in ["add", "subtract", "replace"] {
        let mut s = two_regions(8);
        s.execute("select.all", json!({})).unwrap();
        let undo = s.active().unwrap().history.past_len();
        let r = s.execute("select.quick", json!({"points": [[-5000, -5000], [-4000, -5000]], "size": 8, "mode": m, "segments": [1, 1]})).unwrap();
        assert_eq!(r["changed"], false, "{m}");
        assert_eq!(s.active().unwrap().history.past_len(), undo, "{m}: no history step");
        assert!(cov(&s, 10, 50) >= 0.99, "{m}");
    }
}

#[test]
fn hostile_strokes_are_refused_or_survived_never_a_panic() {
    let n = 100_000;
    let many: Vec<[f64; 2]> = (0..n).map(|i| [(i % 140) as f64, ((i / 140) % 100) as f64]).collect();
    let cases = [
        json!({"points": []}),
        json!({"points": null}),
        json!({"points": "x"}),
        json!({"points": [[null, 1]]}),
        json!({"points": [[1e300, 1e300]]}),
        json!({"points": [[-1e300, 5]], "size": 8}),
        json!({"points": [[f64::MAX, f64::MIN]], "size": 8}),
        json!({"points": [[1e9, -1e9], [-1e9, 1e9]], "size": 30}),
        json!({"points": [[20, 50]], "size": 0}),
        json!({"points": [[20, 50]], "size": -5}),
        json!({"points": [[20, 50]], "size": 1e9}),
        json!({"points": [[20, 50]], "size": 1e300}),
        json!({"points": [[20, 50]], "size": "big"}),
        json!({"points": [[20, 50], [21, 50]], "segments": [1, 1], "mode": 3}),
        json!({"points": many, "size": 8}),
        json!({"points": many, "size": 1, "segments": [n / 2, n / 2]}),
        json!({"points": [[20, 50], [1e300, 3], [30, 50]], "segments": [1, 1]}),
        json!({"points": [[20, 50], [1e300, 3], [30, 50]], "segments": [1, 1, 1]}),
    ];
    for p in cases {
        let mut s = two_regions(8);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute("select.quick", p.clone()))).unwrap_or_else(|_| panic!("panicked on {p}"));
    }
    // Without points or a document: Err.
    let mut s = two_regions(8);
    assert!(s.execute("select.quick", json!({"points": []})).is_err());
    assert!(s.execute("select.quick", json!({"points": [[1, 1]], "segments": [2]})).is_err());
    assert!(Session::new().execute("select.quick", json!({"points": [[1, 1]]})).is_err());
    // The live path takes the same hostile points.
    let src = QuickSource::new(&s, &json!({"size": 1e9})).unwrap();
    assert_eq!(src.size(), 5000.0);
    let mut live = QuickStroke::new(None);
    for pts in [vec![], vec![(f32::NAN, 1.0)], vec![(f32::INFINITY, f32::NEG_INFINITY)], vec![(1e30, -1e30), (-1e30, 1e30)], vec![(20.0, 50.0)]] {
        live.push(&src, &pts, SelectionMode::Add);
    }
    assert_eq!(live.points(), &[(1e30, -1e30), (-1e30, 1e30), (20.0, 50.0)], "non-finite points are skipped");
    for size in [f64::NAN, -1.0, 0.0] {
        let src = QuickSource::new(&s, &json!({"size": size})).unwrap();
        assert!(src.size() >= 1.0 && src.size().is_finite());
    }
}

/// A 500 × 500 document with a 300 px red square on blue (the algorithm's Photoshop-measured
/// short-drag case, `quick_select_a_short_drag_takes_what_a_click_does_not`).
fn square_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 500, "height": 500})).unwrap();
    s.edit("paint", |doc, a| {
        let surf = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(0, 0, 500, 500), &[60.0 / 255.0, 60.0 / 255.0, 200.0 / 255.0, 1.0]);
        surf.fill_rect(Rect::new(100, 100, 400, 400), &[200.0 / 255.0, 60.0 / 255.0, 60.0 / 255.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Selected pixels inside the square, and outside it.
fn in_square(s: &Session) -> (usize, usize) {
    let st = s.active().unwrap();
    let Some(sel) = st.doc.selection.as_ref() else { return (0, 0) };
    let (mut i, mut o) = (0, 0);
    for y in 0..500 {
        for x in 0..500 {
            if sel.sample_channel(x, y, 0) >= 0.5 {
                if (100..400).contains(&x) && (100..400).contains(&y) { i += 1 } else { o += 1 }
            }
        }
    }
    (i, o)
}

#[test]
fn a_short_drag_takes_the_square_however_it_is_cut() {
    // Review of #9145: segments seeded only by their own points took a brush band (3,196 px)
    // where the whole stroke takes the square (90,000): a fast machine cutting the drag finer
    // selected less. Seeded by the stroke so far, every cut takes the square, as Photoshop does.
    let pts: Vec<[f64; 2]> = (0..9).map(|i| [220.0 + 7.5 * f64::from(i), 250.0]).collect();
    let mut cutter = Cutter::new(30.0);
    for p in &pts {
        cutter.push((p[0] as f32, p[1] as f32), SelectionMode::Replace);
    }
    cutter.finish();
    let live: Vec<Value> = cutter.closed().iter().map(|&(n, _)| json!(n)).collect();
    for segments in [None, Some(json!([1, 8])), Some(json!([1, 2, 2, 2, 2])), Some(json!([1, 1, 1, 1, 1, 1, 1, 1, 1])), Some(json!(live))] {
        let mut s = square_doc();
        let mut p = json!({"points": pts, "size": 30, "mode": "replace"});
        if let Some(seg) = &segments {
            p["segments"] = seg.clone();
        }
        s.execute("select.quick", p).unwrap();
        let (i, o) = in_square(&s);
        assert!(i >= 300 * 300 - 10 && o == 0, "segments {segments:?}: {i} in, {o} out");
    }
    // A click alone still takes about the brush.
    let mut s = square_doc();
    s.execute("select.quick", json!({"points": [[250, 250]], "size": 30})).unwrap();
    assert!(in_square(&s).0 < 2000);
}

#[test]
fn the_cut_follows_the_pointer_travel_not_the_timing() {
    let travel = segment_travel(30.0);
    assert_eq!(travel, 37.5);
    assert_eq!(segment_travel(4.0), MIN_TRAVEL);
    // The press is a segment of its own; then one segment per `travel` of the pointer, however
    // finely the moves arrive.
    let cut = |step: f32| {
        let mut c = Cutter::new(30.0);
        let mut x = 0.0;
        while x <= 300.0 {
            c.push((x, 10.0), SelectionMode::Add);
            x += step;
        }
        c.finish();
        c.closed().to_vec()
    };
    for step in [2.5, 7.5, 12.5] {
        let segs = cut(step);
        assert_eq!(segs[0], (1, SelectionMode::Add), "step {step}");
        // Each closed segment but the last spans the travel (to within one move).
        let mut at = 1usize;
        for &(n, _) in &segs[1..segs.len() - 1] {
            let span = n as f32 * step;
            assert!(span >= travel && span < travel + step, "step {step}: {span}");
            at += n;
        }
        assert_eq!(at + segs.last().unwrap().0, (300.0 / step) as usize + 1);
    }
    // A mode change ends the open segment before the point painted in the new mode.
    let mut c = Cutter::new(30.0);
    for (x, m) in [(0.0, SelectionMode::Add), (5.0, SelectionMode::Add), (10.0, SelectionMode::Subtract), (15.0, SelectionMode::Subtract)] {
        c.push((x, 0.0), m);
    }
    c.finish();
    assert_eq!(c.closed(), &[(1, SelectionMode::Add), (1, SelectionMode::Add), (2, SelectionMode::Subtract)]);
    // Capped: the rest of a very long stroke is its last segment.
    let mut c = Cutter::new(1.0);
    for i in 0..(MAX_SEGMENTS * 2) {
        c.push((i as f32 * 100.0, 0.0), if i % 2 == 0 { SelectionMode::Add } else { SelectionMode::Subtract });
    }
    c.finish();
    assert_eq!(c.closed().len(), MAX_SEGMENTS);
    assert_eq!(c.closed().iter().map(|s| s.0).sum::<usize>(), MAX_SEGMENTS * 2);
}

#[test]
fn a_later_replace_segment_adds_and_keeps_the_strokes_subtraction() {
    // New mode with ⌥ held at the press, ⌥ let go during the drag: the stroke subtracts, then
    // its New segments add; they must not replace the selection with their own region.
    let run = |last: &str| {
        let mut s = square_doc();
        s.execute("select.all", json!({})).unwrap();
        let p = json!({"points": [[40, 40], [40, 60], [40, 460]], "size": 30, "mode": "subtract", "segments": [1, 1, {"points": 1, "mode": last}]});
        s.execute("select.quick", p).unwrap();
        s
    };
    let (replace, add) = (run("replace"), run("add"));
    assert_eq!(sel_bytes(&replace), sel_bytes(&add), "a New segment after the stroke has found something adds");
    let selected = sel_bytes(&replace).iter().filter(|c| **c >= 128).count();
    assert!(selected > 200_000 && cov(&replace, 250, 250) >= 0.99, "{selected}: Select All's rest stays, not replaced by a band");
    let subtracted_only = {
        let mut s = square_doc();
        s.execute("select.all", json!({})).unwrap();
        s.execute("select.quick", json!({"points": [[40, 40], [40, 60]], "size": 30, "mode": "subtract", "segments": [1, 1]})).unwrap();
        sel_bytes(&s).iter().filter(|c| **c >= 128).count()
    };
    assert!(subtracted_only < 250_000, "the subtracting segments took something out");
}

#[test]
fn a_replayed_stroke_is_capped_and_can_be_cancelled() {
    let n = MAX_SEGMENTS + 1;
    let pts: Vec<[f64; 2]> = (0..n).map(|i| [(i % 140) as f64, 50.0]).collect();
    let mut s = two_regions(8);
    assert!(s.execute("select.quick", json!({"points": pts, "segments": vec![1; n]})).is_err(), "more segments than the cap");
    // Cancelled between segments: an error, and the selection untouched.
    let ctx = crate::jobs::JobCtx::new();
    ctx.cancel();
    s.set_inline_job_ctx(Some(ctx));
    let r = s.execute("select.quick", json!({"points": [[20, 50], [30, 50]], "segments": [1, 1]}));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert!(s.active().unwrap().doc.selection.is_none());
    s.set_inline_job_ctx(None);
    assert!(s.execute("select.quick", json!({"points": [[20, 50], [30, 50]], "segments": [1, 1]})).is_ok());
}
