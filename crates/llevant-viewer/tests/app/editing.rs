//! Editing the input: cards edited, undone and saved, surfaces dragged, new cards.

use super::*;

/// A cell's fields edited in the inspector (decision 0012, phase 8): a
/// parameter added, Apply, and the card written with only that word new.
#[test]
fn cell_fields_edited() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-fields-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    std::fs::copy(model("universes.i"), &path).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let m = h.state().document().unwrap().model.clone();
    let b = m.bounds(llevant::Scope::Problem);
    let centre = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
    let p = llevant_viewer::pick::pick_point(&m, centre);
    let cell = m.cell_id(p.cell().expect("a cell at the centre"));
    let card = |h: &Harness<'_, ViewerApp>| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().cell(cell).unwrap().text.clone()
    };
    let before = card(&h);
    assert!(!before.to_lowercase().contains("vol="), "{before}");
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Focus(Some(p)));
    panel(&mut h, Panel::Inspector);
    h.run_steps(2);
    h.get_by_label("New parameter key").click();
    h.run_steps(1);
    h.get_by_label("New parameter key").type_text("vol");
    h.run_steps(1);
    h.get_by_label("New parameter value").click();
    h.run_steps(1);
    h.get_by_label("New parameter value").type_text("12.5");
    h.run_steps(1);
    h.get_by_label("Add parameter").click();
    h.run_steps(2);
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("fields.png"));
    }
    h.get_by_label("Apply fields").click();
    h.step();
    step_until(&mut h, "the edit to compile", |a| !a.is_loading());
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (1, true));
    let after = card(&h);
    assert!(after.contains("vol=12.5"), "{after}");
    // Only the word added: the card as it was, but for it.
    assert_eq!(
        after.replace(" vol=12.5", ""),
        before,
        "{before}
{after}"
    );
    // The form closed once the card read so.
    assert!(h.query_by_label("Apply fields").is_none());

    // Its material (the point still inspected, in the model compiled
    // after the edit): fractions written as mass fractions.
    let mat = m.cell(m.cell_index(cell).unwrap()).material;
    h.get_by_label(&format!("Inspect m {mat}")).click();
    h.run_steps(2);
    h.get_by_label("mass").click();
    h.run_steps(2);
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("material.png"));
    }
    h.get_by_label("Apply fields").click();
    h.step();
    step_until(&mut h, "the material's edit", |a| !a.is_loading());
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (2, true));
    let d = h.state().document().unwrap();
    let text = d.source.as_ref().unwrap().material(mat).unwrap().text;
    let (_, f) = &d
        .source
        .as_ref()
        .unwrap()
        .material_fields(mat)
        .unwrap()
        .entries[0];
    assert_eq!(*f, -1.0, "{text}");

    // A PZ of the cell, written as P once confirmed (coefficients named).
    h.get_by_label("Inspector back").click();
    h.run_steps(2);
    let ctx = h.ctx.clone();
    h.state_mut().send(
        Command::Inspect(llevant_viewer::panels::inspector::Subject::Surface(302)),
        &ctx,
    );
    h.run_steps(2);
    h.get_by_label("OTHER FORMS").click();
    h.run_steps(2);
    h.get_by_label("As P").click();
    h.run_steps(2);
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("surface.png"));
    }
    h.get_by_label("Write it").click();
    h.step();
    step_until(&mut h, "the surface's edit", |a| !a.is_loading());
    assert_eq!(h.state().input_edits(), (3, true));
    let d = h.state().document().unwrap();
    let (kind, c) = d.source.as_ref().unwrap().surface_coeffs(302).unwrap();
    assert_eq!((kind.as_str(), c), ("p", vec![0.0, 0.0, 1.0, -4.0]));
}

