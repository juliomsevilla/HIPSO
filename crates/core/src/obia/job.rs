//! The whole analysis as one job: layers → objects → attributes → classes → files.

use std::path::PathBuf;

use super::attributes::{Canopy, attributes_with};
use super::classify::{self, Classified, ForestParams};
use super::layers::{CloudLayer, cloud_layers, local_std};
use super::segment::{SegmentParams, segment};
use super::strata;
use super::vectorize::write_polygons;
use super::{Lattice, LayerStack, write_classes_tiff};
use crate::filter::Filter;
use crate::grid::Grid;
use crate::paths::{self, Formats};
use crate::vector::{ReadOptions, read_regions};
use crate::{Error, Result, check_cancel};

/// How the objects get their class.
#[derive(Clone, Debug, PartialEq)]
pub enum ClassifyBy {
    /// Objects are only described.
    Nothing,
    /// A text file of rules (see [`classify::parse_rules`]).
    Rules(PathBuf),
    /// No rules and no samples: `k` strata by how alike the objects are (k-means on `features`,
    /// the means and percentiles of the layers and the cover when empty).
    Kmeans {
        k: usize,
        features: Vec<String>,
        seed: u64,
    },
    /// Sample polygons (a shapefile) whose `field` names the class; a random forest learns from
    /// the objects they cover.
    Samples {
        file: PathBuf,
        field: String,
        forest: ForestParams,
    },
}

