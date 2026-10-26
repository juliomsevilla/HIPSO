//! The command line as a user runs it: the binary on a small synthetic cloud (a sloping ground
//! with conical trees), checking what it writes, where, and what it says.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_easylidar"))
}

/// Runs the binary; returns its standard output and error, and fails the test if it failed.
fn run(args: &[&str]) -> Result<String> {
    let out = bin().args(args).output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("easylidar {args:?} failed:\n{text}").into());
    }
    Ok(text)
}

/// A temporary folder for one test, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Result<Self> {
        let d = std::env::temp_dir().join(format!("el_cli_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d)?;
        Ok(Self(d))
    }
    fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }
    fn s(&self, p: &str) -> String {
        self.join(p).display().to_string()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A LAS 1.2 file of point format 1: 120 x 120 m of ground rising 5 cm per metre to the east,
/// a point every 0.5 m (ground, class 2), and a grid of conical trees of 12 to 20 m (class 5),
/// with GPS time and one return per point.
fn synthetic_cloud(path: &Path) -> Result<()> {
    let (scale, x0, y0): (f64, f64, f64) = (0.01, 500_000.0, 4_400_000.0);
    let mut pts: Vec<([f64; 3], u8)> = Vec::new();
    let ground = |x: f64| 700.0 + 0.05 * x;
    for i in 0..240 {
        for j in 0..240 {
            let (x, y) = (f64::from(i) * 0.5 + 0.13, f64::from(j) * 0.5 + 0.27);
            pts.push(([x, y, ground(x)], 2));
        }
    }
    for tx in 0..6 {
        for ty in 0..6 {
            let (cx, cy) = (10.0 + f64::from(tx) * 20.0, 10.0 + f64::from(ty) * 20.0);
            let h = 12.0 + f64::from((tx * 7 + ty * 3) % 9);
            for k in 0..900 {
                let a = f64::from(k) * 2.399_963;
                let r = 3.5 * (f64::from(k) / 900.0).sqrt();
                let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
                pts.push(([x, y, ground(x) + h * (1.0 - r / 3.5).max(0.05)], 5));
            }
        }
    }
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[26..35].copy_from_slice(b"EasyLiDAR");
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[104] = 1;
    h[105..107].copy_from_slice(&28u16.to_le_bytes());
    h[107..111].copy_from_slice(&(pts.len() as u32).to_le_bytes());
    h[111..115].copy_from_slice(&(pts.len() as u32).to_le_bytes());
    for k in 0..3 {
        h[131 + 8 * k..139 + 8 * k].copy_from_slice(&scale.to_le_bytes());
    }
    h[155..163].copy_from_slice(&x0.to_le_bytes());
    h[163..171].copy_from_slice(&y0.to_le_bytes());
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    let mut body = Vec::with_capacity(pts.len() * 28);
    for (n, (p, class)) in pts.iter().enumerate() {
        let abs = [p[0] + x0, p[1] + y0, p[2]];
        for k in 0..3 {
            lo[k] = lo[k].min(abs[k]);
            hi[k] = hi[k].max(abs[k]);
        }
        body.extend(((p[0] / scale).round() as i32).to_le_bytes());
        body.extend(((p[1] / scale).round() as i32).to_le_bytes());
        body.extend(((p[2] / scale).round() as i32).to_le_bytes());
        body.extend(((n % 4000) as u16).to_le_bytes());
        body.push(0x09);
        body.push(*class);
        body.push(0);
        body.push(0);
        body.extend(1u16.to_le_bytes());
        body.extend((1000.0 + n as f64 * 1e-4).to_le_bytes());
    }
    for (at, v) in [
        (179, hi[0]),
        (187, lo[0]),
        (195, hi[1]),
        (203, lo[1]),
        (211, hi[2]),
        (219, lo[2]),
    ] {
        h[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    h.extend(body);
    std::fs::write(path, h)?;
    Ok(())
}

fn files(dir: &Path) -> Result<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    Ok(v)
}

#[test]
fn every_command_has_a_description() -> Result<()> {
    let help = run(&["--help"])?;
    let commands: Vec<&str> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .collect();
    assert!(commands.len() > 30, "{help}");
    for line in commands {
        let words: Vec<&str> = line.split_whitespace().collect();
        assert!(
            words.len() >= 2,
            "a command without a description: «{line}»"
        );
    }
    Ok(())
}

#[test]
fn the_chain_of_products_goes_to_the_results_folder() -> Result<()> {
    let d = Dir::new("chain")?;
    let cloud = d.s("nube.las");
    synthetic_cloud(&d.join("nube.las"))?;
    let out = d.join("res");
    let res = out.display().to_string();
    let info = run(&["info", &cloud])?;
    assert!(info.contains("Puntos: 90000"), "{info}");
    let dtm = run(&["--results-dir", &res, "dtm", "--cell", "1", &cloud])?;
    assert!(dtm.contains("MDT de"), "{dtm}");
    assert!(dtm.contains("terminado en"), "{dtm}");
    let chm = run(&["--results-dir", &res, "chm", "--cell", "0.5", &cloud])?;
    assert!(chm.contains("CHM de"), "{chm}");
    let names = files(&out)?;
    let chm_file = names
        .iter()
        .find(|n| n.contains("_chm_") && n.ends_with(".tif"))
        .ok_or("no CHM")?;
    let dtm_file = names
        .iter()
        .find(|n| n.contains("_mdt_") && n.ends_with(".tif"))
        .ok_or("no DTM")?;
    // the tools that used to write next to their input now follow --results-dir
    let other = d.join("otros");
    let o = other.display().to_string();
    run(&[
        "--results-dir",
        &o,
        "trees",
        &out.join(chm_file).display().to_string(),
    ])?;
    run(&[
        "--results-dir",
        &o,
        "hillshade",
        &out.join(dtm_file).display().to_string(),
    ])?;
    run(&[
        "--results-dir",
        &o,
        "contour",
        &out.join(dtm_file).display().to_string(),
        "--interval",
        "1",
    ])?;
    let made = files(&other)?;
    assert!(
        made.iter()
            .any(|n| n.contains("arboles") && n.ends_with(".shp")),
        "{made:?}"
    );
    assert!(
        made.iter().any(|n| n.ends_with("_sombreado.png")),
        "{made:?}"
    );
    assert!(
        made.iter()
            .any(|n| n.contains("curvas") && n.ends_with(".shp")),
        "{made:?}"
    );
    // 36 trees were planted
    let trees = run(&[
        "--results-dir",
        &o,
        "trees",
        &out.join(chm_file).display().to_string(),
    ])?;
    let n: usize = trees
        .split_whitespace()
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| format!("no tree count in «{trees}»"))?;
    assert!((30..=42).contains(&n), "{trees}");
    Ok(())
}

#[test]
fn csf_keeps_the_trees_out_of_the_ground() -> Result<()> {
    let d = Dir::new("csf")?;
    synthetic_cloud(&d.join("nube.las"))?;
    let res = d.s("res");
    let text = run(&[
        "--results-dir",
        &res,
        "classify",
        "--ground",
        "csf",
        &d.s("nube.las"),
    ])?;
    // 57 600 ground points and 32 400 of trees: the ground found is the ground, not the crowns
    let ground: u64 = text
        .split(" de suelo")
        .next()
        .and_then(|s| s.rsplit(' ').next())
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| format!("no ground count in «{text}»"))?;
    assert!((55_000..=61_000).contains(&ground), "{text}");
    Ok(())
}

