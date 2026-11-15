//! Recetas de clasificación: una clasificación entera (ruido, suelo, edificios, vegetación, cables,
//! postes, reglas y teselas) escrita como archivo de texto y leída de vuelta.
//!
//! Una receta son las opciones de `hipso classify`, una o varias por línea, de modo que el archivo
//! se puede dar a la línea de comandos (`--recipe`) y la ventana de clasificación puede guardar y
//! cargar el mismo archivo. Solo se escriben las opciones que tienen nombre en la línea de
//! comandos; los parámetros avanzados que no lo tienen (las alturas de los estratos de vegetación,
//! la celda del análisis de edificios...) no, y [`is_exact`] dice si una receta pierde alguno.

use std::collections::HashMap;

use crate::axelsson::AxelssonParams;
use crate::buildings::BuildingParams;
use crate::csf::CsfParams;
use crate::extras::{Extras, PoleParams, RoadParams, VegParams, WaterParams, WireParams};
use crate::ground::GroundParams;
use crate::mcc::MccParams;
use crate::noise::NoiseParams;
use crate::pipeline::{Ground, Noise, Steps, Tiling};
use crate::pmf::PmfParams;
use crate::rules::{self, ClassRule};
use crate::smrf::SmrfParams;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub steps: Steps,
    pub tiling: Tiling,
}

/// Una receta leída de texto, y las opciones del texto que la clasificación no usa.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub recipe: Recipe,
    pub ignored: Vec<String>,
}

/// Opciones que van solas, sin valor.
const SWITCHES: [&str; 7] = [
    "overlap",
    "vegetation",
    "wires",
    "poles",
    "roads",
    "water",
    "buildings",
];

fn number(key: &str, value: &str) -> Result<f64> {
    value
        .trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| Error::Param(format!("receta: --{key} necesita un número, no '{value}'")))
}

fn count<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value.trim().parse::<T>().map_err(|_| {
        Error::Param(format!(
            "receta: --{key} necesita un número entero, no '{value}'"
        ))
    })
}