/// A cell's geometry edited as text (decision 0012, phase 8): a surface
/// number completed, an unknown one flagged (Apply waits), the surfaces
/// marked on the slice while the form is open, and the card written.
#[test]
fn cell_geometry_edited() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-geometry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    std::fs::copy(model("universes.i"), &path).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let m = h.state().document().unwrap().model.clone();
    let b = m.bounds(llevant::Scope::Problem);
    let centre = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
    let p = llevant_viewer::pick::pick_point(&m, centre);
    let cell = m.cell_id(p.cell().expect("a cell at the centre"));
    assert_eq!(cell, 400, "the model's cell at its centre");
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Focus(Some(p)));
    panel(&mut h, Panel::Inspector);
    h.run_steps(2);
    // Nothing marked while the cell is only looked at.
    assert!(h.state().document().unwrap().marks.request().is_none());
    // Hovering a surface's number in the geometry marks that one: the
    // pointer over "302" of "-301 302 -303" (monospace, the field's margin
    // of 4 points).
    let field = h.get_by_label("Geometry").rect();
    let glyph = h
        .ctx
        .fonts_mut(|f| f.glyph_width(&egui::FontId::monospace(12.5), '0'));
    let over = egui::pos2(
        field.left() + 4.0 + 6.5 * glyph,
        field.top() + 2.0 + 0.5 * 12.5,
    );
    h.event(egui::Event::PointerMoved(over));
    h.run_steps(3);
    let req = h.state().document().unwrap().marks.request().cloned();
    assert_eq!(req.map(|r| r.surfaces), Some(vec![302]));
    h.event(egui::Event::PointerMoved(
        field.right_bottom() + egui::vec2(0.0, 300.0),
    ));
    h.run_steps(2);
    // Its surfaces marked on the slice (the plane through the point) once
    // its geometry has the keyboard.
    h.get_by_label("Geometry").click();
    h.run_steps(3);
    step_until(&mut h, "the marks", |a| {
        !a.document().unwrap().marks.segments.is_empty()
    });
    let req = h
        .state()
        .document()
        .unwrap()
        .marks
        .request()
        .cloned()
        .unwrap();
    assert_eq!((req.cell, req.surfaces), (400, vec![301, 302, 303]));

    h.get_by_label("Geometry").click();
    h.run_steps(1);
    h.key_press(egui::Key::End);
    h.run_steps(1);
    h.get_by_label("Geometry").type_text(" -30");
    h.run_steps(2);
    h.get_by_label("Complete 304").click();
    h.run_steps(2);
    h.get_by_label("Geometry").type_text(" 999");
    h.run_steps(2);
    h.get_by_label_contains("no surface 999");
    assert!(
        h.get_by_label("Apply fields")
            .accesskit_node()
            .is_disabled(),
        "Apply waits while a word is flagged"
    );
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        // Zoomed in on the cell, its surfaces marked.
        {
            let d = h.state_mut().document_mut().unwrap();
            d.slice.width = 16.0;
            d.slice.centre = [285.0, 0.0];
        }
        h.run_steps(5);
        step_until(&mut h, "the marks", |a| {
            let d = a.document().unwrap();
            !d.marks.segments.is_empty() && d.slice.progress() >= 1.0
        });
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("geometry.png"));
    }
    for _ in 0..4 {
        h.key_press(egui::Key::Backspace);
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.query_by_label_contains("no surface 999").is_none());
    let req = h
        .state()
        .document()
        .unwrap()
        .marks
        .request()
        .cloned()
        .unwrap();
    assert_eq!(req.surfaces, [301, 302, 303, 304]);
    // The cell as edited drawn on the slice before Apply.
    assert_eq!(
        req.preview,
        Some(llevant_viewer::marks::Preview::Cell {
            id: 400,
            geometry: "-301 302 -303 -304".into()
        })
    );
    {
        let d = h.state_mut().document_mut().unwrap();
        d.slice.width = 16.0;
        d.slice.centre = [285.0, 0.0];
    }
    h.run_steps(3);
    step_until(&mut h, "the cell as edited", |a| {
        let d = a.document().unwrap();
        d.marks
            .segments
            .iter()
            .any(|s| s.surface == llevant_viewer::marks::EDITED)
    });
    // And in 3D: its shell traced for the camera of the view.
    step_until(&mut h, "the shell in 3D", |a| {
        a.document()
            .unwrap()
            .marks
            .shell()
            .is_some_and(|s| s.words.iter().step_by(2).any(|&t| t > 0.0))
    });
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        // The 3D view close to the cell.
        let around = llevant::Aabb {
            lo: [281.0, -3.0, -3.0],
            hi: [289.0, 3.0, 3.0],
        };
        h.state_mut().document_mut().unwrap().orbit =
            llevant_viewer::camera::Orbit::fit(&around, 35.0, 25.0);
        h.run_steps(3);
        step_until(&mut h, "the shell of the camera moved", |a| {
            let d = a.document().unwrap();
            d.marks
                .shell()
                .is_some_and(|s| s.camera.eye == d.orbit.eye())
                && !d.view3d.busy()
        });
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("edited.png"));
    }
    h.get_by_label("Apply fields").click();
    h.step();
    step_until(&mut h, "the edit to compile", |a| !a.is_loading());
    h.run_steps(3);
    let d = h.state().document().unwrap();
    let card = d.source.as_ref().unwrap().cell(400).unwrap().text;
    assert!(card.contains("-301 302 -303 -304 trcl"), "{card}");
    assert!(
        d.marks.request().is_none_or(|r| r.preview.is_none()),
        "the form closed: no cell as edited drawn"
    );
}

