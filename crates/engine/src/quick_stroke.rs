//! Quick Selection strokes, one segment at a time (#9145).
//!
//! Photoshop's Quick Selection grows the selection under the brush while the button is held: the
//! outline follows the pointer and spreads to the edges near it (Adobe Help, "Paint a selection
//! with Quick Selection tool"). Recomputing the whole stroke on every pointer move costs more
//! the longer the stroke gets, so a stroke is selected **segment by segment**: each segment (its
//! points, joined to the previous segment's last point and led by the stroke's last
//! [`segment_overlap`] of travel before it) is grown by
//! [`quick_select`](photocraft_algo::segment::quick::quick_select) and combined into the selection
//! as it stands, in order. An update costs what a segment and its lead cover, however long the
//! stroke already is, and a stroke takes what it takes as one over that span: a short drag inside
//! an object too big for a click selects the object (measured in Photoshop,
//! `quick_select_a_short_drag_takes_what_a_click_does_not`), as it does in one segment.
//!
//! A live stroke is cut by the pointer's travel alone ([`Cutter`]), never by how long a segment
//! takes to grow, so the same motion gives the same selection on any machine. `select.quick`
//! records the cut (`segments`) and computes exactly these segments in this order, so the
//! selection shown while dragging ([`QuickStroke`], driven by the shell off the UI thread), the
//! one committed on release and the one a replay of the journal makes are the same. Without
//! `segments` the whole stroke is one segment, as before.
//!
//! Modes: a segment adds, subtracts or replaces. A stroke replaces the old selection only with
//! its first segment that finds anything; every later `replace` segment adds (a New Selection
//! stroke keeps growing as it is painted, and never wipes what the stroke itself did). Each
//! segment carries its own mode, so ⌥ pressed or let go during a drag subtracts (or adds again)
//! from that point of the stroke on.

use std::sync::Arc;

use photocraft_algo::selection::{Region, SelectionMode, combine_region};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// Most segments one stroke may be cut into: a live drag adds one per [`segment_travel`] of the
/// pointer, so this is a drag of over 100,000 px with a small brush. Every segment is a
/// segmentation, so a replayed or scripted stroke can't ask for an unbounded number.
pub const MAX_SEGMENTS: usize = 4096;

/// The shortest travel (document px) that ends a live segment, whatever the brush.
pub const MIN_TRAVEL: f32 = 24.0;

/// How far the pointer travels (document px) before a live stroke's segment ends: a little more
/// than the brush width (each segment costs a segmentation in a window around it, so a brush
/// width or so of new stroke per update keeps the outline close behind the pointer without
/// falling behind on a large document).
pub fn segment_travel(size: f32) -> f32 {
    let size = if size.is_finite() { size } else { 30.0 };
    (1.25 * size).max(MIN_TRAVEL)
}

/// How much of the stroke before a segment (document px of travel) seeds it too: one segment's
/// travel, so every segment is grown with at least the last two segments' worth of stroke, enough
/// for a short drag inside an object to take it as one stroke does.
pub fn segment_overlap(size: f32) -> f32 {
    segment_travel(size)
}

/// Cuts a live stroke into segments as its points arrive, by the pointer's travel alone: the
/// press is a segment of its own (a click selects at once), then a segment ends at the first
/// point where the pointer has travelled [`segment_travel`] since it began, or before a point
/// painted in another mode (a modifier pressed or let go). The same motion gives the same cut on
/// any machine; the cut is capped at [`MAX_SEGMENTS`], the rest being the last segment.
#[derive(Clone, Debug)]
pub struct Cutter {
    travel: f32,
    points: usize,
    /// Where the open segment starts, and its mode.
    open: usize,
    open_mode: Option<SelectionMode>,
    /// The pointer's travel since the open segment's start (the previous segment's last point).
    moved: f32,
    last: Option<(f32, f32)>,
    closed: Vec<(usize, SelectionMode)>,
}

impl Cutter {
    /// A cutter for a stroke with brush diameter `size`.
    pub fn new(size: f32) -> Self {
        Self { travel: segment_travel(size), points: 0, open: 0, open_mode: None, moved: 0.0, last: None, closed: Vec::new() }
    }

    fn at_cap(&self) -> bool {
        self.closed.len() + 1 >= MAX_SEGMENTS
    }

    fn close(&mut self) {
        if let Some(m) = self.open_mode.filter(|_| self.points > self.open) {
            self.closed.push((self.points - self.open, m));
            self.open = self.points;
            self.moved = 0.0;
        }
    }

