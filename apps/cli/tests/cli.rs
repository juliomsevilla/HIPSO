//! La línea de comandos como la ejecuta un usuario: el binario sobre una pequeña nube sintética (un
//! suelo en pendiente con árboles cónicos), comprobando lo que escribe, dónde y lo que dice.

use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hipso"))
}

/// Ejecuta el binario; devuelve su salida estándar y de error, y hace fallar la prueba si falló.
fn run(args: &[&str]) -> Result<String> {
    let out = bin().args(args).output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("hipso {args:?} failed:\n{text}").into());
    }
    Ok(text)
}

/// Una carpeta temporal para una prueba, borrada al soltarla.
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

/// Un archivo LAS 1.2 de formato de punto 1: 120 x 120 m de suelo que sube 5 cm por metro hacia el
/// este, un punto cada 0,5 m (suelo, clase 2), y una rejilla de árboles cónicos de 12 a 20 m (clase
/// 5), con tiempo GPS y un retorno por punto.
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
    h[26..31].copy_from_slice(b"HIPSO");
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
    // las herramientas que antes escribían junto a su entrada ahora siguen --results-dir
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
    // se plantaron 36 árboles
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
    // 57 600 puntos de suelo y 32 400 de árboles: el suelo hallado es el suelo, no las copas
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
    // COPC: el formato 1 pasa a 6, se conserva cada punto
    let c = run(&["--results-dir", &res, "copc", &cloud])?;
    assert!(c.contains("90000 puntos en"), "{c}");
    assert!(c.contains("formato 6"), "{c}");
    let copc = d.join("res").join("nube.copc.laz");
    let info = run(&["info", &copc.display().to_string()])?;
    assert!(info.contains("Puntos: 90000"), "{info}");
    // una imagen RGB a partir del MDT apilado tres veces, y luego el color de los puntos
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
    // intensidad por distancia, de un vuelo a 2000 m
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
    // doce parcelas con dos métricas y un volumen de campo que sigue a la primera
    let mut plots = String::from("Muestra,Elev P90,Elev mean\n");
    let mut field = String::from("Muestra,VOL\n");
    for k in 1..=12 {
        let p90 = 8.0 + f64::from(k);
        plots.push_str(&format!("{k},{p90},{}\n", p90 / 2.0 + f64::from(k % 3)));
        field.push_str(&format!("{k},{}\n", 20.0 * p90 + f64::from((k * 7) % 5)));
    }
    std::fs::write(d.join("muestras.csv"), plots)?;
    std::fs::write(d.join("campo.csv"), field)?;
    let res = d.s("res");
    for predictor in ["Elev P90", "EP90"] {
        let m = run(&[
            "--results-dir",
            &res,
            "model",
            "--plots",
            &d.s("muestras.csv"),
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
    // una métrica que no está dice cuáles sí
    let out = bin()
        .args([
            "model",
            "--plots",
            &d.s("muestras.csv"),
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

#[test]
fn the_old_name_of_the_volume_command_still_works() -> Result<()> {
    let new = run(&["stem-volume", "--help"])?;
    let old = run(&["cubicar", "--help"])?;
    assert_eq!(new.lines().nth(1), old.lines().nth(1));
    Ok(())
}

#[test]
fn every_option_that_names_a_field_exists_and_points_at_a_file_option() -> Result<()> {
    // las ventanas dibujan estas opciones como listas con los campos de la tabla del archivo que
    // dice la tabla del modelo: si una orden cambia de nombre una opción, esta prueba lo dice
    let schema = hipso_core::model::Schema::from_json(&run(&["schema"])?)?;
    let mut seen = 0;
    for (tool, arg, src) in hipso_core::model::field_args() {
        let spec = schema.tool(tool).ok_or_else(|| format!("la orden {tool} no existe"))?;
        let a = spec.arg(arg).ok_or_else(|| format!("{tool} no tiene la opción {arg}"))?;
        assert!(!a.path && !a.flag, "{tool} {arg}: una opción de campo no es un archivo ni una casilla");
        let s = spec.arg(src.source).ok_or_else(|| format!("{tool} no tiene la opción de archivo {}", src.source))?;
        assert!(s.path, "{tool} {}: la fuente de los campos debe ser un archivo", src.source);
        seen += 1;
    }
    assert!(seen >= 20);
    Ok(())
}

#[test]
fn fields_drops_and_renames_by_the_names_the_layer_has_and_refuses_a_contradiction() -> Result<()> {
    let d = Dir::new("fields")?;
    std::fs::write(
        d.join("a.geojson"),
        r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":1,"especie":"pino","altura":12.5},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#,
    )?;
    let out = d.s("b.geojson");
    run(&["fields", &d.s("a.geojson"), "--drop", "especie", "--rename", "id=codigo", "-o", &out])?;
    let text = std::fs::read_to_string(&out)?;
    assert!(text.contains("\"codigo\"") && text.contains("\"altura\"") && !text.contains("especie"), "{text}");
    // quitar y renombrar el mismo campo: se rechaza, con el nombre del campo
    let bad = bin()
        .args(["fields", &d.s("a.geojson"), "--drop", "especie", "--rename", "especie=tipo", "-o", &d.s("c.geojson")])
        .output()?;
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("«especie» se quita y se renombra a la vez"));
    // un campo que no existe se dice
    let no = bin().args(["fields", &d.s("a.geojson"), "--drop", "nada", "-o", &d.s("e.geojson")]).output()?;
    assert!(String::from_utf8_lossy(&no.stderr).contains("no tiene el campo «nada»"));
    Ok(())
}
