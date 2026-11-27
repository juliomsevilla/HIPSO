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
        
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            time: Some(t),
            events,
            ..egui::RawInput::default()
        }
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
    let store = hipso_core::view::load(std::slice::from_ref(&path), 3_000_000).unwrap();
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
        if let Some(op) = app.ed.ops.first()
            && let Region::Side { path, .. } | Region::Near { path, .. } = &op.region {
                eprintln!("{}: línea {path:?}, ancho de caja {}", nav.name(), bx.width);
            }
        let ps = app.profile_store(&ctx, &bx).unwrap();
        {
            let mut n_strip = 0;
            let mut n_above = 0;
            let mut zs = (f64::MAX, f64::MIN);
            for i in 0..ps.len() {
                let p = ps.xyz(i);
                let (u, t) = bx.local(p[0], p[1]);
                if (23.4..=90.0).contains(&u) && t.abs() <= 5.0 {
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
        let cyan = img.rgba.as_chunks::<4>().0.iter().filter(|p| [p[0], p[1], p[2]] == hipso_core::view::HIGHLIGHT).count();
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
        let e = hipso_core::view::load(std::slice::from_ref(&edited), 6_000_000).unwrap();
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
    let px = |c: [u8; 3]| img.rgba.as_chunks::<4>().0.iter().filter(|p| [p[0], p[1], p[2]] == c).count();
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

/// Un suelo a cota 0 con un tejado de 8×8 m a cota 5, en rejilla de 0,25 m.
fn ground_and_roof() -> PointStore {
    let mut s = PointStore {
        stride: 1,
        ..PointStore::default()
    };
    for i in 0..120 {
        for j in 0..120 {
            let (x, y) = (i as f32 * 0.25, j as f32 * 0.25);
            let roof = (10.0..18.0).contains(&x) && (10.0..18.0).contains(&y);
            s.x.push(x);
            s.y.push(y);
            s.z.push(if roof { 5.0 } else { 0.0 });
            s.class.push(2);
            s.intensity.push(100);
            s.ret.push(0x11);
            s.src.push(0);
            s.scan.push(0);
            s.file.push(0);
        }
    }
    s.total = s.len() as u64;
    s.bounds = [0.0, 0.0, 0.0, 30.0, 30.0, 5.0];
    s
}

#[test]
fn the_wand_selects_the_roof_and_not_the_ground_and_a_class_change_sticks() {
    let store = ground_and_roof();
    let mut app = App::default();
    app.ed.session = Some(Session::new(&store));
    app.store = Some(Arc::new(store));
    app.plan.scale = 10.0;
    app.ed.wand_link = 0.5;
    let ctx = egui::Context::default();
    app.select_by_wand(&ctx, 14.1, 14.1);
    app.process_edit_cmds();
    let store = app.store.clone().unwrap();
    let on_roof = |i: usize| store.z[i] > 1.0;
    let n_roof = (0..store.len()).filter(|&i| on_roof(i)).count();
    let picked: Vec<usize> = (0..store.len()).filter(|&i| store.selected[i]).collect();
    assert!(picked.iter().all(|&i| on_roof(i)), "se coló suelo");
    assert!(picked.len() * 10 > n_roof * 8, "{} de {n_roof}", picked.len());
    // clasificar lo seleccionado y comprobar que el archivo de ediciones lo repite
    app.queue(editing::Cmd::Reclass(6));
    app.process_edit_cmds();
    let store = app.store.clone().unwrap();
    assert!(picked.iter().all(|&i| store.class[i] == 6));
    assert!((0..store.len()).filter(|&i| !on_roof(i)).all(|i| store.class[i] == 2));
    let ses = app.ed.session.as_ref().unwrap();
    let json = hipso_core::edit::file::to_json(&ses.edits);
    assert!(json.contains("\"points\""));
    assert!(hipso_core::edit::file::from_json(&json).is_ok());
}

/// Una capa de 40 rodales con altura y edad, para los gráficos.
fn charts_app() -> App {
    let mut d = view::VectorData { polygons: false, points: true, ..view::VectorData::default() };
    d.features = (0..40).map(|i| view::Feature { parts: vec![vec![[f64::from(i), 0.0]]] }).collect();
    d.text = vec![Vec::new(); 40];
    d.num = vec![Vec::new(); 40];
    let h = d.add_field("altura", hipso_core::fieldtype::FieldType::Real);
    let e = d.add_field("edad", hipso_core::fieldtype::FieldType::Real);
    for i in 0..40 {
        let _ = d.set_value(i, h, &format!("{}", 5 + i / 2));
        let _ = d.set_value(i, e, &format!("{}", 10 + i));
    }
    let mut app = App::default();
    app.refs.push(RefLayer {
        name: "Rodales".into(),
        path: PathBuf::from("/tmp/rodales.shp"),
        kind: RefKind::Vector(Box::new(layers::VecLayer::new(d, view::GeomKind::Point))),
        visible: true,
        opacity: 1.0,
    });
    app.ch.open = true;
    app
}

fn charts_frame(app: &mut App, ctx: &egui::Context, t: f64, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
        time: Some(t),
        events,
        ..egui::RawInput::default()
    };
    let area = Rect::from_min_max(Pos2::new(0.0, 500.0), Pos2::new(1200.0, 800.0));
    let out = ctx.run_ui(input, |ui| app.charts_dock(ui, area, 600.0));
    out.drop_without_applying_deltas();
}

#[test]
fn marking_a_histogram_range_selects_those_rows_and_the_rest_stay() {
    let mut app = charts_app();
    let ctx = egui::Context::default();
    for k in 0..3 {
        charts_frame(&mut app, &ctx, f64::from(k) * 0.05, vec![mv(Pos2::new(5.0, 5.0))]);
    }
    assert_eq!(app.ch_src_for_test(), Some(0));
    // arrastrar sobre la parte izquierda del histograma (alturas bajas)
    let (a, b) = (Pos2::new(110.0, 640.0), Pos2::new(200.0, 640.0));
    let mut t = 0.2;
    charts_frame(&mut app, &ctx, t, vec![mv(a)]);
    t += 0.05;
    charts_frame(&mut app, &ctx, t, vec![button(a, true, egui::PointerButton::Primary)]);
    for k in 1..=6 {
        t += 0.05;
        charts_frame(&mut app, &ctx, t, vec![mv(a + (b - a) * (k as f32 / 6.0))]);
    }
    t += 0.05;
    charts_frame(&mut app, &ctx, t, vec![button(b, false, egui::PointerButton::Primary)]);
    charts_frame(&mut app, &ctx, t + 0.05, vec![]);
    let RefKind::Vector(v) = &app.refs[0].kind else { panic!("sin capa") };
    assert!(!v.selected.is_empty() && v.selected.len() < 40, "{} seleccionados", v.selected.len());
    // son filas de alturas intermedias-bajas: ni la primera ni la última, y sin huecos
    let (lo, hi) = (v.selected.iter().min().copied().unwrap_or(0), v.selected.iter().max().copied().unwrap_or(0));
    assert!(lo > 0 && hi < 39, "{lo}..{hi}");
    assert_eq!(v.selected.len(), hi - lo + 1, "la selección tiene huecos");
}

#[test]
fn marking_a_scatter_rectangle_of_points_queues_one_rule_over_two_attributes() {
    let store = cloud();
    let mut app = App::default();
    app.ed.session = Some(Session::new(&store));
    app.store = Some(Arc::new(store));
    app.ch.open = true;
    let ctx = egui::Context::default();
    for k in 0..3 {
        charts_frame(&mut app, &ctx, f64::from(k) * 0.05, vec![mv(Pos2::new(5.0, 5.0))]);
    }
    assert!(app.ch.src_is_points_for_test());
    // la dispersión es el segundo gráfico: arrastrar un rectángulo dentro de su trazado
    let (a, b) = (Pos2::new(520.0, 620.0), Pos2::new(640.0, 700.0));
    let mut t = 0.2;
    charts_frame(&mut app, &ctx, t, vec![mv(a)]);
    t += 0.05;
    charts_frame(&mut app, &ctx, t, vec![button(a, true, egui::PointerButton::Primary)]);
    for k in 1..=6 {
        t += 0.05;
        charts_frame(&mut app, &ctx, t, vec![mv(a + (b - a) * (k as f32 / 6.0))]);
    }
    t += 0.05;
    charts_frame(&mut app, &ctx, t, vec![button(b, false, egui::PointerButton::Primary)]);
    charts_frame(&mut app, &ctx, t + 0.05, vec![]);
    let ops: Vec<&hipso_core::edit::SelOp> = app
        .ed
        .cmds
        .iter()
        .filter_map(|c| match c {
            editing::Cmd::AddOp(op) => Some(op.as_ref()),
            _ => None,
        })
        .collect();
    assert_eq!(ops.len(), 1, "la dispersión no puso ninguna regla");
    assert_eq!(ops[0].attr.ranges.len(), 2);
    assert!(matches!(ops[0].region, Region::All));
}

#[test]
fn the_vector_ribbon_groups_are_in_order_and_hold_their_tools() {
    let app = App::default();
    let groups = app.vector_groups();
    let captions: Vec<&str> = groups.iter().map(|g| g.caption).collect();
    assert_eq!(captions, vec!["Capa", "Geometría", "Tabla", "Geoprocesos"]);
    let labels = |caption: &str| -> Vec<&'static str> {
        groups
            .iter()
            .find(|g| g.caption == caption)
            .map(|g| g.columns.iter().flatten().map(|i| i.label).collect())
            .unwrap_or_default()
    };
    let geometry = labels("Geometría");
    for want in ["Comprobar validez", "Reparar geometrías", "Reparar topología", "Simplificar", "Paralela", "Recortar capa"] {
        assert!(geometry.contains(&want), "falta {want} en Geometría: {geometry:?}");
    }
    let table = labels("Tabla");
    for want in ["Unir tabla", "Dividir por campo", "Quitar y renombrar campos", "Extraer valores"] {
        assert!(table.contains(&want), "falta {want} en Tabla: {table:?}");
    }
    let geo = labels("Geoprocesos");
    for want in ["Búfer", "Disolver", "Unión", "Intersección", "Estadística zonal", "Combinar capas", "Cercanía"] {
        assert!(geo.contains(&want), "falta {want} en Geoprocesos: {geo:?}");
    }
    // cada herramienta está en un solo grupo
    let all: Vec<&str> = ["Geometría", "Tabla", "Geoprocesos"].iter().flat_map(|c| labels(c)).collect();
    let mut unique = all.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(all.len(), unique.len(), "hay herramientas repetidas: {all:?}");
}

#[test]
fn the_three_charts_fit_inside_the_dock() {
    let mut app = charts_app();
    let ctx = egui::Context::default();
    for k in 0..4 {
        charts_frame(&mut app, &ctx, f64::from(k) * 0.05, vec![mv(Pos2::new(5.0, 5.0))]);
    }
    let rects = app.ch_rects_for_test();
    for r in rects {
        assert!(r.is_positive(), "un gráfico no se dibujó: {rects:?}");
    }
    // el dock ocupa el ancho de 1200 puntos: ninguno de los tres se sale
    assert!(rects[2].right() <= 1200.0 - 4.0, "el tercer gráfico se sale del dock: {rects:?}");
    assert!(rects[0].left() < rects[1].left() && rects[1].left() < rects[2].left());
}

#[test]
fn clicking_the_map_with_the_coordinates_tool_notes_the_point_and_opens_the_list() {
    let mut app = charts_app();
    app.nav = Nav::Coords;
    let ctx = egui::Context::default();
    let plan = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 500.0));
    let frame_at = |app: &mut App, t: f64, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
            time: Some(t),
            events,
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| {
            let c = ui.ctx().clone();
            app.plan_view(ui, &c, plan);
        });
        out.drop_without_applying_deltas();
    };
    for k in 0..3 {
        frame_at(&mut app, f64::from(k) * 0.05, vec![mv(Pos2::new(300.0, 300.0))]);
    }
    assert!(app.crd.list.is_empty());
    let p = Pos2::new(400.0, 250.0);
    frame_at(&mut app, 0.2, vec![mv(p)]);
    frame_at(&mut app, 0.25, vec![button(p, true, egui::PointerButton::Primary)]);
    frame_at(&mut app, 0.3, vec![button(p, false, egui::PointerButton::Primary)]);
    frame_at(&mut app, 0.35, vec![]);
    assert_eq!(app.crd.list.len(), 1, "el clic no anotó ninguna coordenada");
    assert!(app.crd.open, "la ventana de capturas no se abrió");
    // otro clic, otro punto distinto
    let q = Pos2::new(500.0, 350.0);
    frame_at(&mut app, 0.4, vec![mv(q)]);
    frame_at(&mut app, 0.45, vec![button(q, true, egui::PointerButton::Primary)]);
    frame_at(&mut app, 0.5, vec![button(q, false, egui::PointerButton::Primary)]);
    frame_at(&mut app, 0.55, vec![]);
    assert_eq!(app.crd.list.len(), 2);
    assert_ne!(app.crd.list[0], app.crd.list[1]);
}