/// Joins the classified objects into strata and spreads sample plots among them.
#[derive(Clone, Debug, PartialEq)]
pub struct StrataOpts {
    /// Areas under this many cells take the class of their neighbours.
    pub min_cells: u32,
    /// Sample plots to spread among the strata (0: none).
    pub plots: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObiaJob {
    /// One-band rasters on the same cells, the canopy height model first; each is a layer named
    /// after its file.
    pub rasters: Vec<PathBuf>,
    /// Point clouds to read for more layers, and which layers.
    pub clouds: Vec<PathBuf>,
    pub cloud_layers: Vec<CloudLayer>,
    pub filter: Filter,
    /// Adds the local roughness of the first raster over a window of this many cells around each
    /// cell (0: no).
    pub roughness: usize,
    pub segment: SegmentParams,
    pub classify: ClassifyBy,
    /// Base path of the results (its extension is replaced by what each file needs).
    pub output: PathBuf,
    pub formats: Formats,
    /// Simplifies the polygons by this many map units (0: exact outlines).
    pub simplify: f64,
    /// Height, in the units of the canopy height model, from which a cell counts as covered by
    /// canopy (for the `cobertura` attribute, used by the strata).
    pub cover_height: f64,
    /// Join the classes into strata too.
    pub strata: Option<StrataOpts>,
}

/// The files a job writes.
pub struct Outputs {
    /// Polygons, when vector output is on.
    pub polygons: Option<PathBuf>,
    pub table: Option<PathBuf>,
    /// The object number of every cell.
    pub objects: PathBuf,
    /// The class of every cell, when objects are classified.
    pub classes: Option<PathBuf>,
    /// The strata as polygons, their summary table and the class of every cell.
    pub strata_polygons: Option<PathBuf>,
    pub strata_table: Option<PathBuf>,
    pub strata_classes: Option<PathBuf>,
}

impl ObiaJob {
    pub fn outputs(&self) -> Outputs {
        let f = self.formats.fixed();
        let raster = |tag: &str| {
            let base = paths::swap_ext(&self.output, "tif");
            let stem = base
                .file_stem()
                .map_or(String::new(), |s| s.to_string_lossy().into_owned());
            base.with_file_name(format!("{stem}_{tag}.tif"))
        };
        Outputs {
            polygons: f.vector.then(|| paths::swap_ext(&self.output, "shp")),
            table: f.csv.then(|| paths::swap_ext(&self.output, "csv")),
            objects: raster("objetos"),
            classes: (self.classify != ClassifyBy::Nothing).then(|| raster("clases")),
            strata_polygons: (self.strata.is_some() && f.vector).then(|| {
                let b = paths::swap_ext(&self.output, "shp");
                let stem = b
                    .file_stem()
                    .map_or(String::new(), |s| s.to_string_lossy().into_owned());
                b.with_file_name(format!("{stem}_estratos.shp"))
            }),
            strata_table: self.strata.is_some().then(|| {
                let b = paths::swap_ext(&self.output, "csv");
                let stem = b
                    .file_stem()
                    .map_or(String::new(), |s| s.to_string_lossy().into_owned());
                b.with_file_name(format!("{stem}_estratos.csv"))
            }),
            strata_classes: self.strata.is_some().then(|| raster("estratos")),
        }
    }
}

pub fn obia(j: &ObiaJob) -> Result<String> {
    let first = j.rasters.first().ok_or_else(|| {
        Error::Param("hace falta al menos un ráster (el modelo de alturas)".into())
    })?;
    // everything that is combined must be in one coordinate system
    let mut inputs: Vec<PathBuf> = j.rasters.clone();
    if !j.cloud_layers.is_empty() {
        inputs.extend(j.clouds.iter().cloned());
    }
    if let ClassifyBy::Samples { file, .. } = &j.classify {
        inputs.push(file.clone());
    }
    let unified = crate::crs::unify_like(&inputs, true, Some(first))?;
    let crs_warnings = unified.warnings.clone();
    let rasters = unified.all(&j.rasters);
    let clouds = unified.all(&j.clouds);
    let g = Grid::read(first)?;
    let lat = Lattice::of(&g);
    let epsg = Grid::read_epsg(first);
    let mut stack = LayerStack::new(lat, epsg);
    stack.push_grid(&paths::stem(first), &g)?;
    drop(g);
    for r in rasters.iter().skip(1) {
        // an image of several bands (an orthophoto, a satellite image) gives a layer per band
        if crate::pyramid::Pyramid::open(r).is_ok_and(|p| p.bands > 1) {
            stack.push_image(&paths::stem(r), &crate::raster::read_image(r)?)?;
        } else {
            stack.push_grid(&paths::stem(r), &Grid::read(r)?)?;
        }
    }
    if !clouds.is_empty() && !j.cloud_layers.is_empty() {
        for l in cloud_layers(&clouds, &lat, &j.cloud_layers, &j.filter)? {
            stack.push(&l.name, l.data)?;
        }
    }
    if j.roughness > 0 {
        let rough = local_std(&lat, &stack.layers[0].data, j.roughness);
        stack.push("rugosidad", rough)?;
    }
    check_cancel()?;
    let labels = segment(&stack, &j.segment)?;
    if labels.count == 0 {
        return Err(Error::Param(
            "la segmentación no ha encontrado ningún objeto: revisa el rango de valores o la escala"
                .into(),
        ));
    }
    if j.strata.is_some() && j.classify == ClassifyBy::Nothing {
        return Err(Error::Param(
            "los estratos salen de una clasificación: elige reglas, muestras o agrupar por parecido".into(),
        ));
    }
    let canopy =
        (j.strata.is_some() || matches!(j.classify, ClassifyBy::Kmeans { .. })).then_some(Canopy {
            cover_height: j.cover_height as f32,
        });
    let table = attributes_with(&labels, &stack, &[], canopy)?;
    let classified: Option<Classified> = match &j.classify {
        ClassifyBy::Nothing => None,
        ClassifyBy::Rules(file) => {
            let rules = classify::parse_rules(&std::fs::read_to_string(file)?)?;
            Some(classify::classify_rules(&table, &rules)?)
        }
        ClassifyBy::Kmeans { k, features, seed } => {
            Some(classify::classify_kmeans(&table, *k, features, *seed)?)
        }
        ClassifyBy::Samples {
            file,
            field,
            forest,
        } => {
            let opts = ReadOptions {
                name_field: Some(field.clone()),
                ..Default::default()
            };
            let regions = unified.regions(&read_regions(file, &opts)?, Some(file))?;
            Some(classify::classify_samples(
                &labels, &table, &regions, forest,
            )?)
        }
    };
    check_cancel()?;
    paths::ensure_parent(&j.output)?;
    let mut lines = vec![format!(
        "{} objetos sobre {} celdas ({} capas: {})",
        labels.count,
        lat.len(),
        stack.layers.len(),
        stack
            .layers
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )];
    let out = j.outputs();
    let ids = out.objects.clone();
    labels.write_tiff(&ids, epsg)?;
    lines.push(format!("  objetos (ráster): {}", ids.display()));
    if let Some(path) = &out.polygons {
        let n = write_polygons(path, &labels, &table, classified.as_ref(), j.simplify, epsg)?;
        lines.push(format!("  objetos (polígonos): {} ({n})", path.display()));
    }
    if let Some(path) = &out.table {
        table.write_csv(path)?;
        lines.push(format!("  atributos (tabla): {}", path.display()));
    }
    if let Some(c) = &classified {
        let per_cell: Vec<u16> = labels
            .ids
            .iter()
            .map(|&id| if id == 0 { 0 } else { c.class[id as usize - 1] })
            .collect();
        let path = out.classes.clone().unwrap_or_else(|| j.output.clone());
        write_classes_tiff(&path, &lat, epsg, &per_cell)?;
        lines.push(format!("  clases (ráster): {}", path.display()));
        let legend: Vec<String> = c
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{}={n}", i + 1))
            .collect();
        lines.push(format!("  códigos de clase: {}", legend.join(", ")));
        lines.push(c.report.clone());
        if let Some(opts) = &j.strata {
            lines.extend(write_strata(j, &out, &labels, &stack, c, opts, epsg)?);
        }
    }
    Ok(lines.join("\n") + &crate::crs::notes(&crs_warnings))
}

/// Joins the classes into strata and writes them; returns the lines that say so.
fn write_strata(
    j: &ObiaJob,
    out: &Outputs,
    labels: &super::Labels,
    stack: &LayerStack,
    c: &Classified,
    opts: &StrataOpts,
    epsg: Option<u16>,
) -> Result<Vec<String>> {
    let st = strata::dissolve(labels, &c.class, opts.min_cells)?;
    let canopy = Canopy {
        cover_height: j.cover_height as f32,
    };
    let table = attributes_with(&st.labels, stack, &[], Some(canopy))?;
    let stats = strata::summarize(
        &st.cell_class,
        &stack.layers[0].data,
        &st.class,
        c.names.len(),
    );
    let plots = strata::allocate(&stats, opts.plots);
    let cell_ha = st.labels.lat.cell * st.labels.lat.cell / 10_000.0;
    let mut lines = vec![format!(
        "{} estratos (áreas continuas de una misma clase, de al menos {} celdas cuando es posible):",
        st.labels.count, opts.min_cells
    )];
    let layer = &stack.layers[0].name;
    let mut csv = format!("clase,poligonos,area_ha,{layer}_media,{layer}_desv,cv_pct,parcelas\n");
    for (s, (name, n_plots)) in stats.iter().zip(c.names.iter().zip(&plots)) {
        if s.cells == 0 {
            continue;
        }
        let area = s.cells as f64 * cell_ha;
        csv.push_str(&format!(
            "{name},{},{area:.2},{:.3},{:.3},{:.1},{n_plots}\n",
            s.polygons,
            s.mean,
            s.sd,
            s.cv()
        ));
        lines.push(format!(
            "  {name}: {} áreas, {area:.1} ha, {layer} {:.2} ± {:.2} (CV {:.0} %){}",
            s.polygons,
            s.mean,
            s.sd,
            s.cv(),
            if opts.plots > 0 {
                format!(", {n_plots} parcelas")
            } else {
                String::new()
            }
        ));
    }
    if let Some(path) = &out.strata_table {
        std::fs::write(path, csv)?;
        lines.push(format!("  resumen de estratos: {}", path.display()));
    }
    if let Some(path) = &out.strata_polygons {
        let classified = Classified {
            class: st.class.clone(),
            confidence: vec![1.0; st.class.len()],
            names: c.names.clone(),
            report: String::new(),
        };
        let n = write_polygons(
            path,
            &st.labels,
            &table,
            Some(&classified),
            j.simplify,
            epsg,
        )?;
        lines.push(format!("  estratos (polígonos): {} ({n})", path.display()));
    }
    if let Some(path) = &out.strata_classes {
        write_classes_tiff(path, &st.labels.lat, epsg, &st.cell_class)?;
        lines.push(format!("  estratos (ráster): {}", path.display()));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obia::segment::{MergeParams, Method};

    #[test]
    fn rasters_to_classified_polygons() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_obia_job_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // a canopy height model: a tall stand on the left, low scrub on the right, noise on top
        let (cols, rows) = (60usize, 40usize);
        let mut chm = Grid::new(500.0, 800.0, 1.0, cols, rows)?;
        for (i, v) in chm.data.iter_mut().enumerate() {
            let (x, y) = (i % cols, i / cols);
            let noise = (((x * 31 + y * 17) % 7) as f32 - 3.0) * 0.05;
            *v = if x < 30 { 15.0 } else { 1.5 } + noise;
        }
        let path = dir.join("chm.tif");
        chm.write_tiff(&path, Some(25830))?;
        let rules = dir.join("reglas.txt");
        std::fs::write(
            &rules,
            "arbolado: chm_media >= 8\nmatorral: chm_media >= 0.5\n",
        )?;
        let job = ObiaJob {
            rasters: vec![path],
            clouds: Vec::new(),
            cloud_layers: Vec::new(),
            filter: Filter::default(),
            roughness: 2,
            segment: SegmentParams {
                method: Method::Merge(MergeParams {
                    scale: 8.0,
                    weights: vec![("rugosidad".into(), 0.0)],
                    ..Default::default()
                }),
                min_cells: 10,
            },
            classify: ClassifyBy::Rules(rules),
            output: dir.join("resultado.shp"),
            formats: Formats {
                vector: true,
                csv: true,
            },
            simplify: 0.0,
            cover_height: 2.0,
            strata: None,
        };
        let text = obia(&job)?;
        assert!(text.contains("2 objetos"), "{text}");
        assert!(
            text.contains("arbolado: 1") && text.contains("matorral: 1"),
            "{text}"
        );
        let csv = std::fs::read_to_string(dir.join("resultado.csv"))?;
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.lines().next().is_some_and(|h| h.contains("chm_media")));
        let ids = super::super::Labels::read_tiff(&dir.join("resultado_objetos.tif"))?;
        assert_eq!(ids.count, 2);
        assert!(dir.join("resultado.shp").exists());
        assert!(dir.join("resultado_clases.tif").exists());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod crs_tests {
    use super::*;
    use crate::obia::segment::{MergeParams, Method};

    fn job(rasters: Vec<PathBuf>, out: PathBuf) -> ObiaJob {
        ObiaJob {
            rasters,
            clouds: Vec::new(),
            cloud_layers: Vec::new(),
            filter: Filter::default(),
            roughness: 0,
            segment: SegmentParams {
                method: Method::Merge(MergeParams::default()),
                min_cells: 4,
            },
            classify: ClassifyBy::Nothing,
            output: out,
            formats: Formats {
                vector: false,
                csv: true,
            },
            simplify: 0.0,
            cover_height: 2.0,
            strata: None,
        }
    }

    #[test]
    fn layers_in_different_systems_are_moved_onto_the_first() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_obia_crs_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut g = Grid::new(500_000.0, 4_500_000.0, 1.0, 20, 20)?;
        for (i, v) in g.data.iter_mut().enumerate() {
            *v = (i % 20) as f32;
        }
        g.write_tiff(&dir.join("chm.tif"), Some(25830))?;
        g.write_tiff(&dir.join("misma.tif"), Some(25830))?;
        // the same ground in UTM 29N
        crate::reproject::grid(&g, 25830, 25829)?.write_tiff(&dir.join("otra.tif"), Some(25829))?;
        let moved = obia(&job(
            vec![dir.join("chm.tif"), dir.join("otra.tif")],
            dir.join("r.shp"),
        ))?;
        assert!(
            moved.contains("otra.tif") && moved.contains("EPSG:25829"),
            "{moved}"
        );
        let ok = obia(&job(
            vec![dir.join("chm.tif"), dir.join("misma.tif")],
            dir.join("r.shp"),
        ))?;
        assert!(!ok.contains("Aviso"), "{ok}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod strata_tests {
    use super::*;
    use crate::obia::segment::{MergeParams, Method};

    #[test]
    fn strata_from_kmeans_with_plots() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("rl_obia_strata_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        // three stands side by side: tall, medium, low
        let (cols, rows) = (90usize, 40usize);
        let mut chm = Grid::new(500.0, 800.0, 1.0, cols, rows)?;
        for (i, v) in chm.data.iter_mut().enumerate() {
            let (x, y) = (i % cols, i / cols);
            let noise = (((x * 31 + y * 17) % 7) as f32 - 3.0) * 0.1;
            *v = [18.0, 8.0, 1.0][x / 30] + noise;
        }
        let path = dir.join("chm.tif");
        chm.write_tiff(&path, Some(25830))?;
        let job = ObiaJob {
            rasters: vec![path],
            clouds: Vec::new(),
            cloud_layers: Vec::new(),
            filter: Filter::default(),
            roughness: 0,
            segment: SegmentParams {
                method: Method::Merge(MergeParams {
                    scale: 6.0,
                    ..Default::default()
                }),
                min_cells: 10,
            },
            classify: ClassifyBy::Kmeans {
                k: 3,
                features: Vec::new(),
                seed: 1,
            },
            output: dir.join("est.shp"),
            formats: Formats {
                vector: true,
                csv: false,
            },
            simplify: 0.0,
            cover_height: 2.0,
            strata: Some(StrataOpts {
                min_cells: 50,
                plots: 12,
            }),
        };
        let text = obia(&job)?;
        assert!(text.contains("3 estratos"), "{text}");
        let table = std::fs::read_to_string(dir.join("est_estratos.csv"))?;
        let rows: Vec<&str> = table.lines().collect();
        assert_eq!(rows.len(), 4, "{table}");
        let plots: usize = rows[1..]
            .iter()
            .filter_map(|r| r.rsplit(',').next()?.parse::<usize>().ok())
            .sum();
        assert_eq!(plots, 12);
        assert!(dir.join("est_estratos.shp").exists());
        assert!(dir.join("est_estratos.tif").exists());
        // strata need a classification
        let none = ObiaJob {
            classify: ClassifyBy::Nothing,
            ..job
        };
        assert!(obia(&none).is_err());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

/// Timing on a large synthetic canopy: `cargo test -p rustlidar-core --release obia::job::bench -- --ignored --nocapture`
/// (set `RL_OBIA_CELLS=20000000` to change the size).
#[cfg(test)]
mod bench {
    use super::*;
    use crate::obia::attributes::attributes;
    use crate::obia::segment::{MergeParams, Method, segment};
    use crate::obia::vectorize::Tracer;
    use std::time::Instant;

    #[test]
    #[ignore]
    fn timings() -> Result<()> {
        let want: usize = std::env::var("RL_OBIA_CELLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000_000);
        let cols = (want as f64 * 1.25).sqrt() as usize;
        let rows = want / cols;
        let lat = Lattice {
            x0: 0.0,
            y0: 0.0,
            cell: 0.5,
            cols,
            rows,
        };
        // crowns of about 5 m: a smooth bump per 10 x 10 cells plus noise
        let data: Vec<f32> = (0..cols * rows)
            .map(|i| {
                let (x, y) = ((i % cols) as f32, (i / cols) as f32);
                let (u, v) = ((x % 10.0) - 5.0, (y % 10.0) - 5.0);
                let seed = ((i % cols / 10) * 7919 + (i / cols / 10) * 104_729) % 17;
                let h = 8.0 + seed as f32;
                (h - 0.5 * (u * u + v * v).sqrt() * (h / 8.0)
                    + (((i * 2_654_435_761) >> 7) % 100) as f32 / 400.0)
                    .max(0.0)
            })
            .collect();
        let mut stack = LayerStack::new(lat, None);
        stack.push("chm", data)?;
        let t = Instant::now();
        let rough = crate::obia::layers::local_std(&lat, &stack.layers[0].data, 2);
        println!("rugosidad: {:.2?}", t.elapsed());
        stack.push("rugosidad", rough)?;
        let t = Instant::now();
        let labels = segment(
            &stack,
            &SegmentParams {
                method: Method::Merge(MergeParams {
                    scale: 8.0,
                    ..Default::default()
                }),
                min_cells: 20,
            },
        )?;
        println!(
            "{} celdas -> {} objetos: segmentación {:.2?}",
            cols * rows,
            labels.count,
            t.elapsed()
        );
        let t = Instant::now();
        let table = attributes(&labels, &stack, &[])?;
        println!(
            "atributos: {:.2?} ({} columnas)",
            t.elapsed(),
            table.cols.len()
        );
        let t = Instant::now();
        let tracer = Tracer::new(&labels)?;
        println!("índice de bordes: {:.2?}", t.elapsed());
        let t = Instant::now();
        let n: usize = (1..=labels.count.min(20_000))
            .map(|id| tracer.outline(id, 0.0).outers.len())
            .sum();
        println!(
            "contornos de {} objetos: {:.2?} ({n} anillos)",
            labels.count.min(20_000),
            t.elapsed()
        );
        Ok(())
    }
}
