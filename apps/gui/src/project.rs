//! Projects: a `.elp` file (JSON) that remembers which clouds and layers are open, in what order,
//! how they are shown and where the views look, so that a session starts again from there. It
//! holds paths, not data: the files stay where they are.

use super::*;
use rustlidar_core::json::{Json, quote};

pub const EXT: &str = "elp";
const LAYOUTS: [Layout; 4] = [
    Layout::Plan,
    Layout::PlanProfile,
    Layout::View3d,
    Layout::All,
];

fn text(p: &Path) -> String {
    quote(&p.to_string_lossy())
}

impl App {
    pub fn save_project(&mut self) {
        let Some(file) = crate::prefs::dialog()
            .add_filter("Proyecto de EasyLiDAR", &[EXT])
            .set_file_name(format!("proyecto.{EXT}"))
            .save_file()
        else {
            return;
        };
        let clouds: Vec<String> = self.clouds.iter().map(|p| text(p)).collect();
        let layers: Vec<String> = self
            .refs
            .iter()
            .map(|r| {
                // a GeoPackage table is told apart by the « · table» its name carries
                let table = match (&r.kind, r.name.split_once(" · ")) {
                    (RefKind::Vector(_), Some((_, t))) => quote(t),
                    _ => "null".to_string(),
                };
                format!(
                    "{{\"path\":{},\"table\":{},\"visible\":{},\"opacity\":{:?}}}",
                    text(&r.path),
                    table,
                    r.visible,
                    r.opacity
                )
            })
            .collect();
        let layout = LAYOUTS.iter().position(|l| *l == self.layout).unwrap_or(0);
        let (p, o) = (&self.plan, &self.orbit);
        let body = format!(
            "{{\"version\":1,\n\"clouds\":[{}],\n\"layers\":[{}],\n\"layout\":{layout},\n\
             \"plan\":[{:?},{:?},{:?}],\n\"orbit\":[{:?},{:?},{:?}],\n\"point_size\":{}}}\n",
            clouds.join(","),
            layers.join(",\n"),
            p.cx,
            p.cy,
            p.scale,
            o.yaw,
            o.pitch,
            o.exaggeration,
            self.style.point_size
        );
        match std::fs::write(&file, body) {
            Ok(()) => self.log(format!("Proyecto guardado: {}", file.display())),
            Err(e) => self.log(format!("No se pudo guardar el proyecto: {e}")),
        }
    }

    pub fn open_project_dialog(&mut self, ctx: &egui::Context) {
        if let Some(f) = crate::prefs::dialog()
            .add_filter("Proyecto de EasyLiDAR", &[EXT])
            .pick_file()
        {
            self.open_project(ctx, &f);
        }
    }

    /// Closes what is open and opens what the project lists; files that are gone are skipped
    /// and named in the log.
    pub fn open_project(&mut self, ctx: &egui::Context, file: &Path) {
        let parsed = std::fs::read_to_string(file)
            .map_err(|e| e.to_string())
            .and_then(|t| Json::parse(&t).map_err(|e| e.to_string()));
        let j = match parsed {
            Ok(j) if j.get("version").is_some() => j,
            Ok(_) => return self.log("Ese archivo no es un proyecto de EasyLiDAR."),
            Err(e) => return self.log(format!("No se pudo leer el proyecto: {e}")),
        };
        let nums = |k: &str| -> Vec<f64> {
            j.get(k)
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(Json::as_f64).collect())
                .unwrap_or_default()
        };
        self.act(ctx, Action::CloseAll);
        while !self.refs.is_empty() {
            self.remove_ref(0);
        }
        if let Some(l) = j.get("layout").and_then(Json::as_f64) {
            self.layout = LAYOUTS[(l as usize).min(3)];
        }
        if let Some(s) = j.get("point_size").and_then(Json::as_f64) {
            self.style.point_size = (s as usize).clamp(1, 12);
        }
        if let [yaw, pitch, exag] = nums("orbit")[..] {
            (self.orbit.yaw, self.orbit.pitch, self.orbit.exaggeration) = (yaw, pitch, exag);
        }
        let mut missing = Vec::new();
        let clouds: Vec<PathBuf> = j
            .get("clouds")
            .and_then(Json::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Json::as_str)
            .map(PathBuf::from)
            .filter(|p| {
                p.exists() || {
                    missing.push(p.clone());
                    false
                }
            })
            .collect();
        if !clouds.is_empty() {
            self.open_files(ctx, clouds);
        }
        for l in j.get("layers").and_then(Json::as_array).unwrap_or_default() {
            let Some(p) = l.get("path").and_then(Json::as_str).map(PathBuf::from) else {
                continue;
            };
            if !p.exists() {
                missing.push(p);
                continue;
            }
            match l.get("table").and_then(Json::as_str) {
                Some(t) => self.add_vector_table(&p, Some(t)),
                None => self.open_path(ctx, p, false),
            }
            if let Some(r) = self.refs.last_mut() {
                r.visible = l.get("visible") != Some(&Json::Bool(false));
                r.opacity = l
                    .get("opacity")
                    .and_then(Json::as_f64)
                    .map_or(1.0, |v| v as f32);
            }
        }
        if let [cx, cy, scale] = nums("plan")[..] {
            self.plan_fit = false;
            self.forced_view = Some(Plan { cx, cy, scale });
        }
        for m in missing {
            self.log(format!("No está el archivo del proyecto: {}", m.display()));
        }
        self.log(format!("Proyecto abierto: {}", file.display()));
    }
}
