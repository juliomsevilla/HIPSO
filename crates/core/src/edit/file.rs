//! Las ediciones en un archivo de texto (JSON). Como las ediciones son reglas geométricas y no listas
//! de puntos, se pueden guardar, recuperar tras un cierre inesperado y aplicar a otra nube con
//! `hipso apply-edits` sin la interfaz: el resultado es el mismo que da guardar desde el visor.
//!
//! Las coordenadas se guardan absolutas (las del mapa), no relativas al origen de un almacén, de
//! modo que valen para cualquier nube del mismo sistema de coordenadas.

use std::path::Path;

use super::{Action, Attr, AttrKind, Brush, Edit, Mode, RangeSel, Region, SelOp, ViewCut};
use crate::classes::ClassSet;
use crate::json::{Json, quote};
use crate::vector::Polygon;
use crate::view::{ProfileBox, Projector};
use crate::{Error, Result};

/// El número de versión del formato; sube cuando cambia algo que un lector antiguo no entendería.
pub const VERSION: u32 = 1;

// ---------------------------------------------------------------------------------------------
// Escritura
// ---------------------------------------------------------------------------------------------

/// Un número JSON. Lo que no es finito se acota a lo mayor que cabe: los límites abiertos de un
/// filtro valen igual.
fn num(v: f64) -> String {
    let v = if v.is_nan() {
        0.0
    } else if v.is_infinite() {
        f64::MAX.copysign(v)
    } else {
        v
    };
    format!("{v}")
}

fn pair(a: [f64; 2]) -> String {
    format!("[{},{}]", num(a[0]), num(a[1]))
}

fn rings_json(poly: &Polygon) -> String {
    let rings: Vec<String> = poly
        .rings
        .iter()
        .map(|r| format!("[{}]", r.iter().map(|p| pair(*p)).collect::<Vec<_>>().join(",")))
        .collect();
    format!("[{}]", rings.join(","))
}

fn box_json(b: &ProfileBox) -> String {
    format!(
        "{{\"a\":{},\"b\":{},\"width\":{}}}",
        pair(b.a),
        pair(b.b),
        num(b.width)
    )
}

fn brush_json(b: &Brush) -> String {
    let stamps: Vec<String> = b.stamps().iter().map(|p| pair(*p)).collect();
    format!(
        "\"half\":{},\"square\":{},\"stamps\":[{}]",
        num(b.half),
        b.square,
        stamps.join(",")
    )
}

fn kind_name(k: AttrKind) -> &'static str {
    match k {
        AttrKind::Z => "z",
        AttrKind::Intensity => "intensity",
        AttrKind::ReturnNumber => "return_number",
        AttrKind::NumReturns => "num_returns",
        AttrKind::Class => "class",
        AttrKind::Source => "source",
        AttrKind::ScanAngle => "scan_angle",
    }
}

fn kind_of(name: &str) -> Result<AttrKind> {
    AttrKind::ALL
        .into_iter()
        .find(|k| kind_name(*k) == name)
        .ok_or_else(|| Error::Format(format!("atributo desconocido «{name}» en las ediciones")))
}

fn region_json(r: &Region) -> String {
    match r {
        Region::All => "{\"type\":\"all\"}".to_string(),
        Region::Plan(p) => format!("{{\"type\":\"plan\",\"rings\":{}}}", rings_json(p)),
        Region::Profile { bx, poly } => format!(
            "{{\"type\":\"profile\",\"box\":{},\"rings\":{}}}",
            box_json(bx),
            rings_json(poly)
        ),
        Region::View(cut) => {
            let cam: Vec<String> = cut.cam.to_array().iter().map(|v| num(*v)).collect();
            let depth = cut
                .depth
                .map_or("null".to_string(), |(lo, hi)| pair([lo, hi]));
            format!(
                "{{\"type\":\"view\",\"camera\":[{}],\"rings\":{},\"depth\":{}}}",
                cam.join(","),
                rings_json(&cut.poly),
                depth
            )
        }
        Region::PlanBrush(b) => format!("{{\"type\":\"plan_brush\",{}}}", brush_json(b)),
        Region::ProfileBrush { bx, brush } => format!(
            "{{\"type\":\"profile_brush\",\"box\":{},{}}}",
            box_json(bx),
            brush_json(brush)
        ),
        Region::Side {
            bx,
            a,
            b,
            above,
            height,
        } => format!(
            "{{\"type\":\"side\",\"box\":{},\"a\":{},\"b\":{},\"above\":{},\"height\":{}}}",
            box_json(bx),
            pair(*a),
            pair(*b),
            above,
            num(*height)
        ),
        Region::Within(ops) => format!("{{\"type\":\"within\",\"ops\":{}}}", ops_json(ops)),
    }
}

