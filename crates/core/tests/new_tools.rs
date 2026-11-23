//! Herramientas de control de calidad e inventario con nubes sintéticas: la validación del MDT, los
//! huecos del dosel, las métricas de vóxeles, los árboles sobre los puntos y sus métricas.

mod common;

use common::{write_las, write_las_with};
use hipso_core::grid::Grid;
use hipso_core::las::Reader;
use hipso_core::{Error, Result};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("hipso_{}_{name}", std::process::id()))
}

const X0: f64 = 500_000.0;
const Y0: f64 = 450_000.0;

/// Una copa cónica de `h` m de alto con su cima en `(cx, cy)`.
fn cone(cx: f64, cy: f64, h: f64, radius: f64, out: &mut Vec<[f64; 3]>) {
    out.push([cx, cy, h]);
    for ring in 1..=6 {
        let f = f64::from(ring) / 6.0;
        let (r, z) = (radius * f, h - 0.55 * h * f);
        for k in 0..(10 * ring) {
            let a = f64::from(k) / f64::from(10 * ring) * std::f64::consts::TAU;
            out.push([cx + r * a.cos(), cy + r * a.sin(), z]);
        }
    }
}

#[test]
fn dtm_validation_measures_a_known_offset() -> Result<()> {
    // un terreno plano a 100 m y un MDT 0,25 m por encima
    let ground: Vec<[f64; 3]> = (0..40)
        .flat_map(|i| {
            (0..40).map(move |j| [X0 + 2.0 + f64::from(i), Y0 + 2.0 + f64::from(j), 100.0])
        })
        .collect();
    let las = write_las_with("dv_ground", &ground, |_| (2, 0))?;
    let mut dtm = Grid::new(X0, Y0, 1.0, 44, 44)?;
    dtm.data.fill(100.25);
    let tif = tmp("dv_dtm.tif");
    dtm.write_tiff(&tif, Some(25830))?;
    let mut job = hipso_core::dtmcheck::DtmCheckJob::new(tif.clone(), las, tmp("dv_out.csv"));
    job.shp = true;
    let text = hipso_core::dtmcheck::run(&job)?;
    assert!(text.contains("1600 puntos de control"), "{text}");
    let summary = std::fs::read_to_string(tmp("dv_out_resumen.csv"))?;
    let row = summary
        .lines()
        .nth(1)
        .ok_or_else(|| Error::Param("sin fila".into()))?;
    let v: Vec<&str> = row.split(',').collect();
    // N, fuera, descartados, sesgo, mediana, MAE, RMSE, desviación, NMAD, LE95
    assert_eq!(v[0], "1600");
    for k in [3, 4, 5, 6] {
        assert!(
            (v[k]
                .parse::<f64>()
                .map_err(|e| Error::Param(e.to_string()))?
                - 0.25)
                .abs()
                < 1e-3,
            "{row}"
        );
    }
    assert_eq!(v[7], "0.0000");
    assert_eq!(v[8], "0.0000");
    // con un límite de error, todos se descartan
    let mut strict = hipso_core::dtmcheck::DtmCheckJob::new(
        tif,
        write_las_with("dv_ground2", &ground, |_| (2, 0))?,
        tmp("dv_out2.csv"),
    );
    strict.max_error = Some(0.1);
    assert!(hipso_core::dtmcheck::run(&strict).is_err());
    Ok(())
}

#[test]
fn canopy_gaps_are_written_with_their_table() -> Result<()> {
    let mut chm = Grid::new(X0, Y0, 1.0, 30, 30)?;
    chm.data.fill(25.0);
    for r in 10..15 {
        for c in 10..16 {
            chm.data[r * 30 + c] = 0.0;
        }
    }
    let tif = tmp("cg_chm.tif");
    chm.write_tiff(&tif, Some(25830))?;
    let out = tmp("cg_out.shp");
    let mut job = hipso_core::canopygaps::GapJob::new(tif, out.clone());
    job.raster = true;
    hipso_core::canopygaps::run(&job)?;
    let csv = std::fs::read_to_string(out.with_extension("csv"))?;
    let row = csv
        .lines()
        .nth(1)
        .ok_or_else(|| Error::Param("sin huecos".into()))?;
    assert!(row.starts_with("1,30.00,22.00,"), "{row}");
    assert!(out.with_extension("shp").exists() && out.with_extension("tif").exists());
    Ok(())
}

