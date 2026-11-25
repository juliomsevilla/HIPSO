//! El cubicado TLS sobre una nube hecha de cilindros: un árbol solo, una cepa con dos brotes y un
//! árbol que se bifurca por encima de la altura del diámetro.

use hipso_core::Result;
use hipso_core::tls::{TlsParams, analyse};

/// Puntos sobre la superficie de un cilindro vertical (o inclinado), cada ~1,5 cm.
fn cylinder(out: &mut Vec<[f64; 3]>, c: (f64, f64), r: f64, z0: f64, z1: f64, lean: (f64, f64)) {
    let step = 0.015;
    let rings = ((z1 - z0) / step).round() as usize;
    let around = ((std::f64::consts::TAU * r / step).round() as usize).max(8);
    for k in 0..=rings {
        let z = z0 + k as f64 * step;
        let (cx, cy) = (c.0 + lean.0 * (z - z0), c.1 + lean.1 * (z - z0));
        // un anillo de cada dos se gira medio paso, para que la superficie no tenga rayas
        let phase = if k % 2 == 0 { 0.0 } else { 0.5 };
        for a in 0..around {
            let t = (a as f64 + phase) * std::f64::consts::TAU / around as f64;
            out.push([cx + r * t.cos(), cy + r * t.sin(), z]);
        }
    }
}

