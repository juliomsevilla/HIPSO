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
fn with_an_axis_the_wheel_turns_the_profile_along_the_bend() {
    let mut app = app_with_profile(Nav::Zoom);
    app.profile_width = 4.0;
    app.profile_step = ProfileStep::Full;
    // un eje en L; el perfil se coloca al principio, a través del primer tramo
    let axis = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
    let (bx, s) = view::axis_section(&axis, 0.0, 20.0, 4.0).unwrap();
    app.profile = Some(bx);
    app.profile_s = s;
    app.profile_axis = Some(axis);
    let ctx = egui::Context::default();
    let mut t = warm_up(&mut app, &ctx);
    let wheel = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut app, &ctx, t, vec![mv(Pos2::new(300.0, 200.0))]);
    for _ in 0..3 {
        t += 0.02;
        frame(&mut app, &ctx, t, vec![wheel.clone()]);
        for _ in 0..30 {
            t += 0.02;
            frame(&mut app, &ctx, t, vec![]);
        }
    }
    // tres muescas de 4 m: estación 12, ya en el segundo tramo (hacia +y), con la caja girada
    let b = app.profile.unwrap();
    assert!((app.profile_s - 12.0).abs() < 1e-6, "estación {}", app.profile_s);
    assert!((b.a[1] - 2.0).abs() < 1e-6 && (b.b[1] - 2.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn shift_click_cuts_perpendicular_and_the_cursor_is_shared() {
    let mut app = app_with_profile(Nav::Zoom);
    app.profile_width = 4.0;
    let ctx = egui::Context::default();
    let mut t = warm_up(&mut app, &ctx);
    let (lo, hi) = app.tex_prof.as_ref().map(|t| (t.2, t.3)).unwrap();
    let (w, h) = (RECT.width() as f64, RECT.height() as f64);
    let len = 20.0;
    let k = (w / len).min(h / (hi - lo));
    let (ox, oy) = ((w - len * k) / 2.0, (h - (hi - lo) * k) / 2.0);
    let at = |u: f64, z: f64| Pos2::new((ox + u * k) as f32, (h - oy - (z - lo) * k) as f32);
    // el cursor sobre el perfil da el punto del mundo al fotograma siguiente
    let p = at(5.0, 12.0);
    frame(&mut app, &ctx, t, vec![mv(p)]);
    t += 0.05;
    frame(&mut app, &ctx, t, vec![]);
    let Some(Hover::Point(w3)) = app.hover_next else {
        panic!("sin cursor compartido");
    };
    assert!((w3[0] - 5.0).abs() < 0.2 && w3[1].abs() < 1e-6 && (w3[2] - 12.0).abs() < 0.2, "{w3:?}");
    // Mayús + clic: caja perpendicular por ese punto, con el mismo largo y ancho
    let shifted = |t: f64, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            time: Some(t),
            events,
            ..egui::RawInput::default()
        };
        input
    };
    // las teclas modificadoras llegan con los eventos de botón
    let sbutton = |p: Pos2, pressed: bool, button: egui::PointerButton| egui::Event::PointerButton {
        pos: p,
        button,
        pressed,
        modifiers: egui::Modifiers::SHIFT,
    };
    let run = |app: &mut App, t: f64, mut events: Vec<egui::Event>| {
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
        let out = ctx.run_ui(shifted(t, events), |ui| {
            let ctx = ui.ctx().clone();
            app.profile_view(ui, &ctx, RECT);
        });
        out.drop_without_applying_deltas();
    };
    run(&mut app, t + 0.1, vec![mv(p)]);
    run(&mut app, t + 0.15, vec![sbutton(p, true, egui::PointerButton::Primary)]);
    run(&mut app, t + 0.2, vec![sbutton(p, false, egui::PointerButton::Primary)]);
    let b = app.profile.unwrap();
    assert!((b.a[0] - 5.0).abs() < 0.2 && (b.b[0] - 5.0).abs() < 0.2, "{b:?}");
    assert!((b.length() - 20.0).abs() < 1e-6 && (b.a[1] + b.b[1]).abs() < 1e-6, "{b:?}");
    assert_eq!(b.width, 4.0);
    assert_eq!(app.profile_back.len(), 1);
    // Mayús + clic derecho: vuelve a la caja anterior
    let q = at(10.0, 12.0);
    run(&mut app, t + 0.8, vec![mv(q)]);
    run(&mut app, t + 0.85, vec![sbutton(q, true, egui::PointerButton::Secondary)]);
    run(&mut app, t + 0.9, vec![sbutton(q, false, egui::PointerButton::Secondary)]);
    assert_eq!(app.profile.unwrap().a, [0.0, 0.0]);
    assert!(app.profile_back.is_empty());
}

