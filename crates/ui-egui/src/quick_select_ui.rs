//! Quick Selection while the button is held (#9145): the selection grows under the brush as it
//! is painted, as in Photoshop (Adobe Help, "Paint a selection with Quick Selection tool"), not
//! only when the button goes up.
//!
//! A stroke is selected segment by segment (`photocraft_engine::quick_stroke`), cut by the
//! pointer's travel alone (`Cutter`), so the same motion gives the same selection on any machine,
//! and each segment is seeded by the stroke painted so far. On the desktop a segment grows on a
//! worker thread, in a thread pool of its own, and the worker also traces the new outline for the
//! view, so the UI thread never waits for either; on the web (no threads) a segment grows on the
//! UI thread, throttled so it takes at most a third of the time ([`Throttle`]); with background
//! jobs off (tests, `PHOTOCRAFT_INLINE_JOBS`) it grows inline. The canvas outlines the stroke's
//! selection so far ([`shown`]) instead of the document's.
//!
//! When the button goes up the last points become the final segment. Once every segment has
//! grown the stroke is committed as one `select.quick` (one history step, "Quick Selection") whose
//! `segments` record the cut, offered the live result so nothing grows twice: what the canvas
//! showed is what is committed, and replaying the journal makes the same selection. Nothing waits
//! for that on the UI thread: a new stroke started meanwhile queues behind the one still growing
//! and starts from its result, and a command that edits the document runs once it has been
//! committed, in order ([`defer`]); view and tool commands run at once. A document edited under a
//! stroke being dragged (⌘Z, ⌘D) rebases it: its segments grow again on the new document, on the
//! worker.
//!
//! Modes, as on Photoshop's options bar: New, Add and Subtract (Quick Selection has no Intersect).
//! ⌥ subtracts and ⇧ adds while held, read as the stroke goes: ⌥ pressed during the drag (on
//! desktops whose window manager takes ⌥-drag to move the window, #9145) subtracts from there on,
//! and the Subtract button needs no key at all. When a stroke ends, New switches itself to Add.

use std::sync::Arc;

use egui::Modifiers;
use photocraft_algo::selection::SelectionMode;
use photocraft_doc::DocId;
use photocraft_engine::quick_stroke::{self, Cutter, QuickCommit, QuickSource, QuickStroke};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::state::Tool;
use crate::{ControlResponse, PhotocraftApp};

/// Options-bar mode: New selection (the default; the first stroke replaces the selection).
pub const NEW: u8 = 0;
/// Options-bar mode: Add to selection (where New goes after a stroke).
pub const ADD: u8 = 1;
/// Options-bar mode: Subtract from selection.
pub const SUBTRACT: u8 = 2;

/// On the web a segment grows on the UI thread: at most this often (ms).
const WEB_INTERVAL_MS: f64 = 100.0;

/// The engine mode of a segment painted with options-bar mode `bar` and modifiers `m` held.
pub fn segment_mode(bar: u8, m: Modifiers) -> SelectionMode {
    quick_stroke::mode_named(crate::tool_feedback::selection_mode(Tool::QuickSelection, bar, m))
}

/// The content bounds of the stroke's selection (the outline traces within them).
fn bounds(stroke: &QuickStroke) -> Rect {
    stroke.selection().map_or(Rect::EMPTY, Surface::content_bounds)
}

/// The threads segments grow on: one fewer than the machine has (at least one), apart from the
/// global pool, so the UI thread's own parallel work (tracing an outline, compositing on the CPU)
/// never queues behind a segment's cuts. Made once, on the first stroke.
#[cfg(not(target_arch = "wasm32"))]
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get).saturating_sub(1).max(1);
        rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("quick-selection-{i}")).build().ok()
    })
    .as_ref()
}

/// The web's throttle for growing segments on the UI thread: at most one every
/// [`WEB_INTERVAL_MS`], and after a slow one, a pause of twice its length, so growing takes at
/// most a third of the UI thread's time however large the document is. Fed a frame clock (ms),
/// which stands still within a frame: a grow is timed by the first frame that starts after it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Throttle {
    /// When the last segment started growing, until a later frame has timed it.
    grew_at: Option<f64>,
    /// No segment grows before this.
    next: f64,
}