/// Cards edited in a universe opened on its own: compiled against the
/// whole model, the universe cut out of it again and still shown (its
/// camera kept), the whole model behind it edited too; what is shared with
/// cells outside it said; a universe the edit takes away goes back to the
/// whole model.
#[test]
fn edits_in_a_universe_opened_on_its_own() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-open-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    std::fs::copy(model("universes.i"), &path).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let whole_before = h.state().document().unwrap().model.clone();
    // u=2: cell 211 inside surface 111 (filled with u=1), 212 around it.
    h.state_mut().send(Command::OpenUniverse(2), &ctx);
    step_until(&mut h, "u=2 to open", |a| a.open_universes() == vec![2]);
    h.run_steps(2);
    let cell_at = |h: &Harness<'_, ViewerApp>, p: [f64; 3]| {
        let m = &h.state().document().unwrap().model;
        let l = m.classify_point(p);
        l.path.first().map(|s| m.cell(s.cell).id)
    };
    assert_eq!(cell_at(&h, [8.5, 0.0, 0.0]), Some(211));
    let camera = h.state().document().unwrap().orbit.state;

    // Material 2 is used by cells outside u=2: said on its page.
    h.state_mut().send(
        Command::Inspect(llevant_viewer::panels::inspector::Subject::Material(2)),
        &ctx,
    );
    panel(&mut h, Panel::Inspector);
    h.run_steps(3);
    h.get_by_label_contains("outside u=2");

    // Surface 111 made smaller, in u=2 as shown.
    let kind = llevant_scene::source::CardKind::Surface;
    h.state_mut()
        .send(Command::EditCard(kind, 111, "111 so 8".into()), &ctx);
    h.step();
    step_until(&mut h, "the edit to compile", |a| !a.is_loading());
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (1, true));
    assert_eq!(h.state().open_universes(), vec![2], "still in u=2");
    let d = h.state().document().unwrap();
    assert_eq!(d.universe, Some(2));
    assert_eq!(d.orbit.state, camera, "the camera kept");
    assert!(d.whole.is_some());
    // The new u=2 shown: 8.5 cm out is beyond 211 now (in the void around
    // u=2, whose box is measured again), 7.5 cm still in it.
    assert_ne!(cell_at(&h, [8.5, 0.0, 0.0]), Some(211), "the new u=2 shown");
    assert_eq!(cell_at(&h, [7.5, 0.0, 0.0]), Some(211));
    // The whole model behind it: the edited one.
    h.state_mut().send(Command::Back, &ctx);
    h.run_steps(2);
    let d = h.state().document().unwrap();
    assert_eq!(d.universe, None);
    assert!(!std::sync::Arc::ptr_eq(&d.model, &whole_before));
    assert!(d.whole.is_none());

    // Undo from inside u=2 works the same way.
    h.state_mut().send(Command::OpenUniverse(2), &ctx);
    step_until(&mut h, "u=2 to open again", |a| {
        a.open_universes() == vec![2]
    });
    h.state_mut().send(Command::UndoInputEdit, &ctx);
    h.step();
    step_until(&mut h, "the edit taken back", |a| !a.is_loading());
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (0, false));
    assert_eq!(h.state().open_universes(), vec![2]);
    assert_eq!(cell_at(&h, [8.5, 0.0, 0.0]), Some(211));

    // Dragged live in u=2 (the draft of u=2 alone), released: written and
    // compiled as any edit, u=2 still shown.
    for r in [9.2, 9.4, 9.5] {
        h.state_mut().send(Command::DragSurface(111, vec![r]), &ctx);
        h.step();
    }
    step_until(&mut h, "the drag's steps", |a| {
        a.document()
            .and_then(|d| d.drag.as_ref())
            .is_some_and(|i| i.live && i.steps > 0)
    });
    assert!(h
        .state()
        .document()
        .unwrap()
        .drag
        .as_ref()
        .unwrap()
        .problem
        .is_none());
    h.state_mut().send(Command::EndDrag { commit: true }, &ctx);
    step_until(&mut h, "the drag's edit to land", |a| {
        !a.is_loading() && a.document().is_some_and(|d| d.drag.is_none())
    });
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (1, true));
    assert_eq!(h.state().open_universes(), vec![2]);
    let card = h
        .state()
        .document()
        .unwrap()
        .source
        .as_ref()
        .unwrap()
        .surface(111)
        .unwrap()
        .text;
    assert!(card.contains("9.5"), "{card}");
    assert_eq!(cell_at(&h, [9.3, 0.0, 0.0]), Some(211));

    // An edit that takes u=2 away (its cells moved to u=3): back to the
    // whole model, saying so.
    let cell = llevant_scene::source::CardKind::Cell;
    let edit = |h: &mut Harness<'_, ViewerApp>, id: i64, text: &str| {
        let ctx = h.ctx.clone();
        h.state_mut()
            .send(Command::EditCard(cell, id, text.into()), &ctx);
        h.step();
        step_until(h, "the edit to compile", |a| !a.is_loading());
        h.run_steps(2);
    };
    edit(&mut h, 212, "212 2 -2.0 111 u=3 imp:n=1");
    assert_eq!(h.state().open_universes(), vec![2], "u=2 still has 211");
    edit(&mut h, 211, "211 0 -111 fill=1 (3) u=3 imp:n=1");
    assert!(h.state().open_universes().is_empty());
    let d = h.state().document().unwrap();
    assert_eq!(d.universe, None);
    assert!(
        all_said(d).contains("no longer in the model"),
        "{}",
        all_said(d)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A card edited in the viewer: compiled again in place, taken back, edited
/// again and saved (the watcher not taking the viewer's own write for a
/// change on disk); an edit that cannot be made changes nothing.
#[test]
fn cards_edited_undone_and_saved() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    let text = std::fs::read_to_string(model("universes.i")).unwrap();
    std::fs::write(&path, &text).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    // The new text of a surface card, as migjorn writes it.
    let mut src = migjorn::Model::parse(&text);
    let id = src.surfaces().filter_map(|s| s.id()).next().unwrap();
    let c = src.surface(id).unwrap().coeffs().unwrap();
    let k = c.len() - 1;
    src.surface_mut(id)
        .unwrap()
        .set_coeff(k, c[k] + 0.25)
        .unwrap();
    let card = src.surface(id).unwrap().text().trim_end().to_string();
    let edited = src.to_source();
    let kind = llevant_scene::source::CardKind::Surface;
    let card_of = |h: &Harness<'_, ViewerApp>| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().surface(id).unwrap().text.clone()
    };
    let before = card_of(&h);

    let edit = |h: &mut Harness<'_, ViewerApp>, c: Command| {
        // The checks an edit starts finish first, so that their message
        // does not take the place of this one's.
        step_until(h, "the jobs before", |a| {
            a.document().is_none_or(|d| d.results.jobs.is_empty())
        });
        let ctx = h.ctx.clone();
        h.state_mut().send(c, &ctx);
        h.step();
        step_until(h, "the edit to compile", |a| !a.is_loading());
        last_said(h.state().document().unwrap())
    };
    let msg = edit(&mut h, Command::EditCard(kind, id, card.clone()));
    assert!(msg.starts_with(&format!("edited surface {id}")), "{msg}");
    assert_eq!(h.state().input_edits(), (1, true));
    assert_eq!(card_of(&h).trim_end(), card);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "not saved");
    assert!(h.state().document().unwrap().changed_boxes.is_some());

    let msg = edit(&mut h, Command::UndoInputEdit);
    assert!(
        msg.starts_with(&format!("took back the edit of surface {id}")),
        "{msg}"
    );
    assert_eq!(h.state().input_edits(), (0, false));
    assert_eq!(card_of(&h), before);

    let msg = edit(&mut h, Command::EditCard(kind, id, "not a surface".into()));
    assert!(msg.starts_with("could not edit"), "{msg}");
    assert_eq!(h.state().input_edits(), (0, false));

    edit(&mut h, Command::EditCard(kind, id, card.clone()));
    let ctx = h.ctx.clone();
    h.state_mut().send(Command::SaveInput, &ctx);
    step_until(&mut h, "the input to be saved", |a| {
        a.input_edits() == (1, false)
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    // Its own write is no change on disk.
    h.state_mut().check_input_now();
    assert!(!h.state().input_changed(), "the viewer's own write");

    // From the inspector: a point's cell card, Edit card, a comment typed
    // (at the end of its first line), Apply.
    let m = h.state().document().unwrap().model.clone();
    let b = m.bounds(llevant::Scope::Problem);
    let centre = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
    let p = llevant_viewer::pick::pick_point(&m, centre);
    let cell = m.cell_id(p.cell().expect("a cell at the centre"));
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Focus(Some(p)));
    panel(&mut h, Panel::Inspector);
    h.run_steps(2);
    // (A cell's card is folded at first: its fields are above.)
    h.get_by_label_contains(&format!("cell {cell} card · line"))
        .click();
    h.run_steps(2);
    h.get_by_label("Edit card").click();
    h.run_steps(2);
    for _ in 0..20 {
        if h.query_by_label("Card text").is_some() {
            break;
        }
        h.step();
    }
    let field = h.get_by_label("Card text");
    field.click();
    h.run_steps(2);
    h.key_press(egui::Key::End);
    h.run_steps(1);
    h.get_by_label("Card text").type_text(" $ edited here");
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.step();
    step_until(&mut h, "the edit from the inspector", |a| !a.is_loading());
    assert_eq!(h.state().input_edits(), (2, true));
    let d = h.state().document().unwrap();
    let text = &d.source.as_ref().unwrap().cell(cell).unwrap().text;
    assert!(text.contains(" $ edited here"), "{text}");
    assert!(text.contains(&format!("{cell} ")), "{text}");
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("edit.png"));
    }

    // A TR card: the cells it places move.
    let tr = llevant_scene::source::CardKind::Transform;
    let msg = edit(&mut h, Command::EditCard(tr, 1, "tr1 14.0 1.5 -1".into()));
    assert!(msg.starts_with("edited transform 1"), "{msg}");
    assert_eq!(h.state().input_edits(), (3, true));
    let d = h.state().document().unwrap();
    let changed = d.changed_boxes.as_ref().map_or(0, |b| b.len());
    assert!(changed > 0, "the cells filled through tr1 changed");
    let tr1 = d.source.as_ref().unwrap().transform(1).unwrap().text;
    assert_eq!(tr1.trim_end(), "tr1 14.0 1.5 -1");

    // Save as: another file, which is the input from then on.
    let other = dir.join("edited.i");
    let ctx = h.ctx.clone();
    h.state_mut()
        .send(Command::SaveInputAs(Some(other.clone())), &ctx);
    step_until(&mut h, "the input to be saved as", |a| {
        a.input_edits() == (3, false)
    });
    assert_eq!(h.state().path(), other.as_path());
    let saved = std::fs::read_to_string(&other).unwrap();
    assert!(saved.contains("tr1 14.0 1.5 -1\n") && saved.contains(" $ edited here"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edited,
        "the first file kept"
    );

    // Opening a file drops the edits of the one open (Open asks first when
    // they are not saved; here the app is told directly).
    h.state_mut().open_file(path.clone(), &ctx);
    assert_eq!(h.state().input_edits(), (0, false));
    step_until(&mut h, "the file opened again", |a| !a.is_loading());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A surface dragged live: the 3D view patched and the slice tracing the
/// draft step by step, the card written and compiled by Apply; a drag
/// cancelled, or taken back to where it began, edits nothing and shows the
/// model again. The inspector offers the coefficients as fields to drag.
#[test]
fn surfaces_dragged_live() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-drag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    let text = std::fs::read_to_string(model("universes.i")).unwrap();
    std::fs::write(&path, &text).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    settle(&mut h);
    // A surface that can be dragged live.
    let base = h.state().document().unwrap().model.clone();
    let tree = migjorn::Model::parse(&text);
    let id = tree
        .surfaces()
        .filter_map(|s| s.id())
        .find(|&id| llevant::draft::Draft::begin(base.clone(), &tree, id).is_ok())
        .expect("a surface to drag");
    let start = tree.surface(id).unwrap().coeffs().unwrap();
    let k = start.len() - 1;
    let at = |by: f64| {
        let mut c = start.clone();
        c[k] += by;
        c
    };
    let send = |h: &mut Harness<'_, ViewerApp>, c: Command| {
        let ctx = h.ctx.clone();
        h.state_mut().send(c, &ctx);
        h.step();
    };
    let steps = |a: &ViewerApp| {
        a.document()
            .and_then(|d| d.drag.as_ref())
            .map_or(0, |i| i.steps)
    };
    let card_of = |h: &Harness<'_, ViewerApp>| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().surface(id).unwrap().text.clone()
    };
    let before = card_of(&h);

    // Dragged, then cancelled.
    for by in [0.05, 0.1, 0.15] {
        send(&mut h, Command::DragSurface(id, at(by)));
    }
    step_until(&mut h, "the drag's steps", |a| {
        a.document()
            .and_then(|d| d.drag.as_ref())
            .is_some_and(|i| i.live && i.steps > 0 && i.coeffs == at(0.15))
    });
    {
        let d = h.state().document().unwrap();
        assert!(d.drag.as_ref().unwrap().problem.is_none());
        eprintln!("dragged on the {} backend", d.view3d.backend_name());
    }
    step_until(&mut h, "the slice to trace the draft", |a| {
        let d = a.document().unwrap();
        !Arc::ptr_eq(d.slice.model(), &d.model) && steps(a) > 0
    });
    settle(&mut h);
    h.run_steps(5);
    send(&mut h, Command::EndDrag { commit: false });
    step_until(&mut h, "the drag to be taken back", |a| {
        a.document().is_some_and(|d| d.drag.is_none())
    });
    let d = h.state().document().unwrap();
    assert!(Arc::ptr_eq(d.slice.model(), &d.model));
    assert!(Arc::ptr_eq(&d.model, &base), "nothing compiled");
    assert_eq!(h.state().input_edits(), (0, false));
    assert_eq!(card_of(&h), before);

    // Dragged back to where it began: nothing to write.
    send(&mut h, Command::DragSurface(id, at(0.2)));
    send(&mut h, Command::DragSurface(id, at(0.0)));
    send(&mut h, Command::EndDrag { commit: true });
    step_until(&mut h, "the drag to end", |a| {
        a.document().is_some_and(|d| d.drag.is_none())
    });
    assert_eq!(h.state().input_edits(), (0, false));
    assert!(!h.state().is_loading());

    // Dragged and released: the card written, compiled as an edit.
    for by in [0.1, 0.2, 0.25] {
        send(&mut h, Command::DragSurface(id, at(by)));
    }
    step_until(&mut h, "a step", |a| steps(a) > 0);
    send(&mut h, Command::EndDrag { commit: true });
    step_until(&mut h, "the edit to land", |a| {
        !a.is_loading() && a.document().is_some_and(|d| d.drag.is_none())
    });
    let d = h.state().document().unwrap();
    let msg = last_said(d);
    assert!(msg.starts_with(&format!("edited surface {id}")), "{msg}");
    assert_eq!(h.state().input_edits(), (1, true));
    let mut edited = migjorn::Model::parse(&text);
    edited
        .surface_mut(id)
        .unwrap()
        .set_coeffs(&at(0.25))
        .unwrap();
    assert_eq!(
        card_of(&h).trim_end(),
        edited.surface(id).unwrap().text().trim_end()
    );
    assert!(Arc::ptr_eq(d.slice.model(), &d.model));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "not saved");

    // The inspector: a point on the surface, its coefficients as fields.
    let m = d.model.clone();
    let shape = (0..m.shape_count() as u32)
        .find(|&s| m.surface_id(s).is_some_and(|x| x.id == id))
        .unwrap();
    let b = m.bounds(llevant::Scope::Problem);
    let centre = [0, 1, 2].map(|k| 0.5 * (b.lo[k] + b.hi[k]));
    // Where a line from the centre crosses the surface.
    let len = (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max);
    let lines: Vec<llevant::Line> = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [-1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, -1.0],
    ]
    .into_iter()
    .map(|d| llevant::Line::new(centre, d, 0.0, len))
    .collect();
    let tr = m.trace(&lines, llevant::Depth::All).unwrap();
    let p = (0..tr.line_count()).find_map(|i| {
        tr.segments(i)
            .find(|&k| tr.surface_out[k] == shape)
            .map(|k| {
                let at = lines[i].at(tr.t1[k]);
                let mut p = llevant_viewer::pick::pick_point(&m, at);
                p.surface = Some(llevant::NearSurface {
                    shape,
                    point: at,
                    distance: 0.0,
                    dir: lines[i].dir,
                });
                p
            })
    });
    let Some(p) = p else {
        eprintln!("no point found on surface {id}: inspector part skipped");
        return;
    };
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Focus(Some(p)));
    panel(&mut h, Panel::Inspector);
    h.run_steps(3);
    let fields = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .count();
    assert!(fields >= start.len(), "{fields} fields to drag");

    // The last coefficient scrubbed with the pointer: the view follows;
    // released, the card as it would be written is shown, and Apply
    // writes it (decision 0012, phase 8: typed or dragged values wait).
    // (Spin buttons elsewhere too: the one at the coefficient written.)
    let field = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.value().and_then(|v| v.trim().parse::<f64>().ok()) == Some(at(0.25)[k]))
        .expect("the coefficient's field")
        .rect();
    let from = field.center();
    let card = card_of(&h);
    drag_between(&mut h, from, from + egui::vec2(40.0, 0.0));
    h.run_steps(3);
    assert!(h.state().document().unwrap().drag.is_some(), "held");
    assert_eq!(card_of(&h), card, "not written on release");
    assert_eq!(h.state().input_edits(), (1, true));
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(path).with_extension("held.png"));
    }
    h.get_by_label("Apply coefficients").click();
    h.step();
    step_until(&mut h, "the scrub's edit to land", |a| {
        !a.is_loading() && a.document().is_some_and(|d| d.drag.is_none())
    });
    // (The message may be the check of what changed by now.)
    assert_eq!(h.state().input_edits(), (2, true));
    assert_ne!(card_of(&h), card);
    if let Some(path) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(path).with_extension("drag.png"));
    }
}

