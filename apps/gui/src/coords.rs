//! Coordenadas en el mapa: capturarlas con el ratón (una herramienta de la cinta, junto a «Zoom
//! anterior») e ir a unas coordenadas escritas (un botón de la barra de estado, junto a la
//! escala).
//!
//! Las coordenadas se dan en el sistema de la vista (el de la primera nube); tanto la lista de
//! capturas como el destino admiten otro sistema por código EPSG, y se pasan con las mismas
//! transformaciones que el resto del programa.

use super::*;

/// La ventana de «Ir a coordenadas».
pub struct Goto {
    pub x: String,
    pub y: String,
    /// Sistema en que están escritas; vacío, el de la vista.
    pub epsg: String,
    pub msg: String,
}

impl Default for Goto {
    fn default() -> Self {
        Self { x: String::new(), y: String::new(), epsg: String::new(), msg: String::new() }
    }
}

#[derive(Default)]
pub struct Coords {
    /// Lo que se ha pulsado con la herramienta, en el sistema de la vista.
    pub list: Vec<[f64; 2]>,
    pub open: bool,
    /// Sistema en que se muestran las capturas; vacío, el de la vista.
    pub show_epsg: String,
    pub goto: Option<Goto>,
    /// El último destino de «Ir a coordenadas», que se marca en el mapa.
    pub mark: Option<[f64; 2]>,
}

/// El código EPSG que escribe el usuario (`25830`, `EPSG:25830`).
fn parse_code(s: &str) -> Option<u16> {
    s.trim().trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ':').parse().ok()
}

/// Un número escrito con coma o con punto decimal.
fn number(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite())
}

/// X e Y de los dos cuadros; si Y está vacío, X puede traer los dos separados por un espacio, un
/// tabulador o un punto y coma (lo que copia la lista de capturas).
pub fn parse_xy(x: &str, y: &str) -> Option<(f64, f64)> {
    if y.trim().is_empty() {
        let parts: Vec<&str> = x
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|p| !p.is_empty())
            .collect();
        return match parts.as_slice() {
            [a, b] => Some((number(a)?, number(b)?)),
            _ => None,
        };
    }
    Some((number(x)?, number(y)?))
}

/// Cuántos decimales tiene sentido escribir: grados en un sistema geográfico, milímetros si no.
fn decimals(code: Option<u16>) -> usize {
    if code.is_some_and(|c| (4000..5000).contains(&c)) { 7 } else { 3 }
}

impl App {
    /// Anota una captura y copia sus coordenadas (las que se muestran) al portapapeles.
    pub fn capture_coord(&mut self, ctx: &egui::Context, m: (f64, f64)) {
        self.crd.list.push([m.0, m.1]);
        self.crd.open = true;
        if let Some(text) = self.coord_line(m.0, m.1) {
            ctx.copy_text(text);
        }
    }

    /// El código EPSG en que se muestran las capturas: el escrito o el de la vista.
    fn shown_code(&mut self) -> Option<u16> {
        parse_code(&self.crd.show_epsg).or_else(|| self.project_crs())
    }

    /// Un punto de la vista llevado al sistema en que se muestran las capturas.
    fn shown_point(&mut self, p: [f64; 2]) -> Result<[f64; 2], String> {
        match (parse_code(&self.crd.show_epsg), self.project_crs()) {
            (Some(to), Some(from)) if to != from => hipso_core::reproject::transformer(from, to)
                .and_then(|t| t.transform(p[0], p[1]).map_err(|e| hipso_core::Error::Param(e.to_string())))
                .map(|(x, y)| [x, y])
                .map_err(|e| e.to_string()),
            (Some(_), None) => Err("la vista no declara su sistema de coordenadas".into()),
            _ => Ok(p),
        }
    }

    /// «X Y» de un punto en el sistema que se muestra, separados por un espacio.
    fn coord_line(&mut self, x: f64, y: f64) -> Option<String> {
        let d = decimals(self.shown_code());
        let p = self.shown_point([x, y]).ok()?;
        Some(format!("{:.d$} {:.d$}", p[0], p[1]))
    }

