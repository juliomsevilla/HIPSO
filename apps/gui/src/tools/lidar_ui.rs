//! The point-cloud tools beyond the forestry chain: thinning, duplicates, comparison of two
//! clouds, alignment, swath separation, colouring from an image, intensity normalised for range,
//! COPC and the extra classifiers.
//! Each loaded cloud is a sheet and gets its own result, like the other tools.

use std::collections::HashMap;

use super::*;
use rustlidar_core::cloudtools::{self, DupFields};
use rustlidar_core::hag::{self, HagParams};
use rustlidar_core::thin::{self, ThinMethod, ThinParams};
use rustlidar_core::tls::{self, TlsJob, TlsParams};
use rustlidar_core::volume::{self, ReferenceSource, VolumeJob};

pub struct State {
    pub thin_method: usize,
    pub thin_cell: f64,
    pub thin_vcell: f64,
    pub thin_pct: f64,
    pub thin_n: f64,
    pub thin_fraction: f64,
    pub thin_density: f64,
    pub thin_seed: f64,
    pub tls: TlsParams,
    pub tls_clouds: bool,
    pub tls_stems: bool,
    pub dup: DupFields,
    pub dup_all: bool,
    pub hag: HagParams,
    pub cmp_ref: String,
    pub cmp_dist: f64,
    pub cmp_neigh: f64,
    pub fit_ref: String,
    pub fit_thin: f64,
    pub fit_xy: f64,
    pub fit_z: f64,
    pub fit_iter: f64,
    pub fit_stop: f64,
    pub fit_shift: bool,
    pub sw_cell: f64,
    pub sw_returns: usize,
    pub sw_by_cloud: bool,
    pub vol_source: usize,
    /// Rasters chosen by hand (empty: the one found for each cloud).
    pub vol_surface: String,
    pub vol_dtm: String,
    pub vol_level: f64,
    pub vol_min: f64,
    /// 0 the whole raster, 1 the drawn AOIs, 2 the regions of a vector file.
    pub vol_regions: usize,
    pub vol_file: String,
    /// The image the points take their colour from, its bands for red, green and blue, and
    /// whether points outside it keep their colour.
    pub col_image: String,
    pub col_bands: [f64; 3],
    pub col_keep: bool,
    /// Intensity: 0 trajectory, 1 flight altitude; the trajectory file, the altitude, the
    /// exponent and the reference range (0: the mean range).
    pub int_mode: usize,
    pub int_traj: String,
    pub int_alt: f64,
    pub int_exp: f64,
    pub int_ref: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            thin_method: 0,
            thin_cell: 1.0,
            thin_vcell: 1.0,
            thin_pct: 50.0,
            thin_n: 2.0,
            thin_fraction: 0.5,
            thin_density: 1.0,
            thin_seed: 1.0,
            tls: TlsParams::default(),
            tls_clouds: false,
            tls_stems: false,
            dup: DupFields::default(),
            dup_all: false,
            hag: HagParams::default(),
            cmp_ref: String::new(),
            cmp_dist: 0.5,
            cmp_neigh: 3.0,
            fit_ref: String::new(),
            fit_thin: 1.0,
            fit_xy: 5.0,
            fit_z: 2.0,
            fit_iter: 30.0,
            fit_stop: 0.01,
            fit_shift: false,
            sw_cell: 1.0,
            sw_returns: 0,
            sw_by_cloud: false,
            vol_source: 0,
            vol_surface: String::new(),
            vol_dtm: String::new(),
            vol_level: 0.0,
            vol_min: 0.0,
            vol_regions: 0,
            vol_file: String::new(),
            col_image: String::new(),
            col_bands: [1.0, 2.0, 3.0],
            col_keep: false,
            int_mode: 0,
            int_traj: String::new(),
            int_alt: 3000.0,
            int_exp: 2.0,
            int_ref: 0.0,
        }
    }
}

const THIN_METHODS: [&str; 9] = [
    "Densidad objetivo (puntos/m², al azar)",
    "Un punto por celda: el más cercano al centro",
    "Un punto por celda: el más bajo",
    "Un punto por celda: el más alto",
    "Un punto por celda: percentil de la elevación (50 = mediana)",
    "Celdas 3D (conserva paredes y pendientes)",
    "Conservar un punto de cada n",
    "Eliminar un punto de cada n",
    "Muestreo al azar (fracción)",
];

impl Tools {
    fn lx_ext(first: &Path) -> String {
        first
            .extension()
            .map_or("las".to_string(), |e| e.to_string_lossy().into_owned())
    }

