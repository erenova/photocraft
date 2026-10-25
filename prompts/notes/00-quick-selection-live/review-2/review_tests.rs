//! Review round 2 proofs (temporary; never part of the branch).
use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::{Value, json};
use std::sync::Arc;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

const RED: [f32; 4] = [0.85, 0.15, 0.1, 1.0];
const BLUE: [f32; 4] = [0.1, 0.25, 0.8, 1.0];

fn paint(app: &mut PhotocraftApp) {
    app.session.execute("file.new", json!({"width": 1600, "height": 600})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 1200, 600), &RED);
            s.fill_rect(Rect::new(1200, 0, 1600, 600), &BLUE);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::QuickSelection;
    app.session.tools.brush.size = 20.0;
}

/// A stroke released while its worker is held on `gate` (the caller holds the write lock).
fn released_stroke_held(app: &mut PhotocraftApp, gate: &Arc<std::sync::RwLock<()>>) {
    app.background_jobs = true;
    app.quick.gate = Some(gate.clone());
    tool_event(app, ToolEvent::Down { x: 100.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 160.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 200.0, y: 300.0 }, Modifiers::NONE);
    assert!(finishing(app));
}

#[test]
fn review_a_deferred_command_lands_on_another_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    paint(&mut app);
    let doc_a = app.session.active().unwrap().doc.id;
    let gate = Arc::new(std::sync::RwLock::new(()));
    let held = gate.write().unwrap();
    released_stroke_held(&mut app, &gate);
    // Meant for document A: deferred behind its stroke.
    let r = app.run("select.all", json!({})).unwrap();
    assert_eq!(r["pending"], true, "{r}");
    // A new document: independent of the document, so it runs at once and becomes active.
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let doc_b = app.session.active().unwrap().doc.id;
    assert_ne!(doc_a, doc_b);
    drop(held);
    settle(&mut app);
    assert!(idle(&app));
    let a_sel = app.session.documents().iter().find(|st| st.doc.id == doc_a).unwrap().doc.selection.is_some();
    let b_sel = app.session.active().unwrap().doc.selection.is_some();
    let journal: Vec<String> = app.session.journal.iter().map(|(id, _)| id.clone()).collect();
    eprintln!("REVIEW-A: doc A selected {a_sel}, doc B selected {b_sel}, status {:?}, journal {journal:?}", app.ui.status);
    assert!(!a_sel && b_sel, "proof: A lost its stroke and B received the Select All meant for A");
}

fn deny_new_layer(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "layer.new.layer" { Err(photocraft_engine::EngineError::Other("layer.new.layer is denied to automation (review)".into())) } else { Ok(()) }
}

#[test]
fn review_a_deferred_control_command_escapes_the_automation_gate_and_loses_its_reply() {
    let services = crate::Services {
        automation_command: Some(Box::new(|id, _| if id == "layer.new.layer" { Err("layer.new.layer is denied to automation (review)".into()) } else { Ok(()) })),
        automation_authorize: Some(deny_new_layer),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    paint(&mut app);
    app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
        name: "Add".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        ..Default::default()
    });
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, method: &str, params: Value| -> Value {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    };
    let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
    let n0 = layers(&app);
    // Baseline, no stroke: the play's step is refused by the gate; bad params are an error.
    let r = call(&mut app, "engine.execute", json!({"command": "actions.play", "params": {"action": "Add"}}));
    eprintln!("REVIEW-B baseline play: {r}");
    assert_eq!(layers(&app), n0, "the gate refuses the step: {r}");
    let r = call(&mut app, "engine.execute", json!({"command": "select.quick", "params": {"points": []}}));
    eprintln!("REVIEW-B baseline bad params: {r}");
    assert!(r.get("error").is_some(), "{r}");
    let r = call(&mut app, "engine.commands", json!({}));
    assert!(r["result"].is_array(), "baseline engine.commands is an array");
    // The same requests while a released stroke still grows.
    let gate = Arc::new(std::sync::RwLock::new(()));
    let held = gate.write().unwrap();
    released_stroke_held(&mut app, &gate);
    let play = call(&mut app, "engine.execute", json!({"command": "actions.play", "params": {"action": "Add"}}));
    let bad = call(&mut app, "engine.execute", json!({"command": "select.quick", "params": {"points": []}}));
    let list = call(&mut app, "engine.commands", json!({}));
    let all = call(&mut app, "engine.execute", json!({"command": "select.all", "params": {}}));
    eprintln!("REVIEW-B while finishing: play {play}\n  bad params {bad}\n  engine.commands {list}\n  select.all {all}");
    drop(held);
    settle(&mut app);
    eprintln!("REVIEW-B after commit: layers {} (was {n0}), status {:?}", layers(&app), app.ui.status);
    assert!(bad.get("error").is_none(), "proof: an erroring command replied ok");
    assert!(!list["result"].is_array(), "proof: engine.commands replied without the command list");
    assert_eq!(layers(&app), n0 + 1, "proof: the deferred play ran its step without the automation gate");
}

#[test]
fn review_a_view_prefixed_document_edit_jumps_the_queue_and_rebases_the_released_stroke() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    paint(&mut app);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let held = gate.write().unwrap();
    released_stroke_held(&mut app, &gate);
    let before = app.quick.strokes.first().map(|l| l.generation);
    // A guide dragged out of the ruler after the release: a history step, but `view.` runs at once.
    let r = app.run("view.newGuide", json!({"orientation": "vertical", "position": 40})).unwrap();
    pump(&mut app);
    let after = app.quick.strokes.first().map(|l| l.generation);
    drop(held);
    settle(&mut app);
    assert!(idle(&app));
    let journal: Vec<&str> = app.session.journal.iter().map(|(id, _)| id.as_str()).filter(|id| *id == "view.newGuide" || *id == "select.quick").collect();
    let undo = app.session.active().unwrap().history.undo_label().map(str::to_string);
    eprintln!("REVIEW-C guide reply {r}; stroke generation {before:?} -> {after:?}; journal {journal:?}; undo label {undo:?}");
    assert_eq!(journal, ["view.newGuide", "select.quick"], "proof: the guide added after the release is applied before the stroke");
    assert_ne!(before, after, "proof: the released stroke was rebased (every segment regrown)");
}
