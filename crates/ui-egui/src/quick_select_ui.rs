//! Quick Selection while the button is held (#9145): the selection grows under the brush as it
//! is painted, as in Photoshop (Adobe Help, "Paint a selection with Quick Selection tool"), not
//! only when the button goes up.
//!
//! A stroke is selected segment by segment (`photocraft_engine::quick_stroke`): whenever the
//! previous segment is done, the points that arrived since become the next one. On the desktop a
//! segment grows on a worker thread, so the UI never waits for it and the outline catches up with
//! the pointer as fast as the machine allows; on the web (no threads) and with background jobs
//! off (tests, `PHOTOCRAFT_INLINE_JOBS`) it grows inline, the web's at most a few times a second.
//! The canvas outlines the stroke's selection so far ([`shown`]) instead of the document's.
//!
//! When the button goes up the last points become the final segment. Once every segment is done
//! the stroke is committed as one `select.quick` (one history step, "Quick Selection") whose
//! `segments` record how it was cut, offered the live result so nothing grows twice: what the
//! canvas showed is what is committed, and replaying the journal makes the same selection. A
//! command, or a new stroke, that comes before that commits the stroke first ([`flush`]).
//!
//! Modes, as on Photoshop's options bar: New, Add and Subtract (Quick Selection has no Intersect).
//! ⌥ subtracts and ⇧ adds while held, read as the stroke goes: ⌥ pressed during the drag (on
//! desktops whose window manager takes ⌥-drag to move the window, #9145) subtracts from there on,
//! and the Subtract button needs no key at all. After a stroke, New switches itself to Add.

use egui::Modifiers;
use photocraft_algo::selection::SelectionMode;
use photocraft_doc::DocId;
use photocraft_engine::quick_stroke::{self, QuickCommit, QuickSource, QuickStroke};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// Options-bar mode: New selection (the default; the first stroke replaces the selection).
pub const NEW: u8 = 0;
/// Options-bar mode: Add to selection (where New goes after a stroke).
pub const ADD: u8 = 1;
/// Options-bar mode: Subtract from selection.
pub const SUBTRACT: u8 = 2;

/// On the web a segment grows on the UI thread: at most this often while dragging (ms).
const WEB_INTERVAL_MS: f64 = 100.0;

/// The engine mode of a segment painted with options-bar mode `bar` and modifiers `m` held.
pub fn segment_mode(bar: u8, m: Modifiers) -> SelectionMode {
    quick_stroke::mode_named(crate::tool_feedback::selection_mode(Tool::QuickSelection, bar, m))
}

/// The content bounds of the stroke's selection (the outline traces within them).
fn bounds(stroke: &QuickStroke) -> photocraft_geom::Rect {
    stroke.selection().map_or(photocraft_geom::Rect::EMPTY, Surface::content_bounds)
}

/// The threads segments grow on: one fewer than the machine has (at least one), apart from the
/// global pool, so the UI thread's own parallel work (tracing the outline, compositing on the
/// CPU) never queues behind a segment's cuts.
#[cfg(not(target_arch = "wasm32"))]
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get).saturating_sub(1).max(1);
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("quick-selection-{i}")).build().ok()
    })
    .as_ref()
}

/// A segment being grown on a worker (never on the web, which grows segments inline).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Job {
    #[cfg(not(target_arch = "wasm32"))]
    rx: std::sync::mpsc::Receiver<Option<(QuickStroke, photocraft_geom::Rect)>>,
}

