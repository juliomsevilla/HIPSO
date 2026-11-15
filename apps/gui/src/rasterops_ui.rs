//! Operaciones ráster: tamizar un ráster de clases, medir la distancia a unas celdas y rellenar
//! las celdas vacías con las de alrededor.

use super::*;
use hipso_core::paths;
use hipso_core::{grid::Grid, rasterops};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Sieve,
    Proximity,
    Fill,
    Vectorize,
    Calc,
    Reclass,
    Clip,
}

impl Op {
    pub const ALL: [Op; 7] = [
        Op::Sieve,
        Op::Proximity,
        Op::Fill,
        Op::Vectorize,
        Op::Calc,
        Op::Reclass,
        Op::Clip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Op::Sieve => "Tamizar",
            Op::Proximity => "Proximidad",
            Op::Fill => "Rellenar huecos",
            Op::Vectorize => "Vectorizar",
            Op::Calc => "Calculadora",
            Op::Reclass => "Reclasificar",
            Op::Clip => "Cortar por máscara",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Op::Sieve => "@sieve",
            Op::Proximity => "@proximity",
            Op::Fill => "@fillholes",
            Op::Vectorize => "@vectorize",
            Op::Calc => "@rastercalc",
            Op::Reclass => "@reclass",
            Op::Clip => "@clip",
        }
    }
}

/// One tool (and window) per operation, in the order of [`Op::ALL`].
pub fn tools() -> Vec<RasterOpsTool> {
    Op::ALL.iter().map(|op| RasterOpsTool::new(*op)).collect()
}

pub struct RasterOpsTool {
    pub open: bool,
    pub op: Op,
    pub input: String,
    /// Sieve: patches with fewer cells than this are merged.
    pub threshold: f64,
    pub eight: bool,
    /// Proximity: the values that count as targets, separated by commas (empty: every non-zero cell).
    pub values: String,
    pub max_distance: String,
    pub in_cells: bool,
    /// Fill: how far to look, in cells, and the passes of smoothing.
    pub fill_distance: f64,
    pub smoothing: f64,
    /// Vectorize: 0 classes, 1 range, 2 breaks; the limits as typed; parts under `min_cells` go.
    pub vec_mode: u8,
    pub vec_lo: String,
    pub vec_hi: String,
    pub vec_breaks: String,
    pub min_cells: f64,
    pub simplify: f64,
    pub gpkg: bool,
    /// Calculator: the rasters (named A, B… in this order, with their bands), the expression,
    /// the sample type (0 decimal, 1 u8, 2 u16), classes, and putting the others on the first.
    pub calc_inputs: Vec<(PathBuf, usize)>,
    pub calc_expr: String,
    pub calc_dtype: u8,
    pub calc_nearest: bool,
    pub calc_align: bool,
    /// Reclassify: the rules as typed, what the cells no rule matches become (0 keep, 1 empty,
    /// 2 the value typed) and the band.
    pub rc_rules: String,
    pub rc_else: u8,
    pub rc_else_value: String,
    pub rc_band: f64,
    /// Clip: the mask (a polygon layer or a raster), cutting to its box, keeping the outside,
    /// and the value of the cells left out as typed.
    pub clip_mask: String,
    pub clip_crop: bool,
    pub clip_invert: bool,
    pub clip_nodata: String,
}

impl RasterOpsTool {
    fn new(op: Op) -> Self {
        Self {
            open: false,
            op,
            input: String::new(),
            threshold: 2.0,
            eight: false,
            values: String::new(),
            max_distance: String::new(),
            in_cells: false,
            fill_distance: 100.0,
            smoothing: 0.0,
            vec_mode: 0,
            vec_lo: String::new(),
            vec_hi: String::new(),
            vec_breaks: String::new(),
            min_cells: 4.0,
            simplify: 0.0,
            gpkg: false,
            calc_inputs: Vec::new(),
            calc_expr: String::new(),
            calc_dtype: 0,
            calc_nearest: false,
            calc_align: false,
            rc_rules: String::new(),
            rc_else: 0,
            rc_else_value: "0".into(),
            rc_band: 1.0,
            clip_mask: String::new(),
            clip_crop: true,
            clip_invert: false,
            clip_nodata: String::new(),
        }
    }
}