/// Lee una receta.
pub fn parse(text: &str) -> Result<Loaded> {
    let tokens = rules::recipe_args(text)?;
    let mut options: HashMap<String, String> = HashMap::new();
    let mut rule_texts: Vec<String> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        let Some(body) = token.strip_prefix("--") else {
            ignored.push(token.clone());
            continue;
        };
        let (key, inline) = match body.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (body.to_string(), None),
        };
        let value = if SWITCHES.contains(&key.as_str()) {
            Some(String::new())
        } else if inline.is_some() {
            inline
        } else if i < tokens.len() && !tokens[i].starts_with("--") {
            i += 1;
            Some(tokens[i - 1].clone())
        } else {
            None
        };
        let known = KNOWN.contains(&key.as_str());
        match (known, value) {
            (true, Some(v)) if key == "rule" => rule_texts.push(v),
            (true, Some(v)) => {
                options.insert(key, v);
            }
            (true, None) => {
                return Err(Error::Param(format!("receta: a --{key} le falta el valor")));
            }
            (false, _) => ignored.push(format!("--{key}")),
        }
    }
    let get = |key: &str, default: f64| -> Result<f64> {
        options.get(key).map_or(Ok(default), |v| number(key, v))
    };

    let d = GroundParams::default();
    let ground = match options.get("ground").map_or("kraus", String::as_str) {
        "kraus" | "fusion" => Ground::Fusion(GroundParams {
            filter_cell: get("filter-cell", d.filter_cell)?,
            g: get("g", d.g)?,
            w: get("w", d.w)?,
            a: get("a", d.a)?,
            b: get("b", d.b)?,
            iterations: options
                .get("iterations")
                .map_or(Ok(d.iterations), |v| count("iterations", v))?,
            tolerance: options
                .get("tolerance")
                .map(|v| number("tolerance", v))
                .transpose()?,
        }),
        "mcc" => {
            let m = MccParams::default();
            Ground::Mcc(MccParams {
                spacing: get("mcc-spacing", m.spacing)?,
                threshold: get("mcc-threshold", m.threshold)?,
            })
        }
        "csf" => {
            let c = CsfParams::default();
            Ground::Csf(CsfParams {
                resolution: get("csf-resolution", c.resolution)?,
                rigidness: options
                    .get("csf-rigidness")
                    .map_or(Ok(c.rigidness), |v| count("csf-rigidness", v))?,
                threshold: get("csf-threshold", c.threshold)?,
                ..c
            })
        }
        "pmf" => {
            let p = PmfParams::default();
            Ground::Pmf(PmfParams {
                max_window: get("pmf-window", p.max_window)?,
                slope: get("pmf-slope", p.slope)?,
                ..p
            })
        }
        "smrf" => {
            let p = SmrfParams::default();
            Ground::Smrf(SmrfParams {
                cell: get("smrf-cell", p.cell)?,
                max_window: get("smrf-window", p.max_window)?,
                slope: get("smrf-slope", p.slope)?,
                threshold: get("smrf-threshold", p.threshold)?,
                scalar: get("smrf-scalar", p.scalar)?,
            })
        }
        "axelsson" => {
            let p = AxelssonParams::default();
            Ground::Axelsson(AxelssonParams {
                seed_cell: get("axelsson-seed-cell", p.seed_cell)?,
                iter_angle: get("axelsson-angle", p.iter_angle)?,
                iter_dist: get("axelsson-dist", p.iter_dist)?,
                flat_tolerance: get("axelsson-flat", p.flat_tolerance)?,
                ..p
            })
        }
        "classes" => Ground::Classes,
        other => {
            return Err(Error::Param(format!(
                "receta: método de suelo desconocido '{other}' (kraus, mcc, csf, pmf, smrf, axelsson o classes)"
            )));
        }
    };

    let noise = match options.get("noise").map(String::as_str) {
        None => None,
        Some("isolated") => {
            let n = NoiseParams::default();
            Some(Noise::Isolated(NoiseParams {
                step_xy: get("noise-step", n.step_xy)?,
                step_z: get("noise-step-z", n.step_z)?,
                isolated: options
                    .get("isolated")
                    .map_or(Ok(n.isolated), |v| count("isolated", v))?,
            }))
        }
        Some("sor") => Some(Noise::Sor {
            k: options.get("sor-k").map_or(Ok(10), |v| count("sor-k", v))?,
            m: get("sor-m", 3.0)?,
        }),
        Some("elm") => Some(Noise::Elm {
            cell: get("elm-cell", 10.0)?,
            threshold: get("elm-threshold", 3.0)?,
        }),
        Some(other) => {
            return Err(Error::Param(format!(
                "receta: método de ruido desconocido '{other}' (isolated, sor o elm)"
            )));
        }
    };

    let b = BuildingParams::default();
    let buildings = if options.contains_key("buildings") {
        Some(BuildingParams {
            offset: get("building-offset", b.offset)?,
            window: get("building-window", b.window)?,
            min_area: get("building-area", b.min_area)?,
            ..b
        })
    } else {
        None
    };
    let extras = Extras {
        vegetation: options.contains_key("vegetation").then(VegParams::default),
        wires: options.contains_key("wires").then(WireParams::default),
        poles: options.contains_key("poles").then(PoleParams::default),
        roads: options.contains_key("roads").then(RoadParams::default),
        water: options.contains_key("water").then(WaterParams::default),
    };
    let rules = rule_texts
        .iter()
        .map(|t| ClassRule::parse(t))
        .collect::<Result<Vec<_>>>()?;
    let t = Tiling::default();
    let tiling = Tiling {
        size: get("tile", t.size)?,
        buffer: get("buffer", t.buffer)?,
        threads: options
            .get("threads")
            .map_or(Ok(t.threads), |v| count("threads", v))?,
    };
    Ok(Loaded {
        recipe: Recipe {
            steps: Steps {
                overlap: if options.contains_key("overlap") {
                    Some(get("overlap-cell", 5.0)?)
                } else {
                    None
                },
                noise,
                ground,
                buildings,
                extras,
                rules,
            },
            tiling,
        },
        ignored,
    })
}

/// Toda opción que lee una receta.
const KNOWN: [&str; 48] = [
    "ground",
    "filter-cell",
    "iterations",
    "g",
    "w",
    "a",
    "b",
    "tolerance",
    "mcc-spacing",
    "mcc-threshold",
    "csf-resolution",
    "csf-rigidness",
    "csf-threshold",
    "pmf-window",
    "pmf-slope",
    "smrf-cell",
    "smrf-window",
    "smrf-slope",
    "smrf-threshold",
    "smrf-scalar",
    "axelsson-seed-cell",
    "axelsson-angle",
    "axelsson-dist",
    "axelsson-flat",
    "noise",
    "sor-k",
    "sor-m",
    "noise-step",
    "noise-step-z",
    "elm-cell",
    "elm-threshold",
    "isolated",
    "vegetation",
    "wires",
    "poles",
    "roads",
    "water",
    "buildings",
    "building-offset",
    "building-window",
    "building-area",
    "tile",
    "buffer",
    "threads",
    "rule",
    "recipe",
    "overlap",
    "overlap-cell",
];