#[test]
fn voxel_metrics_count_points_per_column_and_layer() -> Result<()> {
    // una columna de 2 x 2 m con 5 puntos a 0,2 m y 15 a 7,5 m: capas 0 y 7
    let mut pts = Vec::new();
    for k in 0..5 {
        pts.push([X0 + 0.2 + f64::from(k) * 0.3, Y0 + 0.5, 0.2]);
    }
    for k in 0..15 {
        pts.push([X0 + 0.1 + f64::from(k) * 0.1, Y0 + 1.0, 7.5]);
    }
    // otra columna, en la celda de al lado, con un solo punto
    pts.push([X0 + 2.5, Y0 + 0.5, 3.5]);
    let f = write_las("vox", &pts)?;
    let mut job = hipso_core::voxelmetrics::VoxelJob::new(vec![f], tmp("vox_out"));
    job.metrics = vec![
        "puntos".into(),
        "voxeles".into(),
        "hmax".into(),
        "hueco".into(),
        "lai".into(),
    ];
    let (grids, _, dropped) = hipso_core::voxelmetrics::compute(&job)?;
    assert_eq!(dropped, 0);
    let get = |name: &str| {
        grids
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, g)| g.clone())
    };
    let n = get("puntos").ok_or_else(|| Error::Param("sin puntos".into()))?;
    let first = n
        .index(X0 + 1.0, Y0 + 1.0)
        .ok_or_else(|| Error::Param("fuera".into()))?;
    assert_eq!(n.data[first], 20.0);
    let second = n
        .index(X0 + 3.0, Y0 + 1.0)
        .ok_or_else(|| Error::Param("fuera".into()))?;
    assert_eq!(n.data[second], 1.0);
    let v = get("voxeles").ok_or_else(|| Error::Param("sin vóxeles".into()))?;
    assert_eq!(v.data[first], 2.0);
    let h = get("hmax").ok_or_else(|| Error::Param("sin hmax".into()))?;
    assert_eq!(h.data[first], 8.0);
    // 5 de 20 puntos por debajo de 0,5 m: hueco 0,25 y LAI = -ln(0,25) / 0,5
    let gap = get("hueco").ok_or_else(|| Error::Param("sin hueco".into()))?;
    assert!((gap.data[first] - 0.25).abs() < 1e-6);
    let lai = get("lai").ok_or_else(|| Error::Param("sin lai".into()))?;
    assert!((lai.data[first] - 4.0f32.ln() / 0.5).abs() < 1e-5);
    Ok(())
}

#[test]
fn trees_found_on_the_points_get_their_metrics() -> Result<()> {
    // suelo plano (clase 2) y cuatro árboles separados
    let mut pts: Vec<[f64; 3]> = Vec::new();
    for i in 0..60 {
        for j in 0..60 {
            pts.push([X0 + f64::from(i), Y0 + f64::from(j), 0.0]);
        }
    }
    let n_ground = pts.len();
    let tops = [
        (10.0, 10.0, 22.0),
        (30.0, 12.0, 18.0),
        (12.0, 35.0, 25.0),
        (40.0, 40.0, 15.0),
    ];
    for (x, y, h) in tops {
        cone(X0 + x, Y0 + y, h, 3.0, &mut pts);
    }
    let f = write_las_with("t3d", &pts, |i| (if i < n_ground { 2 } else { 5 }, 0))?;
    let out = tmp("t3d_out.laz");
    let mut job = hipso_core::trees3d::Trees3dJob::new(vec![f], out.clone());
    job.tiling.size = 30.0;
    job.tiling.buffer = 8.0;
    job.params.min_points = 15;
    let text = hipso_core::trees3d::run(&job)?;
    assert!(text.starts_with("4 árboles"), "{text}");
    // la nube de salida tiene el atributo y todos los puntos
    let r = Reader::open(&out)?;
    assert_eq!(r.header.count as usize, pts.len());
    assert!(r.extra_field("treeID").is_some());
    // las cimas, con sus alturas
    let csv = std::fs::read_to_string(out.with_extension("csv"))?;
    let mut hs: Vec<f64> = csv
        .lines()
        .skip(1)
        .filter_map(|l| l.split(',').nth(4).and_then(|v| v.parse().ok()))
        .collect();
    hs.sort_by(f64::total_cmp);
    assert_eq!(hs, vec![15.0, 18.0, 22.0, 25.0]);
    // las métricas: alturas dadas por las elevaciones (suelo a 0)
    let mut m = hipso_core::treemetrics::TreeMetricsJob::new(out, tmp("tm_out.csv"));
    m.normalized = true;
    m.params.min_points = 15;
    m.params.gap = 3.0;
    let text = hipso_core::treemetrics::run(&m)?;
    assert!(text.starts_with("4 árboles medidos"), "{text}");
    let table = std::fs::read_to_string(tmp("tm_out.csv"))?;
    assert_eq!(table.lines().count(), 5);
    for l in table.lines().skip(1) {
        let v: Vec<f64> = l.split(',').filter_map(|x| x.parse().ok()).collect();
        // la copa cónica de 3 m de radio: superficie de la envolvente algo menor que π 3²
        assert!(v[10] > 20.0 && v[10] < 29.0, "{l}");
        // el volumen alfa no supera al de la envolvente convexa
        assert!(v[13] <= v[12] + 1e-6, "{l}");
    }
    Ok(())
}