impl RasterOpsTool {
    fn targets(&self) -> Result<rasterops::Targets, String> {
        let t = self.values.trim();
        if t.is_empty() {
            return Ok(rasterops::Targets::NonZero);
        }
        t.split([',', ' ', ';'])
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.replace(',', ".")
                    .parse::<f32>()
                    .map_err(|_| format!("«{s}» no es un número"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(rasterops::Targets::Values)
    }

    /// How the cells are grouped to vectorize, from what was typed.
    fn vec_mode(&self) -> Result<hipso_core::rastervec::Mode, String> {
        use hipso_core::rastervec::Mode;
        let n = |s: &str| {
            s.trim()
                .replace(',', ".")
                .parse::<f64>()
                .map_err(|_| format!("«{}» no es un número", s.trim()))
        };
        match self.vec_mode {
            0 => Ok(Mode::Classes),
            1 => Ok(Mode::Range {
                lo: n(&self.vec_lo)?,
                hi: n(&self.vec_hi)?,
            }),
            _ => {
                let cuts = self
                    .vec_breaks
                    .split([';', ' '])
                    .filter(|s| !s.trim().is_empty())
                    .map(n)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Mode::Breaks(cuts))
            }
        }
    }

    /// The names the calculator gives its rasters, with their bands.
    fn calc_names(&self) -> Vec<(String, usize)> {
        let paths: Vec<PathBuf> = self.calc_inputs.iter().map(|i| i.0.clone()).collect();
        hipso_core::rastercalc::letters(&paths)
            .into_iter()
            .zip(&self.calc_inputs)
            .map(|((n, _), (_, b))| (n, *b))
            .collect()
    }

    /// Why the calculator cannot run, with the column of the error in the expression if so.
    fn calc_problem(&self) -> Option<String> {
        if self.calc_inputs.is_empty() {
            return Some("Añade al menos un ráster.".into());
        }
        let vars = hipso_core::rastercalc::variables(&self.calc_names());
        hipso_core::rastercalc::Calc::compile(&self.calc_expr, &vars)
            .err()
            .map(|e| e.to_string())
    }

    /// The rules typed, with what to do with the other cells.
    fn reclass_table(&self) -> Result<hipso_core::reclass::Table, String> {
        use hipso_core::reclass::{Out, Table};
        let mut t = Table::parse(&self.rc_rules).map_err(|e| e.to_string())?;
        if !t.explicit_else {
            t.otherwise = match self.rc_else {
                0 => Out::Keep,
                1 => Out::NoData,
                _ => Out::Value(
                    self.rc_else_value
                        .trim()
                        .replace(',', ".")
                        .parse::<f32>()
                        .map_err(|_| "el valor para los demás debe ser un número".to_string())?,
                ),
            };
        }
        Ok(t)
    }

    fn max_distance(&self) -> Result<Option<f64>, String> {
        let t = self.max_distance.trim();
        if t.is_empty() {
            return Ok(None);
        }
        t.replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|d| *d > 0.0)
            .map(Some)
            .ok_or_else(|| "la distancia máxima debe ser un número positivo".to_string())
    }
}

impl App {
    /// The windows of the raster operations: each one opens its own.
    pub fn rasterops_windows(&mut self, ctx: &egui::Context) {
        for i in 0..self.ro.len() {
            self.rasterops_window(ctx, i);
        }
    }

