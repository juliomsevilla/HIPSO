//! Pruebas de la interfaz sin ventana: se alimenta a egui con eventos de puntero y se mira qué
//! órdenes de edición salen del perfil.

use super::*;
use hipso_core::edit::{Region, Session};

fn cloud() -> PointStore {
    // un suelo plano a cota 10 y un cable a cota 15, a lo largo de x = 0…20 (y = 0)
    let mut s = PointStore {
        stride: 1,
        ..PointStore::default()
    };
    for i in 0..200 {
        let x = i as f32 / 10.0;
        for (z, c) in [(10.0f32, 2u8), (15.0, 14), (12.5, 5)] {
            s.x.push(x);
            s.y.push(0.0);
            s.z.push(z);
            s.class.push(c);
            s.intensity.push(100);
            s.ret.push(0x11);
            s.src.push(0);
            s.scan.push(0);
            s.file.push(0);
        }
    }
    s.total = s.len() as u64;
    s.bounds = [0.0, 0.0, 10.0, 20.0, 0.0, 15.0];
    s
}

fn app_with_profile(nav: Nav) -> App {
    let store = cloud();
    let mut app = App::default();
    app.ed.session = Some(Session::new(&store));
    app.store = Some(Arc::new(store));
    app.profile = Some(ProfileBox {
        a: [0.0, 0.0],
        b: [20.0, 0.0],
        width: 4.0,
    });
    app.layout = Layout::PlanProfile;
    app.nav = nav;
    app
}

const RECT: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 400.0));

fn frame(app: &mut App, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
        time: Some(t),
        events,
        ..egui::RawInput::default()
    };
    let out = ctx.run_ui(input, |ui| {
        let ctx = ui.ctx().clone();
        app.profile_view(ui, &ctx, RECT);
    });
    out.drop_without_applying_deltas();
}

fn mv(p: Pos2) -> egui::Event {
    egui::Event::PointerMoved(p)
}

fn button(p: Pos2, pressed: bool, button: egui::PointerButton) -> egui::Event {
    egui::Event::PointerButton {
        pos: p,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Dos fotogramas para que el perfil se dibuje y exista la textura con la que se mide.
fn warm_up(app: &mut App, ctx: &egui::Context) -> f64 {
    for k in 0..3 {
        frame(app, ctx, f64::from(k) * 0.05, vec![mv(Pos2::new(50.0, 50.0))]);
    }
    0.2
}

fn drag(app: &mut App, ctx: &egui::Context, mut t: f64, a: Pos2, b: Pos2) -> f64 {
    frame(app, ctx, t, vec![mv(a)]);
    t += 0.05;
    frame(app, ctx, t, vec![button(a, true, egui::PointerButton::Primary)]);
    for k in 1..=6 {
        t += 0.05;
        let p = a + (b - a) * (k as f32 / 6.0);
        frame(app, ctx, t, vec![mv(p)]);
    }
    t += 0.05;
    frame(app, ctx, t, vec![button(b, false, egui::PointerButton::Primary)]);
    t + 0.05
}

fn click(app: &mut App, ctx: &egui::Context, mut t: f64, p: Pos2, which: egui::PointerButton) -> f64 {
    frame(app, ctx, t, vec![mv(p)]);
    t += 0.05;
    frame(app, ctx, t, vec![button(p, true, which)]);
    t += 0.05;
    frame(app, ctx, t, vec![button(p, false, which)]);
    t + 0.4
}

fn regions(app: &App) -> Vec<&'static str> {
    app.ed
        .cmds
        .iter()
        .map(|c| match c {
            editing::Cmd::AddOp(op) => match &op.region {
                Region::Side { .. } => "side",
                Region::Near { .. } => "near",
                _ => "otra",
            },
            editing::Cmd::Apply(_) => "apply",
            _ => "cmd",
        })
        .collect()
}

#[test]
fn dragging_a_line_in_the_profile_selects_above_it() {
    for (nav, want) in [
        (Nav::SelAbove, "side"),
        (Nav::SelBelow, "side"),
        (Nav::SelNear, "near"),
        (Nav::ClassLine, "apply"),
    ] {
        let mut app = app_with_profile(nav);
        let ctx = egui::Context::default();
        let t = warm_up(&mut app, &ctx);
        assert!(app.tex_prof.is_some(), "el perfil no se dibujó");
        drag(&mut app, &ctx, t, Pos2::new(100.0, 200.0), Pos2::new(600.0, 200.0));
        assert_eq!(regions(&app), vec![want], "{}", nav.name());
    }
}

#[test]
fn clicking_vertices_and_double_clicking_makes_a_broken_line() {
    let mut app = app_with_profile(Nav::SelAbove);
    let ctx = egui::Context::default();
    let mut t = warm_up(&mut app, &ctx);
    for p in [Pos2::new(100.0, 250.0), Pos2::new(300.0, 150.0)] {
        t = click(&mut app, &ctx, t, p, egui::PointerButton::Primary);
    }
    assert_eq!(app.ed.pdraft.len(), 2, "cada clic añade un vértice");
    // doble clic: dos pulsaciones seguidas en el mismo sitio
    let p = Pos2::new(600.0, 200.0);
    frame(&mut app, &ctx, t, vec![mv(p)]);
    for k in 0..2 {
        let at = t + 0.05 + f64::from(k) * 0.1;
        frame(&mut app, &ctx, at, vec![button(p, true, egui::PointerButton::Primary)]);
        frame(&mut app, &ctx, at + 0.04, vec![button(p, false, egui::PointerButton::Primary)]);
    }
    assert_eq!(regions(&app), vec!["side"], "pdraft = {:?}", app.ed.pdraft);
    let Some(editing::Cmd::AddOp(op)) = app.ed.cmds.first() else {
        panic!("sin orden");
    };
    let Region::Side { path, .. } = &op.region else {
        panic!("no es una región lateral");
    };
    assert_eq!(path.len(), 3, "{path:?}");
    assert!(path.windows(2).all(|w| w[0][0] <= w[1][0]), "ordenada por u");
}

#[test]
fn the_wheel_moves_the_profile_one_step_per_notch() {
    // (modo de paso, paso fijo en m, avance esperado con un ancho de 4 m)
    for (mode, fixed, want) in [
        (ProfileStep::Half, 1.0, 2.0),
        (ProfileStep::AlmostFull, 1.0, 3.8),
        (ProfileStep::Full, 1.0, 4.0),
        (ProfileStep::Fixed, 1.5, 1.5),
    ] {
        let mut app = app_with_profile(Nav::Zoom);
        app.profile_width = 4.0;
        let ctx = egui::Context::default();
        let mut t = warm_up(&mut app, &ctx);
        app.profile_step = mode;
        app.profile_step_m = fixed;
        let y0 = app.profile.map(|b| b.a[1]).unwrap_or(0.0);
        let wheel = |dy: f32| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, &ctx, t, vec![mv(Pos2::new(300.0, 200.0))]);
        t += 0.02;
        frame(&mut app, &ctx, t, vec![wheel(1.0)]);
        for _ in 0..30 {
            t += 0.02;
            frame(&mut app, &ctx, t, vec![]);
        }
        let moved = (app.profile.map(|b| b.a[1]).unwrap_or(0.0) - y0).abs();
        assert!((moved - want).abs() < 1e-6, "una muesca debe mover {want} m: {moved}");
    }
}

