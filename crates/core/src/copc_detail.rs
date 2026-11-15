//! El detalle de una ventana de nubes COPC: cuando el visor hace zoom, los puntos de toda la nube
//! que guarda (una muestra uniforme) son demasiado pocos para la ventana, pero los archivos COPC
//! guardan cada punto en un octree cuyos niveles son cada vez más densos. Esto lee los niveles de
//! los nodos que tocan la ventana, hasta el más profundo que cabe en un presupuesto de puntos y no
//! más profundo que un punto por `px` unidades del mapa (un píxel).

use rayon::prelude::*;

use crate::copc::{Copc, Node};
use crate::las::Reader;
use crate::view::PointStore;
use crate::{Error, Result};

/// Un registro decodificado: x, y, z, intensidad, byte de retorno, clase, identificador de fuente,
/// ángulo de barrido, color.
type Row = (f64, f64, f64, u16, u8, u8, u16, i16, Option<[u8; 3]>, u32);

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

/// Los puntos dentro de `window` `[x0, y0, x1, y1]` de nubes COPC, como un almacén que usa el
/// `origin` y el orden de archivo del almacén de las mismas nubes, de modo que los estilos del
/// visor funcionan sobre él sin cambios. Como mucho unos `budget` puntos: los niveles del octree se
/// toman desde la raíz mientras caben. `None` cuando un archivo no es COPC (un LAZ normal no puede
/// dar una ventana sin leerlo entero).
pub fn load_detail(
    paths: &[std::path::PathBuf],
    window: [f64; 4],
    budget: usize,
    px: f64,
    origin: [f64; 3],
    files: &[String],
) -> Result<Option<(PointStore, f64)>> {
    let mut clouds = Vec::new();
    for p in paths {
        match Copc::open(p)? {
            Some(c) => clouds.push(c),
            None => return Ok(None),
        }
    }
    let nodes: Vec<Vec<Node>> = clouds
        .iter()
        .map(|c| c.nodes(Some(window), None))
        .collect::<Result<_>>()?;
    // el nivel más profundo cuyos nodos (todos, un nodo llega más allá de la ventana) caben
    let max_level = nodes.iter().flatten().map(|n| n.level).max().unwrap_or(0);
    // el nivel cuyo espaciado es aproximadamente un píxel (`px` unidades del mapa): más fino solo
    // sobredibujaría
    let spacing = clouds
        .iter()
        .map(|c| c.spacing())
        .fold(f64::INFINITY, f64::min);
    let wanted = if px > 0.0 && spacing.is_finite() {
        (spacing / px).log2().ceil().max(0.0) as i32
    } else {
        max_level
    };
    let mut level = 0;
    for l in 0..=max_level {
        let pts: u64 = nodes
            .iter()
            .flatten()
            .filter(|n| n.level <= l)
            .map(|n| u64::from(n.points))
            .sum();
        if (pts > budget as u64 * 2 || l > wanted) && l > 0 {
            break;
        }
        level = l;
    }
    let mut s = PointStore {
        origin,
        files: files.to_vec(),
        stride: 1,
        ..PointStore::default()
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let has_rgb = clouds
        .iter()
        .any(|c| matches!(c.header().format, 2 | 3 | 5 | 7 | 8 | 10));
    let has_tree = clouds
        .iter()
        .any(|c| c.reader().extra_field("treeID").is_some());
    for (fi, (c, ns)) in clouds.iter().zip(&nodes).enumerate() {
        let sel: Vec<Node> = ns.iter().filter(|n| n.level <= level).copied().collect();
        let r: &Reader = c.reader();
        let tree_field = r.extra_field("treeID");
        let h = &r.header;
        let rec_len = usize::from(h.record_len);
        let (src_at, rgb_at) = (
            if h.format >= 6 { 20 } else { 18 },
            match h.format {
                2 => Some(20),
                3 | 5 => Some(28),
                7 | 8 | 10 => Some(30),
                _ => None,
            },
        );
        c.read_raw(&sel, |bytes| {
            // decodificar es lo lento: todos los núcleos lo comparten
            let rows: Vec<Vec<Row>> = bytes
                .par_chunks(rec_len * 16_384)
                .map(|block| {
                    block
                        .chunks_exact(rec_len)
                        .filter_map(|rec| {
                            let p = r.decode(rec)?;
                            if p.x < window[0]
                                || p.x > window[2]
                                || p.y < window[1]
                                || p.y > window[3]
                            {
                                return None;
                            }
                            let rgb = rgb_at.map(|a| {
                                let q = |k: usize| (u16_at(rec, a + 2 * k) >> 8) as u8;
                                [q(0), q(1), q(2)]
                            });
                            Some((
                                p.x,
                                p.y,
                                p.z,
                                p.intensity,
                                p.return_number | (p.number_of_returns << 4),
                                p.class,
                                u16_at(rec, src_at),
                                (r.decode_extra(rec).scan_angle * 10.0).round() as i16,
                                rgb,
                                tree_field.map_or(0, |f| f.id(rec)),
                            ))
                        })
                        .collect()
                })
                .collect();
            for (x, y, z, i, ret, class, src, scan, rgb, tree) in rows.into_iter().flatten() {
                s.x.push((x - origin[0]) as f32);
                s.y.push((y - origin[1]) as f32);
                s.z.push((z - origin[2]) as f32);
                s.intensity.push(i);
                s.ret.push(ret);
                s.class.push(class);
                s.src.push(src);
                s.scan.push(scan);
                s.file.push(fi.min(255) as u8);
                if has_rgb {
                    s.rgb.push(rgb.unwrap_or([150, 150, 150]));
                }
                if has_tree {
                    s.tree.push(tree);
                }
                for (k, v) in [x, y, z].into_iter().enumerate() {
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
            Ok(())
        })
        .map_err(|e| Error::Format(format!("COPC: {e}")))?;
    }
    s.bounds = if s.x.is_empty() {
        [0.0; 6]
    } else {
        [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
    };
    s.total = s.x.len() as u64;
    Ok(Some((s, spacing / f64::from(1u32 << level.min(30) as u32))))
}
