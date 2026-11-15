//! Edición vectorial avanzada: mover, girar y escalar la selección, reformar el contorno de los
//! polígonos con un trazo, copiar y pegar elementos entre capas y comprobar la topología. La
//! lógica geométrica vive en `hipso_core::editops` y `hipso_core::topology`; aquí solo están las
//! ventanas y la conexión con las capas del mapa.

use super::*;
use hipso_core::{editops, topology};

/// Parámetros de la ventana de transformar la selección.
pub struct TransformDlg {
    /// Desplazamiento en las unidades de la capa.
    pub dx: f64,
    pub dy: f64,
    /// Giro en grados, antihorario, alrededor del centro de la selección.
    pub angle: f64,
    /// Factor de escala alrededor del centro de la selección.
    pub scale: f64,
}

impl Default for TransformDlg {
    fn default() -> Self {
        Self { dx: 0.0, dy: 0.0, angle: 0.0, scale: 1.0 }
    }
}

/// Estado de la ventana de comprobación de topología.
pub struct TopoDlg {
    /// Superficie mínima de un solape o un hueco (en polígonos) o distancia de unión de extremos
    /// (en líneas).
    pub tol: f64,
    pub issues: Vec<topology::Issue>,
    pub ran: bool,
}

impl Default for TopoDlg {
    fn default() -> Self {
        Self { tol: 1e-6, issues: Vec::new(), ran: false }
    }
}

impl App {
    /// Mueve, gira y escala los elementos seleccionados de la capa activa.
    pub fn transform_selected(&mut self, d: &TransformDlg) {
        let Some(v) = self.active_vec_mut() else { return };
        let idx: Vec<usize> = v.selected.iter().copied().filter(|i| *i < v.data.features.len()).collect();
        if idx.is_empty() {
            return;
        }
        // el centro de toda la selección, no el de cada elemento, para que conserven su disposición
        let all: Vec<Vec<[f64; 2]>> = idx.iter().flat_map(|&i| v.data.features[i].parts.clone()).collect();
        let Some(pivot) = editops::center_of(&all) else { return };
        v.push_undo();
        for &i in &idx {
            editops::transform(&mut v.data.features[i].parts, pivot, d.dx, d.dy, d.angle, d.scale);
        }
        v.rebuild();
        v.dirty = true;
        v.reclassify();
        self.log(format!("{} elemento(s) transformados.", idx.len()));
    }

