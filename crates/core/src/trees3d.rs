//! Árboles individuales directamente sobre los puntos, de arriba abajo, sin pasar por un ráster:
//! una segmentación por crecimiento de regiones en la línea de Li, Guo, Jakubowski y Kelly (2012).
//!
//! Los puntos con altura de al menos `hmin` se recorren de más alto a más bajo. Un punto que tiene
//! cerca (a menos del umbral de separación) puntos ya asignados se une al árbol del más cercano; si
//! no tiene ninguno, es la cima de un árbol nuevo. El umbral de separación es `dt1` por debajo de la
//! altura `zu` y `dt2` por encima (las copas altas son más anchas y están más separadas), y con
//! un radio de unión `radius` mayor, un punto que no está a `dt` de nadie pero sí a `radius` de
//! algún árbol se une a ese en lugar de crear uno falso. Los árboles de menos de `min_points`
//! puntos se disuelven.
//!
//! No es una transcripción del artículo, cuyas reglas de borde son más finas: es el mismo recorrido
//! de arriba abajo con los umbrales de separación por altura, escrito a partir de su descripción
//! general.
//!
//! La nube se corta en teselas con margen. Cada árbol se numera por la posición de su cima (una
//! retícula de 1 m), así que dos teselas que ven el mismo árbol lo numeran igual y la nube de
//! salida no tiene árboles cortados por el borde de una tesela.

use std::path::PathBuf;

use crate::filter::Filter;
use crate::grid::Grid;
use crate::las::{ExtraAttr, ExtraKind, Reader, Writer, WriterOptions};
use crate::pipeline::Tiling;
use crate::tile::{Layout, Tile, Tiled};
use crate::{Error, Result, paths};

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// Altura mínima (m) de un punto para entrar.
    pub hmin: f64,
    /// Separación (m) para unir un punto a un árbol por debajo de `zu`.
    pub dt1: f64,
    /// Separación (m) por encima de `zu`.
    pub dt2: f64,
    /// Altura (m) a partir de la cual vale `dt2`.
    pub zu: f64,
    /// Radio (m) de unión de los puntos que no están a `dt` de ningún árbol; 0 o menos que `dt`: ninguno.
    pub radius: f64,
    /// Los árboles con menos puntos se disuelven.
    pub min_points: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            hmin: 2.0,
            dt1: 1.5,
            dt2: 2.0,
            zu: 15.0,
            radius: 3.0,
            min_points: 10,
        }
    }
}

impl Params {
    fn check(&self) -> Result<()> {
        if !(self.hmin >= 0.0) || !(self.dt1 > 0.0) || !(self.dt2 > 0.0) || !(self.zu >= 0.0) {
            return Err(Error::Param(
                "la altura mínima no puede ser negativa y los umbrales de separación deben ser positivos".into(),
            ));
        }
        Ok(())
    }
}

/// Un árbol encontrado: su cima y cuántos puntos tiene.
#[derive(Clone, Debug, PartialEq)]
pub struct Top {
    /// El identificador local (desde 1) dentro de la llamada.
    pub id: u32,
    pub x: f64,
    pub y: f64,
    /// Elevación de la cima.
    pub z: f64,
    /// Altura de la cima sobre el suelo.
    pub h: f64,
    pub points: usize,
    /// El punto de la cima en la lista que se segmentó.
    pub index: usize,
}