/// Un archivo LAS 1.2 de formato de punto 1 (escala 1 mm) escrito en la carpeta temporal.
fn write_las(name: &str, pts: &[[f64; 3]]) -> Result<std::path::PathBuf> {
    let min = pts.iter().fold([f64::INFINITY; 3], |m, p| {
        [m[0].min(p[0]), m[1].min(p[1]), m[2].min(p[2])]
    });
    let max = pts.iter().fold([f64::NEG_INFINITY; 3], |m, p| {
        [m[0].max(p[0]), m[1].max(p[1]), m[2].max(p[2])]
    });
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[104] = 1;
    h[105..107].copy_from_slice(&28u16.to_le_bytes());
    h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
    for at in [131, 139, 147] {
        h[at..at + 8].copy_from_slice(&0.001f64.to_le_bytes());
    }
    for (at, v) in [
        (179, max[0]),
        (187, min[0]),
        (195, max[1]),
        (203, min[1]),
        (211, max[2]),
        (219, min[2]),
    ] {
        h[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    for p in pts {
        let mut rec = [0u8; 28];
        for k in 0..3 {
            rec[4 * k..4 * k + 4].copy_from_slice(&((p[k] * 1000.0).round() as i32).to_le_bytes());
        }
        rec[14] = 0x09;
        rec[15] = 1;
        h.extend_from_slice(&rec);
    }
    let path = std::env::temp_dir().join(format!("hipso_{name}_{}.las", std::process::id()));
    std::fs::write(&path, h)?;
    Ok(path)
}

fn params() -> TlsParams {
    TlsParams {
        voxel: 0.02,
        // los cilindros se apoyan en un suelo en z = 0: la nube ya son alturas
        normalized: true,
        // un cilindro perfecto tiene un disco por entorno, tan alto como ancho: la verticalidad de
        // los troncos reales (rugosos, vistos desde un lado) no es lo que se prueba aquí
        vert_min: 0.0,
        ..TlsParams::default()
    }
}

#[test]
fn a_stem_a_stool_and_a_fork() -> Result<()> {
    let mut pts = Vec::new();
    // un árbol solo, de 24 cm de diámetro
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 7.0, (0.0, 0.0));
    // a stool: two sprouts 35 cm apart, 16 cm across
    cylinder(&mut pts, (6.0, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    cylinder(&mut pts, (6.35, 0.0), 0.08, 0.0, 7.0, (0.0, 0.0));
    // un árbol de 30 cm de diámetro que se bifurca a 2,5 m en dos ramas de 18 cm de diámetro
    cylinder(&mut pts, (12.0, 0.0), 0.15, 0.0, 2.5, (0.0, 0.0));
    cylinder(&mut pts, (12.0, 0.0), 0.09, 2.5, 7.0, (0.25, 0.0));
    cylinder(&mut pts, (12.0, 0.0), 0.09, 2.5, 7.0, (-0.25, 0.0));
    let input = write_las("stools", &pts)?;
    let r = analyse(&input, &params());
    let _ = std::fs::remove_file(&input);
    let r = r?;
    let trees: Vec<_> = r.trees.iter().map(|t| (t.x.round(), t)).collect();
    assert_eq!(
        trees.len(),
        4,
        "{:?}",
        r.trees
            .iter()
            .map(|t| (t.x, t.dbh, t.flags.clone()))
            .collect::<Vec<_>>()
    );
    let near = |x: f64| -> Vec<&hipso_core::tls::TreeResult> {
        trees
            .iter()
            .filter(|(rx, _)| (rx - x).abs() <= 1.0)
            .map(|(_, t)| *t)
            .collect()
    };
    // el árbol solo
    let single = near(near_origin(&r));
    assert_eq!(single.len(), 1);
    assert!(single[0].sprouts == 1 && single[0].fork_h.is_none());
    assert!(
        single[0].dbh.is_some_and(|d| (d - 0.24).abs() < 0.03),
        "{:?}",
        single[0].dbh
    );
    // la cepa: dos árboles de una cepa, cada uno con su propio diámetro
    let xs: Vec<f64> = r.trees.iter().map(|t| t.x).collect();
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let sprouts: Vec<_> = r
        .trees
        .iter()
        .filter(|t| (t.x - (min_x + 6.0)).abs() < 1.0)
        .collect();
    assert_eq!(sprouts.len(), 2);
    assert!(
        sprouts
            .iter()
            .all(|t| t.sprouts == 2 && t.flags.contains("BROTES"))
    );
    assert_eq!(sprouts[0].stool, sprouts[1].stool);
    assert!(
        sprouts
            .iter()
            .all(|t| t.dbh.is_some_and(|d| (d - 0.16).abs() < 0.03))
    );
    // la bifurcación: un árbol, con el diámetro de su tronco y la bifurcación donde está
    let forked: Vec<_> = r
        .trees
        .iter()
        .filter(|t| (t.x - (min_x + 12.0)).abs() < 1.0)
        .collect();
    assert_eq!(forked.len(), 1);
    assert!(forked[0].flags.contains("BIFURCADO") && forked[0].sprouts == 1);
    assert!(
        forked[0].dbh.is_some_and(|d| (d - 0.30).abs() < 0.03),
        "{:?}",
        forked[0].dbh
    );
    assert!(
        forked[0].fork_h.is_some_and(|h| (2.0..3.2).contains(&h)),
        "{:?}",
        forked[0].fork_h
    );
    Ok(())
}

/// La x del árbol más a la izquierda, donde está el árbol solo.
fn near_origin(r: &hipso_core::tls::TlsResult) -> f64 {
    r.trees
        .iter()
        .map(|t| t.x.round())
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_stem_with_an_unmeasured_stretch_is_followed_through_it() -> Result<()> {
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 9.0, (0.0, 0.0));
    // una oclusión: nada del tronco entre 4,0 y 5,5 m
    pts.retain(|p| !(4.0..5.5).contains(&p[2]));
    let input = write_las("gap", &pts)?;
    let followed = analyse(&input, &params());
    let lost = analyse(
        &input,
        &TlsParams {
            max_gap: 0.5,
            ..params()
        },
    );
    let _ = std::fs::remove_file(&input);
    let (followed, lost) = (followed?, lost?);
    let top = |r: &hipso_core::tls::TlsResult| {
        r.trees
            .iter()
            .flat_map(|t| t.sections.iter().filter(|s| s.ok).map(|s| s.h))
            .fold(0.0, f64::max)
    };
    // con el tramo por defecto el tronco sigue por encima de la oclusión; con uno corto se pierde
    assert!(top(&followed) > 8.0, "{}", top(&followed));
    assert!(top(&lost) < 4.5, "{}", top(&lost));
    Ok(())
}

#[test]
fn the_treeid_cloud_numbers_each_point_with_its_tree_and_leaves_the_rest_at_zero() -> Result<()> {
    use hipso_core::las::Reader;
    use hipso_core::tls::write_tree_ids;
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 6.0, (0.0, 0.0));
    cylinder(&mut pts, (6.0, 0.0), 0.15, 0.0, 6.0, (0.0, 0.0));
    // un punto lejos de los dos troncos (a más de la distancia de asignación) y otro a ras de suelo
    let n_trunks = pts.len();
    pts.push([20.0, 20.0, 3.0]);
    pts.push([0.05, 0.0, 0.05]);
    let input = write_las("ids", &pts)?;
    let p = params();
    let r = analyse(&input, &p)?;
    assert_eq!(r.trees.len(), 2);
    let out = std::env::temp_dir().join(format!("hipso_ids_{}.laz", std::process::id()));
    let (con, total) = write_tree_ids(&input, &r, &p, &out)?;
    let rd = Reader::open(&out)?;
    let field = rd.extra_field("treeID").ok_or(hipso_core::Error::Param("sin treeID".into()))?;
    let rec_len = usize::from(rd.header.record_len);
    let mut ids = Vec::new();
    rd.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            let q = rd.decode(rec);
            ids.push((q.map_or(0.0, |q| q.x), q.map_or(0.0, |q| q.z), field.id(rec)));
        }
        Ok(())
    })?;
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&out);
    assert_eq!(total as usize, pts.len());
    assert_eq!(ids.len(), pts.len());
    let id_of = |x: f64| ids.iter().find(|i| (i.0 - x).abs() < 0.2 && i.1 > 1.0 && i.2 != 0).map(|i| i.2);
    let (a, b) = (id_of(0.12), id_of(6.15));
    assert!(a.is_some() && b.is_some() && a != b, "{a:?} {b:?}");
    // los puntos del tronco llevan su número, el lejano y el de suelo, 0
    let far = ids.iter().find(|i| i.0 > 19.0).map(|i| i.2);
    assert_eq!(far, Some(0));
    assert_eq!(ids.iter().filter(|i| i.2 == 0).count(), pts.len() - con as usize);
    // todo el tronco por encima de 0,2 m (la regla de las nubes por árbol) lleva número
    let sin: usize = ids.iter().filter(|i| i.0 < 10.0 && i.1 > 0.25 && i.2 == 0).count();
    assert_eq!(sin, 0, "{con} de {n_trunks}");
    Ok(())
}