#[test]
fn the_attribute_table_is_docked_and_filters_rows() {
    let mut data = view::VectorData::default();
    data.fields = vec!["name".to_string()];
    for n in ["Fiji", "Tanzania", "Canada"] {
        data.features.push(view::Feature { parts: vec![vec![[0.0, 0.0]]] });
        data.text.push(vec![n.to_string()]);
        data.num.push(vec![f64::NAN]);
    }
    data.points = true;
    let mut app = App::default();
    app.refs.push(RefLayer {
        name: "Countries".into(),
        path: PathBuf::from("/tmp/countries.shp"),
        kind: RefKind::Vector(Box::new(layers::VecLayer::new(data, view::GeomKind::Point))),
        visible: true,
        opacity: 1.0,
    });
    app.table_layer = Some(0);
    app.vedit.filter = " AN ".into();
    let ctx = egui::Context::default();
    for k in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
            time: Some(f64::from(k) * 0.05),
            ..egui::RawInput::default()
        };
        // la tabla ocupa la franja de abajo con su altura (250), sin crecer hasta ocupar la vista
        let total = 600.0;
        let h = app.table_height(total);
        assert_eq!(h, 250.0);
        let area = Rect::from_min_max(Pos2::new(300.0, 100.0 + total - h), Pos2::new(1200.0, 100.0 + total));
        let out = ctx.run_ui(input, |ui| app.table_window(ui, area, total * 0.75));
        // la barra de herramientas se dibuja dentro de la franja
        let mut found = false;
        for cs in &out.shapes {
            if let egui::epaint::Shape::Text(ts) = &cs.shape
                && ts.galley.text().contains("de 3 seleccionados")
            {
                found = ts.pos.y >= area.top() && ts.pos.y <= area.top() + 40.0;
            }
        }
        assert!(found, "la barra de la tabla no está arriba de su franja");
        out.drop_without_applying_deltas();
    }
    // otra altura se respeta, y no pasa del 75 % de la zona
    app.vedit.height = 150.0;
    assert_eq!(app.table_height(600.0), 150.0);
    app.vedit.height = 5000.0;
    assert_eq!(app.table_height(600.0), 450.0);
    app.vedit.height = 20.0;
    assert_eq!(app.table_height(600.0), 130.0);
    // sigue abierta y el filtro (sin distinguir mayúsculas) deja Tanzania y Canada
    assert_eq!(app.table_layer, Some(0));
    assert_eq!(app.vedit.filter_rows, vec![1, 2]);
}

#[test]
fn the_docked_table_shows_its_toolbar_inside_the_whole_app() {
    use eframe::App as _;
    let mut data = view::VectorData::default();
    data.fields = vec!["Nombre".to_string()];
    for n in ["15", "14", "13"] {
        data.features.push(view::Feature { parts: vec![vec![[0.0, 0.0]]] });
        data.text.push(vec![n.to_string()]);
        data.num.push(vec![n.parse().unwrap_or(f64::NAN)]);
    }
    data.points = true;
    let mut app = App::default();
    app.refs.push(RefLayer {
        name: "Capa 3.shp".into(),
        path: PathBuf::from("/tmp/capa3.shp"),
        kind: RefKind::Vector(Box::new(layers::VecLayer::new(data, view::GeomKind::Point))),
        visible: true,
        opacity: 1.0,
    });
    app.table_layer = Some(0);
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    let mut found = Vec::new();
    for k in 0..4 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 720.0))),
            time: Some(f64::from(k) * 0.05),
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        found.clear();
        for cs in &out.shapes {
            if let egui::epaint::Shape::Text(ts) = &cs.shape {
                let s = ts.galley.text().to_string();
                if s.contains("seleccionados") || s.contains("Capa 3") || s.contains("Nombre") || s.contains("Planta") {
                    found.push((s, cs.clip_rect.min.y, cs.clip_rect.max.y, ts.pos.y));
                }
            }
        }
        out.drop_without_applying_deltas();
    }
    // la barra de la tabla queda debajo de todo lo que dibuja la vista del mapa
    let plan_bottom = found.iter().find(|f| f.0.contains("Planta")).map(|f| f.2).unwrap_or(f32::MAX);
    let bar_top = found.iter().find(|f| f.0.contains("de 3 seleccionados")).map(|f| f.3).unwrap_or(0.0);
    assert!(bar_top >= plan_bottom, "la vista llega a {plan_bottom} y la barra está en {bar_top}: {found:?}");
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