fn attr_json(a: &Attr) -> String {
    let words: Vec<String> = a
        .classes
        .words()
        .iter()
        .map(|w| format!("\"{w:016x}\""))
        .collect();
    let range = |r: Option<(f64, f64)>| r.map_or("null".to_string(), |(lo, hi)| pair([lo, hi]));
    let ranges: Vec<String> = a
        .ranges
        .iter()
        .map(|r| {
            let bins: Vec<String> = r.bins.iter().map(|(lo, hi)| pair([*lo, *hi])).collect();
            format!(
                "{{\"kind\":\"{}\",\"bins\":[{}]}}",
                kind_name(r.kind),
                bins.join(",")
            )
        })
        .collect();
    format!(
        "{{\"classes\":[{}],\"returns\":{},\"z\":{},\"intensity\":{},\"ranges\":[{}]}}",
        words.join(","),
        a.returns,
        range(a.z),
        range(a.intensity.map(|(lo, hi)| (f64::from(lo), f64::from(hi)))),
        ranges.join(",")
    )
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::New => "new",
        Mode::Add => "add",
        Mode::Subtract => "subtract",
        Mode::Intersect => "intersect",
        Mode::Invert => "invert",
    }
}

fn ops_json(ops: &[SelOp]) -> String {
    let items: Vec<String> = ops
        .iter()
        .map(|o| {
            format!(
                "{{\"mode\":\"{}\",\"region\":{},\"attr\":{}}}",
                mode_name(o.mode),
                region_json(&o.region),
                attr_json(&o.attr)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Las ediciones como un documento JSON.
pub fn to_json(edits: &[Edit]) -> String {
    let items: Vec<String> = edits
        .iter()
        .map(|e| {
            let action = match e.action {
                Action::Delete => "{\"type\":\"delete\"}".to_string(),
                Action::Reclassify(c) => format!("{{\"type\":\"reclassify\",\"class\":{c}}}"),
            };
            format!(
                "{{\"label\":{},\"enabled\":{},\"action\":{},\"sel\":{}}}",
                quote(&e.label),
                e.enabled,
                action,
                ops_json(&e.sel)
            )
        })
        .collect();
    format!(
        "{{\"hipso\":\"edits\",\"version\":{VERSION},\"edits\":[{}]}}\n",
        items.join(",\n")
    )
}

/// Escribe las ediciones en `path` sin dejar nunca un archivo a medias: primero un temporal y luego
/// el cambio de nombre.
pub fn write(path: &Path, edits: &[Edit]) -> Result<()> {
    crate::paths::ensure_parent(path)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(edits))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Lectura
// ---------------------------------------------------------------------------------------------

fn bad(what: &str) -> Error {
    Error::Format(format!("ediciones: {what}"))
}

fn field<'a>(j: &'a Json, key: &str) -> Result<&'a Json> {
    j.get(key)
        .ok_or_else(|| bad(&format!("falta el campo «{key}»")))
}

fn f64_of(j: &Json, what: &str) -> Result<f64> {
    j.as_f64().ok_or_else(|| bad(&format!("«{what}» no es un número")))
}

fn str_of<'a>(j: &'a Json, what: &str) -> Result<&'a str> {
    j.as_str().ok_or_else(|| bad(&format!("«{what}» no es un texto")))
}

fn bool_of(j: &Json, what: &str) -> Result<bool> {
    match j {
        Json::Bool(b) => Ok(*b),
        _ => Err(bad(&format!("«{what}» no es verdadero o falso"))),
    }
}

fn arr_of<'a>(j: &'a Json, what: &str) -> Result<&'a [Json]> {
    j.as_array().ok_or_else(|| bad(&format!("«{what}» no es una lista")))
}

fn pair_of(j: &Json, what: &str) -> Result<[f64; 2]> {
    match arr_of(j, what)? {
        [a, b] => Ok([f64_of(a, what)?, f64_of(b, what)?]),
        _ => Err(bad(&format!("«{what}» no es un par de números"))),
    }
}

fn rings_of(j: &Json) -> Result<Polygon> {
    let mut rings = Vec::new();
    for r in arr_of(field(j, "rings")?, "rings")? {
        let mut ring = Vec::new();
        for p in arr_of(r, "rings")? {
            ring.push(pair_of(p, "rings")?);
        }
        rings.push(ring);
    }
    Ok(Polygon::new(rings))
}