/// Segmenta `pts` (`x`, `y`, altura): para cada punto, el identificador local del árbol (desde 1) o
/// 0, y los árboles, ordenados por identificador.
pub fn segment(pts: &[[f64; 3]], p: &Params) -> (Vec<u32>, Vec<Top>) {
    let n = pts.len();
    let mut label = vec![0u32; n];
    let mut order: Vec<usize> = (0..n).filter(|&i| pts[i][2] >= p.hmin).collect();
    order.sort_by(|&a, &b| pts[b][2].total_cmp(&pts[a][2]).then(a.cmp(&b)));
    if order.is_empty() {
        return (label, Vec::new());
    }
    let reach = p.dt1.max(p.dt2).max(p.radius);
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for &i in &order {
        x0 = x0.min(pts[i][0]);
        y0 = y0.min(pts[i][1]);
        x1 = x1.max(pts[i][0]);
        y1 = y1.max(pts[i][1]);
    }
    let cols = (((x1 - x0) / reach).floor() as usize + 1).max(1);
    let rows = (((y1 - y0) / reach).floor() as usize + 1).max(1);
    // lista enlazada por celda de los puntos ya asignados
    let mut head = vec![u32::MAX; cols * rows];
    let mut next = vec![u32::MAX; n];
    let mut tops: Vec<Top> = Vec::new();
    for &i in &order {
        let [x, y, h] = pts[i];
        let dt = if h < p.zu { p.dt1 } else { p.dt2 };
        let (c, r) = (((x - x0) / reach) as usize, ((y - y0) / reach) as usize);
        let (mut best, mut best_d) = (0u32, f64::INFINITY);
        let (mut near, mut near_d) = (0u32, f64::INFINITY);
        for rr in r.saturating_sub(1)..=(r + 1).min(rows - 1) {
            for cc in c.saturating_sub(1)..=(c + 1).min(cols - 1) {
                let mut j = head[rr * cols + cc];
                while j != u32::MAX {
                    let q = pts[j as usize];
                    let d = ((q[0] - x).powi(2) + (q[1] - y).powi(2)).sqrt();
                    let l = label[j as usize];
                    if d < dt && d < best_d {
                        best_d = d;
                        best = l;
                    }
                    if d <= p.radius && d < near_d {
                        near_d = d;
                        near = l;
                    }
                    j = next[j as usize];
                }
            }
        }
        let id = if best != 0 {
            best
        } else if near != 0 {
            near
        } else {
            tops.push(Top {
                id: tops.len() as u32 + 1,
                x,
                y,
                z: h,
                h,
                points: 0,
                index: i,
            });
            tops.len() as u32
        };
        label[i] = id;
        tops[id as usize - 1].points += 1;
        let cell = r * cols + c;
        next[i] = head[cell];
        head[cell] = i as u32;
    }
    // disolver los árboles pequeños y numerar de nuevo
    let mut renumber = vec![0u32; tops.len() + 1];
    let mut kept = Vec::new();
    for (k, t) in tops.iter().enumerate() {
        if t.points >= p.min_points {
            let mut t = t.clone();
            t.id = kept.len() as u32 + 1;
            renumber[k + 1] = t.id;
            kept.push(t);
        }
    }
    for l in &mut label {
        *l = renumber[*l as usize];
    }
    (label, kept)
}

pub struct Trees3dJob {
    pub inputs: Vec<PathBuf>,
    /// MDT con el que se normalizan las alturas; sin él se usa la clase suelo de la propia nube
    /// (un TIN de sus puntos) y, si no la hay, la elevación se toma como altura.
    pub dtm: Option<PathBuf>,
    pub params: Params,
    pub filter: Filter,
    pub tiling: Tiling,
    /// Ruta base: `<salida>.laz` (la nube con `treeID`), `<salida>.csv` y `<salida>.shp` (cimas).
    pub output: PathBuf,
    pub cloud: bool,
    pub csv: bool,
    pub shp: bool,
}

impl Trees3dJob {
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            inputs,
            dtm: None,
            params: Params::default(),
            filter: Filter {
                skip_noise: true,
                ..Filter::default()
            },
            tiling: Tiling {
                size: 250.0,
                buffer: 25.0,
                threads: 0,
            },
            output,
            cloud: true,
            csv: true,
            shp: true,
        }
    }
}

/// `path` con la extensión `ext`, también cuando la tiene de nube (`.laz`, `.las`).
fn with_ext(path: &std::path::Path, ext: &str) -> PathBuf {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".laz") || lower.ends_with(".las") {
        return path.with_extension(ext);
    }
    paths::swap_ext(path, ext)
}

/// El identificador global de un árbol: la posición de su cima en una retícula de 1 m.
fn global_id(x: f64, y: f64, org: [f64; 2], width: u64) -> u32 {
    let (ix, iy) = (
        ((x - org[0]).floor().max(0.0)) as u64,
        ((y - org[1]).floor().max(0.0)) as u64,
    );
    (iy * width + ix + 1) as u32
}