#[test]
fn the_table_toolbar_does_not_overlap_its_texts_even_with_a_long_layer_name() {
    let mut data = view::VectorData::default();
    data.fields = vec!["name".to_string()];
    for n in ["a", "b", "c"] {
        data.features.push(view::Feature { parts: vec![vec![[0.0, 0.0]]] });
        data.text.push(vec![n.to_string()]);
        data.num.push(vec![f64::NAN]);
    }
    data.points = true;
    for (name, width) in [
        ("PARCELA_TLS_DE_PRUEBA_cubicado_con_un_nombre_larguisimo_de_capa.shp", 1100.0f32),
        ("PARCELA_TLS_DE_PRUEBA_cubicado.shp", 1480.0),
        ("capa.shp", 900.0),
    ] {
        let mut app = App::default();
        app.refs.push(RefLayer {
            name: name.into(),
            path: PathBuf::from(format!("/tmp/{name}")),
            kind: RefKind::Vector(Box::new(layers::VecLayer::new(data.clone(), view::GeomKind::Point))),
            visible: true,
            opacity: 1.0,
        });
        app.table_layer = Some(0);
        let ctx = egui::Context::default();
        let area = Rect::from_min_max(Pos2::new(0.0, 500.0), Pos2::new(width, 800.0));
        for k in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width + 20.0, 900.0))),
                time: Some(f64::from(k) * 0.05),
                ..egui::RawInput::default()
            };
            let out = ctx.run_ui(input, |ui| app.table_window(ui, area, 600.0));
            if k < 3 {
                out.drop_without_applying_deltas();
                continue;
            }
            // los textos visibles de la barra de herramientas (franja de arriba de la tabla)
            let mut texts: Vec<(String, Rect)> = Vec::new();
            for cs in &out.shapes {
                match &cs.shape {
                    egui::epaint::Shape::Text(ts) => {
                        let r = ts.galley.rect.translate(ts.pos.to_vec2()).intersect(cs.clip_rect);
                        if r.is_positive() && r.top() >= area.top() && r.bottom() <= area.top() + 40.0 {
                            texts.push((ts.galley.text().to_string(), r));
                        }
                    }
                    // los iconos son mallas con textura
                    egui::epaint::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() || !m.indices.is_empty() => {
                        let r = m.calc_bounds().intersect(cs.clip_rect);
                        if r.is_positive() && r.width() < 40.0 && r.height() < 40.0 && r.top() >= area.top() && r.bottom() <= area.top() + 40.0 {
                            texts.push(("(icono)".to_string(), r));
                        }
                    }
                    _ => {}
                }
            }
            assert!(texts.iter().any(|t| t.0.contains("seleccionados")), "{name}: no se dibujó la barra");
            // con un ancho razonable no se queda fuera ningún botón: los 12 iconos (tabla, seleccionar por
            // atributos, 5 de selección, deshacer, rehacer, guardar, añadir campo y cerrar) se ven
            if width >= 1100.0 {
                let icons = texts.iter().filter(|t| t.0 == "(icono)").count();
                assert!(icons >= 12, "{name} ({width}): solo se ven {icons} iconos de la barra");
            }
            for (i, a) in texts.iter().enumerate() {
                for b in texts.iter().skip(i + 1) {
                    let inter = a.1.intersect(b.1);
                    assert!(!(inter.width() > 1.0 && inter.height() > 1.0), "{name} ({width}): «{}» pisa a «{}»", a.0, b.0);
                }
            }
            out.drop_without_applying_deltas();
        }
    }
}