#[test]
fn picking_a_line_tool_shows_the_profile_and_asks_for_the_box_first() {
    let ctx = egui::Context::default();
    // sin caja y con solo la planta: se muestra el perfil y se pasa a trazar la caja
    let mut app = App::default();
    app.layout = Layout::Plan;
    app.act(&ctx, Action::Nav(Nav::ClassLine));
    assert!(app.layout == Layout::PlanProfile);
    assert!(app.nav == Nav::Profile && app.pending_nav == Some(Nav::ClassLine));
    // con caja: la herramienta queda elegida y el perfil a la vista
    let mut app = app_with_profile(Nav::Zoom);
    app.layout = Layout::Plan;
    app.act(&ctx, Action::Nav(Nav::SelNear));
    assert!(app.layout == Layout::PlanProfile);
    assert!(app.nav == Nav::SelNear && app.pending_nav.is_none());
}

#[test]
fn a_line_selection_is_seen_in_the_profile_and_can_be_edited() {
    for nav in [Nav::SelAbove, Nav::SelBelow, Nav::SelNear] {
        let mut app = app_with_profile(nav);
        let ctx = egui::Context::default();
        let t = warm_up(&mut app, &ctx);
        let (lo, hi) = app.tex_prof.as_ref().map(|t| (t.2, t.3)).unwrap();
        let (w, h) = (RECT.width() as f64, RECT.height() as f64);
        let k = (w / 20.0).min(h / (hi - lo));
        let (ox, oy) = ((w - 20.0 * k) / 2.0, (h - (hi - lo) * k) / 2.0);
        let at = |u: f64, z: f64| Pos2::new((ox + u * k) as f32, (h - oy - (z - lo) * k) as f32);
        // la línea a cota 12,5, a lo largo del perfil: «sobre» toma el cable, «bajo» el suelo y
        // «cerca» la vegetación
        let key_before = app.tex_prof.as_ref().map(|t| t.1);
        let t = drag(&mut app, &ctx, t, at(1.0, 12.5), at(19.0, 12.5));
        app.process_edit_cmds();
        frame(&mut app, &ctx, t, vec![]);
        frame(&mut app, &ctx, t + 0.05, vec![]);
        assert_ne!(
            app.tex_prof.as_ref().map(|t| t.1),
            key_before,
            "{}: el perfil no se vuelve a dibujar con la selección",
            nav.name()
        );
        let bx = app.profile.unwrap();
        let picked = app
            .profile_store(&ctx, &bx)
            .map(|s| (0..s.len()).filter(|&i| s.selected.get(i).copied().unwrap_or(false)).count())
            .unwrap_or(0);
        assert!(picked > 100, "{}: solo {picked} puntos seleccionados", nav.name());
        // y se puede editar lo seleccionado
        app.queue(editing::Cmd::Reclass(9));
        app.process_edit_cmds();
        let store = app.store.clone().unwrap();
        assert!(store.class.iter().filter(|c| **c == 9).count() > 100, "{}", nav.name());
    }
}