/// Cells edited together from the selection (one edit, one Undo), and
/// renumbered with every reference to them (decision 0012, phase 8).
#[test]
fn cells_edited_together() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-many-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    std::fs::copy(model("universes.i"), &path).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let text = |h: &Harness<'_, ViewerApp>| {
        h.state()
            .document()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .to_source()
    };
    let card = |h: &Harness<'_, ViewerApp>, id: i64| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().cell(id).map(|c| c.text)
    };
    let wait = |h: &mut Harness<'_, ViewerApp>| {
        h.step();
        step_until(h, "the edit to compile", |a| !a.is_loading());
        last_said(h.state().document().unwrap())
    };
    // The checks an edit starts finish first, so that their message does
    // not take the place of the next one's.
    let send = |h: &mut Harness<'_, ViewerApp>, c: Command| {
        step_until(h, "the jobs before", |a| {
            a.document().is_none_or(|d| d.results.jobs.is_empty())
        });
        let ctx = h.ctx.clone();
        h.state_mut().send(c, &ctx);
    };
    let before = text(&h);
    let three = llevant_scene::query::Query::Cells(vec![400, 402, 404]);
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Select(three, SelectMode::Replace));
    h.state_mut().send(Command::EditSelection, &ctx);
    h.run_steps(3);
    h.get_by_label("IMP:N: set").click();
    h.run_steps(2);
    h.get_by_label("IMP:N of the cells").click();
    h.run_steps(1);
    h.get_by_label("IMP:N of the cells").type_text("4");
    h.run_steps(2);
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("many.png"));
    }
    h.get_by_label("Apply to the cells").click();
    let msg = wait(&mut h);
    assert!(msg.starts_with("edited 3 cells"), "{msg}");
    h.run_steps(2);
    assert_eq!(h.state().input_edits(), (1, true));
    for id in [400, 402, 404] {
        let c = card(&h, id).unwrap();
        assert!(c.contains("imp:n=4"), "{c}");
    }
    assert!(card(&h, 401).unwrap().contains("imp:n=1"));
    let d = h.state().document().unwrap();
    assert_eq!(d.selection.cells.count(), 3, "the cells kept selected");
    assert!(h.query_by_label("Apply to the cells").is_none(), "closed");

    send(&mut h, Command::UndoInputEdit);
    let msg = wait(&mut h);
    assert!(msg.starts_with("took back the edit of 3 cells"), "{msg}");
    assert_eq!(text(&h), before, "one Undo for the three");

    // Renumbered: the references (#400 in cell 401, LIKE 400) follow.
    send(
        &mut h,
        Command::Renumber(
            llevant_scene::source::IdKind::Cell,
            vec![(400, 900), (402, 902)],
        ),
    );
    let msg = wait(&mut h);
    assert!(msg.starts_with("edited the numbers of 2 cells"), "{msg}");
    h.run_steps(2);
    assert!(card(&h, 400).is_none() && card(&h, 900).is_some());
    assert!(card(&h, 401).unwrap().contains("#900"));
    assert!(card(&h, 406).unwrap().contains("like 900"));
    let d = h.state().document().unwrap();
    assert!(d.model.cell_index(900).is_some() && d.model.cell_index(400).is_none());
    assert_eq!(
        d.selection.cells.count(),
        2,
        "the cells renumbered selected"
    );
    // Into a number in use: refused, nothing changed.
    send(
        &mut h,
        Command::Renumber(llevant_scene::source::IdKind::Cell, vec![(900, 401)]),
    );
    let msg = wait(&mut h);
    assert!(msg.contains("in use"), "{msg}");
    assert_eq!(h.state().input_edits(), (1, true));
    send(&mut h, Command::UndoInputEdit);
    wait(&mut h);
    assert_eq!(text(&h), before, "the renumbering taken back exactly");
    assert!(
        std::fs::read_to_string(&path).unwrap() == before,
        "not saved"
    );
}