fn box_of(j: &Json) -> Result<ProfileBox> {
    let b = field(j, "box")?;
    Ok(ProfileBox {
        a: pair_of(field(b, "a")?, "a")?,
        b: pair_of(field(b, "b")?, "b")?,
        width: f64_of(field(b, "width")?, "width")?,
    })
}

fn brush_of(j: &Json) -> Result<Brush> {
    let mut stamps = Vec::new();
    for p in arr_of(field(j, "stamps")?, "stamps")? {
        stamps.push(pair_of(p, "stamps")?);
    }
    Ok(Brush::from_stamps(
        stamps,
        f64_of(field(j, "half")?, "half")?,
        bool_of(field(j, "square")?, "square")?,
    ))
}

fn region_of(j: &Json) -> Result<Region> {
    let kind = str_of(field(j, "type")?, "type")?;
    Ok(match kind {
        "all" => Region::All,
        "plan" => Region::Plan(rings_of(j)?),
        "profile" => Region::Profile {
            bx: box_of(j)?,
            poly: rings_of(j)?,
        },
        "view" => {
            let cam = arr_of(field(j, "camera")?, "camera")?;
            let mut a = [0.0; 12];
            if cam.len() != a.len() {
                return Err(bad("la cámara de una selección en 3D no tiene 12 números"));
            }
            for (slot, v) in a.iter_mut().zip(cam) {
                *slot = f64_of(v, "camera")?;
            }
            let depth = match field(j, "depth")? {
                Json::Null => None,
                d => {
                    let [lo, hi] = pair_of(d, "depth")?;
                    Some((lo, hi))
                }
            };
            Region::View(ViewCut {
                cam: Projector::from_array(a),
                poly: rings_of(j)?,
                depth,
            })
        }
        "plan_brush" => Region::PlanBrush(brush_of(j)?),
        "profile_brush" => Region::ProfileBrush {
            bx: box_of(j)?,
            brush: brush_of(j)?,
        },
        "side" => Region::Side {
            bx: box_of(j)?,
            a: pair_of(field(j, "a")?, "a")?,
            b: pair_of(field(j, "b")?, "b")?,
            above: bool_of(field(j, "above")?, "above")?,
            height: f64_of(field(j, "height")?, "height")?,
        },
        "within" => Region::Within(ops_of(field(j, "ops")?)?),
        other => return Err(bad(&format!("región desconocida «{other}»"))),
    })
}

fn opt_pair(j: &Json, what: &str) -> Result<Option<[f64; 2]>> {
    match j {
        Json::Null => Ok(None),
        p => Ok(Some(pair_of(p, what)?)),
    }
}

fn attr_of(j: &Json) -> Result<Attr> {
    let words = arr_of(field(j, "classes")?, "classes")?;
    if words.len() != 4 {
        return Err(bad("el conjunto de clases no tiene cuatro palabras"));
    }
    let mut w = [0u64; 4];
    for (slot, v) in w.iter_mut().zip(words) {
        *slot = u64::from_str_radix(str_of(v, "classes")?, 16)
            .map_err(|_| bad("una palabra del conjunto de clases no es hexadecimal"))?;
    }
    let to_u16 = |v: f64| v.clamp(0.0, f64::from(u16::MAX)) as u16;
    let mut ranges = Vec::new();
    for r in arr_of(field(j, "ranges")?, "ranges")? {
        let mut bins = Vec::new();
        for b in arr_of(field(r, "bins")?, "bins")? {
            let [lo, hi] = pair_of(b, "bins")?;
            bins.push((lo, hi));
        }
        ranges.push(RangeSel {
            kind: kind_of(str_of(field(r, "kind")?, "kind")?)?,
            bins,
        });
    }
    Ok(Attr {
        classes: ClassSet::from_words(w),
        returns: f64_of(field(j, "returns")?, "returns")?.clamp(0.0, 255.0) as u8,
        z: opt_pair(field(j, "z")?, "z")?.map(|[lo, hi]| (lo, hi)),
        intensity: opt_pair(field(j, "intensity")?, "intensity")?
            .map(|[lo, hi]| (to_u16(lo), to_u16(hi))),
        ranges,
    })
}

fn mode_of(name: &str) -> Result<Mode> {
    Ok(match name {
        "new" => Mode::New,
        "add" => Mode::Add,
        "subtract" => Mode::Subtract,
        "intersect" => Mode::Intersect,
        "invert" => Mode::Invert,
        other => return Err(bad(&format!("modo de selección desconocido «{other}»"))),
    })
}