/// Una carpeta temporal para una prueba, borrada al soltarla.
struct Carpeta(PathBuf);

impl Carpeta {
    fn new(name: &str) -> std::io::Result<Self> {
        let d = std::env::temp_dir().join(format!("hipso_gui_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d)?;
        Ok(Self(d))
    }
}

impl Drop for Carpeta {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn saving_a_layer_over_its_file_keeps_long_texts_or_leaves_it_alone() {
    let dir = Carpeta::new("capa").unwrap();
    let path = dir.0.join("rodales.shp");
    let layer = |text: &str| view::VectorData {
        fields: vec!["nota".into()],
        text: vec![vec![text.to_string()]],
        num: vec![vec![f64::NAN]],
        features: vec![view::Feature { parts: vec![vec![[1.0, 2.0]]] }],
        points: true,
        ..view::VectorData::default()
    };
    hipso_core::vector::write_layer(&path, &layer("antes"), view::GeomKind::Point, None).unwrap();
    let mut app = App::default();
    let long = "x".repeat(200);
    app.refs.push(RefLayer {
        name: "rodales".into(),
        path: path.clone(),
        kind: RefKind::Vector(Box::new(layers::VecLayer::new(layer(&long), view::GeomKind::Point))),
        visible: true,
        opacity: 1.0,
    });
    app.save_layer(0, false);
    assert_eq!(view::read_vector(&path).unwrap().text[0][0], long);
    // lo que no cabe en un shapefile: se dice, el archivo queda como estaba y la capa sigue sin
    // guardar
    let dbf = std::fs::read(dir.0.join("rodales.dbf")).unwrap();
    if let RefKind::Vector(v) = &mut app.refs[0].kind {
        v.data.text[0][0] = "y".repeat(300);
        v.dirty = true;
    }
    app.save_layer(0, false);
    assert_eq!(std::fs::read(dir.0.join("rodales.dbf")).unwrap(), dbf);
    assert!(app.log.last().unwrap().contains("254"), "{:?}", app.log.last());
    assert!(matches!(&app.refs[0].kind, RefKind::Vector(v) if v.dirty));
}

/// Una nube LAS 1.2 mínima de diez puntos.
fn write_las(path: &Path) {
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[105..107].copy_from_slice(&20u16.to_le_bytes());
    h[107..111].copy_from_slice(&10u32.to_le_bytes());
    for k in 0..3 {
        h[131 + 8 * k..139 + 8 * k].copy_from_slice(&0.01f64.to_le_bytes());
    }
    for i in 0..10i32 {
        let mut r = [0u8; 20];
        r[..4].copy_from_slice(&(i * 100).to_le_bytes());
        r[14] = 0x09;
        r[15] = 1;
        h.extend(r);
    }
    std::fs::write(path, h).unwrap();
}

/// Espera a que termine la tarea en marcha.
fn finish_task(app: &mut App, ctx: &egui::Context) {
    for _ in 0..500 {
        app.poll(ctx);
        if app.running.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("la tarea no termina");
}

#[test]
fn an_edit_is_marked_as_saved_only_when_the_cloud_was_written() {
    let dir = Carpeta::new("edicion").unwrap();
    let cloud_path = dir.0.join("a.las");
    write_las(&cloud_path);
    let original = std::fs::read(&cloud_path).unwrap();
    let ctx = egui::Context::default();
    let store = cloud();
    let mut app = App::default();
    let mut session = Session::new(&store);
    let mut store = store;
    let ring = hipso_core::edit::circle(0.0, 0.0, 1.5);
    let sel = hipso_core::edit::SelOp {
        mode: hipso_core::edit::Mode::New,
        region: Region::Plan(ring),
        attr: hipso_core::edit::Attr::default(),
    };
    let edit = hipso_core::edit::Edit::new(vec![sel], hipso_core::edit::Action::Delete, "borrar".into());
    session.apply(&mut store, edit);
    app.ed.session = Some(session);
    app.ed.auto_save = true;
    app.clouds = vec![cloud_path.clone()];
    // primero falla: el archivo de entrada ha desaparecido
    std::fs::remove_file(&cloud_path).unwrap();
    app.save_edited(&ctx);
    assert_eq!(app.ed.saved, 0, "no se marca antes de terminar");
    finish_task(&mut app, &ctx);
    assert_eq!(app.ed.saved, 0, "un guardado fallido no marca nada");
    // ahora va bien
    std::fs::write(&cloud_path, &original).unwrap();
    app.save_edited(&ctx);
    finish_task(&mut app, &ctx);
    assert_eq!(app.ed.saved, 1, "{:?}", app.log.last());
    assert_eq!(std::fs::read(&cloud_path).unwrap(), original, "el original no se toca");
    assert!(dir.0.join("a_editada.las").is_file());
}

/// Las nubes se reordenan dentro de su grupo arrastrando la fila, y la que queda arriba tapa.
#[test]
fn a_cloud_is_dragged_above_another_in_the_contents_tree() {
    let mut app = App::default();
    app.clouds = vec!["primera.laz".into(), "segunda.laz".into(), "tercera.laz".into()];
    let ctx = egui::Context::default();
    // los nombres de las nubes en pantalla, de arriba abajo, con su sitio
    let draw = |app: &mut App, t: f64, events: Vec<egui::Event>| -> Vec<(String, Pos2)> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 700.0))),
            time: Some(t),
            events,
            ..egui::RawInput::default()
        };
        let out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.contents_ui(ui, &ctx);
        });
        let mut v = Vec::new();
        for cs in &out.shapes {
            if let egui::Shape::Text(t) = &cs.shape
                && t.galley.text().ends_with(".laz")
            {
                v.push((t.galley.text().to_string(), t.pos + t.galley.size() / 2.0));
            }
        }
        v.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
        out.drop_without_applying_deltas();
        v
    };
    let names = |v: &[(String, Pos2)]| v.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    draw(&mut app, 0.0, vec![]);
    let before = draw(&mut app, 0.05, vec![]);
    assert_eq!(names(&before), ["primera.laz", "segunda.laz", "tercera.laz"]);
    // la tercera, a lo alto
    let (a, b) = (before[2].1, before[0].1 - Vec2::new(0.0, 8.0));
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(&mut app, 0.1, vec![mv(a)]);
    draw(&mut app, 0.15, vec![press(a, true)]);
    for k in 1..=6 {
        draw(&mut app, 0.15 + 0.05 * f64::from(k), vec![mv(a + (b - a) * (k as f32 / 6.0))]);
    }
    draw(&mut app, 0.6, vec![press(b, false)]);
    let after = draw(&mut app, 0.65, vec![]);
    assert_eq!(names(&after), ["tercera.laz", "primera.laz", "segunda.laz"]);
    assert_eq!(app.cloud_stack(), [2, 0, 1]);
    // los archivos no cambian de número: solo el orden de dibujo
    assert_eq!(app.clouds[0], PathBuf::from("primera.laz"));
    // una nube recién abierta queda arriba, y cerrar una no desordena las demás
    app.clouds.insert(0, "nueva.laz".into());
    assert_eq!(app.cloud_stack(), [0, 3, 1, 2]);
    app.drop_cloud(1);
    assert_eq!(app.cloud_stack(), [0, 2, 1]);
}

/// El grupo de nubes entero sube y baja respecto de las capas.
#[test]
fn the_cloud_group_moves_above_and_below_the_layers() {
    let mut app = App::default();
    app.clouds = vec!["una.laz".into()];
    app.refs.push(RefLayer {
        name: "capa".into(),
        path: PathBuf::from("capa.shp"),
        kind: RefKind::Vector(Box::new(layers::VecLayer::new(Default::default(), view::GeomKind::Point))),
        visible: true,
        opacity: 1.0,
    });
    app.cloud_slot = 0;
    assert_eq!(app.order_items(), [layers::Item::Clouds, layers::Item::Layer(0)]);
    app.step_item(layers::Item::Clouds, true);
    assert_eq!(app.order_items(), [layers::Item::Layer(0), layers::Item::Clouds]);
    app.step_item(layers::Item::Clouds, false);
    assert_eq!(app.order_items(), [layers::Item::Clouds, layers::Item::Layer(0)]);
}