#[test]
fn the_profile_is_colored_by_height_above_ground() {
    let mut app = app_with_profile(Nav::Zoom);
    app.style.color_by = ColorBy::HeightAboveGround;
    let ctx = egui::Context::default();
    app.ensure_hag(&ctx);
    for k in 0..4 {
        frame(&mut app, &ctx, f64::from(k) * 0.05, vec![]);
    }
    let store = app.store.clone().unwrap();
    assert_eq!(store.hag.len(), store.len(), "la nube debe tener la altura calculada");
    assert!(store.hag.iter().any(|h| (*h - 5.0).abs() < 0.1), "el cable está 5 m sobre el suelo");
}

#[test]
fn classifying_along_a_line_changes_three_bands_and_one_undo_puts_them_back() {
    let mut app = app_with_profile(Nav::ClassLine);
    let ctx = egui::Context::default();
    let t = warm_up(&mut app, &ctx);
    // la línea a lo largo del cable, a cota 15: el cable queda cerca, lo de 12,5 y 10 queda bajo
    let (lo, hi) = app.tex_prof.as_ref().map(|t| (t.2, t.3)).unwrap();
    let rect = RECT;
    let (w, h) = (rect.width() as f64, rect.height() as f64);
    let len = 20.0;
    let k = (w / len).min(h / (hi - lo));
    let (ox, oy) = ((w - len * k) / 2.0, (h - (hi - lo) * k) / 2.0);
    let at = |u: f64, z: f64| Pos2::new((ox + u * k) as f32, (h - oy - (z - lo) * k) as f32);
    app.ed.line_classes = [(true, 5), (true, 14), (true, 2)];
    drag(&mut app, &ctx, t, at(1.0, 15.0), at(19.0, 15.0));
    app.process_edit_cmds();
    let store = app.store.clone().unwrap();
    let ses = app.ed.session.as_ref().unwrap();
    assert_eq!(ses.edits.len(), 3, "una edición por tramo");
    // en la parte central (u de 5 a 18), el cable (z = 15) queda «cerca» y pasa a 14, y lo que está
    // más abajo (z = 12,5 y z = 10) queda «bajo» y pasa a 2; los extremos, fuera del alcance de la
    // línea, no cambian
    for i in 0..store.len() {
        let (x, z) = (store.x[i], store.z[i]);
        if (5.0..=18.0).contains(&x) {
            let want = if (z - 15.0).abs() < 0.1 { 14 } else { 2 };
            assert_eq!(store.class[i], want, "x = {x}, z = {z}");
        }
        if x < 0.5 {
            let was = if (z - 15.0).abs() < 0.1 { 14 } else if (z - 12.5).abs() < 0.1 { 5 } else { 2 };
            assert_eq!(store.class[i], was, "fuera del alcance: x = {x}, z = {z}");
        }
    }
    // un solo deshacer quita los tres tramos
    app.queue(editing::Cmd::Undo);
    app.process_edit_cmds();
    assert!(app.ed.session.as_ref().unwrap().edits.is_empty());
    let store = app.store.clone().unwrap();
    assert_eq!(store.class.iter().filter(|c| **c == 14).count(), 200);
    assert_eq!(store.class.iter().filter(|c| **c == 5).count(), 200);
}