/// The Quick Selection stroke under way (`PhotocraftApp::quick_live`).
pub(crate) struct QuickLive {
    doc: DocId,
    /// `select.quick`'s settings (brush size, Sample All Layers, Enhance Edge), as at the press.
    settings: Value,
    src: QuickSource,
    /// The stroke after the segments done so far: what the canvas outlines.
    shown: QuickStroke,
    /// Its content bounds (computed with it, off the UI thread).
    shown_bounds: photocraft_geom::Rect,
    /// The stroke's points (valid ones, document px) and the drag points read so far.
    points: Vec<(f32, f32)>,
    read: usize,
    /// Segments cut but not grown yet: their end in `points` and their mode. Points after the last
    /// one form the open segment, painted in `open_mode`.
    queue: std::collections::VecDeque<(usize, SelectionMode)>,
    open_mode: SelectionMode,
    /// End (in `points`) of the segments sent to grow, done or not.
    sent: usize,
    job: Option<Job>,
    /// The button is up: commit once every point is grown.
    released: bool,
    /// The outline changed this many times (a segment grew).
    pub(crate) updates: usize,
    /// When a segment last grew inline (ms; the web's throttle).
    last_inline_ms: f64,
    /// Woken when a worker finishes.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ctx: Option<egui::Context>,
}

impl QuickLive {
    fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Read the drag points not read yet, painted in `mode`: a change of mode ends the open
    /// segment where it happened.
    fn read(&mut self, drag: &[[f64; 3]], mode: SelectionMode) {
        let new: Vec<(f32, f32)> = drag.get(self.read..).unwrap_or_default().iter().filter_map(|q| quick_stroke::point(q[0], q[1])).collect();
        self.read = drag.len();
        if new.is_empty() {
            return;
        }
        let open_from = self.queue.back().map_or(self.sent, |s| s.0);
        let cuts = self.sent_segments() + self.queue.len();
        if mode != self.open_mode && self.points.len() > open_from && cuts + 1 < quick_stroke::MAX_SEGMENTS {
            self.queue.push_back((self.points.len(), self.open_mode));
        }
        self.open_mode = mode;
        self.points.extend(new);
    }

    fn sent_segments(&self) -> usize {
        self.shown.segments().len() + usize::from(self.busy())
    }

    /// The next segment to grow: a queued one, else (unless `hold`) the open one.
    fn next(&mut self, hold: bool) -> Option<(Vec<(f32, f32)>, SelectionMode)> {
        let (end, mode) = match self.queue.pop_front() {
            Some(s) => s,
            None if !hold && self.points.len() > self.sent && self.sent_segments() + 1 < quick_stroke::MAX_SEGMENTS => (self.points.len(), self.open_mode),
            // The segment limit: the rest of the stroke is its last segment.
            None if self.released && self.points.len() > self.sent => (self.points.len(), self.open_mode),
            None => return None,
        };
        let pts = self.points.get(self.sent..end)?.to_vec();
        self.sent = end;
        Some((pts, mode))
    }

    /// Every point is grown and nothing is running.
    fn caught_up(&self) -> bool {
        !self.busy() && self.queue.is_empty() && self.sent == self.points.len()
    }

    /// Grow `pts` in `mode` now, on this thread.
    fn grow_inline(&mut self, pts: &[(f32, f32)], mode: SelectionMode, now_ms: f64) {
        self.shown.push(&self.src, pts, mode);
        self.shown_bounds = bounds(&self.shown);
        self.updates += 1;
        self.last_inline_ms = now_ms;
    }