    /// The stroke's next point `p` (finite, document px), painted in `mode`.
    pub fn push(&mut self, p: (f32, f32), mode: SelectionMode) {
        if self.open_mode.is_some_and(|m| m != mode) && !self.at_cap() {
            self.close();
        }
        if self.points == self.open || self.open_mode.is_none() {
            self.open_mode = Some(mode);
        }
        if let Some(l) = self.last {
            let d = (p.0 - l.0).hypot(p.1 - l.1);
            self.moved += if d.is_finite() { d } else { 0.0 };
        }
        self.last = Some(p);
        self.points += 1;
        if !self.at_cap() && (self.points == 1 || self.moved >= self.travel) {
            self.close();
        }
    }

    /// The stroke ended: its open points become the last segment.
    pub fn finish(&mut self) {
        self.close();
    }

    /// The segments closed so far: point counts (adding up to the points before the open ones)
    /// and modes.
    pub fn closed(&self) -> &[(usize, SelectionMode)] {
        &self.closed
    }

    /// Points pushed so far.
    pub fn points(&self) -> usize {
        self.points
    }
}

/// The brush diameter range `select.quick` works with (px).
const SIZE: std::ops::RangeInclusive<f32> = 1.0..=5000.0;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "select.quick".into(), msg: msg.into() }
}

/// A stroke point as `select.quick` reads it: document pixels, `None` unless both coordinates are
/// finite once in `f32` (`null`, a string, `1e300`).
pub fn point(x: f64, y: f64) -> Option<(f32, f32)> {
    let (x, y) = (x as f32, y as f32);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// The stroke's points from `select.quick` params: malformed and non-finite ones are skipped.
pub fn points(p: &Value) -> Vec<(f32, f32)> {
    p.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(|q| point(q.get(0)?.as_f64()?, q.get(1)?.as_f64()?)).collect()).unwrap_or_default()
}

/// A mode name as `select.quick` reads it: Quick Selection has no Intersect (it adds).
pub fn mode_named(m: &str) -> SelectionMode {
    match m {
        "subtract" => SelectionMode::Subtract,
        "replace" | "new" => SelectionMode::Replace,
        _ => SelectionMode::Add,
    }
}

/// The name `select.quick` takes for `m`.
pub fn mode_name(m: SelectionMode) -> &'static str {
    match m {
        SelectionMode::Subtract => "subtract",
        SelectionMode::Replace => "replace",
        _ => "add",
    }
}

/// The stroke's mode (`"mode"`, default add).
pub fn stroke_mode(p: &Value) -> SelectionMode {
    mode_named(p.get("mode").and_then(Value::as_str).unwrap_or("add"))
}

/// How the `n` points of a stroke are cut into segments (`"segments"`): each entry is a point
/// count in the stroke's mode, or `{"points": count, "mode": …}`. The counts are positive and add
/// up to `n`. Without `segments`, the stroke is one segment.
pub fn segments(p: &Value, n: usize) -> Result<Vec<(usize, SelectionMode)>> {
    let mode = stroke_mode(p);
    let list = match p.get("segments") {
        None | Some(Value::Null) => return Ok(vec![(n, mode)]),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(bad("segments must be a list of point counts")),
    };
    if list.is_empty() || list.len() > n.min(MAX_SEGMENTS) {
        return Err(bad(format!("segments must have between 1 and {} entries (one per point at most)", n.clamp(1, MAX_SEGMENTS))));
    }
    let mut out = Vec::with_capacity(list.len());
    let mut total = 0usize;
    for item in list {
        let (count, m) = match item {
            Value::Object(o) => {
                let m = match o.get("mode") {
                    None | Some(Value::Null) => mode,
                    Some(v) => mode_named(v.as_str().ok_or_else(|| bad("a segment's mode is add, subtract or replace"))?),
                };
                (o.get("points").and_then(Value::as_u64), m)
            }
            v => (v.as_u64(), mode),
        };
        let count =
            count.filter(|c| *c > 0).and_then(|c| usize::try_from(c).ok()).ok_or_else(|| bad("each segment needs a positive whole number of points"))?;
        total = total.checked_add(count).filter(|t| *t <= n).ok_or_else(|| bad(format!("segments cover more than the stroke's {n} points")))?;
        out.push((count, m));
    }
    if total != n {
        return Err(bad(format!("segments cover {total} of the stroke's {n} points")));
    }
    Ok(out)
}

/// What a Quick Selection stroke reads: the document as it was when the stroke began, the layer
/// it samples and the brush. Cheap to clone and `Send`, so segments can be grown on a worker.
#[derive(Clone)]
pub struct QuickSource {
    doc: Arc<Document>,
    base: (DocId, u64),
    layer: Option<LayerId>,
    all_layers: bool,
    size: f32,
    enhance: bool,
}

