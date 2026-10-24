//! Quick Selection strokes, one segment at a time (#9145).
//!
//! Photoshop's Quick Selection grows the selection under the brush while the button is held: the
//! outline follows the pointer and spreads to the edges near it (Adobe Help, "Paint a selection
//! with Quick Selection tool"). Recomputing the whole stroke on every pointer move costs more
//! the longer the stroke gets, so a stroke is selected **segment by segment**: each segment (the
//! points that arrived since the previous one, joined to that one's last point) is grown on its
//! own by [`quick_select`](photocraft_algo::segment::quick::quick_select) and combined into the selection as it stands, in order. An
//! update costs what its new segment costs, however long the stroke already is.
//!
//! `select.quick` records how its stroke was cut (`segments`), and computes exactly these
//! segments in this order, so the selection shown while dragging ([`QuickStroke`], driven by the
//! shell off the UI thread), the one committed on release and the one a replay of the journal
//! makes are the same. Without `segments` the whole stroke is one segment, as before.
//!
//! Modes: a segment adds, subtracts or replaces. A stroke replaces the old selection once, with
//! the first segment that finds anything; its later `replace` segments add to that (a New
//! Selection stroke keeps growing as it is painted). Each segment carries its own mode, so ⌥
//! pressed or let go during a drag subtracts (or adds again) from that point of the stroke on.

use std::sync::Arc;

use photocraft_algo::selection::{Region, SelectionMode, combine_region};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// Most segments one stroke may be cut into: a live drag adds at most one per frame, so this is
/// many minutes of dragging.
pub const MAX_SEGMENTS: usize = 100_000;

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

    /// The region one segment (`pts`, a polyline) grows to, or `None` when it misses the canvas.
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
    /// A segment has replaced the old selection (later `replace` segments add).
    replaced: bool,
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
        Self { selection: base, last: None, replaced: false, changed: false, bounds: Rect::EMPTY, points: Vec::new(), segments: Vec::new() }
    }

    /// Select the next segment, `pts` (non-finite points are skipped), in `mode`. Returns the
    /// region the segment changed (empty when it found nothing).
    pub fn push(&mut self, src: &QuickSource, pts: &[(f32, f32)], mode: SelectionMode) -> Rect {
        let pts: Vec<(f32, f32)> = pts.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
        let Some(&end) = pts.last() else { return Rect::EMPTY };
        let mut line = Vec::with_capacity(pts.len() + 1);
        line.extend(self.last);
        line.extend_from_slice(&pts);
        self.last = Some(end);
        self.points.extend_from_slice(&pts);
        self.segments.push((pts.len(), mode));
        let Some(region) = src.region(&line) else { return Rect::EMPTY };
        let m = if mode == SelectionMode::Replace && self.replaced { SelectionMode::Add } else { mode };
        self.selection = combine_region(self.selection.as_ref(), Some(&region), m);
        self.replaced |= mode == SelectionMode::Replace;
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

    /// Select the stroke `pts` cut into `segments` (as [`segments`] reads them) from `base`.
    pub fn run(
        src: &QuickSource,
        base: Option<Surface>,
        pts: &[(f32, f32)],
        segments: &[(usize, SelectionMode)],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let mut stroke = Self::new(base);
        let mut at = 0usize;
        for &(n, m) in segments {
            check()?;
            let end = at.saturating_add(n).min(pts.len());
            stroke.push(src, pts.get(at..end).unwrap_or_default(), m);
            at = end;
        }
        Ok(stroke)
    }
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
    let live = offered.and_then(|c| c.take_if(&src, &pts, &segments));
    let from_live = live.is_some();
    let stroke = match live {
        Some(stroke) => stroke,
        None => QuickStroke::run(&src, src.selection().cloned(), &pts, &segments, &|| Ok(()))?,
    };
    if !stroke.changed {
        // Nothing found: the selection (and history) is left alone.
        let selected = src.selection().is_some();
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