    /// Sustituye el tramo de contorno que cruza el trazo `stroke` en los polígonos seleccionados o,
    /// si no hay selección, en los que toca el trazo.
    pub fn reshape_polygons(&mut self, stroke: &[[f64; 2]]) {
        let Some(v) = self.active_vec_mut() else { return };
        let b = stroke.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
            [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
        });
        let cands: Vec<usize> = if v.selected.is_empty() {
            v.in_window(b)
        } else {
            v.selected.iter().copied().collect()
        };
        // primero se calcula todo y después se aplica, para deshacer de una vez
        let mut changes: Vec<(usize, usize, Vec<[f64; 2]>)> = Vec::new();
        for i in cands {
            let Some(f) = v.data.features.get(i) else { continue };
            for (k, ring) in f.parts.iter().enumerate() {
                if let Some(r) = editops::reshape_ring(ring, stroke) {
                    changes.push((i, k, r));
                }
            }
        }
        if changes.is_empty() {
            self.log("El trazo no cruza el contorno de ningún polígono dos veces: empieza y termina fuera o dentro de él, atravesando el borde.");
            return;
        }
        v.push_undo();
        let n = changes.len();
        for (i, k, r) in changes {
            v.data.features[i].parts[k] = r;
        }
        v.rebuild();
        v.dirty = true;
        v.reclassify();
        self.log(format!("{n} contorno(s) reformados."));
    }

    /// Guarda en el portapapeles interno los elementos seleccionados con sus atributos.
    pub fn copy_selected(&mut self) {
        let Some(v) = self.active_vec() else { return };
        let mut idx: Vec<usize> = v.selected.iter().copied().filter(|i| *i < v.data.features.len()).collect();
        idx.sort_unstable();
        if idx.is_empty() {
            return;
        }
        let mut c = v.data.clone();
        c.features = idx.iter().map(|&i| v.data.features[i].clone()).collect();
        c.text = idx.iter().map(|&i| v.data.text.get(i).cloned().unwrap_or_default()).collect();
        c.num = idx.iter().map(|&i| v.data.num.get(i).cloned().unwrap_or_default()).collect();
        let kind = v.kind;
        let n = idx.len();
        self.vm.clip = Some((c, kind));
        self.log(format!("{n} elemento(s) copiados."));
    }

    /// Pega los elementos copiados en la capa activa.
    pub fn paste_into_active(&mut self) {
        let Some((src, sk)) = self.vm.clip.clone() else { return };
        let Some(v) = self.active_vec_mut() else { return };
        v.push_undo();
        let rows: Vec<usize> = (0..src.features.len()).collect();
        let kind = v.kind;
        match editops::paste(&mut v.data, kind, &src, sk, &rows) {
            Ok(added) => {
                v.rebuild();
                v.selected = added.iter().copied().collect();
                v.dirty = true;
                v.reclassify();
                self.log(format!("{} elemento(s) pegados.", added.len()));
            }
            Err(e) => {
                // no se cambió nada: se retira el punto de deshacer
                v.undo.pop();
                self.log(format!("No se pega: {e}."));
            }
        }
    }

    /// Crea una capa nueva con los elementos copiados.
    pub fn paste_as_layer(&mut self) {
        let Some((data, kind)) = self.vm.clip.clone() else { return };
        let mut layer = layers::VecLayer::new(data, kind);
        layer.dirty = true;
        let name = format!("Copia {}", self.refs.len() + 1);
        self.add_ref(
            RefLayer { name, path: PathBuf::new(), kind: RefKind::Vector(Box::new(layer)), visible: true, opacity: 1.0 },
            false,
        );
        let k = self.refs.len() - 1;
        self.active = Some(k);
        self.sel_layer = Some(k);
    }

    /// La ventana de mover, girar y escalar.
    pub fn transform_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vm.transform.take() else { return };
        let (mut open, mut go) = (true, false);
        egui::Window::new("Transformar selección")
            .open(&mut open)
            .collapsible(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label("Se mueve, gira y escala la selección entera alrededor de su centro; las distancias van en las unidades de la capa.");
                egui::Grid::new("transform_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Desplazamiento en X");
                    ui.add(egui::DragValue::new(&mut d.dx).speed(0.1));
                    ui.end_row();
                    ui.label("Desplazamiento en Y");
                    ui.add(egui::DragValue::new(&mut d.dy).speed(0.1));
                    ui.end_row();
                    ui.label("Giro (grados, antihorario)");
                    ui.add(egui::DragValue::new(&mut d.angle).speed(0.5));
                    ui.end_row();
                    ui.label("Escala");
                    ui.add(egui::DragValue::new(&mut d.scale).speed(0.01).range(0.001..=1000.0));
                    ui.end_row();
                });
                go = ui.button("▶ Aplicar").clicked();
            });
        if go {
            self.transform_selected(&d);
            d = TransformDlg::default();
        }
        if open {
            self.vm.transform = Some(d);
        }
    }

    /// La ventana de topología: comprueba la capa activa y lista los incumplimientos; pulsar uno
    /// selecciona los elementos implicados y acerca el mapa.
    pub fn topology_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.vm.topo.take() else { return };
        let (mut open, mut run, mut pick) = (true, false, None);
        let kind = self.active_vec().map(|v| v.kind);
        egui::Window::new("Comprobar topología")
            .open(&mut open)
            .collapsible(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                match kind {
                    Some(view::GeomKind::Polygon) => {
                        ui.label("Polígonos: solapes, huecos entre ellos y contornos que se cruzan consigo mismos.");
                    }
                    Some(view::GeomKind::Line) => {
                        ui.label("Líneas: extremos que no tocan ninguna otra línea (cabos sueltos).");
                    }
                    _ => {
                        ui.label("Elige una capa de polígonos o de líneas.");
                        return;
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Tolerancia");
                    ui.add(egui::DragValue::new(&mut d.tol).speed(0.001).range(0.0..=1e9))
                        .on_hover_text("Superficie mínima de un solape o un hueco; en líneas, la distancia a la que dos extremos se tienen por unidos.");
                });
                run = ui.button("▶ Comprobar").clicked();
                if d.ran {
                    if d.issues.is_empty() {
                        ui.label("Sin incumplimientos.");
                    } else {
                        ui.label(format!("{} incumplimiento(s):", d.issues.len()));
                    }
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for (n, i) in d.issues.iter().enumerate().take(500) {
                            let rows: Vec<String> = i.features.iter().map(|r| (r + 1).to_string()).collect();
                            let area = if i.area > 0.0 { format!(", {:.3}", i.area) } else { String::new() };
                            if ui.selectable_label(false, format!("{}: {}{area}", i.rule.label(), if rows.is_empty() { "—".into() } else { rows.join(" y ") })).clicked() {
                                pick = Some(n);
                            }
                        }
                        if d.issues.len() > 500 {
                            ui.label("Solo se muestran los 500 primeros.");
                        }
                    });
                }
            });
        if run && let Some(v) = self.active_vec() {
            d.issues = topology::check(&v.data, v.kind, d.tol);
            d.ran = true;
        }
        if let Some(n) = pick
            && let Some(i) = d.issues.get(n).cloned()
        {
            if let Some(v) = self.active_vec_mut() {
                v.selected = i.features.iter().copied().filter(|r| *r < v.data.features.len()).collect();
            }
            let r = i.area.sqrt().max(1.0) * 3.0;
            self.zoom_to_bounds([i.at[0] - r, i.at[1] - r, i.at[0] + r, i.at[1] + r]);
        }
        if open {
            self.vm.topo = Some(d);
        }
    }
}