fn ops_of(j: &Json) -> Result<Vec<SelOp>> {
    let mut out = Vec::new();
    for o in arr_of(j, "sel")? {
        out.push(SelOp {
            mode: mode_of(str_of(field(o, "mode")?, "mode")?)?,
            region: region_of(field(o, "region")?)?,
            attr: attr_of(field(o, "attr")?)?,
        });
    }
    Ok(out)
}

/// Las ediciones de un documento JSON hecho por [`to_json`].
pub fn from_json(text: &str) -> Result<Vec<Edit>> {
    let doc = Json::parse(text)?;
    if doc.get("hipso").and_then(Json::as_str) != Some("edits") {
        return Err(bad("no es un archivo de ediciones de HIPSO"));
    }
    let version = f64_of(field(&doc, "version")?, "version")?;
    if version > f64::from(VERSION) {
        return Err(bad(&format!(
            "el archivo es de una versión más nueva ({version}) que la que entiende este programa ({VERSION})"
        )));
    }
    let mut out = Vec::new();
    for e in arr_of(field(&doc, "edits")?, "edits")? {
        let a = field(e, "action")?;
        let action = match str_of(field(a, "type")?, "type")? {
            "delete" => Action::Delete,
            "reclassify" => {
                let c = f64_of(field(a, "class")?, "class")?;
                if !(0.0..=255.0).contains(&c) {
                    return Err(bad("una clase está fuera de 0–255"));
                }
                Action::Reclassify(c as u8)
            }
            other => return Err(bad(&format!("acción desconocida «{other}»"))),
        };
        out.push(Edit {
            sel: ops_of(field(e, "sel")?)?,
            action,
            label: str_of(field(e, "label")?, "label")?.to_string(),
            enabled: bool_of(field(e, "enabled")?, "enabled")?,
        });
    }
    Ok(out)
}

/// Lee un archivo de ediciones.
pub fn read(path: &Path) -> Result<Vec<Edit>> {
    from_json(&std::fs::read_to_string(path)?)
}

/// Dónde guarda el visor, mientras se edita, las ediciones que aún no están en una nube guardada:
/// la carpeta de la caché, para no dejar archivos junto a las nubes del usuario. Depende solo de las
/// nubes abiertas y de su orden, de modo que al abrirlas de nuevo se encuentra la misma sesión.
/// `None` cuando no hay nubes.
pub fn session_path(clouds: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let first = clouds.first()?;
    let mut h = DefaultHasher::new();
    for c in clouds {
        std::fs::canonicalize(c)
            .unwrap_or_else(|_| c.clone())
            .hash(&mut h);
    }
    Some(crate::copc_cache::cache_dir().join(format!(
        "{}_{:016x}.edits.json",
        crate::paths::stem(first),
        h.finish()
    )))
}

#[cfg(test)]
mod tests {
    use super::super::{Sample, rectangle, selected};
    use super::*;
    use crate::view::Orbit;

    fn sample(x: f64, y: f64, z: f64, class: u8) -> Sample {
        Sample {
            x,
            y,
            z,
            class,
            intensity: 100,
            ret: 0x11,
            src: 3,
            scan: 10.0,
        }
    }

    fn probe() -> Vec<Sample> {
        let mut v = Vec::new();
        for ix in 0..12 {
            for iy in 0..12 {
                for (iz, class) in [(0.0, 2u8), (4.0, 5), (9.0, 7)] {
                    v.push(sample(f64::from(ix), f64::from(iy), iz, class));
                }
            }
        }
        v
    }

    fn op(mode: Mode, region: Region, attr: Attr) -> SelOp {
        SelOp { mode, region, attr }
    }