impl Throttle {
    /// May a segment grow at `now`?
    pub(crate) fn ready(&mut self, now: f64) -> bool {
        if let Some(t0) = self.grew_at {
            if now <= t0 {
                return false;
            }
            // This frame started after the grow ended: `now - t0` bounds what it cost.
            self.grew_at = None;
            self.next = now + (2.0 * (now - t0)).max(WEB_INTERVAL_MS);
        }
        now >= self.next
    }

    /// A segment grew in the frame that started at `now`.
    pub(crate) fn grew(&mut self, now: f64) {
        self.grew_at = Some(now);
    }
}

/// Where and how finely the canvas last traced a live stroke's outline: a worker traces the next
/// one there too, so the canvas finds it in its cache (`canvas::outline_key`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OutlineView {
    pub doc: DocId,
    pub step: u32,
    pub vis: Rect,
}

/// An outline traced on a worker: document, cache key and segments.
type Traced = (DocId, u64, Arc<Vec<crate::outline::Segment>>);

/// What a worker hands back for a segment.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Grown {
    /// The stroke's base when the segment was sent (a rebase makes it stale).
    generation: u64,
    stroke: QuickStroke,
    sel: Option<Arc<Surface>>,
    bounds: Rect,
    outline: Option<Traced>,
}

/// A segment growing on a worker (never on the web, which grows segments inline).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Job {
    #[cfg(not(target_arch = "wasm32"))]
    rx: std::sync::mpsc::Receiver<Option<Grown>>,
}

/// A Quick Selection stroke: being dragged, or released and still growing.
pub(crate) struct QuickLive {
    doc: DocId,
    /// `select.quick`'s settings (brush size, Sample All Layers, Enhance Edge), as at the press.
    settings: Value,
    src: QuickSource,
    /// Bumped when the stroke is rebased on an edited document.
    generation: u64,
    /// The stroke after the segments grown on the current base: what the canvas outlines.
    shown: QuickStroke,
    /// Its selection, shared with the canvas, and its content bounds (both made with it, off the
    /// UI thread).
    shown_sel: Option<Arc<Surface>>,
    shown_bounds: Rect,
    /// Segments grown on the current base (until one has, the canvas shows the document's).
    grown: usize,
    /// The stroke's points (valid ones, document px), the drag points read so far, and the cut.
    points: Vec<(f32, f32)>,
    read: usize,
    cutter: Cutter,
    /// Closed segments sent to grow (done or growing), and the points they cover.
    sent: usize,
    sent_points: usize,
    job: Option<Job>,
    /// The button is up: commit once every segment has grown.
    released: bool,
    /// A segment failed: the stroke shows nothing more and is dropped on release.
    failed: bool,
    /// Commands that came after the release, run once the stroke is committed ([`defer`]).
    after: Vec<(String, Value)>,
    /// The outline changed this many times (a segment grew).
    pub(crate) updates: usize,
    throttle: Throttle,
    /// Woken when a worker finishes.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ctx: Option<egui::Context>,
    #[cfg(test)]
    hooks: (Option<Arc<std::sync::RwLock<()>>>, bool),
}

/// The Quick Selection strokes of the session (`PhotocraftApp::quick`): the oldest grows and
/// commits first; the others wait behind it.
#[derive(Default)]
pub(crate) struct QuickState {
    strokes: Vec<QuickLive>,
    /// A stroke's commit (and the commands that waited for it) is running: nothing is deferred.
    committing: bool,
    pub(crate) outline_view: Option<OutlineView>,
    /// Control replies (`ui.pointer`) waiting for the strokes they released to be committed.
    waiters: Vec<std::sync::mpsc::Sender<ControlResponse>>,
    /// Test hooks: the last commit's result; segments grown on the UI thread while background
    /// jobs are on (there must be none); a lock workers wait on before growing; workers that fail.
    #[cfg(test)]
    pub(crate) last_commit: Option<Result<Value, String>>,
    #[cfg(test)]
    pub(crate) inline_grows: usize,
    #[cfg(test)]
    pub(crate) gate: Option<Arc<std::sync::RwLock<()>>>,
    #[cfg(test)]
    pub(crate) fail_workers: bool,
}