    fn rasterops_window(&mut self, ctx: &egui::Context, i: usize) {
        if !self.ro[i].open {
            return;
        }
        let loaded: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| matches!(r.kind, RefKind::Raster { .. }))
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let mask_layers: Vec<(String, PathBuf)> = self
            .refs
            .iter()
            .filter(|r| match &r.kind {
                RefKind::Raster { .. } => true,
                RefKind::Vector(v) => matches!(v.kind, hipso_core::view::GeomKind::Polygon),
                RefKind::Image(_) => false,
            })
            .map(|r| (r.name.clone(), r.path.clone()))
            .collect();
        let weak = self.tokens().weak;
        let red = egui::Color32::from_rgb(200, 60, 40);
        let mut open = true;
        let mut go = false;
        egui::Window::new(self.ro[i].op.label())
            .default_pos([(ctx.content_rect().right() - 560.0).max(320.0), 190.0])
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 90.0).max(240.0))
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                crate::tools::output_row(ui, &mut self.tools.out_dir);
                ui.separator();
                let t = &mut self.ro[i];
                let help = match t.op {
                    Op::Sieve => "Quita los parches pequeños de un ráster de clases: las celdas conexas de un mismo valor que son menos que el umbral toman el valor del parche vecino más grande. Las celdas vacías no se tocan.",
                    Op::Proximity => "Calcula para cada celda la distancia a la celda más cercana de unos valores (distancia euclídea exacta). Las celdas objetivo valen 0.",
                    Op::Calc => "Calcula una expresión celda a celda sobre uno o varios rásteres que se llaman A, B, C… en el orden de la lista (una banda concreta, A.2). Admite operaciones, comparaciones, funciones y condiciones: if(A > 0, A - B, 0). Los rásteres deben tener las mismas celdas, o se ponen sobre las del primero (también si están en otro sistema de coordenadas: se reproyectan). La memoria sigue una franja del ráster, no el ráster entero.",
                    Op::Vectorize => "Convierte un ráster en polígonos: las celdas contiguas con la misma clase, con valores en un rango o entre dos cortes forman un polígono, con el valor, el área y las celdas como atributos. Los bordes siguen las celdas, o se simplifican si lo pides. Sirve, por ejemplo, para pasar a polígonos una clasificación o una máscara de agua.",
                    Op::Reclass => "Cambia los valores de un ráster según una tabla de reglas: un valor, un intervalo (0..10) o una comparación (< 0, >= 100) pasan a un valor nuevo o a «sin datos». Si varias reglas valen para una celda, manda la primera. Sirve, por ejemplo, para pasar un modelo de alturas o una pendiente a clases, o para unir clases. El resultado usa el tipo entero más pequeño que cabe.",
                    Op::Clip => "Deja vacío lo que cae fuera de una máscara y, si quieres, corta el ráster a la caja de la máscara. La máscara puede ser una capa de polígonos o otro ráster (se conservan las celdas donde tiene datos distintos de cero). Las celdas son las del ráster original: no se remuestrea nada.",
                    Op::Fill => "Rellena las celdas vacías con las celdas con valor de alrededor, ponderadas por el inverso del cuadrado de la distancia en ocho direcciones. Lo que no tiene ninguna a su alcance sigue vacío.",
                };
                ui.label(egui::RichText::new(help).color(weak).size(11.0));
                if t.op != Op::Calc {
                ui.horizontal(|ui| {
                    ui.label("Ráster");
                    ui.add(
                        egui::TextEdit::singleline(&mut t.input)
                            .hint_text("archivo GeoTIFF")
                            .desired_width(300.0),
                    );
                    if ui.button("Archivo…").clicked()
                        && let Some(p) = crate::prefs::dialog()
                            .add_filter("Ráster", &["tif", "tiff"])
                            .pick_file()
                    {
                        t.input = p.display().to_string();
                    }
                    ui.menu_button("De las capas cargadas", |ui| {
                        for (name, path) in &loaded {
                            if ui.button(name).clicked() {
                                t.input = path.display().to_string();
                                ui.close();
                            }
                        }
                        if loaded.is_empty() {
                            ui.label("No hay rásteres cargados");
                        }
                    });
                });
                }
                ui.separator();
                let mut problem: Option<String> = None;
                match t.op {
                    Op::Sieve => {
                        crate::tools::num(ui, "Umbral (celdas)", &mut t.threshold, 2.0..=100000.0, 1.0);
                        ui.checkbox(&mut t.eight, "Las celdas se tocan también por las esquinas (8 vecinas)");
                    }
                    Op::Proximity => {
                        ui.horizontal(|ui| {
                            ui.label("Valores objetivo");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.values)
                                    .hint_text("2, 5 (vacío: toda celda distinta de 0)")
                                    .desired_width(200.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Distancia máxima");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.max_distance)
                                    .hint_text("sin límite")
                                    .desired_width(80.0),
                            );
                        });
                        ui.checkbox(&mut t.in_cells, "Distancias en celdas (si no, en unidades del mapa)");
                        if let Err(m) = t.targets() {
                            problem = Some(m);
                        } else if let Err(m) = t.max_distance() {
                            problem = Some(m);
                        }
                    }
                    Op::Fill => {
                        crate::tools::num(ui, "Alcance (celdas)", &mut t.fill_distance, 1.0..=2000.0, 1.0);
                        crate::tools::num(ui, "Pasadas de suavizado", &mut t.smoothing, 0.0..=20.0, 1.0);
                    }
                    Op::Calc => {
                        ui.horizontal(|ui| {
                            ui.label("Rásteres");
                            if ui.button("Archivo…").clicked()
                                && let Some(v) = crate::prefs::dialog()
                                    .add_filter("Ráster", &["tif", "tiff"])
                                    .pick_files()
                            {
                                for p in v {
                                    if !t.calc_inputs.iter().any(|i| i.0 == p) {
                                        let b = hipso_core::pyramid::Pyramid::open(&p).map_or(1, |y| y.bands);
                                        t.calc_inputs.push((p, b));
                                    }
                                }
                            }
                            ui.menu_button("De las capas cargadas", |ui| {
                                for (name, path) in &loaded {
                                    if ui.button(name).clicked() {
                                        if !t.calc_inputs.iter().any(|i| &i.0 == path) {
                                            let b = hipso_core::pyramid::Pyramid::open(path).map_or(1, |y| y.bands);
                                            t.calc_inputs.push((path.clone(), b));
                                        }
                                        ui.close();
                                    }
                                }
                                if loaded.is_empty() {
                                    ui.label("No hay rásteres cargados");
                                }
                            });
                            if ui.button("Vaciar lista").clicked() {
                                t.calc_inputs.clear();
                            }
                        });
                        let names = t.calc_names();
                        let mut remove = None;
                        for (k, (n, bands)) in names.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("🗑").on_hover_text("Quitar de la lista").clicked() {
                                    remove = Some(k);
                                }
                                ui.label(egui::RichText::new(n).strong().monospace());
                                ui.label(paths::stem(&t.calc_inputs[k].0));
                                if *bands > 1 {
                                    ui.label(egui::RichText::new(format!("{bands} bandas: {n}.1 a {n}.{bands}")).color(weak).size(11.0));
                                }
                            });
                        }
                        if let Some(k) = remove {
                            t.calc_inputs.remove(k);
                        }
                        ui.horizontal(|ui| {
                            ui.label("Expresión");
                            ui.menu_button("Ejemplos", |ui| {
                                for (label, text) in [
                                    ("Diferencia: A − B", "A - B"),
                                    ("Máscara: 1 donde A supera 2", "if(A > 2, 1, 0)"),
                                    ("Quitar valores negativos", "if(A > 0, A, 0)"),
                                    ("Reescalar a 0-255", "clamp((A - 0) / (30 - 0) * 255, 0, 255)"),
                                    ("Rellenar vacíos de A con B", "if(isnan(A), B, A)"),
                                    ("NDVI de una imagen de 4 bandas", "(B4 - B3) / (B4 + B3)"),
                                ] {
                                    if ui.button(label).clicked() {
                                        t.calc_expr = text.to_string();
                                        ui.close();
                                    }
                                }
                            });
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut t.calc_expr)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("if(A > 0, A - B, 0)")
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                        ui.horizontal(|ui| {
                            ui.label("Resultado");
                            ui.radio_value(&mut t.calc_dtype, 0, "Decimales");
                            ui.radio_value(&mut t.calc_dtype, 1, "Enteros 0-255");
                            ui.radio_value(&mut t.calc_dtype, 2, "Enteros 0-65535");
                        });
                        ui.checkbox(&mut t.calc_nearest, "Es un ráster de clases (las vistas reducidas no promedian)");
                        ui.checkbox(&mut t.calc_align, "Si las celdas de los rásteres difieren, poner los demás sobre las del primero");
                        ui.label(egui::RichText::new("Funciones: abs sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan. Comparaciones, and, or, not, %, ^. Una celda vacía da vacío, salvo dentro de isnan().").color(weak).size(11.0));
                        problem = t.calc_problem();
                    }
                    Op::Reclass => {
                        ui.horizontal(|ui| {
                            ui.label("Reglas");
                            ui.menu_button("Ejemplos", |ui| {
                                for (label, text) in [
                                    ("Unir clases: 3 y 4 pasan a 3", "3 = 3\n4 = 3"),
                                    ("Intervalos a clases", "0..10 = 1\n10..25 = 2\n25..100 = 3"),
                                    ("Valores negativos a «sin datos»", "< 0 = nodata"),
                                    ("Máscara 1 / 0", ">= 5 = 1\nelse = 0"),
                                    ("Rellenar vacíos con 0", "nodata = 0"),
                                ] {
                                    if ui.button(label).clicked() {
                                        t.rc_rules = text.to_string();
                                        ui.close();
                                    }
                                }
                            });
                            if ui
                                .button("Una regla por clase")
                                .on_hover_text("Rellena una regla «valor = valor» por cada clase que tiene el ráster, para cambiar solo las que quieras")
                                .clicked()
                                && let Ok(v) = hipso_core::reclass::values_of(Path::new(t.input.trim()), t.rc_band as usize, 200)
                            {
                                t.rc_rules = v
                                    .iter()
                                    .map(|(c, _)| format!("{} = {}", paths::num(f64::from(*c)), paths::num(f64::from(*c))))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                            }
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut t.rc_rules)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("0..10 = 1\n10..25 = 2\n< 0 = nodata")
                                .desired_rows(5)
                                .desired_width(f32::INFINITY),
                        );
                        ui.label(egui::RichText::new("Una regla por línea: «valor = nuevo», «a..b = nuevo» (ambos incluidos), «< b», «<= b», «> a», «>= a» o «nodata», y como resultado un número o «nodata». «else = …» da el valor de las demás celdas.").color(weak).size(11.0));
                        ui.horizontal(|ui| {
                            ui.label("Las celdas que ninguna regla recoge");
                            ui.radio_value(&mut t.rc_else, 0, "Conservan su valor");
                            ui.radio_value(&mut t.rc_else, 1, "Quedan vacías");
                            ui.radio_value(&mut t.rc_else, 2, "Pasan a");
                            if t.rc_else == 2 {
                                ui.add(egui::TextEdit::singleline(&mut t.rc_else_value).desired_width(60.0));
                            }
                        });
                        crate::tools::num(ui, "Banda", &mut t.rc_band, 1.0..=64.0, 1.0);
                        if let Err(m) = t.reclass_table() {
                            problem = Some(m);
                        }
                    }
                    Op::Clip => {
                        ui.horizontal(|ui| {
                            ui.label("Máscara");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.clip_mask)
                                    .hint_text("capa de polígonos o ráster")
                                    .desired_width(280.0),
                            );
                            if ui.button("Archivo…").clicked()
                                && let Some(p) = crate::prefs::dialog()
                                    .add_filter("Polígonos o ráster", &["shp", "gpkg", "geojson", "json", "kml", "kmz", "tif", "tiff", "asc"])
                                    .pick_file()
                            {
                                t.clip_mask = p.display().to_string();
                            }
                            ui.menu_button("De las capas cargadas", |ui| {
                                let mut any = false;
                                for (name, path) in &mask_layers {
                                    any = true;
                                    if ui.button(name).clicked() {
                                        t.clip_mask = path.display().to_string();
                                        ui.close();
                                    }
                                }
                                if !any {
                                    ui.label("No hay capas de polígonos ni rásteres cargados");
                                }
                            });
                        });
                        ui.checkbox(&mut t.clip_crop, "Cortar el ráster a la caja de la máscara (si no, conserva su tamaño)");
                        ui.checkbox(&mut t.clip_invert, "Conservar lo que queda fuera de la máscara (y vaciar lo de dentro)");
                        ui.horizontal(|ui| {
                            ui.label("Valor de las celdas vacías");
                            ui.add(
                                egui::TextEdit::singleline(&mut t.clip_nodata)
                                    .hint_text("el del archivo, o 0 / -9999")
                                    .desired_width(110.0),
                            );
                        });
                        if t.clip_mask.trim().is_empty() {
                            problem = Some("Elige la máscara.".into());
                        } else if !Path::new(t.clip_mask.trim()).is_file() {
                            problem = Some("El archivo de la máscara no existe.".into());
                        }
                    }
                    Op::Vectorize => {
                        ui.horizontal(|ui| {
                            ui.label("Agrupar");
                            ui.radio_value(&mut t.vec_mode, 0, "Por clase (mismo valor)");
                            ui.radio_value(&mut t.vec_mode, 1, "Rango de valores");
                            ui.radio_value(&mut t.vec_mode, 2, "Intervalos entre cortes");
                        });
                        match t.vec_mode {
                            1 => {
                                ui.horizontal(|ui| {
                                    ui.label("Desde");
                                    ui.add(egui::TextEdit::singleline(&mut t.vec_lo).desired_width(70.0));
                                    ui.label("hasta");
                                    ui.add(egui::TextEdit::singleline(&mut t.vec_hi).desired_width(70.0));
                                });
                            }
                            2 => {
                                ui.horizontal(|ui| {
                                    ui.label("Cortes");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut t.vec_breaks)
                                            .hint_text("0; 5; 10")
                                            .desired_width(200.0),
                                    );
                                });
                            }
                            _ => {}
                        }
                        crate::tools::num(ui, "Celdas mínimas por polígono", &mut t.min_cells, 1.0..=1.0e6, 1.0);
                        crate::tools::num(ui, "Simplificar (unidades del mapa, 0: sigue las celdas)", &mut t.simplify, 0.0..=1000.0, 0.1);
                        ui.checkbox(&mut t.eight, "Las celdas se tocan también por las esquinas (8 vecinas)");
                        ui.checkbox(&mut t.gpkg, "Guardar como GeoPackage (si no, shapefile)");
                        if let Err(m) = t.vec_mode() {
                            problem = Some(m);
                        }
                    }
                }
                if t.op == Op::Calc {
                } else if t.input.trim().is_empty() {
                    problem = Some("Elige un ráster.".into());
                } else if !Path::new(t.input.trim()).is_file() {
                    problem = Some("El archivo no existe.".into());
                }
                if let Some(m) = &problem {
                    ui.label(egui::RichText::new(m).color(red).size(11.0));
                }
                if ui
                    .add_enabled(
                        problem.is_none() && self.running.is_none(),
                        egui::Button::new("▶ Ejecutar"),
                    )
                    .clicked()
                {
                    go = true;
                }
            });
        if go {
            let t = &self.ro[i];
            if t.op == Op::Calc {
                let paths_in: Vec<PathBuf> = t.calc_inputs.iter().map(|i| i.0.clone()).collect();
                let Some(first) = paths_in.first().cloned() else {
                    return;
                };
                let dst = paths::unique(&paths::product(&first, "calc", "tif"));
                let mut job = hipso_core::rastercalc::CalcJob::new(
                    t.calc_expr.trim().to_string(),
                    hipso_core::rastercalc::letters(&paths_in),
                    dst.clone(),
                );
                job.dtype = match t.calc_dtype {
                    1 => hipso_core::rastertools::Dtype::U8,
                    2 => hipso_core::rastertools::Dtype::U16,
                    _ => hipso_core::rastertools::Dtype::F32,
                };
                job.nearest = t.calc_nearest;
                job.align = t.calc_align.then_some(hipso_core::rastertools::Resample::Bilinear);
                let task: tools::Task = Box::new(move || hipso_core::rastercalc::run(&job));
                self.start(
                    ctx,
                    Launch {
                        label: "Calculadora".to_string(),
                        task,
                        show: vec![dst.clone()],
                        outputs: vec![dst],
                    },
                );
                if !open {
                    self.ro[i].open = false;
                }
                return;
            }
            let src = PathBuf::from(t.input.trim());
            if t.op == Op::Reclass {
                let Ok(table) = t.reclass_table() else {
                    return;
                };
                let dst = paths::unique(&paths::product(&src, "reclas", "tif"));
                let mut job = hipso_core::reclass::ReclassJob::new(src, dst.clone(), table);
                job.band = t.rc_band.round().max(1.0) as usize;
                let task: tools::Task = Box::new(move || hipso_core::reclass::run(&job));
                self.start(
                    ctx,
                    Launch {
                        label: "Reclasificar".to_string(),
                        task,
                        show: vec![dst.clone()],
                        outputs: vec![dst],
                    },
                );
                if !open {
                    self.ro[i].open = false;
                }
                return;
            }
            if t.op == Op::Clip {
                use hipso_core::rastermask::{Mask, MaskJob, clip, regions_of};
                let layer = PathBuf::from(t.clip_mask.trim());
                let (crop, invert) = (t.clip_crop, t.clip_invert);
                let nodata = t.clip_nodata.trim().replace(',', ".").parse::<f32>().ok();
                let dst = paths::unique(&paths::product(&src, "recorte", "tif"));
                let out = dst.clone();
                let task: tools::Task = Box::new(move || {
                    let ext = layer
                        .extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    let mask = if hipso_core::raster::is_raster_ext(&ext) {
                        Mask::Raster(layer)
                    } else {
                        Mask::Regions(regions_of(&layer, hipso_core::crs::epsg_of(&src))?)
                    };
                    let mut job = MaskJob::new(src, dst, mask);
                    job.crop = crop;
                    job.invert = invert;
                    job.nodata = nodata;
                    clip(&job)
                });
                self.start(
                    ctx,
                    Launch {
                        label: "Cortar por máscara".to_string(),
                        task,
                        show: vec![out.clone()],
                        outputs: vec![out],
                    },
                );
                if !open {
                    self.ro[i].open = false;
                }
                return;
            }
            let (op, threshold, eight) = (t.op, t.threshold.round().max(1.0) as usize, t.eight);
            let (targets, maxd, cells) = (t.targets(), t.max_distance(), t.in_cells);
            let (reach, smooth) = (t.fill_distance.round().max(1.0) as usize, t.smoothing.round() as usize);
            if op == Op::Vectorize {
                let mode = match t.vec_mode() {
                    Ok(m) => m,
                    Err(_) => return,
                };
                let ext = if t.gpkg { "gpkg" } else { "shp" };
                let dst = paths::unique(&paths::product(&src, "poligonos", ext));
                let mut job = hipso_core::rastervec::VecJob::new(src, dst.clone());
                job.mode = mode;
                job.min_cells = t.min_cells.round().max(1.0) as u32;
                job.simplify = t.simplify;
                job.diagonal = t.eight;
                let task: tools::Task = Box::new(move || hipso_core::rastervec::run(&job));
                self.start(
                    ctx,
                    Launch {
                        label: "Vectorizar".to_string(),
                        task,
                        show: vec![dst.clone()],
                        outputs: vec![dst],
                    },
                );
                if !open {
                    self.ro[i].open = false;
                }
                return;
            }
            let (tag, label) = match op {
                Op::Sieve => ("sieve", "Tamizar"),
                Op::Proximity => ("proximity", "Proximidad"),
                Op::Fill => ("relleno", "Rellenar huecos"),
                Op::Vectorize | Op::Calc | Op::Reclass | Op::Clip => return,
            };
            let dst = paths::unique(&paths::product(&src, tag, "tif"));
            let out = dst.clone();
            let task: tools::Task = Box::new(move || {
                let g = Grid::read(&src)?;
                let msg;
                let r = match op {
                    Op::Sieve => {
                        let conn = if eight {
                            rasterops::Connectivity::Eight
                        } else {
                            rasterops::Connectivity::Four
                        };
                        let r = rasterops::sieve(&g, threshold, conn);
                        let changed = g
                            .data
                            .iter()
                            .zip(&r.data)
                            .filter(|(a, b)| a.to_bits() != b.to_bits())
                            .count();
                        msg = format!("{changed} celdas cambiadas");
                        r
                    }
                    Op::Proximity => {
                        let targets = targets.map_err(hipso_core::Error::Param)?;
                        let maxd = maxd.map_err(hipso_core::Error::Param)?;
                        msg = "distancias calculadas".to_string();
                        rasterops::proximity(&g, &targets, maxd, cells)?
                    }
                    Op::Fill => {
                        let r = rasterops::fill_nodata(&g, reach, smooth);
                        let empty = |g: &Grid| g.data.iter().filter(|v| v.is_nan()).count();
                        let (before, after) = (empty(&g), empty(&r));
                        msg = format!("{} celdas rellenadas, {after} siguen vacías", before - after);
                        r
                    }
                    Op::Vectorize | Op::Calc | Op::Reclass | Op::Clip => {
                        return Err(hipso_core::Error::Param("esta operación no escribe así su resultado".into()));
                    }
                };
                paths::ensure_parent(&dst)?;
                r.write_tiff(&dst, Grid::read_epsg(&src))?;
                Ok(format!("{msg} -> {}", dst.display()))
            });
            self.start(
                ctx,
                Launch {
                    label: label.to_string(),
                    task,
                    show: vec![out.clone()],
                    outputs: vec![out],
                },
            );
        }
        if !open {
            self.ro[i].open = false;
        }
    }
}