/// Una receta como texto, con una línea de comentario por cada paso.
pub fn write(r: &Recipe) -> String {
    let s = &r.steps;
    let mut o = String::from(
        "# Receta de clasificación de HIPSO: se aplica con `hipso classify nube.laz --recipe receta.txt`\n",
    );
    if let Some(cell) = s.overlap {
        o.push_str(&format!(
            "# solape entre pasadas (clase 12)\n--overlap\n--overlap-cell {cell}\n"
        ));
    }
    if let Some(n) = &s.noise {
        o.push_str("# ruido\n");
        match n {
            Noise::Isolated(p) => o.push_str(&format!(
                "--noise isolated\n--noise-step {}\n--noise-step-z {}\n--isolated {}\n",
                p.step_xy, p.step_z, p.isolated
            )),
            Noise::Sor { k, m } => o.push_str(&format!("--noise sor\n--sor-k {k}\n--sor-m {m}\n")),
            Noise::Elm { cell, threshold } => o.push_str(&format!(
                "--noise elm\n--elm-cell {cell}\n--elm-threshold {threshold}\n"
            )),
        }
    }
    o.push_str("# suelo\n");
    match &s.ground {
        Ground::Fusion(p) => {
            o.push_str(&format!(
                "--ground kraus\n--filter-cell {}\n--iterations {}\n--g {}\n--w {}\n--a {}\n--b {}\n",
                p.filter_cell, p.iterations, p.g, p.w, p.a, p.b
            ));
            if let Some(t) = p.tolerance {
                o.push_str(&format!("--tolerance {t}\n"));
            }
        }
        Ground::Mcc(p) => o.push_str(&format!(
            "--ground mcc\n--mcc-spacing {}\n--mcc-threshold {}\n",
            p.spacing, p.threshold
        )),
        Ground::Csf(p) => o.push_str(&format!(
            "--ground csf\n--csf-resolution {}\n--csf-rigidness {}\n--csf-threshold {}\n",
            p.resolution, p.rigidness, p.threshold
        )),
        Ground::Pmf(p) => o.push_str(&format!(
            "--ground pmf\n--pmf-window {}\n--pmf-slope {}\n",
            p.max_window, p.slope
        )),
        Ground::Smrf(p) => o.push_str(&format!(
            "--ground smrf\n--smrf-cell {}\n--smrf-window {}\n--smrf-slope {}\n--smrf-threshold {}\n--smrf-scalar {}\n",
            p.cell, p.max_window, p.slope, p.threshold, p.scalar
        )),
        Ground::Axelsson(p) => o.push_str(&format!(
            "--ground axelsson\n--axelsson-seed-cell {}\n--axelsson-angle {}\n--axelsson-dist {}\n--axelsson-flat {}\n",
            p.seed_cell, p.iter_angle, p.iter_dist, p.flat_tolerance
        )),
        Ground::Classes => o.push_str("--ground classes\n"),
    }
    if let Some(b) = &s.buildings {
        o.push_str(&format!(
            "# edificios\n--buildings\n--building-offset {}\n--building-window {}\n--building-area {}\n",
            b.offset, b.window, b.min_area
        ));
    }
    if s.extras.any() {
        o.push_str("# vegetación por alturas, cables, postes, carreteras y agua\n");
        if s.extras.vegetation.is_some() {
            o.push_str("--vegetation\n");
        }
        if s.extras.wires.is_some() {
            o.push_str("--wires\n");
        }
        if s.extras.poles.is_some() {
            o.push_str("--poles\n");
        }
        if s.extras.roads.is_some() {
            o.push_str("--roads\n");
        }
        if s.extras.water.is_some() {
            o.push_str("--water\n");
        }
    }
    if !s.rules.is_empty() {
        o.push_str("# reglas, en orden\n");
        for rule in &s.rules {
            o.push_str(&format!("--rule \"{}\"\n", rule.to_arg()));
        }
    }
    o.push_str(&format!(
        "# teselas\n--tile {}\n--buffer {}\n--threads {}\n",
        r.tiling.size, r.tiling.buffer, r.tiling.threads
    ));
    o
}