/// Con una nube real (`HIPSO_TEST_LAZ=ruta cargo test -p hipso-gui real_cloud -- --ignored --nocapture`):
/// el perfil lee la franja a toda densidad en segundo plano, se traza una línea y se mira que la
/// selección llega a los puntos y se pinta.
#[test]
#[ignore]
fn real_cloud_profile_line_tools() {
    let Ok(path) = std::env::var("HIPSO_TEST_LAZ") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let store = hipso_core::view::load(&[path.clone()], 3_000_000).unwrap();
    eprintln!("cargados {} de {} puntos (1 de cada {})", store.len(), store.total, store.stride);
    let b = store.bounds;
    let _ = b;
    // el centro de la caja, en el punto mediano de la nube (la caja del extremo puede caer en un hueco)
    let mut xs: Vec<f64> = (0..store.len()).step_by(7).map(|i| store.xyz(i)[0]).collect();
    let mut ys: Vec<f64> = (0..store.len()).step_by(7).map(|i| store.xyz(i)[1]).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    let (cx, cy) = (xs[xs.len() / 2], ys[ys.len() / 2]);
    for (nav, color) in [
        (Nav::SelAbove, ColorBy::Elevation),
        (Nav::SelNear, ColorBy::HeightAboveGround),
        (Nav::ClassLine, ColorBy::HeightAboveGround),
    ] {
        let mut app = App::default();
        app.ed.session = Some(Session::new(&store));
        app.store = Some(Arc::new(store.clone()));
        app.clouds = vec![path.clone()];
        app.style = Style::fit(&store);
        app.style.color_by = color;
        app.profile_width = 10.0;
        app.profile = Some(ProfileBox {
            a: [cx - 50.0, cy],
            b: [cx + 50.0, cy],
            width: 10.0,
        });
        app.layout = Layout::PlanProfile;
        app.nav = nav;
        let ctx = egui::Context::default();
        let mut t = 0.0;
        // hasta que el perfil tenga sus puntos a toda densidad
        for _ in 0..400 {
            frame(&mut app, &ctx, t, vec![mv(Pos2::new(50.0, 50.0))]);
            t += 0.05;
            if app.tex_prof.is_some() && !app.profile_loading() && t > 1.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let bx = app.profile.unwrap();
        let ps = app.profile_store(&ctx, &bx).unwrap();
        eprintln!("{}: perfil con {} puntos (stride {}), hag {}", nav.name(), ps.len(), ps.stride, ps.hag.len());
        let (lo, hi) = app.tex_prof.as_ref().map(|t| (t.2, t.3)).unwrap();
        let (w, h) = (RECT.width() as f64, RECT.height() as f64);
        let k = (w / 100.0).min(h / (hi - lo));
        let (ox, oy) = ((w - 100.0 * k) / 2.0, (h - (hi - lo) * k) / 2.0);
        let at = |u: f64, z: f64| Pos2::new((ox + u * k) as f32, (h - oy - (z - lo) * k) as f32);
        // la línea a la cota mediana del suelo de la franja: pasa por la nube
        let mut gz: Vec<f64> = (0..ps.len()).filter(|&i| ps.class[i] == 2).map(|i| ps.xyz(i)[2]).collect();
        gz.sort_by(f64::total_cmp);
        let zmid = gz[gz.len() / 2];
        eprintln!("{}: rango del perfil {lo:.1}–{hi:.1}, suelo mediano {zmid:.1}", nav.name());
        t = drag(&mut app, &ctx, t, at(10.0, zmid), at(90.0, zmid));
        app.process_edit_cmds();
        for _ in 0..3 {
            frame(&mut app, &ctx, t, vec![]);
            t += 0.05;
        }
        eprintln!("{}: órdenes sin procesar {}, ops {}, ediciones {}", nav.name(), app.ed.cmds.len(), app.ed.ops.len(), app.ed.session.as_ref().unwrap().edits.len());
        if let Some(op) = app.ed.ops.first() {
            if let Region::Side { path, .. } | Region::Near { path, .. } = &op.region {
                eprintln!("{}: línea {path:?}, ancho de caja {}", nav.name(), bx.width);
            }
        }
        let ps = app.profile_store(&ctx, &bx).unwrap();
        {
            let mut n_strip = 0;
            let mut n_above = 0;
            let mut zs = (f64::MAX, f64::MIN);
            for i in 0..ps.len() {
                let p = ps.xyz(i);
                let (u, t) = bx.local(p[0], p[1]);
                if u >= 23.4 && u <= 90.0 && t.abs() <= 5.0 {
                    n_strip += 1;
                    zs = (zs.0.min(p[2]), zs.1.max(p[2]));
                    if p[2] > 657.34 {
                        n_above += 1;
                    }
                }
            }
            eprintln!("  manual: en la franja {n_strip}, sobre la línea {n_above}, z {:.1}–{:.1}", zs.0, zs.1);
        }
        let sel = ps.selected.iter().filter(|s| **s).count();
        let img = hipso_core::view::render_profile(&ps, &bx, 800, 400, &app.style, None).0;
        let cyan = img.rgba.chunks_exact(4).filter(|p| [p[0], p[1], p[2]] == hipso_core::view::HIGHLIGHT).count();
        eprintln!("{}: seleccionados {sel}, píxeles resaltados {cyan}", nav.name());
    }
}

#[test]
fn the_cloud_on_top_of_the_list_hides_the_one_below_where_they_overlap() {
    let mut app = app_with_profile(Nav::Zoom);
    // la misma nube dos veces: la de arriba (archivo 0) es la editada, la de abajo (archivo 1) la original
    let mut s = cloud();
    let n = s.len();
    s.files = vec!["editada.laz".into(), "original.laz".into()];
    s.file = (0..n).map(|i| u8::from(i % 2 == 1)).collect();
    s.file_bounds = vec![s.bounds, s.bounds];
    app.store = Some(Arc::new(s));
    app.clouds = vec!["editada.laz".into(), "original.laz".into()];
    app.update_shadows();
    let s = app.store.clone().unwrap();
    let shown = (0..n).filter(|&i| app.style.visible(&s, i)).count();
    assert_eq!(shown, n / 2, "solo se ve la de arriba");
    // al ocultar la de arriba (bit 0) se ve la de abajo
    app.style.files = 0b10;
    app.update_shadows();
    let shown = (0..n).filter(|&i| app.style.visible(&s, i)).count();
    assert_eq!(shown, n / 2);
    assert!((0..n).all(|i| !app.style.visible(&s, i) || s.file[i] == 1));
}

/// Con la nube editada y la original abiertas a la vez (`HIPSO_TEST_LAZ` y `HIPSO_TEST_EDITED`):
/// el perfil debe enseñar la clasificación de la editada, que está arriba en la lista.
#[test]
#[ignore]
fn real_cloud_profile_with_edited_on_top() {
    let (Ok(orig), Ok(edited)) = (std::env::var("HIPSO_TEST_LAZ"), std::env::var("HIPSO_TEST_EDITED")) else {
        return;
    };
    let (orig, edited) = (std::path::PathBuf::from(orig), std::path::PathBuf::from(edited));
    // el flujo del programa: primero solo la original, con su perfil ya leído; después se añade
    // la editada arriba y se recarga
    let mut app = App::default();
    app.max_points_m = 6.0;
    let ctx = egui::Context::default();
    app.clouds = vec![orig.clone()];
    app.reload(&ctx);
    let pump = |app: &mut App| {
        for _ in 0..2000 {
            app.poll(&ctx);
            if app.loading.is_none() && app.store.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    };
    pump(&mut app);
    let c = {
        // la misma caja antes y después de añadir la editada: sobre un punto que ella clasificó como 5
        let e = hipso_core::view::load(&[edited.clone()], 6_000_000).unwrap();
        let k = (0..e.len()).find(|&i| e.class[i] == 5).unwrap();
        let c = e.xyz(k);
        app.profile_width = 20.0;
        app.profile = Some(ProfileBox { a: [c[0] - 20.0, c[1]], b: [c[0] + 20.0, c[1]], width: 20.0 });
        app.layout = Layout::PlanProfile;
        let mut t = 0.0;
        app.start_cache(&ctx, true);
        for _ in 0..400 {
            app.update_shadows();
            app.update_detail(&ctx, [c[0] - 50.0, c[1] - 50.0, c[0] + 50.0, c[1] + 50.0], false);
            frame(&mut app, &ctx, t, vec![mv(Pos2::new(50.0, 50.0))]);
            t += 0.05;
            if app.tex_prof.is_some() && !app.profile_loading() && !app.detail_src.is_empty() && t > 1.0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        eprintln!("fase 1: perfil de la original listo, detail_src {:?}", app.detail_src);
        c
    };
    app.clouds = vec![edited.clone(), orig.clone()];
    app.reload(&ctx);
    // mientras carga, el perfil sigue dibujándose cada fotograma, como en el programa
    let mut tt = 50.0;
    for _ in 0..2000 {
        app.poll(&ctx);
        app.update_shadows();
        app.update_detail(&ctx, [c[0] - 50.0, c[1] - 50.0, c[0] + 50.0, c[1] + 50.0], false);
        frame(&mut app, &ctx, tt, vec![mv(Pos2::new(50.0, 50.0))]);
        tt += 0.05;
        if app.loading.is_none() && app.store.is_some() && tt > 60.0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let store = app.store.clone().expect("no se cargó");
    eprintln!("cargados {} puntos, archivos {:?}, nubes {:?}", store.len(), store.files, app.clouds);
    app.style.color_by = ColorBy::Class;
    app.style.files = 0b01; // la original sin marcar, como en la captura
    let mut t = 0.0;
    // como en el programa: se prepara la copia COPC de cada nube y el perfil lee de ellas
    app.start_cache(&ctx, true);
    for _ in 0..2000 {
        app.update_detail(&ctx, [c[0] - 50.0, c[1] - 50.0, c[0] + 50.0, c[1] + 50.0], false);
        if !app.detail_src.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    eprintln!("detail_src = {:?}", app.detail_src);
    for _ in 0..400 {
        app.update_shadows();
        frame(&mut app, &ctx, t, vec![mv(Pos2::new(50.0, 50.0))]);
        t += 0.05;
        if app.tex_prof.is_some() && !app.profile_loading() && t > 1.0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let bx = app.profile.unwrap();
    let ps = app.profile_store(&ctx, &bx).unwrap();
    let mut by = std::collections::BTreeMap::new();
    for i in 0..ps.len() {
        let p = ps.xyz(i);
        let (u, tt) = bx.local(p[0], p[1]);
        if u >= 0.0 && u <= bx.length() && tt.abs() <= 10.0 {
            *by.entry((ps.file[i], ps.class[i], app.style.visible(&ps, i))).or_insert(0usize) += 1;
        }
    }
    eprintln!("(archivo, clase, visible) -> puntos: {by:?}");
    eprintln!("sombras: {:?}", app.style.shadows);
    let img = hipso_core::view::render_profile(&ps, &bx, 800, 400, &app.style, None).0;
    let px = |c: [u8; 3]| img.rgba.chunks_exact(4).filter(|p| [p[0], p[1], p[2]] == c).count();
    eprintln!(
        "píxeles: clase 5 {}, clase 3 {}, clase 14 {}",
        px(app.style.class_colors[5]),
        px(app.style.class_colors[3]),
        px(app.style.class_colors[14])
    );
    eprintln!("tex_prof lleva la clave {:?}", app.tex_prof.as_ref().map(|t| t.1));

    // lo que se dibuja de verdad: se pasa por `views` con la ventana de planta y perfil y se lee la
    // textura del perfil que sale en los cambios de textura de egui
    let mut images: std::collections::HashMap<egui::TextureId, egui::ColorImage> = Default::default();
    let mut t2 = 100.0;
    for _ in 0..120 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 720.0))),
            time: Some(t2),
            events: vec![mv(Pos2::new(700.0, 600.0))],
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.update_shadows();
            app.views(ui, &ctx);
        });
        for (id, ds) in &out.textures_delta.set {
            for d in ds {
                if d.pos.is_none()
                    && let egui::ImageData::Color(img) = &d.image
                {
                    images.insert(*id, (**img).clone());
                }
            }
        }
        out.drop_without_applying_deltas();
        t2 += 0.05;
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if let Some((tex, _, _, _)) = &app.tex_prof {
        if let Some(img) = images.get(&tex.id()) {
            let want = |c: [u8; 3]| img.pixels.iter().filter(|p| [p.r(), p.g(), p.b()] == c).count();
            eprintln!(
                "textura del perfil {}x{}: clase 5 {}, clase 3 {}",
                img.size[0], img.size[1], want(app.style.class_colors[5]), want(app.style.class_colors[3])
            );
            assert!(want(app.style.class_colors[5]) > 1000, "el perfil no enseña la clasificación de la editada");
        } else {
            eprintln!("no encuentro la textura del perfil en los cambios");
        }
    }
}

/// Un control que abre una lista, dentro de un grupo de la cinta plegado en un botón: ¿se puede
/// elegir algo? `kind`: 0 cuadro combinado, 1 menú, 2 `ribbon::small_menu`.
fn folded_pick(kind: u8) -> bool {
    use std::cell::Cell;
    let app = App::default();
    let t = app.tokens();
    let ctx = egui::Context::default();
    let chosen = Cell::new(false);
    let item = Cell::new(None::<Rect>);
    let trigger = Cell::new(None::<Rect>);
    let open = Cell::new(false);
    let run = |avail: f32, t_now: f64, events: Vec<egui::Event>| {
        open.set(false);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0))),
            time: Some(t_now),
            events,
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            ribbon::fold_begin(&ctx, avail);
            ui.horizontal(|ui| {
                ribbon::fold_group(ui, &t, "Paso", "@profilegear", 0, |ui| {
                    if ribbon::in_folded_popup(&ctx) {
                        open.set(true);
                    }
                    let pick = |ui: &mut egui::Ui| {
                        let r = ui.selectable_label(false, "Fijo");
                        item.set(Some(r.rect));
                        if r.clicked() {
                            chosen.set(true);
                            ui.close();
                        }
                    };
                    match kind {
                        0 => {
                            let r = egui::ComboBox::from_id_salt("x")
                                .selected_text("Medio")
                                .show_ui(ui, pick);
                            trigger.set(Some(r.response.rect));
                        }
                        1 => {
                            let r = ui.menu_button("Medio  ▾", pick);
                            trigger.set(Some(r.response.rect));
                        }
                        _ => {
                            let r = ribbon::small_menu(ui, &t, "@recent", "Medio", true, pick);
                            trigger.set(Some(r.rect));
                        }
                    }
                });
            });
        });
        out.drop_without_applying_deltas();
    };
    let click = |p: Pos2, mut now: f64| -> f64 {
        run(10.0, now, vec![mv(p)]);
        now += 0.05;
        run(10.0, now, vec![button(p, true, egui::PointerButton::Primary)]);
        now += 0.05;
        run(10.0, now, vec![button(p, false, egui::PointerButton::Primary)]);
        now += 0.05;
        run(10.0, now, vec![]);
        now + 0.4
    };
    // primero con sitio de sobra, para que el grupo mida su ancho; después sin él: se pliega
    let mut now = 0.0;
    for _ in 0..3 {
        run(10_000.0, now, vec![]);
        now += 0.05;
    }
    run(10.0, now, vec![]);
    // se busca el botón del grupo plegado: el primer sitio donde un clic abre su menú
    'scan: for y in (10..80).step_by(8) {
        for x in (10..200).step_by(10) {
            now = click(Pos2::new(x as f32, y as f32), now);
            if open.get() {
                break 'scan;
            }
        }
    }
    if !open.get() {
        return false;
    }
    // con el menú del grupo abierto: abrir la lista y elegir
    let Some(tr) = trigger.get() else { return false };
    now = click(tr.center(), now);
    let Some(it) = item.get() else { return false };
    click(it.center(), now);
    chosen.get()
}