#[test]
fn copc_colour_and_intensity_round_trip() -> Result<()> {
    let d = Dir::new("new")?;
    synthetic_cloud(&d.join("nube.las"))?;
    let cloud = d.s("nube.las");
    let res = d.s("res");
    // COPC: format 1 becomes 6, every point is kept
    let c = run(&["--results-dir", &res, "copc", &cloud])?;
    assert!(c.contains("90000 puntos en"), "{c}");
    assert!(c.contains("formato 6"), "{c}");
    let copc = d.join("res").join("nube.copc.laz");
    let info = run(&["info", &copc.display().to_string()])?;
    assert!(info.contains("Puntos: 90000"), "{info}");
    // an RGB image from the DTM stacked three times, then the colour of the points
    run(&["--results-dir", &res, "dtm", "--cell", "1", &cloud])?;
    let dtm = files(&d.join("res"))?
        .into_iter()
        .find(|n| n.contains("_mdt_") && n.ends_with(".tif"))
        .ok_or("no DTM")?;
    let dtm = d.join("res").join(dtm).display().to_string();
    let rgb = d.s("rgb.tif");
    let st = run(&[
        "stack",
        &dtm,
        &dtm,
        &dtm,
        "-o",
        &rgb,
        "--compress",
        "deflate",
    ])?;
    assert!(st.contains("3 bandas"), "{st}");
    let col = run(&["--results-dir", &res, "colorize", &cloud, "--image", &rgb])?;
    assert!(col.contains("puntos coloreados"), "{col}");
    let coloured = d.join("res").join("nube_rgb.las");
    let info = run(&["info", &coloured.display().to_string()])?;
    assert!(info.contains("formato 7"), "{info}");
    assert!(info.contains("Color RGB: sí"), "{info}");
    // intensity for range, from a flight at 2000 m
    let int = run(&[
        "--results-dir",
        &res,
        "intensity",
        &cloud,
        "--altitude",
        "2000",
    ])?;
    assert!(int.contains("intensidades normalizadas"), "{int}");
    Ok(())
}