#[test]
fn classes_and_treeid_go_in_one_cloud_and_the_ground_stays_ground() -> Result<()> {
    use hipso_core::las::Reader;
    use hipso_core::tls::classify_stems;
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.12, 0.0, 6.0, (0.0, 0.0));
    // suelo junto al tronco a 1 m de altura (una ladera): clase 2 en el archivo
    let ground_at = pts.len();
    pts.push([1.0, 0.0, 1.0]);
    let input = write_las("cls", &pts)?;
    // la clase 2 de ese punto: el byte de clase del formato 1 está en 15
    let mut raw = std::fs::read(&input)?;
    let off = 227 + ground_at * 28 + 15;
    raw[off] = 2;
    std::fs::write(&input, raw)?;
    let p = params();
    let r = analyse(&input, &p)?;
    let out = std::env::temp_dir().join(format!("hipso_cls_{}.laz", std::process::id()));
    let rep = classify_stems(&input, &r, &p, &out, true)?;
    let rd = Reader::open(&out)?;
    let field = rd.extra_field("treeID").ok_or(hipso_core::Error::Param("sin treeID".into()))?;
    let rec_len = usize::from(rd.header.record_len);
    let mut rows = Vec::new();
    rd.scan_raw(|bytes| {
        for rec in bytes.chunks_exact(rec_len) {
            if let Some(q) = rd.decode(rec) {
                rows.push((q.x, q.z, q.class, field.id(rec)));
            }
        }
        Ok(())
    })?;
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&out);
    assert!(rep.stem > 0);
    assert!(rows.iter().any(|r| r.2 == 64 && r.3 != 0));
    let g = rows.iter().find(|r| (r.0 - 1.0).abs() < 0.01 && (r.1 - 1.0).abs() < 0.01);
    assert_eq!(g.map(|g| g.2), Some(2));
    assert!(g.is_some_and(|g| g.3 != 0));
    Ok(())
}

#[test]
fn a_long_branch_goes_to_its_own_tree_by_the_shortest_path_not_to_the_nearest_trunk() -> Result<()> {
    use hipso_core::las::Reader;
    use hipso_core::tls::write_tree_ids;
    let mut pts = Vec::new();
    cylinder(&mut pts, (0.0, 0.0), 0.15, 0.0, 8.0, (0.0, 0.0));
    cylinder(&mut pts, (6.0, 0.0), 0.15, 0.0, 8.0, (0.0, 0.0));
    // una rama horizontal del árbol de la izquierda a 6 m de altura que llega a 1 m del otro tronco
    let mut x = 0.15;
    while x < 5.0 {
        pts.push([x, 0.0, 6.0]);
        pts.push([x, 0.05, 6.0]);
        x += 0.02;
    }
    let input = write_las("branch", &pts)?;
    let read_ids = |crowns: bool| -> Result<(u32, u32, u32)> {
        let p = TlsParams {
            crowns,
            ..params()
        };
        let r = analyse(&input, &p)?;
        assert_eq!(r.trees.len(), 2);
        let out = std::env::temp_dir().join(format!(
            "hipso_branch_{}_{crowns}.laz",
            std::process::id()
        ));
        write_tree_ids(&input, &r, &p, &out)?;
        let rd = Reader::open(&out)?;
        let field = rd
            .extra_field("treeID")
            .ok_or(hipso_core::Error::Param("sin treeID".into()))?;
        let rec_len = usize::from(rd.header.record_len);
        let (mut left, mut right, mut tip) = (0, 0, 0);
        rd.scan_raw(|bytes| {
            for rec in bytes.chunks_exact(rec_len) {
                if let Some(q) = rd.decode(rec) {
                    let id = field.id(rec);
                    if (q.z - 2.0).abs() < 0.1 && (q.x - 0.15).abs() < 0.02 {
                        left = id;
                    }
                    if (q.z - 2.0).abs() < 0.1 && (q.x - 6.15).abs() < 0.02 {
                        right = id;
                    }
                    if (q.z - 6.0).abs() < 0.01 && (q.x - 4.6).abs() < 0.015 && q.y.abs() < 0.01 {
                        tip = id;
                    }
                }
            }
            Ok(())
        })?;
        let _ = std::fs::remove_file(&out);
        Ok((left, right, tip))
    };
    let axis = read_ids(false);
    let path = read_ids(true);
    let _ = std::fs::remove_file(&input);
    let ((l0, r0, t0), (l1, r1, t1)) = (axis?, path?);
    assert!(l0 != 0 && r0 != 0 && l0 != r0 && l1 == l0 && r1 == r0, "{l0} {r0} {l1} {r1}");
    // por distancia al eje, el extremo de la rama es del tronco más cercano; por el camino, de su árbol
    assert_eq!(t0, r0, "eje: {t0}");
    assert_eq!(t1, l1, "camino: {t1}");
    Ok(())
}
