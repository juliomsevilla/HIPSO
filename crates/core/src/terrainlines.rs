//! Líneas de cresta y de drenaje trazadas a partir de un modelo del terreno.
//!
//! La red de drenaje son las celdas que recogen al menos un área mínima, seguidas aguas abajo con
//! las direcciones de flujo de [`crate::hydro`]. Las crestas son el drenaje de la superficie al
//! revés: lo que un cauce es para el terreno, una cresta lo es para su imagen especular. La red se
//! corta en tramos (de un nacimiento o una confluencia a la siguiente confluencia) y cada tramo es
//! una línea con las alturas del modelo original, su orden de Strahler (un nacimiento es 1; dos
//! tramos del mismo orden que se unen hacen el orden siguiente), su longitud y su pendiente media.
//!
//! En crestas o cauces perfectamente llanos (alturas iguales a lo largo de la línea) el flujo se
//! divide en el medio y la línea sale en dos trozos que se juntan allí.

use std::path::PathBuf;

use crate::grid::Grid;
use crate::hydro::{self, NONE, neighbour};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Líneas de cresta.
    Ridge,
    /// Líneas de drenaje (cauces).
    Drainage,
}

/// Memoria que ocupa una celda (el modelo, el flujo, la acumulación, el orden y las marcas).
const BYTES_PER_CELL: f64 = 30.0;

pub struct LinesJob {
    pub dtm: PathBuf,
    pub kind: Kind,
    /// Superficie drenada que forma una línea (m²).
    pub min_area: f64,
    /// Las líneas más cortas que esto se descartan (m).
    pub min_length: f64,
    /// Pasadas de suavizado de Chaikin; 0 conserva las celdas.
    pub smooth: u8,
    pub output: PathBuf,
}

impl LinesJob {
    pub fn new(dtm: PathBuf, kind: Kind, output: PathBuf) -> Self {
        Self {
            dtm,
            kind,
            min_area: 1000.0,
            min_length: 0.0,
            smooth: 0,
            output,
        }
    }
}

/// Un tramo de la red.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Vértices x, y y la altura del modelo.
    pub pts: Vec<[f64; 3]>,
    /// Orden de Strahler.
    pub order: u32,
    /// Longitud en planta (m).
    pub length: f64,
    /// Pendiente media entre los extremos, por ciento.
    pub mean_slope: f64,
}