#[test]
fn a_model_names_its_metrics_by_short_or_long_name_and_never_overwrites() -> Result<()> {
    let d = Dir::new("model")?;
    // twelve plots with two metrics and a field volume that follows the first
    let mut plots = String::from("Parcela,Elev P90,Elev mean\n");
    let mut field = String::from("Parcela,VOL\n");
    for k in 1..=12 {
        let p90 = 8.0 + f64::from(k);
        plots.push_str(&format!("{k},{p90},{}\n", p90 / 2.0 + f64::from(k % 3)));
        field.push_str(&format!("{k},{}\n", 20.0 * p90 + f64::from((k * 7) % 5)));
    }
    std::fs::write(d.join("parcelas.csv"), plots)?;
    std::fs::write(d.join("campo.csv"), field)?;
    let res = d.s("res");
    for predictor in ["Elev P90", "EP90"] {
        let m = run(&[
            "--results-dir",
            &res,
            "model",
            "--plots",
            &d.s("parcelas.csv"),
            "--field",
            &d.s("campo.csv"),
            "--response",
            "VOL",
            "--predictors",
            predictor,
        ])?;
        assert!(m.contains("Elev P90"), "{m}");
    }
    let made = files(&d.join("res"))?;
    let reports = made.iter().filter(|n| n.ends_with("_modelo.txt")).count();
    assert_eq!(reports, 2, "{made:?}");
    // a metric that is not there says which ones are
    let out = bin()
        .args([
            "model",
            "--plots",
            &d.s("parcelas.csv"),
            "--field",
            &d.s("campo.csv"),
            "--response",
            "VOL",
            "--predictors",
            "P90",
        ])
        .output()?;
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success());
    assert!(text.contains("¿Quizá «Elev P90»"), "{text}");
    Ok(())
}

#[test]
fn damaged_files_are_reported_in_spanish_with_their_path() -> Result<()> {
    let d = Dir::new("bad")?;
    synthetic_cloud(&d.join("nube.las"))?;
    let bytes = std::fs::read(d.join("nube.las"))?;
    std::fs::write(d.join("cortada.las"), &bytes[..bytes.len() / 2])?;
    std::fs::write(d.join("vacia.las"), b"")?;
    for (file, expect) in [
        ("cortada.las", "incompleto o dañado"),
        ("vacia.las", "no es un archivo LAS legible"),
        ("no_existe.las", "no existe el archivo"),
    ] {
        let out = bin().args(["dtm", "--cell", "1", &d.s(file)]).output()?;
        let text = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(!out.status.success(), "{file}");
        assert!(text.contains(expect), "{file}: {text}");
        if file == "no_existe.las" {
            assert!(text.contains("no_existe.las"), "{text}");
        }
    }
    Ok(())
}