impl QuickLive {
    fn new(doc: DocId, settings: Value, src: QuickSource) -> Self {
        let shown = QuickStroke::new(src.selection().cloned());
        let cutter = Cutter::new(src.size());
        Self {
            doc,
            settings,
            src,
            generation: 0,
            shown_sel: shown.selection().cloned().map(Arc::new),
            shown_bounds: bounds(&shown),
            shown,
            grown: 0,
            points: Vec::new(),
            read: 0,
            cutter,
            sent: 0,
            sent_points: 0,
            job: None,
            released: false,
            failed: false,
            after: Vec::new(),
            updates: 0,
            throttle: Throttle::default(),
            ctx: None,
            #[cfg(test)]
            hooks: (None, false),
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Read the drag points not read yet, painted in `mode`, into the cut.
    fn read(&mut self, drag: &[[f64; 3]], mode: SelectionMode) {
        for q in drag.get(self.read..).unwrap_or_default() {
            if let Some(p) = quick_stroke::point(q[0], q[1]) {
                self.points.push(p);
                self.cutter.push(p, mode);
            }
        }
        self.read = drag.len();
    }

    /// The button went up: the open points become the last segment.
    fn release(&mut self) {
        self.cutter.finish();
        self.released = true;
    }

    /// The next closed segment to grow.
    fn next(&mut self) -> Option<(Vec<(f32, f32)>, SelectionMode)> {
        let &(n, mode) = self.cutter.closed().get(self.sent)?;
        let end = self.sent_points.saturating_add(n);
        let pts = self.points.get(self.sent_points..end)?.to_vec();
        self.sent += 1;
        self.sent_points = end;
        Some((pts, mode))
    }

    /// Every closed segment has grown (and, released, the cut is complete).
    fn caught_up(&self) -> bool {
        !self.busy() && self.sent == self.cutter.closed().len()
    }

    /// The document changed under the stroke: grow its segments again from the start, on what the
    /// document is now (a segment growing for the old one is dropped when it arrives).
    fn rebase(&mut self, session: &photocraft_engine::Session) -> Result<(), String> {
        self.src = QuickSource::new(session, &self.settings).map_err(|e| e.to_string())?;
        self.generation += 1;
        self.shown = QuickStroke::new(self.src.selection().cloned());
        self.shown_sel = self.shown.selection().cloned().map(Arc::new);
        self.shown_bounds = bounds(&self.shown);
        self.grown = 0;
        self.sent = 0;
        self.sent_points = 0;
        self.job = None;
        Ok(())
    }

    /// Grow `pts` in `mode` now, on this thread.
    fn grow_inline(&mut self, pts: &[(f32, f32)], mode: SelectionMode) {
        self.shown.push(&self.src, pts, mode);
        self.shown_sel = self.shown.selection().cloned().map(Arc::new);
        self.shown_bounds = bounds(&self.shown);
        self.grown += 1;
        self.updates += 1;
    }

    /// Grow `pts` in `mode` on a worker, from the stroke as shown now, and trace its outline for
    /// `view`.
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn(&mut self, pts: Vec<(f32, f32)>, mode: SelectionMode, view: Option<OutlineView>) -> bool {
        let (src, mut stroke, ctx, generation) = (self.src.clone(), self.shown.clone(), self.ctx.clone(), self.generation);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        #[cfg(test)]
        let hooks = self.hooks.clone();
        let work = move || {
            #[cfg(test)]
            let _held = hooks.0.as_ref().map(|g| g.read().unwrap_or_else(std::sync::PoisonError::into_inner));
            let grow = move || {
                #[cfg(test)]
                if hooks.1 {
                    std::panic::resume_unwind(Box::new("a failing worker (test)"));
                }
                stroke.push(&src, &pts, mode);
                let bounds = bounds(&stroke);
                let outline = view.zip(stroke.selection()).map(|(v, sel)| {
                    let segs = crate::outline::outline_scaled(sel, bounds.intersect(&v.vis), v.step);
                    (v.doc, crate::canvas::outline_key(sel, v.step, v.doc, v.vis, 1), Arc::new(segs))
                });
                let sel = stroke.selection().cloned().map(Arc::new);
                Grown { generation, stroke, sel, bounds, outline }
            };
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

    /// Take a finished worker's segment (`wait`: block until it is done): its traced outline, if
    /// any. An error when the worker failed.
    #[cfg(not(target_arch = "wasm32"))]
    fn collect(&mut self, wait: bool) -> Result<Option<Traced>, String> {
        let Some(job) = &self.job else { return Ok(None) };
        let got = if wait { job.rx.recv().map_err(|_| std::sync::mpsc::TryRecvError::Disconnected) } else { job.rx.try_recv() };
        let grown = match got {
            Ok(g) => g,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        self.job = None;
        let grown = grown.ok_or_else(|| tl!("Quick Selection failed with an internal error; the selection is unchanged").to_string())?;
        if grown.generation != self.generation {
            return Ok(None);
        }
        self.shown_bounds = grown.bounds;
        self.shown_sel = grown.sel;
        self.shown = grown.stroke;
        self.grown += 1;
        self.updates += 1;
        Ok(grown.outline)
    }

    #[cfg(target_arch = "wasm32")]
    fn collect(&mut self, _wait: bool) -> Result<Option<Traced>, String> {
        Ok(None)
    }
}

/// `select.quick`'s settings from the options bar and the brush, without the stroke.
pub(crate) fn settings(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options;
    json!({"size": app.session.tools.brush.size, "sampleAllLayers": o.sample_all.quick_selection, "enhanceEdge": o.enhance_edge})
}

/// Start the live stroke of the Quick Selection drag that just began (`app.drag`): it queues
/// behind any stroke still growing. Without one (no document, say) the release selects the whole
/// stroke at once, as before.
pub(crate) fn begin(app: &mut PhotocraftApp) {
    // Any stroke still marked as dragged lost its release: this press starts the next one.
    end_lost_drags(app, false);
    if !app.drag.as_ref().is_some_and(|d| d.tool == Tool::QuickSelection) {
        return;
    }
    let settings = settings(app);
    let Some(doc) = app.session.active().map(|st| st.doc.id) else { return };
    let Ok(src) = QuickSource::new(&app.session, &settings) else { return };
    #[allow(unused_mut)]
    let mut live = QuickLive::new(doc, settings, src);
    #[cfg(test)]
    {
        live.hooks = (app.quick.gate.clone(), app.quick.fail_workers);
    }
    app.quick.strokes.push(live);
    pump(app);
}

/// A stroke still marked as dragged whose drag ended some other way (a lost release, a closed
/// document) is released: it commits what it has. With `keep_current`, the newest stroke stays
/// as it is while a Quick Selection drag is under way (it is that drag's).
fn end_lost_drags(app: &mut PhotocraftApp, keep_current: bool) {
    let dragging = keep_current && app.drag.as_ref().is_some_and(|d| d.tool == Tool::QuickSelection);
    let current = app.quick.strokes.len().checked_sub(1);
    for (i, l) in app.quick.strokes.iter_mut().enumerate() {
        if !l.released && !(dragging && Some(i) == current) {
            l.release();
        }
    }
}

/// Grows whatever is ready: reads the drag's new points into the cut, takes a finished segment,
/// rebases a stroke whose document changed, starts the next segment, commits a released stroke
/// once it has caught up and then lets the next one go. Called for every batch of pointer moves
/// and once a frame ([`tick`]).
pub(crate) fn pump(app: &mut PhotocraftApp) {
    let bar = app.ui.quick_selection_mode;
    if let Some(d) = app.drag.as_ref().filter(|d| d.tool == Tool::QuickSelection)
        && let Some(live) = app.quick.strokes.last_mut().filter(|l| !l.released)
    {
        live.read(&d.points, segment_mode(bar, d.live));
    }
    while step(app) {}
    if !finishing(app) {
        let status = app.ui.status.clone();
        for w in app.quick.waiters.drain(..) {
            let _ = w.send(json!({"ok": true, "result": {"status": status}}));
        }
    }
}

/// Advance the oldest stroke. True when it has been committed (or dropped): the next one may go.
fn step(app: &mut PhotocraftApp) -> bool {
    let inline = cfg!(target_arch = "wasm32") || !app.background_jobs;
    let now = app.last_frame_time * 1000.0;
    let active = app.session.active().map(|st| (st.doc.id, st.revision));
    let view = app.quick.outline_view;
    let Some(front) = app.quick.strokes.first_mut() else { return false };
    if front.failed {
        if !front.released {
            return false;
        }
        drop_front(app);
        return true;
    }
    match front.collect(false) {
        Ok(Some(outline)) => app.outline_cache = Some(outline),
        Ok(None) => {}
        Err(e) => {
            front.failed = true;
            app.ui.status = e;
            app.ui.status_error = true;
            return false;
        }
    }
    match active {
        Some((doc, revision)) if doc == front.doc => {
            if front.src.base().1 != revision
                && let Err(e) = front.rebase(&app.session)
            {
                front.failed = true;
                app.ui.status = e;
                app.ui.status_error = true;
                return false;
            }
        }
        // Its document isn't the active one: a released stroke can't be committed there.
        _ if front.released => {
            app.ui.status = tl!("The Quick Selection stroke was dropped: its document is no longer the active one").into();
            app.ui.status_error = true;
            drop_front(app);
            return true;
        }
        _ => return false,
    }
    let view = view.filter(|v| v.doc == front.doc);
    while !front.busy() {
        if cfg!(target_arch = "wasm32") && !front.throttle.ready(now) {
            break;
        }
        let Some((pts, mode)) = front.next() else { break };
        if inline {
            #[cfg(test)]
            if app.background_jobs {
                app.quick.inline_grows += 1;
            }
            front.grow_inline(&pts, mode);
            if cfg!(target_arch = "wasm32") {
                front.throttle.grew(now);
                break;
            }
            continue;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !front.spawn(pts.clone(), mode, view) {
            // No thread to be had: grow it here rather than lose it.
            #[cfg(test)]
            {
                app.quick.inline_grows += 1;
            }
            front.grow_inline(&pts, mode);
        }
    }
    let _ = view;
    if front.released && front.caught_up() {
        commit_front(app);
        return true;
    }
    false
}

/// Drop the oldest stroke uncommitted (it failed, or its document went), running the commands
/// that waited for it.
fn drop_front(app: &mut PhotocraftApp) {
    if app.quick.strokes.is_empty() {
        return;
    }
    let live = app.quick.strokes.remove(0);
    run_after(app, live.after);
}

/// Commit the oldest stroke as one `select.quick`, taking its live result, then run the commands
/// that waited for it.
fn commit_front(app: &mut PhotocraftApp) {
    if app.quick.strokes.is_empty() {
        return;
    }
    let live = app.quick.strokes.remove(0);
    if !live.shown.segments().is_empty() {
        let mut p = live.settings.clone();
        if let (Some(o), Value::Object(s)) = (p.as_object_mut(), live.shown.params()) {
            o.extend(s);
        }
        app.session.offer_live_quick(QuickCommit::new(live.src, live.shown));
        app.quick.committing = true;
        let r = app.run("select.quick", p);
        app.quick.committing = false;
        // An offer the command didn't take (it was refused) must not linger.
        app.session.clear_live_stroke();
        #[cfg(test)]
        {
            app.quick.last_commit = Some(r.clone());
        }
        committed(app, r);
    }
    run_after(app, live.after);
}

/// Run commands that waited for a stroke, in order, before any later stroke.
fn run_after(app: &mut PhotocraftApp, after: Vec<(String, Value)>) {
    if after.is_empty() {
        return;
    }
    app.quick.committing = true;
    for (id, p) in after {
        if let Err(e) = app.run(&id, p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    app.quick.committing = false;
}

/// The button went up on the stroke `d`: its last points become the final segment, committed
/// once grown, and New switches to Add, as in Photoshop. False when the stroke has no live state
/// (the caller selects it at once).
pub(crate) fn release(app: &mut PhotocraftApp, d: &crate::canvas::Drag) -> bool {
    let bar = app.ui.quick_selection_mode;
    let Some(live) = app.quick.strokes.last_mut().filter(|l| !l.released) else { return false };
    live.read(&d.points, segment_mode(bar, d.live));
    live.release();
    if app.ui.quick_selection_mode == NEW {
        app.ui.quick_selection_mode = ADD;
    }
    pump(app);
    true
}

/// A command `id` coming while a released stroke still grows: one that edits the document is
/// kept to run after that stroke's commit, in order (`Some`, a pending result); view, tool and
/// other document-independent commands run at once (`None`), as does everything else.
pub(crate) fn defer(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if app.quick.committing || photocraft_engine::jobs::independent_of_document(id) {
        return None;
    }
    let live = app.quick.strokes.iter_mut().rev().find(|l| l.released)?;
    live.after.push((id.to_string(), params.clone()));
    Some(Ok(json!({"pending": true, "after": "select.quick"})))
}

/// A released stroke is still growing (its commit is to come).
pub fn finishing(app: &PhotocraftApp) -> bool {
    app.quick.strokes.iter().any(|l| l.released)
}

/// Reply to a control request once every released stroke has been committed (`ui.pointer`).
pub(crate) fn reply_when_committed(app: &mut PhotocraftApp, reply: std::sync::mpsc::Sender<ControlResponse>) {
    app.quick.waiters.push(reply);
    pump(app);
}

/// Once a frame: keep strokes growing (and committing) after their last pointer event, and keep
/// frames coming while one grows.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.quick.strokes.is_empty() {
        return;
    }
    for l in &mut app.quick.strokes {
        if l.ctx.is_none() {
            l.ctx = Some(ctx.clone());
        }
    }
    end_lost_drags(app, true);
    pump(app);
    if !app.quick.strokes.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// After a Quick Selection stroke's `select.quick`: New switches to Add once a stroke has
/// selected something, as in Photoshop; a stroke that found nothing says so.
pub(crate) fn committed(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if r.as_ref().is_ok_and(|v| v.get("changed").and_then(Value::as_bool) != Some(false)) && app.ui.quick_selection_mode == NEW {
        app.ui.quick_selection_mode = ADD;
    }
    crate::retouch_ui::report_smart_result(app, r, tl!("Quick Selection found nothing there"));
}

/// The selection the canvas outlines on document `doc` while a stroke there has grown, shared
/// (no copy), with its content bounds: the newest stroke that has grown on its current base
/// (`None`: the document's; `Some((None, _))`: nothing selected so far).
pub(crate) fn shown(app: &PhotocraftApp, doc: DocId) -> Option<(Option<Arc<Surface>>, Rect)> {
    let live = app.quick.strokes.iter().rev().filter(|l| l.doc == doc && !l.failed).find(|l| l.grown > 0)?;
    Some((live.shown_sel.clone(), live.shown_bounds))
}

/// Whether the stroke being dragged has grown on its base (until then the canvas draws the
/// brush trail under the pointer instead).
pub(crate) fn has_grown(app: &PhotocraftApp) -> bool {
    app.quick.strokes.last().is_some_and(|l| l.grown > 0 && !l.failed)
}

/// No stroke is under way or waiting to be committed.
pub fn idle(app: &PhotocraftApp) -> bool {
    app.quick.strokes.is_empty()
}

/// Wait for the segments growing now and those they let start, until nothing grows (tests and
/// the bench drive the threaded path step by step).
#[cfg(not(target_arch = "wasm32"))]
pub fn settle(app: &mut PhotocraftApp) {
    for _ in 0..100_000 {
        let Some(front) = app.quick.strokes.first_mut() else { return };
        if !front.busy() {
            pump(app);
            if app.quick.strokes.first().is_none_or(|f| !f.busy()) {
                return;
            }
            continue;
        }
        match front.collect(true) {
            Ok(Some(outline)) => app.outline_cache = Some(outline),
            Ok(None) => {}
            Err(e) => {
                front.failed = true;
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        pump(app);
    }
}

/// Times the outline changed during the newest stroke, and whether a segment is growing (for the
/// bench and tests).
pub fn progress(app: &PhotocraftApp) -> Option<(usize, bool)> {
    let last = app.quick.strokes.last()?;
    Some((last.updates, app.quick.strokes.iter().any(QuickLive::busy)))
}

#[cfg(test)]
#[path = "quick_select_ui_tests.rs"]
mod tests;