    /// La ventana de las capturas: la lista, el sistema en que verlas y copiarlas.
    pub fn coords_window(&mut self, ctx: &egui::Context) {
        if !self.crd.open {
            return;
        }
        let weak = self.tokens().weak;
        let mut open = true;
        let code = self.shown_code();
        let d = decimals(code);
        let system = code.map_or("sin declarar".to_string(), hipso_core::crs::name);
        let list = self.crd.list.clone();
        let rows: Vec<Result<[f64; 2], String>> = list.iter().map(|p| self.shown_point(*p)).collect();
        let mut clear = false;
        let mut copy_all = None;
        let mut copy_one = None;
        egui::Window::new("Coordenadas capturadas")
            .open(&mut open)
            .collapsible(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Pulsa en el mapa con la herramienta «Capturar coordenadas»: cada clic anota el punto y copia sus coordenadas al portapapeles.",
                    )
                    .color(weak)
                    .size(11.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Mostrar en EPSG");
                    ui.add(egui::TextEdit::singleline(&mut self.crd.show_epsg).desired_width(70.0).hint_text("la vista"))
                        .on_hover_text("Vacío: el sistema de la vista. Con un código (4326, 25830…) las capturas se pasan a ese sistema, y también lo que se copia.");
                    ui.label(egui::RichText::new(system).color(weak).size(11.0));
                });
                if let Some(Err(e)) = rows.iter().find(|r| r.is_err()) {
                    ui.colored_label(Color32::from_rgb(255, 150, 60), e);
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    if rows.is_empty() {
                        ui.label(egui::RichText::new("Todavía no hay capturas.").color(weak));
                    }
                    for (k, r) in rows.iter().enumerate() {
                        if let Ok(p) = r {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("{}", k + 1)).color(weak).monospace());
                                ui.label(egui::RichText::new(format!("X {:.d$}   Y {:.d$}", p[0], p[1])).monospace());
                                if ui.small_button("Copiar").clicked() {
                                    copy_one = Some(format!("{:.d$} {:.d$}", p[0], p[1]));
                                }
                            });
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let any = rows.iter().any(Result::is_ok);
                    if ui.add_enabled(any, egui::Button::new("Copiar todo")).on_hover_text("Una línea por punto: número, X e Y separados por tabuladores, listas para pegar en una hoja de cálculo.").clicked() {
                        let mut s = String::from("n\tx\ty\n");
                        for (k, r) in rows.iter().enumerate() {
                            if let Ok(p) = r {
                                s.push_str(&format!("{}\t{:.d$}\t{:.d$}\n", k + 1, p[0], p[1]));
                            }
                        }
                        copy_all = Some(s);
                    }
                    if ui.add_enabled(!list.is_empty(), egui::Button::new("Vaciar")).clicked() {
                        clear = true;
                    }
                });
            });
        if let Some(s) = copy_one.or(copy_all) {
            ctx.copy_text(s);
        }
        if clear {
            self.crd.list.clear();
        }
        if !open {
            self.crd.open = false;
        }
    }

    /// La ventana de «Ir a coordenadas»: escribe X e Y (en el sistema de la vista o en otro) y el
    /// mapa se centra ahí con la misma ampliación que tenía.
    pub fn goto_window(&mut self, ctx: &egui::Context) {
        let Some(mut g) = self.crd.goto.take() else { return };
        let weak = self.tokens().weak;
        let view_code = self.project_crs();
        let (mut open, mut go, mut clear) = (true, false, false);
        egui::Window::new("Ir a coordenadas")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                let system = view_code.map_or("sin declarar".to_string(), hipso_core::crs::name);
                ui.label(egui::RichText::new(format!("Sistema de la vista: {system}")).color(weak).size(11.0));
                let mut enter = false;
                egui::Grid::new("goto_grid").num_columns(2).show(ui, |ui| {
                    ui.label("X (o longitud)");
                    let r = ui.add(egui::TextEdit::singleline(&mut g.x).desired_width(150.0));
                    enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.end_row();
                    ui.label("Y (o latitud)");
                    let r = ui.add(egui::TextEdit::singleline(&mut g.y).desired_width(150.0));
                    enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.end_row();
                    ui.label("EPSG");
                    ui.add(egui::TextEdit::singleline(&mut g.epsg).desired_width(70.0).hint_text("el de la vista"))
                        .on_hover_text("El sistema en que están escritas las coordenadas. Vacío: el de la vista. Para longitud y latitud, 4326 o 4258.");
                    ui.end_row();
                });
                ui.label(
                    egui::RichText::new("Admite coma o punto decimal. Si dejas Y vacío, X puede traer las dos coordenadas separadas por un espacio o un punto y coma.")
                        .color(weak)
                        .size(11.0),
                );
                if !g.msg.is_empty() {
                    ui.colored_label(Color32::from_rgb(255, 150, 60), &g.msg);
                }
                ui.horizontal(|ui| {
                    let ready = parse_xy(&g.x, &g.y).is_some();
                    if ui.add_enabled(ready, egui::Button::new("Ir")).clicked() || (enter && ready) {
                        go = true;
                    }
                    if ui.add_enabled(self.crd.mark.is_some(), egui::Button::new("Quitar la marca")).clicked() {
                        clear = true;
                    }
                });
            });
        if clear {
            self.crd.mark = None;
        }
        if go {
            g.msg = match self.goto_point(&g) {
                Ok(()) => String::new(),
                Err(e) => e,
            };
        }
        if open {
            self.crd.goto = Some(g);
        }
    }

    /// Centra el mapa en las coordenadas de la ventana, sin cambiar la ampliación.
    fn goto_point(&mut self, g: &Goto) -> Result<(), String> {
        let (x, y) = parse_xy(&g.x, &g.y).ok_or("las coordenadas no son números")?;
        let view = self.project_crs();
        let (x, y) = match (parse_code(&g.epsg), view) {
            (Some(from), Some(to)) if from != to => {
                let t = hipso_core::reproject::transformer(from, to).map_err(|e| e.to_string())?;
                t.transform(x, y).map_err(|e| format!("el punto no pasa al sistema de la vista: {e}"))?
            }
            (Some(_), None) => return Err("la vista no declara su sistema: no se puede pasar a él".into()),
            (None, _) if !g.epsg.trim().is_empty() => return Err("el código EPSG no es un número".into()),
            _ => (x, y),
        };
        let (w, h) = self.plan_px;
        let s = self.plan.scale.max(1e-9) * 0.96;
        let (hw, hh) = (f64::from(w.max(8) as u32) / s / 2.0, f64::from(h.max(8) as u32) / s / 2.0);
        self.zoom_to_bounds([x - hw, y - hh, x + hw, y + hh]);
        self.crd.mark = Some([x, y]);
        Ok(())
    }

    /// Las capturas y el destino de «Ir a coordenadas» sobre el mapa.
    pub fn paint_coords(&self, painter: &egui::Painter, to_screen: impl Fn(f64, f64) -> Pos2) {
        let red = Color32::from_rgb(255, 90, 70);
        for (k, p) in self.crd.list.iter().enumerate() {
            let c = to_screen(p[0], p[1]);
            painter.line_segment([c - Vec2::new(7.0, 0.0), c + Vec2::new(7.0, 0.0)], Stroke::new(1.5, red));
            painter.line_segment([c - Vec2::new(0.0, 7.0), c + Vec2::new(0.0, 7.0)], Stroke::new(1.5, red));
            painter.text(c + Vec2::new(8.0, -8.0), Align2::LEFT_BOTTOM, format!("{}", k + 1), FontId::proportional(11.5), red);
        }
        if let Some(p) = self.crd.mark {
            let c = to_screen(p[0], p[1]);
            let blue = Color32::from_rgb(70, 160, 255);
            painter.circle_stroke(c, 9.0, Stroke::new(2.0, blue));
            painter.circle_filled(c, 2.5, blue);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_numbers_are_read_from_one_box_or_from_two() {
        assert_eq!(parse_xy("440000,5", "4470000,25"), Some((440000.5, 4470000.25)));
        assert_eq!(parse_xy("440000.5 4470000.25", ""), Some((440000.5, 4470000.25)));
        assert_eq!(parse_xy("-3,7038;40,4168", ""), Some((-3.7038, 40.4168)));
        assert_eq!(parse_xy("440000.5\t4470000.25", ""), Some((440000.5, 4470000.25)));
        assert_eq!(parse_xy("440000", ""), None);
        assert_eq!(parse_xy("a", "b"), None);
        assert_eq!(parse_xy("inf", "1"), None);
    }

    #[test]
    fn epsg_codes_are_read_with_or_without_the_prefix() {
        assert_eq!(parse_code("25830"), Some(25830));
        assert_eq!(parse_code("EPSG:4326"), Some(4326));
        assert_eq!(parse_code(" epsg:4258 "), Some(4258));
        assert_eq!(parse_code(""), None);
        assert_eq!(decimals(Some(4326)), 7);
        assert_eq!(decimals(Some(25830)), 3);
        assert_eq!(decimals(None), 3);
    }
}