#[test]
fn a_menu_inside_a_folded_ribbon_group_lets_you_choose() {
    assert!(folded_pick(1), "un menú dentro de un grupo plegado debe dejar elegir");
    assert!(folded_pick(2), "small_menu dentro de un grupo plegado debe dejar elegir");
}

#[test]
fn a_combo_box_inside_a_folded_ribbon_group_does_not() {
    // el fallo que había en el paso del perfil: se deja como prueba de que la prueba lo detecta
    assert!(!folded_pick(0));
}

#[test]
fn the_generic_tool_window_draws_file_options_like_the_other_windows() {
    use hipso_core::model::{ArgSpec, Kind};
    let app = App::default();
    let t = app.tokens();
    let ctx = egui::Context::default();
    let spec = |id: &str, multiple: bool| ArgSpec {
        id: id.into(),
        long: Some(id.into()),
        short: None,
        help: "Ayuda de la opción, entera y no solo su primera línea.".into(),
        positional: false,
        flag: false,
        multiple,
        required: true,
        path: true,
        values: Vec::new(),
        default: None,
    };
    let files = vec![("mdt.tif".to_string(), std::path::PathBuf::from("/tmp/mdt.tif"))];
    let mut args = std::collections::BTreeMap::new();
    args.insert("inputs".to_string(), vec!["/tmp/a.tif".to_string(), "/tmp/b.tif".to_string()]);
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
        ..egui::RawInput::default()
    };
    let out = ctx.run_ui(input, |ui| {
        app.cli_file_row(ui, &t, "rasterize", &spec("inputs", true), Kind::Raster, &files, &mut args);
        app.cli_file_row(ui, &t, "rasterize", &spec("layer", false), Kind::Vector, &[], &mut args);
    });
    out.drop_without_applying_deltas();
    assert_eq!(args["inputs"].len(), 2, "dibujar no cambia lo elegido");
}