    pub fn thin_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf]) {
        ui.label(egui::RichText::new("Reduce la densidad de la nube sin cambiar su extensión. Sirve para llevar a la misma densidad nubes de coberturas distintas (por ejemplo la primera y la tercera del PNOA) antes de compararlas. Se escribe una copia nueva por hoja (…_delgada_…); la original no cambia.").color(t.weak).size(11.0));
        ui.label(format!(
            "{} hoja(s): cada una se adelgaza por separado",
            inputs.len()
        ));
        for p in inputs {
            if let Ok(r) = rustlidar_core::las::Reader::open(p) {
                let b = r.header.bounds;
                let area = (b[3] - b[0]).max(0.0) * (b[4] - b[1]).max(0.0);
                if area > 0.0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "• {}: {} puntos, {:.2} puntos/m² (sobre el rectángulo envolvente)",
                            paths::stem(p),
                            r.header.count,
                            r.header.count as f64 / area
                        ))
                        .size(11.0),
                    );
                }
            }
        }
        let s = &mut self.lx;
        egui::ComboBox::from_id_salt("thin_method")
            .width(380.0)
            .selected_text(THIN_METHODS[s.thin_method])
            .show_ui(ui, |ui| {
                for (k, n) in THIN_METHODS.iter().enumerate() {
                    ui.selectable_value(&mut s.thin_method, k, *n);
                }
            });
        match s.thin_method {
            0 => {
                num(
                    ui,
                    "Puntos por m² que quedan",
                    &mut s.thin_density,
                    0.01..=10000.0,
                    0.1,
                );
                num(ui, "Semilla", &mut s.thin_seed, 0.0..=1e9, 1.0);
                ui.label(
                    egui::RichText::new("Una nube que ya es menos densa se deja como está.")
                        .color(t.weak)
                        .size(11.0),
                );
            }
            1..=3 => num(ui, "Celda (m)", &mut s.thin_cell, 0.05..=1000.0, 0.1),
            4 => {
                num(ui, "Celda (m)", &mut s.thin_cell, 0.05..=1000.0, 0.1);
                num(ui, "Percentil", &mut s.thin_pct, 0.0..=100.0, 1.0);
            }
            5 => {
                num(
                    ui,
                    "Celda horizontal (m)",
                    &mut s.thin_cell,
                    0.05..=1000.0,
                    0.1,
                );
                num(
                    ui,
                    "Celda vertical (m)",
                    &mut s.thin_vcell,
                    0.05..=1000.0,
                    0.1,
                );
            }
            6 | 7 => num(ui, "n", &mut s.thin_n, 2.0..=1e6, 1.0),
            _ => {
                num(
                    ui,
                    "Fracción que se conserva (0–1)",
                    &mut s.thin_fraction,
                    0.001..=1.0,
                    0.01,
                );
                num(ui, "Semilla", &mut s.thin_seed, 0.0..=1e9, 1.0);
            }
        }
    }

    pub fn tls_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let weak = |s: &str| egui::RichText::new(s.to_string()).color(t.weak).size(11.0);
        ui.label(weak("Mide el diámetro del tronco a lo largo de todo el fuste en nubes de escáner terrestre, móvil o fotogramétrico, y cubica cada árbol con la fórmula de Smalian: V = (A₁ + A₂)/2 · L por cada troza, desde la altura de la base hasta donde el diámetro baja al diámetro de punta delgada. Cada nube es una hoja y da su propio resultado."));
        let s = &mut self.lx.tls;
        ui.horizontal(|ui| {
            ui.label("Equipo:");
            if ui
                .selectable_label(s.voxel < 0.04, "Terrestre estático (vóxel 3 cm)")
                .on_hover_text("Escáner sobre trípode: nubes densas y de poco ruido")
                .clicked()
            {
                s.voxel = 0.03;
            }
            if ui
                .selectable_label(s.voxel >= 0.04, "Móvil o SLAM (vóxel 5 cm)")
                .on_hover_text("Escáner de mano o de mochila (GeoSLAM…): el ruido es de varios centímetros y con vóxeles más finos los troncos se pierden a pocos metros")
                .clicked()
            {
                s.voxel = 0.05;
            }
        });
        ui.label(egui::RichText::new("Cubicación").strong());
        let (mut base_cm, mut top_cm) = (s.base_height * 100.0, s.top_diam * 100.0);
        num(
            ui,
            "Altura de la base sobre el suelo (cm)",
            &mut base_cm,
            0.0..=300.0,
            1.0,
        );
        num(
            ui,
            "Diámetro en punta delgada (cm)",
            &mut top_cm,
            1.0..=100.0,
            0.5,
        );
        num(
            ui,
            "Longitud de la troza (m): distancia entre diámetros",
            &mut s.log_len,
            0.1..=10.0,
            0.05,
        );
        (s.base_height, s.top_diam) = (base_cm / 100.0, top_cm / 100.0);
        ui.separator();
        ui.checkbox(
            &mut s.normalized,
            "La nube ya tiene alturas sobre el suelo (no hay que calcular el terreno)",
        );
        egui::CollapsingHeader::new("Búsqueda de troncos").show(ui, |ui| {
            num(
                ui,
                "Franja: desde (m sobre el suelo)",
                &mut s.stripe.0,
                0.0..=10.0,
                0.1,
            );
            num(ui, "hasta (m)", &mut s.stripe.1, 0.5..=20.0, 0.1);
            num(
                ui,
                "Hueco máximo sin medir (m): tras él se da el tronco por perdido",
                &mut s.max_gap,
                0.25..=5.0,
                0.25,
            );
            let mut share = s.lost_top_share * 100.0;
            num(
                ui,
                "Altura comercial esperada (% de la total) de los fustes perdidos estando gruesos; 0: no estimar",
                &mut share,
                0.0..=95.0,
                1.0,
            );
            s.lost_top_share = share / 100.0;
            let mut v = s.voxel * 100.0;
            num(
                ui,
                "Vóxel (cm): la nube se reduce a un punto por vóxel",
                &mut v,
                0.5..=10.0,
                0.1,
            );
            s.voxel = v / 100.0;
            num(
                ui,
                "Diámetro máximo esperado (m)",
                &mut s.max_diam,
                0.2..=3.0,
                0.05,
            );
            let mut peel = s.peel as f64;
            num(ui, "Pasadas de poda de ramas", &mut peel, 1.0..=5.0, 1.0);
            s.peel = peel as usize;
            num(
                ui,
                "Verticalidad mínima (0–1)",
                &mut s.vert_min,
                0.3..=0.99,
                0.01,
            );
            let mut np = s.min_stem_points as f64;
            num(
                ui,
                "Vóxeles mínimos de un tronco en la franja",
                &mut np,
                20.0..=100000.0,
                10.0,
            );
            s.min_stem_points = np as usize;
        });
        egui::CollapsingHeader::new("Secciones y suelo").show(ui, |ui| {
            let mut sl = s.slice * 100.0;
            num(
                ui,
                "Semigrosor de la sección (cm)",
                &mut sl,
                1.0..=20.0,
                0.5,
            );
            s.slice = sl / 100.0;
            let mut a = [s.min_section_points as f64, s.min_sectors as f64];
            num(
                ui,
                "Puntos mínimos por sección",
                &mut a[0],
                8.0..=2000.0,
                1.0,
            );
            num(ui, "Sectores ocupados (de 16)", &mut a[1], 3.0..=16.0, 1.0);
            (s.min_section_points, s.min_sectors) = (a[0] as usize, a[1] as usize);
            num(
                ui,
                "Malla del terreno simulado (m)",
                &mut s.cloth,
                0.2..=5.0,
                0.05,
            );
            num(
                ui,
                "Altura máxima medida (m)",
                &mut s.max_height,
                5.0..=80.0,
                1.0,
            );
        });
        egui::CollapsingHeader::new("Cepas y bifurcaciones (rebrotes)").show(ui, |ui| {
            ui.label(weak("Varios troncos de un mismo pie. Si están separados a la altura del diámetro, son brotes de una cepa y cada uno es un árbol (columnas CEPA y BROTES). Si son un solo tronco a esa altura y se bifurcan por encima, es un árbol bifurcado: se mide el diámetro del tronco y se cubica el tronco y cada rama (H_BIF, RAMA)."));
            num(
                ui,
                "Altura del diámetro (m): separa brotes de bifurcación",
                &mut s.fork_height,
                0.5..=3.0,
                0.05,
            );
            num(
                ui,
                "Distancia entre las bases de una cepa (m)",
                &mut s.stool_dist,
                0.0..=3.0,
                0.05,
            );
        });
        ui.separator();
        ui.checkbox(
            &mut self.lx.tls_stems,
            "Guardar la nube con fustes (clase 64) y ramas (clase 65)",
        )
        .on_hover_text("Para editar: los puntos dentro del círculo medido del tronco pasan a la clase 64 y el resto de los que están junto a un tronco a la 65. Se usa la distancia máxima al eje de abajo.");
        ui.checkbox(
            &mut self.lx.tls_clouds,
            "Guardar también la nube de puntos de cada árbol",
        );
        if self.lx.tls_clouds || self.lx.tls_stems {
            num(
                ui,
                "Distancia máxima al eje (m)",
                &mut self.lx.tls.assign_dist,
                1.0..=50.0,
                0.5,
            );
        }
        self.formats_row(ui);
        ui.label(weak("Resultado: puntos en la base de cada tronco con DAP, alturas, volumen, nº de trozas, inclinación y calidad (.shp y/o .csv) y <nombre>_trozas.csv con el detalle de cada troza."));
    }

    fn reference_row(ui: &mut egui::Ui, text: &mut String) {
        ui.horizontal(|ui| {
            ui.label("Nube de referencia");
            ui.add(egui::TextEdit::singleline(text).desired_width(330.0));
            if ui.button("…").clicked()
                && let Some(p) = pick_file("Nube", &["las", "laz"])
            {
                *text = p.display().to_string();
            }
        });
    }

    pub fn dups_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Quita los puntos que están en el mismo sitio (x, y, z iguales). Marca los atributos que también deben coincidir para que dos puntos cuenten como repetidos. Se escribe una copia nueva (…_sin_duplicados).").color(t.weak).size(11.0));
        ui.checkbox(&mut s.dup.class, "Misma clase");
        ui.checkbox(&mut s.dup.gps_time, "Mismo tiempo GPS");
        ui.checkbox(&mut s.dup.intensity, "Misma intensidad");
        ui.checkbox(&mut s.dup.return_number, "Mismo número de retorno");
        ui.checkbox(&mut s.dup.return_count, "Mismo número de retornos");
        ui.checkbox(&mut s.dup.source_id, "Mismo ID de fuente (pasada)");
        ui.checkbox(
            &mut s.dup_all,
            "Quitar todas las copias (si no, se deja un punto de cada grupo)",
        );
    }

    pub fn colorize_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Da a cada punto el color del píxel de una imagen del mismo sitio (una ortofoto PNOA, por ejemplo): rojo, verde y azul, y el infrarrojo si la nube es de formato 8 y la imagen tiene esa banda. La imagen debe estar en el mismo sistema de coordenadas que la nube. Las nubes sin color (formatos 0, 1 y 6) se guardan en formato 7. Se escribe una copia nueva (…_rgb).").color(t.weak).size(11.0));
        ui.horizontal(|ui| {
            ui.label("Imagen");
            ui.add(
                egui::TextEdit::singleline(&mut s.col_image)
                    .hint_text("ortofoto .tif o .jp2")
                    .desired_width(260.0),
            );
            if ui.button("…").clicked()
                && let Some(p) = pick_file("Imagen", rustlidar_core::raster::EXTENSIONS)
            {
                s.col_image = p.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Bandas: rojo");
            ui.add(
                crate::numfield::NumField::new(&mut s.col_bands[0])
                    .range(1.0..=64.0)
                    .speed(1.0),
            );
            ui.label("verde");
            ui.add(
                crate::numfield::NumField::new(&mut s.col_bands[1])
                    .range(1.0..=64.0)
                    .speed(1.0),
            );
            ui.label("azul");
            ui.add(
                crate::numfield::NumField::new(&mut s.col_bands[2])
                    .range(1.0..=64.0)
                    .speed(1.0),
            );
        });
        ui.checkbox(
            &mut s.col_keep,
            "Los puntos fuera de la imagen conservan su color (si no, quedan en negro)",
        );
    }

    pub fn intensity_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("La intensidad cae con el cuadrado de la distancia al escáner: la misma superficie sale más oscura en el borde de una pasada o desde una pasada más alta. Se corrige multiplicando por (R / R_ref)^a (Höfle y Pfeifer, 2007). R sale de la trayectoria del vuelo (lo más exacto) o, si no la tienes, de la altura de vuelo y el ángulo de escaneo. Se escribe una copia nueva (…_intensidad).").color(t.weak).size(11.0));
        ui.radio_value(
            &mut s.int_mode,
            0,
            "Trayectoria del vuelo (archivo de texto «tiempo x y z», con el tiempo GPS de la nube)",
        );
        if s.int_mode == 0 {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.add(
                    egui::TextEdit::singleline(&mut s.int_traj)
                        .hint_text("trayectoria .txt / .csv")
                        .desired_width(260.0),
                );
                if ui.button("…").clicked()
                    && let Some(p) = pick_file("Trayectoria", &["txt", "csv", "xyz", "traj"])
                {
                    s.int_traj = p.display().to_string();
                }
            });
        }
        ui.radio_value(
            &mut s.int_mode,
            1,
            "Altura de vuelo constante y ángulo de escaneo",
        );
        if s.int_mode == 1 {
            num(
                ui,
                "      Altitud del vuelo (m, mismo sistema de alturas que la nube)",
                &mut s.int_alt,
                1.0..=20_000.0,
                10.0,
            );
        }
        num(
            ui,
            "Exponente a (2 para suelo y copas, ~3 para cables)",
            &mut s.int_exp,
            0.5..=6.0,
            0.1,
        );
        num(
            ui,
            "Distancia de referencia R_ref (m; 0 = la media de la nube)",
            &mut s.int_ref,
            0.0..=20_000.0,
            10.0,
        );
    }

    pub fn copc_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let _ = &self.lx;
        ui.label(egui::RichText::new("Reescribe cada nube como COPC (Cloud Optimized Point Cloud): un LAZ normal, que lee cualquier programa, cuyos bloques son los nodos de un octree. Así un visor o un servidor (QGIS, visores web) lee solo la zona y el detalle que necesita, y se publica en internet sin servidor especial. Se conservan todos los atributos; los formatos 0 a 3 pasan a 6 o 7, como pide el formato. Se escribe una copia nueva (….copc.laz).").color(t.weak).size(11.0));
        ui.label(egui::RichText::new("La nube se guarda entera en memoria mientras se ordena (unos 40 bytes por punto): para nubes de cientos de millones de puntos, divídelas antes en teselas.").color(t.weak).size(11.0));
    }

    pub fn hag_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Calcula la altura de cada punto sobre el suelo a partir de los puntos de suelo que ya tiene la nube (clasifícala antes si no los tiene): una TIN de esos puntos da el suelo bajo cada punto. No hace falta un MDT aparte. Se escribe una copia nueva (…_alturas).").color(t.weak).size(11.0));
        ui.radio_value(
            &mut s.hag.replace_z,
            false,
            "Guardar la altura como atributo «HeightAboveGround» y conservar las cotas",
        );
        ui.radio_value(
            &mut s.hag.replace_z,
            true,
            "Poner la altura en lugar de la cota (nube normalizada)",
        );
        num(ui, "Celda del suelo (m)", &mut s.hag.cell, 0.1..=10.0, 0.1);
        let mut class = f64::from(s.hag.ground_class);
        num(ui, "Clase del suelo", &mut class, 0.0..=255.0, 1.0);
        s.hag.ground_class = class as u8;
    }

    pub fn compare_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Detección de cambios: escribe los puntos de la nube cargada que están a más de la distancia indicada de todos los puntos de la referencia (por ejemplo, la primera y la tercera cobertura del PNOA adelgazadas a la misma densidad). Da también la distancia media y la compleción.").color(t.weak).size(11.0));
        Self::reference_row(ui, &mut s.cmp_ref);
        num(
            ui,
            "Distancia mínima de cambio (m)",
            &mut s.cmp_dist,
            0.02..=100.0,
            0.05,
        );
        num(
            ui,
            "Vecinos cambiados que necesita un punto",
            &mut s.cmp_neigh,
            0.0..=1000.0,
            1.0,
        );
    }

    pub fn fit_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Mueve la nube cargada para que encaje con la de referencia (punto más cercano iterativo, ICP) y escribe una copia (…_ajustada). Usa los últimos retornos de ambas.").color(t.weak).size(11.0));
        Self::reference_row(ui, &mut s.fit_ref);
        num(
            ui,
            "Adelgazar para calcular (m)",
            &mut s.fit_thin,
            0.1..=50.0,
            0.1,
        );
        num(
            ui,
            "Distancia horizontal máxima (m)",
            &mut s.fit_xy,
            0.1..=500.0,
            0.5,
        );
        num(
            ui,
            "Distancia vertical máxima (m)",
            &mut s.fit_z,
            0.1..=500.0,
            0.5,
        );
        num(ui, "Iteraciones máximas", &mut s.fit_iter, 1.0..=200.0, 1.0);
        num(
            ui,
            "Parar con distancia media menor de (m)",
            &mut s.fit_stop,
            0.001..=5.0,
            0.005,
        );
        ui.checkbox(&mut s.fit_shift, "Solo desplazar (sin girar)");
    }

    pub fn swath_form(&mut self, ui: &mut egui::Ui, t: &Tokens) {
        let s = &mut self.lx;
        ui.label(egui::RichText::new("Diferencia de altura entre pasadas de vuelo: por celda, la mayor distancia entre las elevaciones medias de las pasadas (ID de fuente) que la cubren. Se guarda un ráster (…_pasadas) y se da el porcentaje de celdas que pasa de 8 y 16 cm.").color(t.weak).size(11.0));
        num(ui, "Celda (m)", &mut s.sw_cell, 0.1..=100.0, 0.1);
        ui.horizontal(|ui| {
            ui.label("Retornos");
            egui::ComboBox::from_id_salt("Retornos")
                .selected_text(["Últimos y únicos", "Todos", "Solo únicos"][s.sw_returns])
                .show_ui(ui, |ui| {
                    for (k, n) in ["Últimos y únicos", "Todos", "Solo únicos"]
                        .iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut s.sw_returns, k, *n);
                    }
                });
        });
        ui.checkbox(
            &mut s.sw_by_cloud,
            "Cada nube cargada es una pasada (en vez del ID de fuente)",
        );
    }

    /// A text box for a path with a file picker; empty means "the one found for each cloud".
    pub(super) fn raster_row(ui: &mut egui::Ui, label: &str, text: &mut String, exts: &[&str]) {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("vacío: el de cada nube")
                    .desired_width(300.0),
            );
            if ui.button("…").clicked()
                && let Some(p) = pick_file("Ráster", exts)
            {
                *text = p.display().to_string();
            }
        });
    }

    pub fn volume_form(&mut self, ui: &mut egui::Ui, t: &Tokens, inputs: &[PathBuf], aois: &[Aoi]) {
        self.sheet_note(ui, t, inputs, "volumen", "csv");
        ui.label(egui::RichText::new("Volumen entre una superficie y el terreno, celda a celda: V = Σ (altura de la superficie − MDT) · celda². Las celdas por encima del MDT suman al volumen sobre él y las que quedan por debajo, al volumen bajo él (vaciados). Con un CHM las alturas ya son sobre el suelo y no hace falta MDT. Todas las hojas dan una sola tabla (.csv); busca los rásteres de esa nube en su carpeta de resultados.").color(t.weak).size(11.0));
        let s = &mut self.lx;
        egui::ComboBox::from_id_salt("vol_source")
            .width(400.0)
            .selected_text(
                [
                    "CHM de cada nube (alturas sobre el suelo)",
                    "MDS menos MDT (de cada nube)",
                    "MDS sobre una cota horizontal",
                ][s.vol_source],
            )
            .show_ui(ui, |ui| {
                for (k, n) in [
                    "CHM de cada nube (alturas sobre el suelo)",
                    "MDS menos MDT (de cada nube)",
                    "MDS sobre una cota horizontal",
                ]
                .iter()
                .enumerate()
                {
                    ui.selectable_value(&mut s.vol_source, k, *n);
                }
            });
        let surface = ["CHM", "MDS", "MDS"][s.vol_source];
        Self::raster_row(
            ui,
            surface,
            &mut s.vol_surface,
            rustlidar_core::raster::EXTENSIONS,
        );
        match s.vol_source {
            1 => Self::raster_row(
                ui,
                "MDT",
                &mut s.vol_dtm,
                rustlidar_core::raster::EXTENSIONS,
            ),
            2 => num(
                ui,
                "Cota de referencia (m)",
                &mut s.vol_level,
                -1e5..=1e5,
                0.1,
            ),
            _ => {}
        }
        for p in inputs {
            let found = |auto: Option<PathBuf>, manual: &str| {
                if manual.trim().is_empty() {
                    auto.is_some()
                } else {
                    Path::new(manual.trim()).is_file()
                }
            };
            let ok = match s.vol_source {
                0 => found(paths::find_chm(p), &s.vol_surface),
                1 => {
                    found(paths::find_dsm(p), &s.vol_surface)
                        && found(paths::find_dtm(p), &s.vol_dtm)
                }
                _ => found(paths::find_dsm(p), &s.vol_surface),
            };
            if !ok {
                ui.colored_label(
                    egui::Color32::from_rgb(230, 160, 60),
                    format!(
                        "{}: faltan rásteres; genéralos antes (CHM, MDS, MDT) o elígelos a mano.",
                        paths::stem(p)
                    ),
                );
            }
        }
        num(
            ui,
            "Altura mínima que cuenta (m)",
            &mut s.vol_min,
            0.0..=1000.0,
            0.05,
        );
        ui.label(
            egui::RichText::new(
                "Las celdas más cerca del MDT que esto no suman: quita el ruido del terreno.",
            )
            .color(t.weak)
            .size(11.0),
        );
        ui.separator();
        ui.label(egui::RichText::new("Zonas").strong());
        ui.radio_value(&mut s.vol_regions, 0, "Todo el ráster");
        ui.radio_value(
            &mut s.vol_regions,
            1,
            format!("AOI dibujadas ({}): un volumen por AOI", aois.len()),
        );
        ui.radio_value(&mut s.vol_regions, 2, "Polígonos de un archivo vectorial");
        if s.vol_regions == 2 {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut s.vol_file).desired_width(330.0));
                if ui.button("…").clicked()
                    && let Some(p) = pick_file("Polígonos", &["shp", "geojson", "json", "gpkg"])
                {
                    s.vol_file = p.display().to_string();
                }
            });
        }
    }

    pub fn build_volume(
        &mut self,
        inputs: &[PathBuf],
        aois: &[Aoi],
        log: &mut Vec<String>,
    ) -> Option<Launch> {
        let s = &self.lx;
        let regions: Vec<Region> = match s.vol_regions {
            1 => aois
                .iter()
                .filter(|a| a.points.len() >= 3)
                .map(|a| Region {
                    name: a.name.clone(),
                    shape: Shape2::Polygon(Polygon::new(vec![a.points.clone()])),
                    attrs: vec![],
                })
                .collect(),
            2 => match volume::read_regions(Path::new(s.vol_file.trim())) {
                Ok(r) => r,
                Err(e) => {
                    log.push(format!("Volúmenes: {e}"));
                    return None;
                }
            },
            _ => vec![],
        };
        if s.vol_regions != 0 && regions.is_empty() {
            log.push("Volúmenes: no hay zonas; dibuja AOI o elige un archivo.".into());
            return None;
        }
        let manual = |t: &str| (!t.trim().is_empty()).then(|| PathBuf::from(t.trim()));
        let mut jobs = Vec::new();
        for p in inputs {
            let surface = manual(&s.vol_surface).or_else(|| {
                if s.vol_source == 0 {
                    paths::find_chm(p)
                } else {
                    paths::find_dsm(p)
                }
            });
            let reference = match s.vol_source {
                0 => Some(ReferenceSource::Zero),
                1 => manual(&s.vol_dtm)
                    .or_else(|| paths::find_dtm(p))
                    .map(ReferenceSource::Dtm),
                _ => Some(ReferenceSource::Level(s.vol_level)),
            };
            let (Some(surface), Some(reference)) = (surface, reference) else {
                log.push(format!(
                    "Volúmenes: faltan rásteres de {}; genéralos antes (CHM, MDS, MDT).",
                    paths::stem(p)
                ));
                return None;
            };
            jobs.push(VolumeJob {
                surface,
                reference,
                regions: regions.clone(),
                min_height: s.vol_min,
                sheet: paths::stem(p),
            });
        }
        let output = paths::unique(&paths::product(&inputs[0], "volumen", "csv"));
        let outputs = vec![output.clone()];
        let task: Task = Box::new(move || volume::run_all(&jobs, &output));
        Some(Launch {
            label: ToolId::Volume.title().to_string(),
            task,
            show: vec![],
            outputs,
        })
    }

    fn thin_params(&self) -> ThinParams {
        let s = &self.lx;
        let seed = s.thin_seed as u64;
        ThinParams {
            method: match s.thin_method {
                0 => ThinMethod::Density {
                    per_m2: s.thin_density,
                    seed,
                },
                1 => ThinMethod::Nearest,
                2 => ThinMethod::Lowest,
                3 => ThinMethod::Highest,
                4 => ThinMethod::Percentile(s.thin_pct),
                5 => ThinMethod::Voxel {
                    vertical: s.thin_vcell,
                },
                6 => ThinMethod::EveryNth(s.thin_n as u64),
                7 => ThinMethod::RemoveNth(s.thin_n as u64),
                _ => ThinMethod::Random {
                    fraction: s.thin_fraction,
                    seed,
                },
            },
            cell: s.thin_cell,
        }
    }

    pub fn build_lidar(
        &mut self,
        tool: ToolId,
        inputs: &[PathBuf],
        _log: &mut Vec<String>,
    ) -> Option<Launch> {
        let first = inputs.first()?.clone();
        let label = tool.title().to_string();
        let ext = Self::lx_ext(&first);
        let names = |tag: &str| -> HashMap<PathBuf, PathBuf> {
            inputs
                .iter()
                .map(|p| (p.clone(), paths::unique(&paths::product(p, tag, &ext))))
                .collect()
        };
        let names_ext = |tag: &str, e: &str| -> HashMap<PathBuf, PathBuf> {
            inputs
                .iter()
                .map(|p| (p.clone(), paths::unique(&paths::product(p, tag, e))))
                .collect()
        };
        let inputs_v = inputs.to_vec();
        let (task, outputs): (Task, Vec<PathBuf>) = match tool {
            ToolId::Thin => {
                let params = self.thin_params();
                let outs = names(&format!("delgada_{}", params.tag()));
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let (r, k) = thin::thin_cloud(
                                std::slice::from_ref(p),
                                &out,
                                &params,
                                &Filter::default(),
                            )?;
                            lines.push(thin::report(r, k, &out));
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Hag => {
                let p = self.lx.hag.clone();
                let outs = names(if p.replace_z {
                    "normalizada"
                } else {
                    "alturas"
                });
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for f in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(f).cloned().unwrap_or_default();
                            lines.push(hag::run(f, &out, &p)?);
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Colorize => {
                let image = PathBuf::from(self.lx.col_image.trim());
                if !image.is_file() {
                    _log.push("Elige la imagen con la que colorear.".into());
                    return None;
                }
                let b = self.lx.col_bands.map(|v| v.round().max(1.0) as usize);
                let keep = self.lx.col_keep;
                let outs = names("rgb");
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                let pairs: Vec<(PathBuf, PathBuf)> = inputs_v
                    .iter()
                    .map(|p| (p.clone(), outs.get(p).cloned().unwrap_or_default()))
                    .collect();
                (
                    Box::new(move || {
                        rustlidar_core::colorize::run(&rustlidar_core::colorize::ColorizeJob {
                            image,
                            rgb: b,
                            nir: None,
                            inputs: pairs,
                            keep_outside: keep,
                        })
                    }),
                    outputs,
                )
            }
            ToolId::Intensity => {
                use rustlidar_core::intensity::{IntensityJob, Sensor};
                let sensor = if self.lx.int_mode == 0 {
                    let p = PathBuf::from(self.lx.int_traj.trim());
                    if !p.is_file() {
                        _log.push(
                            "Elige el archivo de la trayectoria, o usa la altura de vuelo.".into(),
                        );
                        return None;
                    }
                    Sensor::Trajectory(p)
                } else {
                    Sensor::Altitude(self.lx.int_alt)
                };
                let (exponent, r) = (self.lx.int_exp, self.lx.int_ref);
                let outs = names("intensidad");
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                let pairs: Vec<(PathBuf, PathBuf)> = inputs_v
                    .iter()
                    .map(|p| (p.clone(), outs.get(p).cloned().unwrap_or_default()))
                    .collect();
                (
                    Box::new(move || {
                        rustlidar_core::intensity::run(&IntensityJob {
                            sensor,
                            exponent,
                            reference: (r > 0.0).then_some(r),
                            inputs: pairs,
                        })
                    }),
                    outputs,
                )
            }
            ToolId::Copc => {
                let outs: HashMap<PathBuf, PathBuf> = inputs
                    .iter()
                    .map(|p| {
                        let stem = paths::stem(p);
                        let stem = stem.strip_suffix(".copc").unwrap_or(&stem).to_string();
                        (
                            p.clone(),
                            paths::unique(&paths::results_dir(p).join(format!("{stem}.copc.laz"))),
                        )
                    })
                    .collect();
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let format = rustlidar_core::las::Reader::open(p)?.header.format;
                            let r = rustlidar_core::copc_write::write(p, &out)?;
                            lines.push(rustlidar_core::copc_write::describe(&r, format, &out));
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Dups => {
                let (f, all) = (self.lx.dup, self.lx.dup_all);
                let outs = names("sin_duplicados");
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let (read, gone) = cloudtools::remove_duplicates(p, &out, f, all)?;
                            lines.push(format!(
                                "{read} puntos leídos, {gone} duplicados quitados: {}",
                                out.display()
                            ));
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Compare | ToolId::Fit => {
                let reference = PathBuf::from(if tool == ToolId::Compare {
                    self.lx.cmp_ref.trim()
                } else {
                    self.lx.fit_ref.trim()
                });
                if !reference.is_file() {
                    _log.push("Elige la nube de referencia.".into());
                    return None;
                }
                let s = &self.lx;
                let (cp, fp) = (
                    cloudtools::CompareParams {
                        min_dist: s.cmp_dist,
                        min_neighbors: s.cmp_neigh as usize,
                    },
                    cloudtools::FitParams {
                        thin: s.fit_thin,
                        max_xy: s.fit_xy,
                        max_z: s.fit_z,
                        max_iter: s.fit_iter as usize,
                        stop_dist: s.fit_stop,
                        shift_only: s.fit_shift,
                    },
                );
                let is_cmp = tool == ToolId::Compare;
                let outs = names(if is_cmp { "cambios" } else { "ajustada" });
                let outputs: Vec<PathBuf> = outs.values().cloned().collect();
                (
                    Box::new(move || {
                        let mut lines = Vec::new();
                        for p in &inputs_v {
                            rustlidar_core::check_cancel()?;
                            let out = outs.get(p).cloned().unwrap_or_default();
                            let refs = std::slice::from_ref(&reference);
                            lines.push(if is_cmp {
                                cloudtools::compare_text(
                                    &cloudtools::compare_clouds(refs, p, &out, &cp)?,
                                    &out,
                                )
                            } else {
                                cloudtools::fit_text(
                                    &cloudtools::fit_cloud(refs, p, &out, &fp)?,
                                    &out,
                                )
                            });
                        }
                        Ok(lines.join("\n"))
                    }),
                    outputs,
                )
            }
            ToolId::Swath => {
                let s = &self.lx;
                let (cell, by_cloud) = (s.sw_cell, s.sw_by_cloud);
                let returns = [
                    cloudtools::SwathReturns::Last,
                    cloudtools::SwathReturns::All,
                    cloudtools::SwathReturns::Single,
                ][s.sw_returns];
                let out = paths::unique(&paths::product(&first, "pasadas", "tif"));
                let outputs = vec![out.clone()];
                let show = vec![out.clone()];
                let task: Task = Box::new(move || {
                    let rep =
                        cloudtools::swath_separation(&inputs_v, &out, cell, returns, by_cloud)?;
                    Ok(cloudtools::swath_text(&rep, &out))
                });
                return Some(Launch {
                    label,
                    task,
                    show,
                    outputs,
                });
            }
            ToolId::Tls => {
                let formats = self.formats();
                let params = self.lx.tls.clone();
                let clouds = self.lx.tls_clouds;
                let stems = self.lx.tls_stems;
                let outs = names_ext("cubicado", "shp");
                let mut outputs: Vec<PathBuf> = Vec::new();
                let mut show = Vec::new();
                for o in outs.values() {
                    outputs.extend(super::related(o));
                    outputs.push(o.with_file_name(format!("{}_trozas.csv", paths::stem(o))));
                    if clouds {
                        outputs.push(o.with_file_name(format!("{}_arboles", paths::stem(o))));
                    }
                    if stems {
                        for i in &inputs_v {
                            if outs.get(i) == Some(o) {
                                outputs.push(
                                    TlsJob {
                                        input: i.clone(),
                                        output: o.clone(),
                                        params: params.clone(),
                                        formats,
                                        tree_clouds: false,
                                        stem_classes: true,
                                    }
                                    .stems_path(),
                                );
                            }
                        }
                    }
                    if formats.vector {
                        show.push(o.clone());
                    }
                }
                let task: Task = Box::new(move || {
                    let mut lines = Vec::new();
                    for p in &inputs_v {
                        rustlidar_core::check_cancel()?;
                        let job = TlsJob {
                            input: p.clone(),
                            output: outs.get(p).cloned().unwrap_or_default(),
                            params: params.clone(),
                            formats,
                            tree_clouds: clouds,
                            stem_classes: stems,
                        };
                        lines.push(tls::run(&job)?);
                    }
                    Ok(lines.join("\n"))
                });
                return Some(Launch {
                    label,
                    task,
                    show,
                    outputs,
                });
            }
            _ => return None,
        };
        Some(Launch {
            label,
            task,
            show: vec![],
            outputs,
        })
    }
}