    /// Grow `pts` in `mode` on a worker, from the stroke as shown now.
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(&mut self, pts: Vec<(f32, f32)>, mode: SelectionMode) -> bool {
        let (src, mut stroke, ctx) = (self.src.clone(), self.shown.clone(), self.ctx.clone());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let work = move || {
            let grow = move || {
                stroke.push(&src, &pts, mode);
                let bounds = bounds(&stroke);
                (stroke, bounds)
            };
            // In a pool of its own: the segmentation's parallel cuts must not fill the global one,
            // which the UI thread's own parallel work (tracing the outline) would then wait for.
            let grown = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || match pool() {
                Some(pool) => pool.install(grow),
                None => grow(),
            }))
            .ok();
            let _ = tx.send(grown);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        };
        match std::thread::Builder::new().name("quick-selection".into()).spawn(work) {
            Ok(_) => {
                self.job = Some(Job { rx });
                true
            }
            Err(_) => false,
        }
    }

    /// Take a finished worker's segment (`wait`: block until it is done).
    #[cfg(not(target_arch = "wasm32"))]
    fn collect(&mut self, wait: bool) -> Result<(), String> {
        let Some(job) = &self.job else { return Ok(()) };
        let got = if wait { job.rx.recv().map_err(|_| std::sync::mpsc::TryRecvError::Disconnected) } else { job.rx.try_recv() };
        let grown = match got {
            Ok(g) => g,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        self.job = None;
        let (stroke, bounds) = grown.ok_or_else(|| tl!("Quick Selection failed with an internal error; the selection is unchanged").to_string())?;
        self.shown_bounds = bounds;
        self.shown = stroke;
        self.updates += 1;
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn collect(&mut self, _wait: bool) -> Result<(), String> {
        Ok(())
    }
}

/// `select.quick`'s settings from the options bar and the brush, without the stroke.
pub(crate) fn settings(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    json!({"size": app.session.tools.brush.size, "sampleAllLayers": o.sample_all.quick_selection, "enhanceEdge": o.enhance_edge})
}

/// Start the live stroke of the Quick Selection drag that just began (`app.drag`). Without one
/// (no document, say) the release selects the whole stroke at once, as before.
pub(crate) fn begin(app: &mut PhotocraftApp) {
    flush(app);
    app.quick_live = None;
    let Some(d) = app.drag.as_ref().filter(|d| d.tool == Tool::QuickSelection) else { return };
    let mode = segment_mode(app.ui.quick_selection_mode, d.live);
    let settings = settings(app);
    let Some(doc) = app.session.active().map(|st| st.doc.id) else { return };
    let Ok(src) = QuickSource::new(&app.session, &settings) else { return };
    let shown = QuickStroke::new(src.selection().cloned());
    let shown_bounds = bounds(&shown);
    app.quick_live = Some(QuickLive {
        doc,
        settings,
        src,
        shown,
        shown_bounds,
        points: Vec::new(),
        read: 0,
        queue: Default::default(),
        open_mode: mode,
        sent: 0,
        job: None,
        released: false,
        updates: 0,
        last_inline_ms: f64::NEG_INFINITY,
        ctx: None,
    });
    pump(app);
}

/// Grows whatever is ready: takes a finished segment, reads the drag's new points and starts the
/// next segment; commits a released stroke once it has caught up. Called for every batch of
/// pointer moves and once a frame ([`tick`]).
pub(crate) fn pump(app: &mut PhotocraftApp) {
    let inline = cfg!(target_arch = "wasm32") || !app.background_jobs;
    let bar = app.ui.quick_selection_mode;
    let now = crate::gpu_canvas::now_ms();
    let Some(live) = app.quick_live.as_mut() else { return };
    if let Err(e) = live.collect(false) {
        app.quick_live = None;
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    if !live.released
        && let Some(d) = app.drag.as_ref().filter(|d| d.tool == Tool::QuickSelection)
    {
        live.read(&d.points, segment_mode(bar, d.live));
    }
    // The web grows a segment at most every `WEB_INTERVAL_MS` while dragging; points arriving
    // meanwhile join the open segment.
    let hold = cfg!(target_arch = "wasm32") && !live.released && now - live.last_inline_ms < WEB_INTERVAL_MS;
    while !live.busy() {
        let Some((pts, mode)) = live.next(hold) else { break };
        if inline {
            live.grow_inline(&pts, mode, now);
            if cfg!(target_arch = "wasm32") && !live.released {
                break;
            }
            continue;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !live.spawn(pts.clone(), mode) {
            // No thread to be had: grow it here rather than lose it.
            live.grow_inline(&pts, mode, now);
        }
    }
    if live.released && live.caught_up() {
        commit(app);
    }
}

/// The button went up on the stroke `d`: its last points become the final segment, committed
/// once grown. False when the stroke has no live state (the caller selects it at once).
pub(crate) fn release(app: &mut PhotocraftApp, d: &crate::canvas::Drag) -> bool {
    let bar = app.ui.quick_selection_mode;
    let Some(live) = app.quick_live.as_mut() else { return false };
    live.read(&d.points, segment_mode(bar, d.live));
    live.released = true;
    pump(app);
    true
}

/// Commit a released stroke now, waiting for its segments: before any other command runs, and
/// before a new stroke starts. A stroke still being dragged is left alone.
pub(crate) fn flush(app: &mut PhotocraftApp) {
    let Some(live) = app.quick_live.as_mut().filter(|l| l.released) else { return };
    if let Err(e) = live.collect(true) {
        app.quick_live = None;
        app.ui.status = e;
        app.ui.status_error = true;
        return;
    }
    let now = crate::gpu_canvas::now_ms();
    while let Some((pts, mode)) = live.next(false) {
        live.grow_inline(&pts, mode, now);
    }
    commit(app);
}

/// Once a frame: keep a stroke growing (and committing) after its last pointer event, and keep
/// frames coming while a segment grows.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(live) = app.quick_live.as_mut() else { return };
    if live.ctx.is_none() {
        live.ctx = Some(ctx.clone());
    }
    // The drag ended some other way (a lost release, a closed document): commit what it grew.
    if !live.released && app.drag.as_ref().is_none_or(|d| d.tool != Tool::QuickSelection) {
        live.released = true;
    }
    pump(app);
    if app.quick_live.as_ref().is_some_and(|l| l.busy() || !l.caught_up()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// Commit the stroke as one `select.quick`, taking its live result.
fn commit(app: &mut PhotocraftApp) {
    let Some(live) = app.quick_live.take() else { return };
    if live.shown.segments().is_empty() {
        return;
    }
    if app.session.active().map(|st| st.doc.id) != Some(live.doc) {
        app.ui.status = tl!("The Quick Selection stroke was dropped: its document is no longer the active one").into();
        app.ui.status_error = true;
        return;
    }
    let mut p = live.settings.clone();
    if let (Some(o), Value::Object(s)) = (p.as_object_mut(), live.shown.params()) {
        o.extend(s);
    }
    app.session.offer_live_quick(QuickCommit::new(live.src, live.shown));
    let r = app.run("select.quick", p);
    // An offer the command didn't take (it was refused) must not linger.
    app.session.clear_live_stroke();
    committed(app, r);
}

/// After a Quick Selection stroke's `select.quick`: New switches to Add once a stroke has selected
/// something, as in Photoshop; a stroke that found nothing says so.
pub(crate) fn committed(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if r.as_ref().is_ok_and(|v| v.get("changed").and_then(Value::as_bool) != Some(false)) && app.ui.quick_selection_mode == NEW {
        app.ui.quick_selection_mode = ADD;
    }
    crate::retouch_ui::report_smart_result(app, r, tl!("Quick Selection found nothing there"));
}

/// The selection the canvas outlines on document `doc` while a stroke is under way there, with
/// its content bounds (`None`: no stroke; `Some((None, _))`: nothing selected so far).
pub(crate) fn shown(app: &PhotocraftApp, doc: DocId) -> Option<(Option<Surface>, photocraft_geom::Rect)> {
    let live = app.quick_live.as_ref().filter(|l| l.doc == doc)?;
    Some((live.shown.selection().cloned(), live.shown_bounds))
}

/// Whether the stroke under way has grown at least once (until then the canvas draws the brush
/// trail under the pointer instead).
pub(crate) fn has_grown(app: &PhotocraftApp) -> bool {
    app.quick_live.as_ref().is_some_and(|l| l.updates > 0)
}

/// Wait for the segment growing now (tests and the bench drive the threaded path step by step).
#[cfg(not(target_arch = "wasm32"))]
pub fn settle(app: &mut PhotocraftApp) {
    if let Some(live) = app.quick_live.as_mut()
        && let Err(e) = live.collect(true)
    {
        app.quick_live = None;
        app.ui.status = e;
    }
    pump(app);
}

/// Times the outline changed during the stroke under way, and whether a segment is growing (for
/// the bench and tests).
pub fn progress(app: &PhotocraftApp) -> Option<(usize, bool)> {
    app.quick_live.as_ref().map(|l| (l.updates, l.busy()))
}

#[cfg(test)]
#[path = "quick_select_ui_tests.rs"]
mod tests;
