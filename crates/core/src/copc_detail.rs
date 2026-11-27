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
    // cada nube baja hasta el nivel que le da la misma separación entre puntos que a las demás:
    // sus octrees parten de separaciones distintas, y un nivel común dejaba ralas a las grandes
    let spacings: Vec<f64> = clouds.iter().map(|c| c.spacing()).collect();
    // lo que cada nodo aporta a la ventana: sus puntos en la parte de su cubo que cae dentro
    let inside: Vec<Vec<(i32, u64)>> = clouds
        .iter()
        .zip(&nodes)
        .map(|(c, ns)| {
            ns.iter()
                .map(|n| {
                    let q = c.cube(n.level, n.x, n.y, n.z);
                    let part = |lo: f64, hi: f64, a: f64, b: f64| ((hi.min(b) - lo.max(a)) / (hi - lo).max(1e-9)).clamp(0.0, 1.0);
                    let share = part(q[0], q[3], window[0], window[2]) * part(q[1], q[4], window[1], window[3]);
                    (n.level, (f64::from(n.points) * share).ceil() as u64)
                })
                .collect()
        })
        .collect();
    let (target, levels) = common_levels(&spacings, &inside, budget, px);
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
        let sel: Vec<Node> = ns.iter().filter(|n| n.level <= levels[fi]).copied().collect();
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
    Ok(Some((s, target)))
}

/// La separación común a la que se leen varias nubes y el nivel de cada una: se parte la separación
/// por la mitad mientras los puntos de los nodos elegidos quepan en el presupuesto y siga siendo
/// mayor que un píxel (`px`; 0, sin ese límite). Los niveles raíz se leen siempre.
///
/// De cada nodo se dan su nivel y los puntos que aporta a la ventana.
fn common_levels(spacings: &[f64], nodes: &[Vec<(i32, u64)>], budget: usize, px: f64) -> (f64, Vec<i32>) {
    let deepest: Vec<i32> = nodes
        .iter()
        .map(|ns| ns.iter().map(|n| n.0).max().unwrap_or(0))
        .collect();
    let at = |t: f64| -> Vec<i32> {
        spacings
            .iter()
            .zip(&deepest)
            .map(|(&s, &max)| ((s / t).log2().ceil().max(0.0) as i32).min(max))
            .collect()
    };
    let count = |levels: &[i32]| -> u64 {
        nodes
            .iter()
            .zip(levels)
            .flat_map(|(ns, &l)| ns.iter().filter(move |n| n.0 <= l))
            .map(|n| n.1)
            .sum()
    };
    let mut target = spacings.iter().copied().fold(0.0, f64::max);
    if !(target.is_finite() && target > 0.0) {
        // sin una separación válida en las cabeceras, solo los niveles raíz
        return (0.0, vec![0; spacings.len()]);
    }
    let mut levels = at(target);
    while levels != deepest && !(px > 0.0 && target <= px) {
        let finer = at(target / 2.0);
        if count(&finer) > budget as u64 * 2 {
            break;
        }
        (target, levels) = (target / 2.0, finer);
    }
    (target, levels)
}

#[cfg(test)]
mod common_level_tests {
    use super::*;

    #[test]
    fn clouds_of_different_size_get_the_same_spacing() {
        // dos nubes con la misma densidad: la grande parte de una separación cuatro veces mayor, y
        // sus puntos para esa separación están dos niveles más abajo
        let nodes = vec![
            (0..=6).map(|l| (l, 100u64)).collect::<Vec<_>>(),
            (0..=6).map(|l| (l, 100u64)).collect::<Vec<_>>(),
        ];
        let (t, l) = common_levels(&[20.0, 5.0], &nodes, 10_000, 1.25);
        assert_eq!((t, l), (1.25, vec![4, 2]));
        // sin presupuesto, solo la separación de partida: la pequeña ya está en su raíz
        let (t, l) = common_levels(&[20.0, 5.0], &nodes, 0, 1.25);
        assert_eq!((t, l), (20.0, vec![0, 0]));
        // con px 0 se baja hasta el fondo de las dos
        let (_, l) = common_levels(&[20.0, 5.0], &nodes, 10_000, 0.0);
        assert_eq!(l, vec![6, 6]);
    }
}