/// New cards from the viewer: a surface, then a cell inside it cut out of
/// the cell it lies in, each one edit taken back by one Undo (decision
/// 0012, phase 8).
#[test]
fn new_cards_added() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("universes.i");
    std::fs::copy(model("universes.i"), &path).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let text = |h: &Harness<'_, ViewerApp>| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().to_source()
    };
    let wait = |h: &mut Harness<'_, ViewerApp>| {
        h.step();
        step_until(h, "the edit to compile", |a| !a.is_loading());
        let msg = last_said(h.state().document().unwrap());
        h.run_steps(2);
        step_until(h, "the jobs after", |a| {
            a.document().is_none_or(|d| d.results.jobs.is_empty())
        });
        msg
    };
    let before = text(&h);
    // Cell 401, around cell 400 (a cylinder of radius 3 about x = 285,
    // y = 1): a point in it, off the cylinder.
    let m = h.state().document().unwrap().model.clone();
    let centre = [285.0, 6.0, 0.0];
    let p = llevant_viewer::pick::pick_point(&m, centre);
    let host = m.cell_id(p.cell().unwrap());
    assert_eq!(host, 401);
    h.state_mut()
        .document_mut()
        .unwrap()
        .execute(Command::Focus(Some(p)));
    panel(&mut h, Panel::Inspector);
    h.run_steps(2);
    let source = h.state().document().unwrap().source.clone().unwrap();
    let s = source.next_free(llevant_scene::source::CardKind::Surface);
    let c = source.next_free(llevant_scene::source::CardKind::Cell);

    // A surface: a ball at the point.
    h.state_mut().send(
        Command::Inspect(llevant_viewer::panels::inspector::Subject::New(
            llevant_scene::source::CardKind::Surface,
        )),
        &ctx,
    );
    h.run_steps(3);
    h.get_by_label("Type of the new surface").click();
    h.run_steps(1);
    h.key_press(egui::Key::End);
    for _ in 0..2 {
        h.key_press(egui::Key::Backspace);
    }
    h.get_by_label("Type of the new surface").type_text("s");
    h.run_steps(1);
    h.get_by_label("Coefficients of the new surface").click();
    h.run_steps(1);
    h.get_by_label("Coefficients of the new surface")
        .type_text("285 6 0 0.4");
    h.run_steps(3);
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(
            &mut h,
            &PathBuf::from(png).with_extension("new-surface.png"),
        );
    }
    h.get_by_label("Add the card").click();
    let msg = wait(&mut h);
    assert!(msg.starts_with(&format!("edited new surface {s}")), "{msg}");
    let card = source.surface(s).expect("added").text;
    assert_eq!(card, format!("{s} s 285 6 0 0.4\n"));

    // A cell inside it, cut out of the cell it is in.
    h.state_mut().send(
        Command::Inspect(llevant_viewer::panels::inspector::Subject::New(
            llevant_scene::source::CardKind::Cell,
        )),
        &ctx,
    );
    h.run_steps(3);
    h.get_by_label("Geometry").click();
    h.run_steps(1);
    h.get_by_label("Geometry").type_text(&format!("-{s}"));
    {
        // Zoomed in on the point: the ball is a few grid steps across.
        let d = h.state_mut().document_mut().unwrap();
        d.slice.width = 4.0;
        d.slice.centre = [285.0, 6.0];
    }
    h.run_steps(3);
    step_until(&mut h, "the new cell drawn", |a| {
        a.document()
            .unwrap()
            .marks
            .segments
            .iter()
            .any(|x| x.surface == llevant_viewer::marks::EDITED)
    });
    if let Some(png) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
        h.run_steps(3);
        save(&mut h, &PathBuf::from(png).with_extension("new-cell.png"));
    }
    h.get_by_label("Add the card").click();
    let msg = wait(&mut h);
    assert!(msg.starts_with(&format!("edited new cell {c}")), "{msg}");
    let d = h.state().document().unwrap();
    let here = llevant_viewer::pick::pick_point(&d.model, centre);
    assert_eq!(
        d.model.cell_id(here.cell().unwrap()),
        c,
        "the point is in it"
    );
    let host_card = source.cell(host).unwrap().text;
    assert!(host_card.contains(&format!("#{c}")), "{host_card}");
    assert_eq!(h.state().input_edits(), (2, true));

    let ctx = h.ctx.clone();
    h.state_mut().send(Command::UndoInputEdit, &ctx);
    wait(&mut h);
    assert!(source.cell(c).is_none());
    h.state_mut().send(Command::UndoInputEdit, &ctx);
    wait(&mut h);
    assert_eq!(text(&h), before, "both taken back");
}