impl QuickSource {
    /// The source `select.quick` with params `p` reads on the active document now.
    pub fn new(s: &Session, p: &Value) -> Result<Self> {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let size = p.get("size").and_then(Value::as_f64).map_or(30.0, |v| v as f32);
        let size = if size.is_nan() { 30.0 } else { size.clamp(*SIZE.start(), *SIZE.end()) };
        Ok(Self {
            doc: Arc::clone(&st.doc),
            base: (st.doc.id, st.revision),
            layer: st.active_layer,
            all_layers: p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(false),
            size,
            enhance: p.get("enhanceEdge").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// The selection when the stroke began.
    pub fn selection(&self) -> Option<&Surface> {
        self.doc.selection.as_ref()
    }

    /// The document and revision the stroke began on.
    pub fn base(&self) -> (DocId, u64) {
        self.base
    }

    /// The brush diameter, as clamped for the selection (px).
    pub fn size(&self) -> f32 {
        self.size
    }

    /// Same pixels, layer and settings (not compared: the document snapshot itself).
    fn same(&self, o: &Self) -> bool {
        self.base == o.base && self.layer == o.layer && self.all_layers == o.all_layers && self.size.to_bits() == o.size.to_bits() && self.enhance == o.enhance
    }

    /// The region a segment (`pts`, a polyline) grows to, or `None` when it misses the canvas.
    pub fn region(&self, pts: &[(f32, f32)]) -> Option<Region> {
        crate::smartselect_cmds::quick_region(&self.doc, self.layer, self.all_layers, pts, self.size, self.enhance)
    }
}

/// A stroke selected segment by segment: the selection after every segment pushed so far.
/// Pushing a segment is the heavy part (it runs the segmentation); everything else is cheap.
#[derive(Clone, Debug)]
pub struct QuickStroke {
    selection: Option<Surface>,
    /// The previous segment's last point: the next segment starts from it.
    last: Option<(f32, f32)>,
    /// A segment has found something: a later `replace` segment adds.
    applied: bool,
    /// The polyline of the stroke since its mode last changed between subtracting and selecting,
    /// and whether it subtracts: the next segment of its kind is led by its end.
    run: (bool, Vec<(f32, f32)>),
    /// A segment found something.
    changed: bool,
    /// Everything the segments found.
    bounds: Rect,
    points: Vec<(f32, f32)>,
    segments: Vec<(usize, SelectionMode)>,
}

impl QuickStroke {
    /// A stroke starting from the selection `base`.
    pub fn new(base: Option<Surface>) -> Self {
        Self {
            selection: base,
            last: None,
            applied: false,
            run: (false, Vec::new()),
            changed: false,
            bounds: Rect::EMPTY,
            points: Vec::new(),
            segments: Vec::new(),
        }
    }

    /// Select the next segment, `pts` (non-finite points are skipped), in `mode`. Returns the
    /// region the segment changed (empty when it found nothing).
    pub fn push(&mut self, src: &QuickSource, pts: &[(f32, f32)], mode: SelectionMode) -> Rect {
        let pts: Vec<(f32, f32)> = pts.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
        let Some(&end) = pts.last() else { return Rect::EMPTY };
        // The stroke's run of this kind, so far: a change between selecting and subtracting starts
        // a new one at the previous segment's last point.
        let subtracts = mode == SelectionMode::Subtract;
        if self.run.0 != subtracts || self.run.1.is_empty() {
            self.run = (subtracts, self.last.into_iter().collect());
        }
        // Led by the run's last `segment_overlap` of travel, then the segment itself.
        let lead = self.run.1.get(lead_start(&self.run.1, segment_overlap(src.size()))..).unwrap_or_default();
        let mut line = Vec::with_capacity(lead.len() + pts.len());
        line.extend_from_slice(lead);
        line.extend_from_slice(&pts);
        self.run.1.extend_from_slice(&pts);
        self.last = Some(end);
        self.points.extend_from_slice(&pts);
        self.segments.push((pts.len(), mode));
        let Some(region) = src.region(&line) else { return Rect::EMPTY };
        // Only the stroke's first segment that finds anything replaces: a later one adds, so it
        // never wipes what the stroke did before it (a subtraction with ⌥ held, say).
        let m = if mode == SelectionMode::Replace && self.applied { SelectionMode::Add } else { mode };
        self.selection = combine_region(self.selection.as_ref(), Some(&region), m);
        self.applied = true;
        self.changed = true;
        self.bounds = self.bounds.union(&region.bbox);
        region.bbox
    }

    /// The selection after the segments so far.
    pub fn selection(&self) -> Option<&Surface> {
        self.selection.as_ref()
    }

    /// Whether any segment found something (a stroke that found nothing leaves the selection and
    /// the history alone).
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The points of the segments so far, as they were selected.
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// The segments so far: point count and mode.
    pub fn segments(&self) -> &[(usize, SelectionMode)] {
        &self.segments
    }

    /// `select.quick`'s `points` and `segments` for the stroke so far (the counts alone where a
    /// segment's mode is the stroke's).
    pub fn params(&self) -> Value {
        let mode = self.segments.first().map_or(SelectionMode::Add, |s| s.1);
        let segments: Vec<Value> = self.segments.iter().map(|&(n, m)| if m == mode { json!(n) } else { json!({"points": n, "mode": mode_name(m)}) }).collect();
        let points: Vec<[f64; 2]> = self.points.iter().map(|&(x, y)| [f64::from(x), f64::from(y)]).collect();
        json!({"points": points, "segments": segments, "mode": mode_name(mode)})
    }
}

/// Where the last `travel` (document px) of the polyline `run` starts: the index of its first point.
fn lead_start(run: &[(f32, f32)], travel: f32) -> usize {
    let mut back = 0.0f32;
    let mut i = run.len().saturating_sub(1);
    while i > 0 && back < travel {
        if let (Some(a), Some(b)) = (run.get(i - 1), run.get(i)) {
            let d = (b.0 - a.0).hypot(b.1 - a.1);
            back += if d.is_finite() { d } else { travel };
        }
        i -= 1;
    }
    i
}

/// A stroke selected live, offered to the `select.quick` that commits it
/// ([`Session::offer_live_quick`]): when that command's document, settings, points and segments
/// are exactly the stroke's, it takes its selection instead of growing every segment again.
pub struct QuickCommit {
    src: QuickSource,
    stroke: QuickStroke,
}

impl QuickCommit {
    pub fn new(src: QuickSource, stroke: QuickStroke) -> Self {
        Self { src, stroke }
    }

    /// The stroke, if `select.quick` with these inputs would compute exactly it.
    pub(crate) fn take_if(self, src: &QuickSource, pts: &[(f32, f32)], segments: &[(usize, SelectionMode)]) -> Option<QuickStroke> {
        // The selection the stroke started from is the document's at that revision.
        let same_points = self.stroke.points.len() == pts.len()
            && self.stroke.points.iter().zip(pts).all(|(a, b)| a.0.to_bits() == b.0.to_bits() && a.1.to_bits() == b.1.to_bits());
        (self.src.same(src) && same_points && self.stroke.segments == segments).then_some(self.stroke)
    }
}

impl Session {
    /// Offer a Quick Selection stroke drawn live to the `select.quick` that commits it: one whose
    /// params match it exactly takes its selection (its result then says `"fromLive": true`); any
    /// other leaves the offer unused. The command runs through the normal dispatch, so the
    /// journal, history and authorization are unchanged.
    pub fn offer_live_quick(&mut self, commit: QuickCommit) {
        self.live_quick = Some(commit);
    }
}

/// `select.quick`: the stroke, segment by segment, as one history step.
pub(crate) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let offered = s.live_quick.take();
    let pts = points(p);
    if pts.is_empty() {
        return Err(bad("needs at least one point"));
    }
    let segments = segments(p, pts.len())?;
    let src = QuickSource::new(s, p)?;
    if let Some(stroke) = offered.and_then(|c| c.take_if(&src, &pts, &segments)) {
        return commit(s, stroke, true);
    }
    // Grown here: a background job where the shell runs those (a scripted or replayed stroke of
    // many segments shows progress and can be cancelled between segments).
    let base = src.selection().cloned();
    crate::jobs::run(
        s,
        "Quick Selection",
        true,
        move |ctx| {
            let total = segments.len().max(1) as f32;
            let mut stroke = QuickStroke::new(base);
            let mut at = 0usize;
            for (k, &(n, m)) in segments.iter().enumerate() {
                ctx.check()?;
                ctx.progress(k as f32 / total, "Selecting");
                let end = at.saturating_add(n).min(pts.len());
                stroke.push(&src, pts.get(at..end).unwrap_or_default(), m);
                at = end;
            }
            Ok(stroke)
        },
        move |s, stroke| commit(s, stroke, false),
    )
}

/// Store a grown stroke's selection as one history step ("Quick Selection"); a stroke that found
/// nothing leaves the selection (and history) alone.
fn commit(s: &mut Session, stroke: QuickStroke, from_live: bool) -> Result<Value> {
    if !stroke.changed {
        let selected = s.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some();
        return Ok(json!({"selected": selected, "changed": false}));
    }
    let b = stroke.bounds;
    let selection = stroke.selection;
    let selected = s.edit("Quick Selection", move |doc, _| {
        doc.selection = selection;
        Ok(doc.selection.is_some())
    })?;
    let mut r = json!({"selected": selected, "changed": true, "bounds": [b.x0, b.y0, b.width() as i32, b.height() as i32]});
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