/// Si escribir la receta y leerla de vuelta da la misma receta: falso cuando se ha cambiado de su
/// valor por defecto un parámetro que no tiene opción de línea de comandos.
pub fn is_exact(r: &Recipe) -> bool {
    parse(&write(r)).is_ok_and(|l| &l.recipe == r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich() -> Recipe {
        Recipe {
            steps: Steps {
                overlap: Some(4.0),
                noise: Some(Noise::Elm {
                    cell: 12.0,
                    threshold: 2.5,
                }),
                ground: Ground::Smrf(SmrfParams {
                    cell: 0.5,
                    slope: 0.3,
                    ..SmrfParams::default()
                }),
                buildings: Some(BuildingParams {
                    offset: 2.5,
                    window: 40.0,
                    min_area: 30.0,
                    ..BuildingParams::default()
                }),
                extras: Extras {
                    vegetation: Some(VegParams::default()),
                    wires: Some(WireParams::default()),
                    poles: None,
                    roads: Some(RoadParams::default()),
                    water: None,
                },
                rules: vec![
                    ClassRule::parse("from=1;height=..0.5;to=3")
                        .unwrap_or_else(|_| ClassRule::new(3)),
                    ClassRule::parse("angle=25;to=12").unwrap_or_else(|_| ClassRule::new(12)),
                    ClassRule::parse("height=2..;slope=60..;planarity=0.7..;radius=2;to=6")
                        .unwrap_or_else(|_| ClassRule::new(6)),
                ],
            },
            tiling: Tiling {
                size: 200.0,
                buffer: 40.0,
                threads: 4,
            },
        }
    }

    #[test]
    fn a_recipe_written_and_read_back_is_the_same() -> Result<()> {
        let r = rich();
        assert!(is_exact(&r));
        let l = parse(&write(&r))?;
        assert_eq!(l.recipe, r);
        assert!(l.ignored.is_empty());
        // todo método de suelo y de ruido
        for ground in [
            Ground::Fusion(GroundParams {
                tolerance: Some(0.4),
                ..GroundParams::default()
            }),
            Ground::Mcc(MccParams::default()),
            Ground::Csf(CsfParams::default()),
            Ground::Pmf(PmfParams::default()),
            Ground::Axelsson(AxelssonParams::default()),
            Ground::Classes,
        ] {
            let r = Recipe {
                steps: Steps {
                    overlap: None,
                    noise: Some(Noise::Sor { k: 8, m: 2.5 }),
                    ground,
                    buildings: None,
                    extras: Extras::default(),
                    rules: Vec::new(),
                },
                tiling: Tiling::default(),
            };
            assert_eq!(parse(&write(&r))?.recipe, r);
        }
        let iso = Recipe {
            steps: Steps {
                noise: Some(Noise::Isolated(NoiseParams::default())),
                ..rich().steps
            },
            tiling: Tiling::default(),
        };
        assert_eq!(parse(&write(&iso))?.recipe, iso);
        Ok(())
    }

    #[test]
    fn what_the_file_does_not_say_is_the_command_line_default() -> Result<()> {
        let l = parse("# only the buildings\n--buildings\n")?;
        let r = l.recipe;
        assert_eq!(r.steps.ground, Ground::Fusion(GroundParams::default()));
        assert_eq!(r.steps.buildings, Some(BuildingParams::default()));
        assert_eq!(r.steps.noise, None);
        assert_eq!(r.tiling, Tiling::default());
        Ok(())
    }

    #[test]
    fn options_that_classification_does_not_use_are_reported() -> Result<()> {
        let l = parse("--ground smrf --results-dir /tmp --mystery 3\n")?;
        assert!(matches!(l.recipe.steps.ground, Ground::Smrf(_)));
        assert_eq!(l.ignored, ["--results-dir", "--mystery"]);
        Ok(())
    }

    #[test]
    fn a_bad_value_or_method_says_what_is_wrong() {
        for bad in [
            "--ground nothing",
            "--noise nothing",
            "--ground smrf --smrf-cell abc",
            "--ground",
            "--rule \"colour=3;to=1\"",
            "--tile",
        ] {
            assert!(matches!(parse(bad), Err(Error::Param(_))), "{bad}");
        }
    }

    #[test]
    fn a_parameter_with_no_option_makes_the_recipe_inexact() {
        let mut r = rich();
        assert!(is_exact(&r));
        r.steps.extras.vegetation = Some(VegParams {
            low: 0.3,
            ..VegParams::default()
        });
        assert!(!is_exact(&r));
    }

    #[test]
    fn negative_numbers_are_values_not_options() -> Result<()> {
        let l = parse("--ground kraus\n--g -3\n")?;
        assert!(matches!(l.recipe.steps.ground, Ground::Fusion(p) if p.g == -3.0));
        Ok(())
    }
}