/// The time of a surface edit made from the whole reference model and from
/// a universe opened on its own (`LLEVANT_BIG_MODEL`, `LLEVANT_UNIVERSE`,
/// 10162 by default).
#[test]
#[ignore = "needs LLEVANT_BIG_MODEL"]
fn edit_time_in_a_universe_on_the_big_model() {
    let Ok(path) = std::env::var("LLEVANT_BIG_MODEL") else {
        return;
    };
    if !adapter_available() {
        return;
    }
    let _one = serial();
    let u: i64 = std::env::var("LLEVANT_UNIVERSE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10162);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(PathBuf::from(path), &ctx);
    let wait = |h: &mut Harness<'_, ViewerApp>, what: &str, done: &dyn Fn(&ViewerApp) -> bool| {
        let t = std::time::Instant::now();
        while !done(h.state()) {
            assert!(t.elapsed().as_secs() < 900, "timed out: {what}");
            h.step();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    wait(&mut h, "the model complete and its cards", &|a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    // A surface of the universe's first cell with a card of its own.
    let d = h.state().document().unwrap();
    let m = &d.model;
    let src = d.source.clone().unwrap();
    let unit = m.units().iter().position(|x| x.universe == u).unwrap();
    let (id, coeffs) = m.units()[unit]
        .cells
        .iter()
        .flat_map(|&c| m.cell_surfaces(c))
        .filter_map(|(s, _)| m.surface_id(s))
        .find_map(|s| src.surface_coeffs(s.id).map(|(_, c)| (s.id, c)))
        .expect("a surface of the universe");
    let moved: Vec<f64> = coeffs.iter().map(|c| c + 1e-3).collect();
    let edit = |h: &mut Harness<'_, ViewerApp>, c: &[f64]| -> f64 {
        let text = src.surface_with_coeffs(id, c).unwrap();
        let ctx = h.ctx.clone();
        let t = std::time::Instant::now();
        h.state_mut().send(
            Command::EditCard(llevant_scene::source::CardKind::Surface, id, text),
            &ctx,
        );
        wait(h, "the edit", &|a| !a.is_loading());
        t.elapsed().as_secs_f64()
    };
    let whole = edit(&mut h, &moved);
    let t = std::time::Instant::now();
    h.state_mut().send(Command::OpenUniverse(u), &ctx);
    wait(&mut h, "the universe", &|a| a.open_universes() == vec![u]);
    let open = t.elapsed().as_secs_f64();
    let inside = edit(&mut h, &coeffs);
    eprintln!(
        "surface {id} of u={u}: edit from the whole model {whole:.2} s; u={u} opened in {open:.2} s; \
         edit from inside it {inside:.2} s"
    );
}

/// An overlap fixed from the Problems panel's command: cell 2 cut out of by
/// cell 1 (shortened by Boolean algebra), one Undo, and a cut that cannot be
/// made (a cell inside the other) changes nothing.
#[test]
fn an_overlap_fixed_by_cutting_a_cell() {
    if !adapter_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _one = serial();
    let dir = std::env::temp_dir().join(format!("llevant-viewer-cut-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("overlap.i");
    let text = "overlap\n1 0 -1 imp:n=1\n2 0 (-2:-4) 3 imp:n=1\n3 0 -1 3 imp:n=1\n\
                4 0 1 -5 imp:n=0\n5 0 5 imp:n=0\n\n\
                1 so 5\n2 sx 4 5\n3 px 0\n4 so 9\n5 so 50\n\n";
    std::fs::write(&path, text).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1300.0))
        .wgpu_setup(llevant_viewer::app::wgpu_options().wgpu_setup)
        .build_eframe(|cc| ViewerApp::empty(cc).unwrap());
    let ctx = h.ctx.clone();
    h.state_mut().open_file(path.clone(), &ctx);
    step_until(&mut h, "the model and its cards", |a| {
        a.document()
            .is_some_and(|d| d.model.is_complete() && d.source.is_some())
            && !a.is_loading()
    });
    let card = |h: &Harness<'_, ViewerApp>, id: i64| {
        let d = h.state().document().unwrap();
        d.source.as_ref().unwrap().cell(id).unwrap().text.clone()
    };
    let send = |h: &mut Harness<'_, ViewerApp>, c: Command| {
        step_until(h, "the jobs before", |a| {
            a.document().is_none_or(|d| d.results.jobs.is_empty())
        });
        let ctx = h.ctx.clone();
        h.state_mut().send(c, &ctx);
        h.step();
        step_until(h, "the edit to compile", |a| !a.is_loading());
        last_said(h.state().document().unwrap())
    };
    let before = card(&h, 2);
    let msg = send(
        &mut h,
        Command::CutCell {
            keep: 2,
            cut: 3,
            simplify: true,
        },
    );
    assert!(msg.contains("cell 2 cut by #3"), "{msg}");
    assert_eq!(h.state().input_edits(), (1, true));
    assert!(
        card(&h, 2).contains("#3") || card(&h, 2) != before,
        "{}",
        card(&h, 2)
    );
    assert_eq!(
        card(&h, 3).trim_end(),
        "3 0 -1 3 imp:n=1",
        "the other cell is as it was"
    );
    send(&mut h, Command::UndoInputEdit);
    assert_eq!(h.state().input_edits(), (0, false));
    assert_eq!(card(&h, 2), before);

    // Cell 3 lies inside cell 1: cutting it out of cell 3 leaves nothing.
    let msg = send(
        &mut h,
        Command::CutCell {
            keep: 3,
            cut: 1,
            simplify: true,
        },
    );
    assert!(msg.contains("inside"), "{msg}");
    assert_eq!(h.state().input_edits(), (0, false));
    let _ = std::fs::remove_dir_all(&dir);
}