    /// Una edición que usa todas las regiones y todos los filtros de atributos.
    fn rich() -> Vec<Edit> {
        let bx = ProfileBox {
            a: [0.0, 5.0],
            b: [11.0, 5.0],
            width: 4.0,
        };
        let cam = Orbit {
            target: [5.0, 5.0, 2.0],
            yaw: 0.3,
            pitch: 0.4,
            dist: 40.0,
            exaggeration: 1.5,
        };
        let attr = Attr {
            classes: ClassSet::only(5),
            returns: 5,
            z: Some((1.0, 8.0)),
            intensity: Some((10, 500)),
            ranges: vec![RangeSel {
                kind: AttrKind::Source,
                bins: vec![(0.0, 4.0)],
            }],
        };
        let inner = vec![op(
            Mode::New,
            Region::Plan(rectangle([2.0, 2.0], [9.0, 9.0])),
            Attr::default(),
        )];
        let sel = vec![
            op(Mode::New, Region::All, attr),
            op(
                Mode::Add,
                Region::Plan(rectangle([1.0, 1.0], [4.0, 4.0])),
                Attr::default(),
            ),
            op(
                Mode::Add,
                Region::Profile {
                    bx,
                    poly: rectangle([1.0, -1.0], [6.0, 6.0]),
                },
                Attr::default(),
            ),
            op(
                Mode::Add,
                Region::View(ViewCut::new(
                    &cam,
                    200,
                    150,
                    rectangle([50.0, 40.0], [120.0, 100.0]),
                    Some((30.0, 50.0)),
                )),
                Attr::default(),
            ),
            op(
                Mode::Add,
                Region::PlanBrush(Brush::new(&[[6.0, 6.0], [9.0, 8.0]], 1.0, false)),
                Attr::default(),
            ),
            op(
                Mode::Add,
                Region::ProfileBrush {
                    bx,
                    brush: Brush::new(&[[3.0, 4.0], [7.0, 5.0]], 0.8, true),
                },
                Attr::default(),
            ),
            op(
                Mode::Subtract,
                Region::Side {
                    bx,
                    a: [1.0, 3.0],
                    b: [9.0, 6.0],
                    above: true,
                    height: 2.0,
                },
                Attr::default(),
            ),
            op(Mode::Intersect, Region::Within(inner), Attr::default()),
            op(Mode::Invert, Region::All, Attr::default()),
        ];
        vec![
            Edit {
                sel: sel.clone(),
                action: Action::Reclassify(66),
                label: "Clase 66 «a» \"b\"\n".to_string(),
                enabled: true,
            },
            Edit {
                sel,
                action: Action::Delete,
                label: "Eliminar".to_string(),
                enabled: false,
            },
        ]
    }

    #[test]
    fn edits_survive_a_round_trip_through_json() -> Result<()> {
        let edits = rich();
        let back = from_json(&to_json(&edits))?;
        assert_eq!(back.len(), edits.len());
        let pts = probe();
        for (a, b) in edits.iter().zip(&back) {
            assert_eq!(a.action, b.action);
            assert_eq!(a.label, b.label);
            assert_eq!(a.enabled, b.enabled);
            let picked = pts.iter().filter(|p| selected(&a.sel, p)).count();
            assert!(picked > 0, "la prueba debe elegir algún punto");
            for p in &pts {
                assert_eq!(selected(&a.sel, p), selected(&b.sel, p), "{p:?}");
            }
        }
        // escribir lo leído da el mismo documento: nada se pierde ni cambia
        assert_eq!(to_json(&back), to_json(&edits));
        Ok(())
    }

    #[test]
    fn open_ended_limits_are_kept_as_open() -> Result<()> {
        let attr = Attr {
            z: Some((f64::NEG_INFINITY, 5.0)),
            ..Attr::default()
        };
        let e = Edit::new(
            vec![op(Mode::New, Region::All, attr)],
            Action::Delete,
            "x".into(),
        );
        let back = from_json(&to_json(&[e]))?;
        let s = &back[0].sel;
        assert!(selected(s, &sample(0.0, 0.0, -1.0e12, 2)));
        assert!(!selected(s, &sample(0.0, 0.0, 6.0, 2)));
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_ours_or_is_newer_is_refused() {
        assert!(from_json("{\"hello\":1}").is_err());
        assert!(from_json("[1,2").is_err());
        let newer = format!(
            "{{\"hipso\":\"edits\",\"version\":{},\"edits\":[]}}",
            VERSION + 1
        );
        assert!(from_json(&newer).is_err());
        assert!(
            from_json(&format!(
                "{{\"hipso\":\"edits\",\"version\":{VERSION},\"edits\":[]}}"
            ))
            .is_ok_and(|v| v.is_empty())
        );
    }

    #[test]
    fn a_session_lives_where_the_same_clouds_find_it_again() {
        use std::path::PathBuf;
        let a = vec![PathBuf::from("zona/hoja_1.laz")];
        let b = vec![PathBuf::from("zona/hoja_1.laz"), PathBuf::from("zona/hoja_2.laz")];
        assert_eq!(session_path(&a), session_path(&a));
        assert_ne!(session_path(&a), session_path(&b));
        assert!(session_path(&[]).is_none());
        assert!(session_path(&a).is_some_and(|p| {
            p.to_string_lossy().ends_with(".edits.json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("hoja_1_"))
        }));
    }

    #[test]
    fn writing_replaces_the_file_whole() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("hipso-edits-{}", std::process::id()));
        let path = dir.join("sesion.json");
        write(&path, &rich())?;
        write(&path, &rich()[..1])?;
        assert_eq!(read(&path)?.len(), 1);
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