fn plan_length(p: &[[f64; 3]]) -> f64 {
    p.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// Chaikin's corner cutting, keeping both ends.
fn chaikin(pts: &[[f64; 3]], passes: u8) -> Vec<[f64; 3]> {
    let mut p = pts.to_vec();
    for _ in 0..passes {
        if p.len() < 3 {
            break;
        }
        let mut q = Vec::with_capacity(p.len() * 2);
        q.push(p[0]);
        for w in p.windows(2) {
            let mix = |t: f64| {
                [
                    w[0][0] + (w[1][0] - w[0][0]) * t,
                    w[0][1] + (w[1][1] - w[0][1]) * t,
                    w[0][2] + (w[1][2] - w[0][2]) * t,
                ]
            };
            q.push(mix(0.25));
            q.push(mix(0.75));
        }
        q.push(p[p.len() - 1]);
        p = q;
    }
    p
}

/// Las líneas de `kind` en `g`: tramos que drenan al menos `min_area` m², más largos que
/// `min_length`.
pub fn lines(
    g: &Grid,
    kind: Kind,
    min_area: f64,
    min_length: f64,
    smooth: u8,
) -> Result<Vec<Line>> {
    if !(min_length >= 0.0) {
        return Err(Error::Param(
            "la longitud mínima no puede ser negativa".into(),
        ));
    }
    let t = hydro::threshold_cells(g, min_area)?;
    let z = if kind == Kind::Ridge { -1.0 } else { 1.0 };
    let d = hydro::drainage(g, z)?;
    let (acc, order) = hydro::accumulate(g, &d.dir);
    let n = g.data.len();
    let stream = |i: usize| acc[i] >= t && !g.data[i].is_nan();
    let down = |i: usize| match d.dir[i] {
        NONE => None,
        k => neighbour(g, i, usize::from(k)).filter(|&j| stream(j)),
    };
    // cauces que desembocan en cada celda de cauce
    let mut inflow = vec![0u8; n];
    for i in 0..n {
        if stream(i)
            && let Some(j) = down(i)
        {
            inflow[j] = inflow[j].saturating_add(1);
        }
    }
    // orden de Strahler, aguas arriba primero
    let mut strahler = vec![0u32; n];
    let (mut top, mut tops) = (vec![0u32; n], vec![0u8; n]);
    for &i in &order {
        let i = i as usize;
        if !stream(i) {
            continue;
        }
        strahler[i] = if inflow[i] == 0 {
            1
        } else if tops[i] >= 2 {
            top[i] + 1
        } else {
            top[i].max(1)
        };
        if let Some(j) = down(i) {
            let o = strahler[i];
            if o > top[j] {
                top[j] = o;
                tops[j] = 1;
            } else if o == top[j] {
                tops[j] = tops[j].saturating_add(1);
            }
        }
    }
    let point = |i: usize| {
        let (x, y) = g.center(i % g.cols, i / g.cols);
        [x, y, f64::from(g.data[i])]
    };
    let mut out = Vec::new();
    for i in 0..n {
        if !stream(i) || inflow[i] == 1 {
            continue;
        }
        if i % (1 << 20) == 0 {
            crate::check_cancel()?;
        }
        // un nacimiento o una confluencia empieza un tramo; baja hasta la siguiente confluencia
        let mut cells = vec![i];
        let mut at = i;
        while let Some(j) = down(at) {
            cells.push(j);
            if inflow[j] != 1 {
                break;
            }
            at = j;
        }
        if cells.len() < 2 {
            continue;
        }
        let pts: Vec<[f64; 3]> = cells.iter().map(|&c| point(c)).collect();
        let length = plan_length(&pts);
        if length < min_length {
            continue;
        }
        let drop = (pts[0][2] - pts[pts.len() - 1][2]).abs();
        out.push(Line {
            pts: chaikin(&pts, smooth),
            order: strahler[i],
            length,
            mean_slope: if length > 0.0 {
                100.0 * drop / length
            } else {
                0.0
            },
        });
    }
    Ok(out)
}

/// Traza las líneas de `job.dtm` y las escribe como shapefile de polilíneas; devuelve el resumen.
pub fn run(job: &LinesJob) -> Result<String> {
    if let Ok(pyr) = crate::pyramid::Pyramid::open(&job.dtm)
        && let Some(mem) = crate::copc_cache::total_memory()
    {
        let e = pyr.extent();
        let cells = ((e[2] - e[0]) / pyr.cell) * ((e[3] - e[1]) / pyr.cell);
        let need = cells * BYTES_PER_CELL;
        if need > mem as f64 * 0.7 {
            return Err(Error::Param(format!(
                "trazar líneas sobre {:.0} millones de celdas necesita unos {:.0} GB de memoria (este equipo tiene {:.0} GB): el agua se sigue por todo el ráster a la vez. Usa una celda mayor o divide la zona en cuencas",
                cells / 1e6,
                need / 1e9,
                mem as f64 / 1e9
            )));
        }
    }
    let g = Grid::read(&job.dtm)?;
    let epsg = Grid::read_epsg(&job.dtm);
    let found = lines(&g, job.kind, job.min_area, job.min_length, job.smooth)?;
    use crate::vector::{ColKind, Table, Val, prj_for_epsg, write_polylines};
    let table = Table {
        columns: vec![
            ("ID".into(), ColKind::Num(10, 0)),
            ("ORDEN".into(), ColKind::Num(4, 0)),
            ("LONG".into(), ColKind::Num(12, 2)),
            ("PEND_MED".into(), ColKind::Num(8, 2)),
        ],
        rows: found
            .iter()
            .enumerate()
            .map(|(k, l)| {
                vec![
                    Val::Num((k + 1) as f64),
                    Val::Num(f64::from(l.order)),
                    Val::Num(l.length),
                    Val::Num(l.mean_slope),
                ]
            })
            .collect(),
    };
    let geom: Vec<Vec<[f64; 3]>> = found.iter().map(|l| l.pts.clone()).collect();
    crate::paths::ensure_parent(&job.output)?;
    let path = crate::paths::swap_ext(&job.output, "shp");
    write_polylines(
        &path,
        &geom,
        true,
        &table,
        epsg.and_then(prj_for_epsg).as_deref(),
    )?;
    let total: f64 = found.iter().map(|l| l.length).sum();
    let top = found.iter().map(|l| l.order).max().unwrap_or(0);
    Ok(format!(
        "{} tramos de {}, {:.0} m en total, orden máximo {} -> {}",
        found.len(),
        if job.kind == Kind::Ridge {
            "cresta"
        } else {
            "drenaje"
        },
        total,
        top,
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distancia de (x, y) al segmento a-b.
    fn dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
    }

    fn model(cols: usize, rows: usize, f: impl Fn(f64, f64) -> f64) -> Result<Grid> {
        let mut g = Grid::new(0.0, 0.0, 1.0, cols, rows)?;
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = g.center(c, r);
                g.data[r * cols + c] = f(x, y) as f32;
            }
        }
        Ok(g)
    }

    /// Un tejado con su cumbrera en x = 10,5, bajando hacia el sur.
    fn roof() -> Result<Grid> {
        model(21, 30, |x, y| 10.0 - 0.5 * (x - 10.5).abs() + 0.05 * y)
    }

    #[test]
    fn the_crest_of_a_roof_is_one_straight_line_with_the_heights_of_the_model() -> Result<()> {
        let g = roof()?;
        let found = lines(&g, Kind::Ridge, 15.0, 0.0, 0)?;
        assert_eq!(found.len(), 1, "{found:?}");
        let l = &found[0];
        assert!(l.pts.iter().all(|p| (p[0] - 10.5).abs() < 1e-9));
        // del extremo norte al extremo sur del tejado: 29 m entre centros
        assert!((l.length - 29.0).abs() < 1.0, "{}", l.length);
        // todo vértice tiene la altura del modelo, no la invertida
        for p in &l.pts {
            let want = f64::from(g.sample(p[0], p[1]).unwrap_or(f32::NAN));
            assert!((p[2] - want).abs() < 1e-4 && p[2] > 0.0, "{p:?} {want}");
        }
        Ok(())
    }

    #[test]
    fn a_v_valley_has_one_drainage_line_along_the_bottom() -> Result<()> {
        let g = model(21, 30, |x, y| 0.5 * (x - 10.5).abs() + 0.05 * y)?;
        let found = lines(&g, Kind::Drainage, 15.0, 0.0, 0)?;
        assert_eq!(found.len(), 1);
        assert!(found[0].pts.iter().all(|p| (p[0] - 10.5).abs() < 1e-9));
        // el mismo tejado al revés no tiene cresta donde el valle no es una cresta
        let none = lines(&g, Kind::Ridge, 15.0, 0.0, 0)?;
        assert!(
            none.iter()
                .all(|l| l.pts.iter().all(|p| (p[0] - 10.5).abs() > 1.0))
        );
        Ok(())
    }

    #[test]
    fn a_flat_crest_comes_out_in_two_pieces_that_add_up() -> Result<()> {
        let g = model(21, 30, |x, _| 10.0 - 0.5 * (x - 10.5).abs())?;
        let found = lines(&g, Kind::Ridge, 15.0, 0.0, 0)?;
        let total: f64 = found.iter().map(|l| l.length).sum();
        assert!(
            found.len() <= 2 && (total - 29.0).abs() < 2.0,
            "{} {total}",
            found.len()
        );
        Ok(())
    }

    #[test]
    fn a_confluence_gives_three_reaches_of_order_1_1_and_2() -> Result<()> {
        // los cauces: el principal por x = 10,5 y un afluente que se le une en y = 14
        let main = ([10.5, 0.0], [10.5, 29.5]);
        let trib = ([10.5, 14.5], [25.5, 28.5]);
        let g = model(30, 30, |x, y| {
            0.05 * y + 0.5 * dist([x, y], main.0, main.1).min(dist([x, y], trib.0, trib.1))
        })?;
        let found = lines(&g, Kind::Drainage, 30.0, 0.0, 0)?;
        let mut orders: Vec<u32> = found.iter().map(|l| l.order).collect();
        orders.sort_unstable();
        assert_eq!(orders, vec![1, 1, 2], "{found:?}");
        // los tramos se encuentran: el final de cada tramo de aguas arriba es el principio del de
        // debajo
        let ends: Vec<[f64; 2]> = found
            .iter()
            .map(|l| [l.pts[l.pts.len() - 1][0], l.pts[l.pts.len() - 1][1]])
            .collect();
        let starts: Vec<[f64; 2]> = found.iter().map(|l| [l.pts[0][0], l.pts[0][1]]).collect();
        let joint = found.iter().position(|l| l.order == 2).map(|k| starts[k]);
        assert!(joint.is_some_and(|j| ends.iter().filter(|e| **e == j).count() == 2));
        Ok(())
    }

    #[test]
    fn empty_cells_cut_the_lines_without_errors() -> Result<()> {
        let mut g = roof()?;
        for r in 12..14 {
            for c in 0..21 {
                g.data[r * 21 + c] = f32::NAN;
            }
        }
        let found = lines(&g, Kind::Ridge, 15.0, 0.0, 0)?;
        assert!(!found.is_empty());
        assert!(found.iter().all(|l| l.pts.iter().all(|p| p[2].is_finite())));
        Ok(())
    }

    #[test]
    fn a_larger_area_never_gives_more_length() -> Result<()> {
        let g = model(30, 30, |x, y| {
            0.05 * y + 0.5 * dist([x, y], [10.5, 0.0], [10.5, 29.5])
        })?;
        let len = |a: f64| -> Result<f64> {
            Ok(lines(&g, Kind::Drainage, a, 0.0, 0)?
                .iter()
                .map(|l| l.length)
                .sum())
        };
        assert!(len(5.0)? >= len(30.0)? && len(30.0)? >= len(120.0)?);
        Ok(())
    }

    #[test]
    fn the_minimum_length_and_smoothing_work() -> Result<()> {
        let g = roof()?;
        assert!(lines(&g, Kind::Ridge, 15.0, 100.0, 0)?.is_empty());
        let smooth = lines(&g, Kind::Ridge, 15.0, 0.0, 2)?;
        let raw = lines(&g, Kind::Ridge, 15.0, 0.0, 0)?;
        // los extremos se quedan donde estaban
        assert_eq!(smooth[0].pts[0], raw[0].pts[0]);
        assert_eq!(
            smooth[0].pts[smooth[0].pts.len() - 1],
            raw[0].pts[raw[0].pts.len() - 1]
        );
        assert!(lines(&g, Kind::Ridge, 15.0, -1.0, 0).is_err());
        Ok(())
    }

    #[test]
    fn the_lines_are_written() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso_lines_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        roof()?.write_tiff(&dir.join("d.tif"), Some(25829))?;
        let mut job = LinesJob::new(dir.join("d.tif"), Kind::Ridge, dir.join("crestas.shp"));
        job.min_area = 15.0;
        let msg = run(&job)?;
        assert!(msg.starts_with("1 tramos de cresta"), "{msg}");
        assert!(dir.join("crestas.shp").exists());
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
