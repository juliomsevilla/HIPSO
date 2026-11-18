//! Edición de una nube con capas vectoriales: borra o reclasifica los puntos dentro de polígonos o
//! a cierta distancia de líneas (ejes, caminos, cauces), con filtros de clase y de cota. Es lo que
//! hace `hipso edit` con `--regions` o `--lines`, y lo comparte la ventana del visor.

use std::path::{Path, PathBuf};

use crate::classes::ClassSet;
use crate::edit::{self, Action, Attr, Brush, Edit, Mode, Region, SaveReport, SelOp};
use crate::filter::ClassSel;
use crate::{Error, Result, vector};

/// De dónde salen las regiones de la edición.
#[derive(Clone, Debug)]
pub enum Layer {
    /// Toda la nube (solo cuentan los filtros de clase y de cota).
    None,
    /// Polígonos, o centros de parcelas con su radio.
    Polygons { path: PathBuf, radius: Option<f64> },
    /// Líneas: los puntos a menos de `distance` de ellas.
    Lines { path: PathBuf, distance: f64 },
}

#[derive(Clone, Debug)]
pub struct LayerEdit {
    pub layer: Layer,
    /// Selecciona en cambio lo que queda fuera de la capa.
    pub outside: bool,
    /// Clases que se seleccionan (p. ej. «2,5» o «~7,18»); vacío: todas.
    pub classes: String,
    pub zmin: Option<f64>,
    pub zmax: Option<f64>,
    pub action: Action,
}

/// Aplica la edición a `input` y escribe `output`, un archivo nuevo.
pub fn edit_by_layer(input: &Path, output: &Path, spec: &LayerEdit) -> Result<SaveReport> {
    let mut attr = Attr {
        z: match (spec.zmin, spec.zmax) {
            (None, None) => None,
            (a, b) => Some((a.unwrap_or(f64::NEG_INFINITY), b.unwrap_or(f64::INFINITY))),
        },
        ..Attr::default()
    };
    if !spec.classes.trim().is_empty() {
        attr.classes = ClassSel::parse(&spec.classes)?.set;
    }
    let shapes: Option<Vec<Region>> = match &spec.layer {
        Layer::None => None,
        Layer::Lines { path, distance } => {
            if !(*distance > 0.0) {
                return Err(Error::Param("la distancia debe ser positiva".into()));
            }
            let layer = crate::view::read_vector(path)?;
            let v: Vec<Region> = layer
                .features
                .iter()
                .flat_map(|f| f.parts.iter())
                .filter(|p| p.len() >= 2)
                .map(|p| Region::PlanBrush(Brush::new(p, *distance, false)))
                .collect();
            if v.is_empty() {
                return Err(Error::Param(format!("{} no tiene líneas", path.display())));
            }
            Some(v)
        }
        Layer::Polygons { path, radius } => {
            let opts = vector::ReadOptions {
                name_field: None,
                radius: *radius,
                radius_field: None,
                ..vector::ReadOptions::default()
            };
            let list = vector::read_regions(path, &opts)?;
            Some(crate::ops::polygons_of(&list))
        }
    };
    let mut sel = Vec::new();
    match shapes {
        Some(list) => {
            for (k, r) in list.into_iter().enumerate() {
                sel.push(SelOp {
                    mode: if k == 0 { Mode::New } else { Mode::Add },
                    region: r,
                    attr: Attr {
                        classes: ClassSet::all(),
                        z: None,
                        ..Attr::default()
                    },
                });
            }
            if spec.outside {
                sel.push(SelOp {
                    mode: Mode::Invert,
                    region: Region::All,
                    attr: Attr::default(),
                });
            }
            sel.push(SelOp {
                mode: Mode::Intersect,
                region: Region::All,
                attr,
            });
        }
        None => sel.push(SelOp {
            mode: Mode::New,
            region: Region::All,
            attr,
        }),
    }
    let label = match spec.action {
        Action::Delete => "eliminar".to_string(),
        Action::Withhold => "retener".to_string(),
        Action::Reclassify(c) => format!("clase {c}"),
    };
    edit::save(input, output, &[Edit::new(sel, spec.action, label)])
}