/// Segmenta las nubes y escribe lo pedido; devuelve el resumen.
pub fn run(job: &Trees3dJob) -> Result<String> {
    job.params.check()?;
    if job.inputs.is_empty() {
        return Err(Error::Param("no hay nubes de entrada".into()));
    }
    let dtm = job.dtm.as_ref().map(|p| Grid::read(p)).transpose()?;
    let readers: Vec<Reader> = job
        .inputs
        .iter()
        .map(|p| Reader::open(p))
        .collect::<Result<_>>()?;
    let template = readers
        .first()
        .ok_or_else(|| Error::Param("no hay nubes de entrada".into()))?;
    let epsg = template.header.epsg;
    let b = readers.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, r| {
            let h = r.header.bounds;
            [
                b[0].min(h[0]),
                b[1].min(h[1]),
                b[2].max(h[3]),
                b[3].max(h[4]),
            ]
        },
    );
    let org = [b[0].floor(), b[1].floor()];
    let width = (b[2] - org[0]).ceil() as u64 + 2;
    let height = (b[3] - org[1]).ceil() as u64 + 2;
    if width.saturating_mul(height) >= u64::from(u32::MAX) {
        return Err(Error::Param(
            "la zona es demasiado grande para numerar los árboles: divídela".into(),
        ));
    }
    let layout = Layout::new(b, job.tiling.size, job.tiling.buffer, 1.0)?;
    let tiled = Tiled::spool(&readers, layout)?;
    let rec_len = usize::from(tiled.header.record_len);
    let paths_out = with_ext(&job.output, "laz");
    paths::ensure_parent(&job.output)?;
    let mut writer = if job.cloud {
        let opts = WriterOptions {
            extra: vec![ExtraAttr {
                name: "treeID".into(),
                description: "tree id".into(),
                kind: ExtraKind::U32,
            }],
            ..WriterOptions::default()
        };
        Some(Writer::create_with(&paths_out, template, &opts)?)
    } else {
        None
    };
    let mut all_tops: Vec<(u32, Top)> = Vec::new();
    let (mut points, mut in_trees) = (0u64, 0u64);
    let ground_class = 2u8;
    tiled.process(
        job.tiling.threads,
        |tile: Tile| {
            // alturas de los puntos de la tesela
            let own = &tile.own;
            let pts = &tile.points.pts;
            let ground_tin: Option<Grid> = if dtm.is_none() {
                let g: Vec<[f64; 3]> = pts
                    .iter()
                    .filter(|q| q.class == ground_class && !q.is_noise())
                    .map(|q| [q.x, q.y, q.z])
                    .collect();
                if g.len() >= 3 {
                    let e = tile.core;
                    let m = job.tiling.buffer;
                    let cell = 1.0;
                    let mut grid = Grid::covering([e[0] - m, e[1] - m, e[2] + m, e[3] + m], cell)?;
                    crate::tin::rasterize(&g, &mut grid);
                    Some(grid)
                } else {
                    None
                }
            } else {
                None
            };
            let surface = dtm.as_ref().or(ground_tin.as_ref());
            let mut cand: Vec<[f64; 3]> = Vec::new();
            let mut at: Vec<usize> = Vec::new();
            for (k, q) in pts.iter().enumerate() {
                if !job.filter.keeps(q) || q.class == ground_class {
                    continue;
                }
                let h = match surface {
                    Some(s) => match s.sample(q.x, q.y) {
                        Some(g) => q.z - f64::from(g),
                        None => continue,
                    },
                    None => q.z,
                };
                if h.is_finite() && job.filter.z_ok(h) {
                    cand.push([q.x, q.y, h]);
                    at.push(k);
                }
            }
            let (labels, tops) = segment(&cand, &job.params);
            let mut global = vec![0u32; tops.len() + 1];
            let mut mine: Vec<(u32, Top)> = Vec::new();
            for t in &tops {
                let g = global_id(t.x, t.y, org, width);
                global[t.id as usize] = g;
                let e = tile.core;
                if t.x >= e[0] && t.x < e[2] && t.y >= e[1] && t.y < e[3] {
                    // la elevación de la cima, no su altura
                    let mut t = t.clone();
                    t.z = at.get(t.index).map_or(t.h, |&k| pts[k].z);
                    mine.push((g, t));
                }
            }
            let mut ids = vec![0u32; pts.len()];
            for (l, &k) in labels.iter().zip(&at) {
                ids[k] = global[*l as usize];
            }
            let mut out = Vec::new();
            let (mut n, mut inside) = (0u64, 0u64);
            if job.cloud {
                for (k, rec) in tile.records.chunks_exact(rec_len).enumerate() {
                    if !own.get(k).copied().unwrap_or(false) {
                        continue;
                    }
                    n += 1;
                    inside += u64::from(ids[k] != 0);
                    out.extend_from_slice(rec);
                    out.extend_from_slice(&ids[k].to_le_bytes());
                }
            } else {
                for (k, _) in pts.iter().enumerate() {
                    if own.get(k).copied().unwrap_or(false) {
                        n += 1;
                        inside += u64::from(ids[k] != 0);
                    }
                }
            }
            Ok((out, mine, n, inside))
        },
        |(out, mine, n, inside)| {
            if let Some(w) = writer.as_mut() {
                for rec in out.chunks_exact(rec_len + 4) {
                    w.write_with_extra(&rec[..rec_len], &rec[rec_len..])?;
                }
            }
            all_tops.extend(mine);
            points += n;
            in_trees += inside;
            Ok(())
        },
    )?;
    if let Some(w) = writer {
        w.finish()?;
    }
    all_tops.sort_by_key(|(g, _)| *g);
    let mut lines = vec![format!(
        "{} árboles; {} de {} puntos ({:.1} %) en algún árbol (altura mínima {} m, separación {} / {} m por debajo y encima de {} m)",
        all_tops.len(),
        in_trees,
        points,
        100.0 * in_trees as f64 / points.max(1) as f64,
        job.params.hmin,
        job.params.dt1,
        job.params.dt2,
        job.params.zu
    )];
    if job.cloud {
        lines.push(format!("  nube con treeID: {}", paths_out.display()));
    }
    if job.csv {
        let p = with_ext(&job.output, "csv");
        let mut text = String::from("treeID,X,Y,Z,H,Puntos\n");
        for (g, t) in &all_tops {
            text.push_str(&format!(
                "{g},{:.3},{:.3},{:.3},{:.3},{}\n",
                t.x, t.y, t.z, t.h, t.points
            ));
        }
        std::fs::write(&p, text)?;
        lines.push(format!("  cimas (tabla): {}", p.display()));
    }
    if job.shp {
        use crate::view::{Feature, GeomKind, VectorData};
        let mut d = VectorData {
            points: true,
            fields: ["treeid", "z", "h", "puntos"].map(String::from).to_vec(),
            bounds: [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            ..Default::default()
        };
        for (g, t) in &all_tops {
            d.bounds = [
                d.bounds[0].min(t.x),
                d.bounds[1].min(t.y),
                d.bounds[2].max(t.x),
                d.bounds[3].max(t.y),
            ];
            d.num.push(vec![f64::from(*g), t.z, t.h, t.points as f64]);
            d.text.push(vec![
                g.to_string(),
                format!("{:.3}", t.z),
                format!("{:.3}", t.h),
                t.points.to_string(),
            ]);
            d.features.push(Feature {
                parts: vec![vec![[t.x, t.y]]],
            });
        }
        let p = with_ext(&job.output, "shp");
        let prj = epsg.and_then(crate::vector::prj_for_epsg);
        crate::vector::write_layer(&p, &d, GeomKind::Point, prj.as_deref())?;
        lines.push(format!("  cimas (puntos): {}", p.display()));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una copa cónica de puntos: la cima en `(cx, cy)` a `h`, con anillos que bajan.
    fn cone(cx: f64, cy: f64, h: f64, radius: f64, out: &mut Vec<[f64; 3]>) {
        out.push([cx, cy, h]);
        for ring in 1..=4 {
            let f = f64::from(ring) / 4.0;
            let (r, z) = (radius * f, h - 0.6 * h * f);
            for k in 0..(8 * ring) {
                let a = f64::from(k) / f64::from(8 * ring) * std::f64::consts::TAU;
                out.push([cx + r * a.cos(), cy + r * a.sin(), z]);
            }
        }
    }

    #[test]
    fn two_separated_trees_are_two() {
        let mut pts = Vec::new();
        cone(0.0, 0.0, 20.0, 3.0, &mut pts);
        cone(12.0, 0.0, 14.0, 3.0, &mut pts);
        let (lab, tops) = segment(&pts, &Params::default());
        assert_eq!(tops.len(), 2);
        assert!(lab.iter().all(|&l| l != 0));
        // la cima más alta es el árbol 1
        assert_eq!(tops[0].h, 20.0);
        assert_eq!(tops[1].h, 14.0);
        assert_eq!(lab[0], 1);
        assert!(lab[1..81].iter().all(|&l| l == 1));
    }

    #[test]
    fn touching_crowns_split_at_the_valley() {
        let mut pts = Vec::new();
        cone(0.0, 0.0, 20.0, 4.0, &mut pts);
        cone(7.0, 0.0, 18.0, 4.0, &mut pts);
        let (lab, tops) = segment(
            &pts,
            &Params {
                dt1: 1.2,
                dt2: 1.2,
                ..Params::default()
            },
        );
        assert_eq!(tops.len(), 2);
        // los puntos de cada cono van con su cima
        let first = lab[0];
        let second = lab[81];
        assert_ne!(first, second);
    }

    #[test]
    fn low_points_are_ignored_and_tiny_trees_dissolved() {
        let mut pts = vec![[0.0, 0.0, 0.5], [5.0, 5.0, 1.0]];
        cone(20.0, 20.0, 10.0, 2.0, &mut pts);
        pts.push([60.0, 60.0, 8.0]); // un punto suelto: un árbol de 1 punto
        let (lab, tops) = segment(&pts, &Params::default());
        assert_eq!(tops.len(), 1);
        assert_eq!(lab[0], 0);
        assert_eq!(lab[1], 0);
        assert_eq!(*lab.last().unwrap_or(&9), 0);
    }

    #[test]
    fn empty_input_is_fine() {
        let (l, t) = segment(&[], &Params::default());
        assert!(l.is_empty() && t.is_empty());
    }
}
