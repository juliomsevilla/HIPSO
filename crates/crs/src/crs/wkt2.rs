//! Un lector de WKT 2 (ISO 19162, 2015 y 2019) para sistemas geográficos y proyectados, que los
//! convierte en cadenas PROJ como hace [`super::wkt1`] con WKT 1, de modo que pasan por las
//! proyecciones de terrio.
//!
//! En WKT 2 el valor de un parámetro está en la unidad de su propio nodo `ANGLEUNIT`, `LENGTHUNIT`
//! o `SCALEUNIT`; sin él, los ángulos están en la unidad angular del sistema base y las longitudes
//! en la unidad del sistema de coordenadas proyectado. Los métodos y parámetros se nombran como en
//! EPSG y se identifican por sus códigos EPSG cuando el WKT los da.

use super::wkt1::{Arg, Converted, Node, norm};
use crate::{Error, Result};

/// Las palabras clave de los sistemas WKT 2 que convierte este lector.
const GEOGRAPHIC: [&str; 4] = ["GEOGCRS", "GEOGRAPHICCRS", "GEODCRS", "GEODETICCRS"];
const PROJECTED: [&str; 2] = ["PROJCRS", "PROJECTEDCRS"];
const BASE: [&str; 4] = [
    "BASEGEOGCRS",
    "BASEGEODCRS",
    "BASEGEOGRAPHICCRS",
    "BASEGEODETICCRS",
];

/// Si `root` es un sistema WKT 2 que convierte este lector.
pub(crate) fn handles(root: &Node) -> bool {
    let key = root.key.to_ascii_uppercase();
    if key == "BOUNDCRS" {
        // Solo de un sistema que convierte este lector.
        return root
            .child("SOURCECRS")
            .and_then(first_node)
            .is_some_and(handles);
    }
    if GEOGRAPHIC.contains(&key.as_str()) {
        // Un GEODCRS geocéntrico (un sistema cartesiano) no es un sistema de longitud y latitud.
        return !root
            .child("CS")
            .is_some_and(|cs| matches!(cs.args.first(), Some(Arg::Ident(t)) if !t.eq_ignore_ascii_case("ellipsoidal")));
    }
    PROJECTED.contains(&key.as_str())
}

/// Convierte en cadena PROJ un sistema geográfico o proyectado WKT 2, o un `BOUNDCRS` a WGS 84 de
/// uno. Falla con lo que no puede convertir fielmente: un método desconocido o un parámetro
/// desconocido.
pub(crate) fn to_proj(root: &Node) -> Result<Converted> {
    let key = root.key.to_ascii_uppercase();
    if key == "BOUNDCRS" {
        let source = root
            .child("SOURCECRS")
            .and_then(first_node)
            .ok_or_else(|| bad("a BOUNDCRS without its SOURCECRS"))?;
        let mut c = to_proj(source)?;
        if let Some(t) = towgs84(root)? {
            c.proj = format!(
                "{} +towgs84={}",
                c.proj,
                t.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            c.geog = None;
        }
        return Ok(c);
    }
    let projected = PROJECTED.contains(&key.as_str());
    if !projected && let Some(conv) = root.child("DERIVINGCONVERSION") {
        return derived(root, conv);
    }
    let base = if projected {
        BASE.iter()
            .find_map(|k| root.child(k))
            .ok_or_else(|| bad("a PROJCRS without its base CRS"))?
    } else {
        root
    };
    let geog = Geodetic::read(base, !projected)?;
    let mut tail = geog.tail();
    if !projected {
        let mut out = vec!["+proj=longlat".to_string()];
        if (geog.ang - 1.0).abs() > 1e-12 {
            out.push(format!("+to_deg={}", geog.ang));
        }
        out.append(&mut tail);
        return Ok(geog.converted(out.join(" ")));
    }

    // La unidad lineal del sistema de coordenadas proyectado y las direcciones de sus ejes.
    let to_meter = root
        .child("LENGTHUNIT")
        .or_else(|| root.child("UNIT"))
        .or_else(|| {
            root.children("AXIS")
                .find_map(|a| a.child("LENGTHUNIT").or_else(|| a.child("UNIT")))
        })
        .map(|u| u.num(1).ok_or_else(|| bad("a unit without its size")))
        .transpose()?
        .unwrap_or(1.0);
    // Los sistemas de coordenadas polares dan las direcciones de sus ejes a lo largo de los
    // meridianos («south» a lo largo de 90° E): esos se leen este-norte, como hace GDAL.
    let polar = root.children("AXIS").any(|a| a.child("MERIDIAN").is_some());
    let directions: Vec<String> = root
        .children("AXIS")
        .filter(|_| !polar)
        .filter_map(|a| {
            a.args.get(1).and_then(|d| {
                if let Arg::Ident(s) = d {
                    Some(s.to_ascii_lowercase())
                } else {
                    None
                }
            })
        })
        .collect();

    let conv = root
        .child("CONVERSION")
        .ok_or_else(|| bad("a PROJCRS without its CONVERSION"))?;
    let method = conv
        .child("METHOD")
        .ok_or_else(|| bad("a CONVERSION without its METHOD"))?;
    // Los métodos propios de PROJ (`METHOD["PROJ hammer"]`), con sus parámetros por sus nombres de
    // PROJ.
    let generic = proj_method(conv, method, geog.ang, to_meter)?;
    let p = if generic.is_some() {
        Params::default()
    } else {
        Params::read(conv, geog.ang, to_meter)?
    };
    let (name, code) = (norm(method.name().unwrap_or("")), method.epsg());
    let is = |names: &[&str], codes: &[u32]| {
        names.contains(&name.as_str()) || code.is_some_and(|c| codes.contains(&c))
    };

    let mut out: Vec<String> = Vec::new();
    let mut push = |k: &str, v: Option<f64>| {
        if let Some(v) = v {
            out.push(format!("+{k}={v}"));
        }
    };
    // Origen natural, origen falso y centro de proyección: el origen de la mayoría de los métodos.
    let lat_0 = || {
        p.get(LAT_NAT)
            .or_else(|| p.get(LAT_FALSE))
            .or_else(|| p.get(LAT_CENTRE))
    };
    let lon_0 = || {
        p.get(LON_NAT)
            .or_else(|| p.get(LON_FALSE))
            .or_else(|| p.get(LON_CENTRE))
            .or_else(|| p.get(LON_ORIGIN))
    };
    let (mut fe, mut fn_) = (p.get(FE).unwrap_or(0.0), p.get(FN).unwrap_or(0.0));
    let mut south_orientated = false;
    let mut no_uoff = false;
    let mut sweep = None;
    let proj = if is(&["transversemercator"], &[9807]) {
        push("lat_0", Some(lat_0().unwrap_or(0.0)));
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "tmerc"
    } else if is(&["transversemercatorsouthorientated"], &[9808]) {
        push("lat_0", Some(lat_0().unwrap_or(0.0)));
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        south_orientated = true;
        "tmerc"
    } else if is(&["lambertconicconformal1sp"], &[9801]) {
        push("lat_1", lat_0());
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("k_0", Some(p.get(K_NAT).unwrap_or(1.0)));
        "lcc"
    } else if is(&["lambertconicconformal2sp"], &[9802]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("lat_1", p.get(SP1));
        push("lat_2", p.get(SP2).or_else(|| p.get(SP1)));
        "lcc"
    } else if is(&["albersequalarea"], &[9822]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("lat_1", p.get(SP1));
        push("lat_2", p.get(SP2));
        "aea"
    } else if is(&["mercatorvarianta", "mercator1sp"], &[9804]) {
        if lat_0().is_some_and(|v| v != 0.0) {
            return Err(bad(
                "Mercator (variant A) with a latitude of natural origin other than 0",
            ));
        }
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "merc"
    } else if is(&["mercatorvariantb", "mercator2sp"], &[9805]) {
        push("lon_0", lon_0());
        push("lat_ts", Some(p.get(SP1).unwrap_or(0.0)));
        "merc"
    } else if is(&["popularvisualisationpseudomercator"], &[1024]) {
        push("lon_0", lon_0());
        // Fórmulas esféricas sobre el semieje mayor.
        tail.retain(|t| !t.starts_with("+rf=") && !t.starts_with("+b="));
        tail.push(format!("+b={}", geog.a));
        "merc"
    } else if is(&["polarstereographicvarianta"], &[9810]) {
        let lat = lat_0().unwrap_or(90.0);
        if (lat.abs() - 90.0).abs() > 1e-9 {
            return Err(bad(
                "Polar Stereographic (variant A) with a latitude of natural origin other than ±90°",
            ));
        }
        push("lat_0", Some(lat.signum() * 90.0));
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "stere"
    } else if is(&["polarstereographicvariantb"], &[9829]) {
        let ts = p
            .get(LAT_TS)
            .ok_or_else(|| bad("Polar Stereographic (variant B) without its standard parallel"))?;
        push("lat_0", Some(if ts < 0.0 { -90.0 } else { 90.0 }));
        push("lat_ts", Some(ts));
        push("lon_0", lon_0());
        "stere"
    } else if is(&["obliquestereographic"], &[9809]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "sterea"
    } else if is(&["stereographic"], &[]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "stere"
    } else if is(&["lambertazimuthalequalarea"], &[9820])
        || (geog.rf == 0.0 && is(&["lambertazimuthalequalareaspherical"], &[1027]))
    {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "laea"
    } else if is(&["hotineobliquemercatorvariantb"], &[9815])
        && p.get(AZIMUTH).is_some_and(|a| (a - 90.0).abs() < 1e-9)
        && p.get(GAMMA).is_some_and(|g| (g - 90.0).abs() < 1e-9)
    {
        // Con un acimut y un ángulo de rejilla rectificada de 90°, la Mercator oblicua suiza, como
        // la lee PROJ (y la escribe, a partir de `+proj=somerc`).
        fe = p.get(FE_CENTRE).unwrap_or(fe);
        fn_ = p.get(FN_CENTRE).unwrap_or(fn_);
        push("lat_0", p.get(LAT_CENTRE).or_else(lat_0));
        push("lon_0", p.get(LON_CENTRE).or_else(lon_0));
        push(
            "k_0",
            Some(p.get(K_CENTRE).or_else(|| p.get(K_NAT)).unwrap_or(1.0)),
        );
        "somerc"
    } else if is(&["hotineobliquemercatorvarianta"], &[9812])
        || is(&["hotineobliquemercatorvariantb"], &[9815])
    {
        let variant_b = is(&["hotineobliquemercatorvariantb"], &[9815]);
        if variant_b {
            fe = p.get(FE_CENTRE).unwrap_or(fe);
            fn_ = p.get(FN_CENTRE).unwrap_or(fn_);
        } else {
            no_uoff = true;
        }
        let alpha = p.get(AZIMUTH);
        push("lat_0", p.get(LAT_CENTRE).or_else(lat_0));
        push("lonc", p.get(LON_CENTRE).or_else(lon_0));
        push("alpha", alpha);
        push("gamma", p.get(GAMMA).or(alpha));
        push(
            "k",
            Some(p.get(K_CENTRE).or_else(|| p.get(K_NAT)).unwrap_or(1.0)),
        );
        "omerc"
    } else if is(&["cassinisoldner"], &[9806]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "cass"
    } else if is(&["americanpolyconic"], &[9818]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "poly"
    } else if is(&["equidistantcylindrical"], &[1028])
        || (geog.rf == 0.0 && is(&["equidistantcylindricalspherical"], &[1029]))
    {
        push("lat_ts", p.get(SP1));
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "eqc"
    } else if is(&["lambertcylindricalequalarea"], &[9835])
        || (geog.rf == 0.0 && is(&["lambertcylindricalequalareaspherical"], &[9834]))
    {
        push("lat_ts", p.get(SP1));
        push("lon_0", lon_0());
        "cea"
    } else if is(&["azimuthalequidistant"], &[1125]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "aeqd"
    } else if is(&["newzealandmapgrid"], &[9811]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "nzmg"
    } else if is(&["equalearth"], &[1078]) {
        push("lon_0", lon_0());
        "eqearth"
    } else if is(&["orthographic"], &[9840]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "ortho"
    } else if is(&["equidistantconic"], &[1119]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("lat_1", p.get(SP1));
        push("lat_2", p.get(SP2));
        "eqdc"
    } else if is(&["gnomonic"], &[]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        "gnom"
    } else if is(&["bonne"], &[9827]) {
        // El `lat_1` de PROJ es la latitud del origen natural de EPSG.
        push("lat_1", lat_0());
        push("lon_0", lon_0());
        "bonne"
    } else if is(
        &[
            "geostationarysatellitesweepx",
            "geostationarysatellitesweepy",
        ],
        &[],
    ) {
        push("lon_0", lon_0());
        // PROJ rechaza un sistema geoestacionario sin altura.
        push(
            "h",
            Some(
                p.get(SAT_HEIGHT)
                    .ok_or_else(|| bad("Geostationary Satellite without its Satellite Height"))?,
            ),
        );
        sweep = Some(if name == "geostationarysatellitesweepx" {
            "x"
        } else {
            "y"
        });
        "geos"
    } else if is(&["krovaknorthorientated"], &[1041]) {
        // Como PROJ, cuya Krovak fija el paralelo estándar pseudo y no escribe ninguno.
        p.get(LAT_PSEUDO_SP);
        push("lat_0", p.get(LAT_CENTRE).or_else(lat_0));
        push("lon_0", lon_0());
        push("alpha", p.get(CO_LAT_CONE));
        push(
            "k",
            Some(
                p.get(K_PSEUDO_SP)
                    .or_else(|| p.get(K_NAT))
                    .unwrap_or(0.9999),
            ),
        );
        "krovak"
    } else if is(&["loximuthal"], &[]) {
        // El `lat_1` de PROJ es la latitud del origen natural.
        push("lat_1", lat_0());
        push("lon_0", lon_0());
        "loxim"
    } else if is(&["transversecylindricalequalarea"], &[]) {
        push("lat_0", lat_0());
        push("lon_0", lon_0());
        push("k", Some(p.get(K_NAT).unwrap_or(1.0)));
        "tcea"
    } else if is(&["twopointequidistant"], &[]) {
        push("lat_1", p.get(LAT_P1));
        push("lon_1", p.get(LON_P1));
        push("lat_2", p.get(LAT_P2));
        push("lon_2", p.get(LON_P2));
        "tpeqd"
    } else if is(&["wagneriii"], &[]) {
        push("lat_ts", p.get(LAT_TRUE_SCALE));
        push("lon_0", lon_0());
        "wag3"
    } else if let Some(proj) = WORLD.iter().find(|(n, _)| *n == name).map(|(_, p)| *p) {
        push("lon_0", lon_0());
        proj
    } else if let Some((proj, parallel)) = [
        ("winkeli", "wink1", "lat_ts"),
        ("winkelii", "wink2", "lat_1"),
        ("winkeltripel", "wintri", "lat_1"),
    ]
    .iter()
    .find(|(n, _, _)| *n == name)
    .map(|(_, p, k)| (*p, *k))
    {
        push("lon_0", lon_0());
        push(parallel, p.get(SP1));
        proj
    } else if let Some((proj, params)) = &generic {
        let mut seen: Vec<&str> = Vec::new();
        for (k, v) in params {
            if seen.contains(&k.as_str()) {
                return Err(bad(&format!(
                    "parameter '{k}' given twice in the method 'PROJ {proj}'"
                )));
            }
            seen.push(k);
            match k.as_str() {
                "x_0" => fe = *v,
                "y_0" => fn_ = *v,
                // La figura, el datum, las unidades y los ejes vienen del sistema, no del método.
                "proj" | "ellps" | "datum" | "a" | "b" | "rf" | "f" | "es" | "e" | "R" | "R_A"
                | "units" | "to_meter" | "towgs84" | "nadgrids" | "geoidgrids" | "init"
                | "type" | "pm" | "axis" | "no_defs" | "wktext" | "over" => {
                    return Err(bad(&format!("parameter '{k}' of the method 'PROJ {proj}'")));
                }
                _ => push(k, Some(*v)),
            }
        }
        proj.as_str()
    } else {
        return Err(Error::Crs(format!(
            "unsupported WKT 2 projection method '{}'",
            method.name().unwrap_or("")
        )));
    };
    // Los parámetros que el método no usa se ignorarían: se rechazan, porque cambian las
    // coordenadas.
    if let Some(unused) = p.unused() {
        return Err(bad(&format!(
            "parameter '{unused}' of the method '{}'",
            method.name().unwrap_or("")
        )));
    }
    let mut proj_str = vec![format!("+proj={proj}")];
    if no_uoff {
        proj_str.push("+no_uoff".into());
    }
    if let Some(s) = sweep {
        proj_str.push(format!("+sweep={s}"));
    }
    proj_str.append(&mut out);
    if south_orientated {
        proj_str.push("+axis=wsu".into());
    } else if let Some(axis) = axis(&directions) {
        proj_str.push(format!("+axis={axis}"));
    }
    proj_str.push(format!("+x_0={fe}"));
    proj_str.push(format!("+y_0={fn_}"));
    proj_str.push(format!("+to_meter={to_meter}"));
    proj_str.append(&mut tail);
    Ok(geog.converted(proj_str.join(" ")))
}

/// Un sistema geográfico derivado (un `GEOGCRS` con un `DERIVINGCONVERSION`) como cadena PROJ: una
/// rejilla de polo girado, en una de las tres formas que escribe PROJ. Falla con cualquier otra
/// conversión, que si no se descartaría.
fn derived(root: &Node, conv: &Node) -> Result<Converted> {
    let base = BASE
        .iter()
        .find_map(|k| root.child(k))
        .ok_or_else(|| bad("a derived geographic CRS without its base CRS"))?;
    let geog = Geodetic::read(base, false)?;
    let method = conv
        .child("METHOD")
        .ok_or_else(|| bad("a DERIVINGCONVERSION without its METHOD"))?;
    let method_name = method.name().unwrap_or("");
    // Los parámetros, en grados, por nombre normalizado.
    let mut values = Vec::new();
    for n in conv.children("PARAMETER") {
        let name = n.name().unwrap_or("");
        let v = n
            .num(1)
            .ok_or_else(|| bad(&format!("parameter '{name}' without a value")))?;
        let unit = n
            .child("ANGLEUNIT")
            .or_else(|| n.child("UNIT"))
            .and_then(|u| u.num(1));
        values.push((
            norm(name),
            name.to_string(),
            v * unit.map(degrees).unwrap_or(geog.ang),
        ));
    }
    let used = std::cell::RefCell::new(Vec::new());
    let get = |key: &str| {
        let v = values.iter().find(|v| v.0 == key)?;
        used.borrow_mut().push(key.to_string());
        Some(v.2)
    };
    let need = |key: &str| {
        get(key).ok_or_else(|| {
            bad(&format!(
                "the method '{method_name}' without its parameter '{key}'"
            ))
        })
    };
    // `+o_lat_p`, `+o_lon_p` y `+lon_0`.
    let (lat_p, lon_p, lon_0) = match norm(method_name).as_str() {
        "projobtranoprojlonglat" => (
            need("olatp")?,
            get("olonp").unwrap_or(0.0),
            get("lon0").unwrap_or(0.0),
        ),
        // El polo norte de la rejilla y la longitud de rejilla del polo norte geográfico.
        "polerotationnetcdfcfconvention" => {
            let lat = need("gridnorthpolelatitudenetcdfcfconvention")?;
            let lon = need("gridnorthpolelongitudenetcdfcfconvention")?;
            (
                lat,
                get("northpolegridlongitudenetcdfcfconvention").unwrap_or(0.0),
                lon + 180.0,
            )
        }
        // El polo sur de la rejilla y el giro alrededor de su eje.
        "polerotationgribconvention" => {
            let lat = need("latitudeofthesouthernpolegribconvention")?;
            let lon = need("longitudeofthesouthernpolegribconvention")?;
            (-lat, -get("axisrotationgribconvention").unwrap_or(0.0), lon)
        }
        _ => {
            return Err(Error::Crs(format!(
                "unsupported WKT 2 deriving conversion '{method_name}'"
            )));
        }
    };
    if let Some(unused) = values.iter().find(|v| !used.borrow().contains(&v.0)) {
        return Err(bad(&format!(
            "parameter '{}' of the method '{method_name}'",
            unused.1
        )));
    }
    let mut out = vec![format!(
        "+proj=ob_tran +o_proj=longlat +o_lat_p={lat_p} +o_lon_p={lon_p} +lon_0={lon_0}"
    )];
    // La unidad angular de la rejilla: la de los ejes de su sistema de coordenadas.
    let unit = root
        .children("AXIS")
        .find_map(|a| a.child("ANGLEUNIT").or_else(|| a.child("UNIT")))
        .and_then(|u| u.num(1));
    if let Some(ang) = unit.map(degrees).filter(|a| (a - 1.0).abs() > 1e-12) {
        out.push(format!("+to_deg={ang}"));
    }
    out.append(&mut geog.tail());
    Ok(geog.converted(out.join(" ")))
}

/// Una proyección PROJ y sus parámetros por sus nombres de PROJ.
type ProjMethod = (String, Vec<(String, f64)>);

/// Un método que PROJ escribe como `PROJ name` (una proyección sin método WKT 2 propio), con sus
/// parámetros, nombrados como en las cadenas PROJ: ángulos en grados y longitudes en metros. `None`
/// para otros métodos.
fn proj_method(conv: &Node, method: &Node, ang: f64, to_meter: f64) -> Result<Option<ProjMethod>> {
    let Some(name) = method.name().and_then(|n| n.strip_prefix("PROJ ")) else {
        return Ok(None);
    };
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        return Ok(None);
    }
    let mut params = Vec::new();
    for n in conv.children("PARAMETER") {
        let key = n.name().unwrap_or("");
        if key.is_empty() || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            return Err(bad(&format!(
                "parameter '{key}' of the method 'PROJ {name}'"
            )));
        }
        let v = n
            .num(1)
            .ok_or_else(|| bad(&format!("parameter '{key}' without a value")))?;
        let v = if let Some(u) = n.child("ANGLEUNIT").and_then(|u| u.num(1)) {
            v * degrees(u)
        } else if let Some(u) = n.child("LENGTHUNIT").and_then(|u| u.num(1)) {
            v * u
        } else if n.child("SCALEUNIT").is_some() {
            v
        } else if key.starts_with("lat")
            || key.starts_with("lon")
            || matches!(key, "alpha" | "gamma" | "azi" | "theta" | "tilt")
        {
            // Sin unidad: ángulos en la unidad angular del sistema, longitudes en su unidad lineal
            // y los demás (factores de escala, parámetros de forma) tal como están.
            v * ang
        } else if matches!(key, "x_0" | "y_0" | "h") {
            v * to_meter
        } else {
            v
        };
        params.push((key.to_string(), v));
    }
    Ok(Some((name.to_string(), params)))
}

/// Proyecciones pseudocilíndricas y otras proyecciones del mundo que PROJ nombra sin método EPSG y
/// que toman solo un meridiano central.
const WORLD: [(&str, &str); 26] = [
    ("mollweide", "moll"),
    ("robinson", "robin"),
    ("sinusoidal", "sinu"),
    ("eckerti", "eck1"),
    ("eckertii", "eck2"),
    ("eckertiii", "eck3"),
    ("eckertiv", "eck4"),
    ("eckertv", "eck5"),
    ("eckertvi", "eck6"),
    ("millercylindrical", "mill"),
    ("vandergrinten", "vandg"),
    ("naturalearth", "natearth"),
    ("quarticauthalic", "qua_aut"),
    ("goodehomolosine", "igh"),
    ("aitoff", "aitoff"),
    ("crasterparabolic", "crast"),
    ("naturalearthii", "natearth2"),
    ("patterson", "patterson"),
    ("times", "times"),
    ("flatpolarquartic", "mbtfpq"),
    ("wagneri", "wag1"),
    ("wagnerii", "wag2"),
    ("wagneriv", "wag4"),
    ("wagnerv", "wag5"),
    ("wagnervi", "wag6"),
    ("gallstereographic", "gall"),
];

/// El orden de ejes de un sistema de coordenadas proyectado como `+axis` de PROJ, cuando no es este
/// y luego norte. Como el orden tradicional de los SIG de GDAL, un sistema norte-este se lee
/// este-norte: solo el oeste y el sur cambian las coordenadas.
fn axis(directions: &[String]) -> Option<String> {
    let has = |d: &str| directions.iter().any(|x| x == d);
    let (e, n) = (
        if has("west") { 'w' } else { 'e' },
        if has("south") { 's' } else { 'n' },
    );
    (e != 'e' || n != 'n').then(|| format!("{e}{n}u"))
}

/// El primer nodo hijo de un nodo.
fn first_node(n: &Node) -> Option<&Node> {
    n.args
        .iter()
        .find_map(|a| if let Arg::Node(n) = a { Some(n) } else { None })
}

fn bad(what: &str) -> Error {
    Error::Crs(format!("unsupported WKT 2: {what}"))
}

/// El elipsoide, el meridiano origen, la unidad angular y el datum de un sistema geodésico.
struct Geodetic {
    node: Node,
    a: f64,
    rf: f64,
    /// La unidad angular, en grados.
    ang: f64,
    /// El meridiano origen, en grados.
    pm: f64,
    /// El sistema geográfico EPSG que es este, por su código o el de su datum.
    geog: Option<u32>,
    datum_name: String,
    ellipsoid_name: String,
}

impl Geodetic {
    fn read(n: &Node, root: bool) -> Result<Self> {
        let datum = ["DATUM", "GEODETICDATUM", "TRF", "ENSEMBLE"]
            .iter()
            .find_map(|k| n.child(k))
            .ok_or_else(|| bad("a geodetic CRS without its datum"))?;
        let ell = datum
            .child("ELLIPSOID")
            .or_else(|| datum.child("SPHEROID"))
            .ok_or_else(|| bad("a datum without its ellipsoid"))?;
        let ell_unit = ell
            .child("LENGTHUNIT")
            .or_else(|| ell.child("UNIT"))
            .and_then(|u| u.num(1))
            .unwrap_or(1.0);
        let a = ell
            .num(1)
            .ok_or_else(|| bad("an ellipsoid without its semi-major axis"))?
            * ell_unit;
        let rf = ell
            .num(2)
            .ok_or_else(|| bad("an ellipsoid without its inverse flattening"))?;
        // La unidad angular del sistema: su propio nodo o, si no, la de los ejes de su sistema de
        // coordenadas (un sistema geográfico) o, si no, la del meridiano origen o, si no, grados.
        let unit = n
            .child("ANGLEUNIT")
            .or_else(|| n.child("UNIT"))
            .or_else(|| {
                if root {
                    n.children("AXIS")
                        .find_map(|a| a.child("ANGLEUNIT").or_else(|| a.child("UNIT")))
                } else {
                    None
                }
            })
            .or_else(|| {
                n.child("PRIMEM")
                    .and_then(|p| p.child("ANGLEUNIT").or_else(|| p.child("UNIT")))
            });
        let ang = unit.and_then(|u| u.num(1)).map(degrees).unwrap_or(1.0);
        let pm = match n.child("PRIMEM") {
            Some(p) => {
                let v = p.num(1).unwrap_or(0.0);
                match p
                    .child("ANGLEUNIT")
                    .or_else(|| p.child("UNIT"))
                    .and_then(|u| u.num(1))
                {
                    Some(r) => v * degrees(r),
                    None => v * ang,
                }
            }
            None => 0.0,
        };
        // El sistema geográfico EPSG: por el código del sistema o, si no, el de su datum (6xxx para
        // 4xxx) o, si no, su nombre, sobre el mismo elipsoide.
        let geog = n.epsg().or_else(|| {
            let d = datum.epsg()?;
            let code = d.checked_sub(2000)?;
            super::epsg::get(code)
                .filter(|e| e.datum == d && e.proj.starts_with("+proj=longlat"))
                .map(|_| code)
        });
        let geog = geog.or_else(|| {
            let name = n.name().filter(|s| !s.eq_ignore_ascii_case("unknown"))?;
            // El nombre oficial es exacto (las realizaciones tienen el suyo, «NAD83(HARN)»); aun
            // así, un nombre ambiguo no identifica nada.
            let es = if rf == 0.0 {
                0.0
            } else {
                (2.0 - 1.0 / rf) / rf
            };
            let found: Vec<u32> = super::epsg::named(name)
                .into_iter()
                .filter(|c| same_ellipsoid(*c, a, es))
                .collect();
            (found.len() == 1).then(|| found[0])
        });
        Ok(Self {
            node: n.clone(),
            a,
            rf,
            ang,
            pm,
            geog,
            datum_name: datum.name().unwrap_or("unknown").to_string(),
            ellipsoid_name: ell.name().unwrap_or("unknown").to_string(),
        })
    }

    /// El elipsoide y el meridiano origen, como final de una cadena PROJ.
    fn tail(&self) -> Vec<String> {
        let mut t = vec![
            format!("+a={}", self.a),
            if self.rf == 0.0 {
                format!("+b={}", self.a)
            } else {
                format!("+rf={}", self.rf)
            },
        ];
        if self.pm != 0.0 {
            t.push(format!("+pm={}", self.pm));
        }
        t
    }

    fn converted(&self, proj: String) -> Converted {
        // Un `GEOGCS` de WKT 1 del datum, para identificarlo por el nombre cuando no tiene código.
        let q = |s: &str| s.replace('"', "\"\"");
        let geogcs_wkt1 = format!(
            "GEOGCS[\"{}\",DATUM[\"{}\",SPHEROID[\"{}\",{},{}]],PRIMEM[\"Greenwich\",{}],UNIT[\"degree\",0.0174532925199433]]",
            q(self.node.name().unwrap_or("unknown")),
            q(&self.datum_name),
            q(&self.ellipsoid_name),
            self.a,
            self.rf,
            self.pm
        );
        Converted {
            proj,
            geogcs: self.node.clone(),
            geog: self.geog,
            geogcs_wkt1: Some(geogcs_wkt1),
        }
    }
}

/// Si un sistema geográfico 2D de EPSG está sobre un elipsoide (su semieje mayor y su excentricidad
/// al cuadrado).
pub(crate) fn same_ellipsoid(code: u32, a: f64, es: f64) -> bool {
    let Some(e) =
        super::epsg::get(code).filter(|e| e.proj.starts_with("+proj=longlat") && !e.three_d)
    else {
        return false;
    };
    let Ok(d) = super::def::Def::from_proj(&e.proj) else {
        return false;
    };
    (d.ell.a - a).abs() < 1e-3 && (d.ell.es - es).abs() < 1e-9
}

/// El tamaño de una unidad angular dada en radianes, en grados, con el grado y el grado centesimal
/// redondeados de WKT hechos exactos.
fn degrees(radians: f64) -> f64 {
    let d = radians.to_degrees();
    [1.0, 0.9, 1.0 / 3600.0]
        .into_iter()
        .find(|u| (d - u).abs() < 1e-9 * u)
        .unwrap_or(d)
}

/// El tipo del valor de un parámetro.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Angle,
    Length,
    Scale,
}

/// Un parámetro: sus nombres EPSG normalizados y su código EPSG.
type Param = (&'static [&'static str], u32, Kind);

const LAT_NAT: Param = (
    &["latitudeofnaturalorigin", "latitudeoforigin"],
    8801,
    Kind::Angle,
);
const LON_NAT: Param = (
    &["longitudeofnaturalorigin", "centralmeridian"],
    8802,
    Kind::Angle,
);
const K_NAT: Param = (&["scalefactoratnaturalorigin"], 8805, Kind::Scale);
const FE: Param = (&["falseeasting"], 8806, Kind::Length);
const FN: Param = (&["falsenorthing"], 8807, Kind::Length);
const LAT_CENTRE: Param = (&["latitudeofprojectioncentre"], 8811, Kind::Angle);
const LON_CENTRE: Param = (&["longitudeofprojectioncentre"], 8812, Kind::Angle);
const AZIMUTH: Param = (
    &["azimuthatprojectioncentre", "azimuthofinitialline"],
    8813,
    Kind::Angle,
);
const GAMMA: Param = (&["anglefromrectifiedtoskewgrid"], 8814, Kind::Angle);
const K_CENTRE: Param = (
    &["scalefactoratprojectioncentre", "scalefactoroninitialline"],
    8815,
    Kind::Scale,
);
const FE_CENTRE: Param = (&["eastingatprojectioncentre"], 8816, Kind::Length);
const FN_CENTRE: Param = (&["northingatprojectioncentre"], 8817, Kind::Length);
const LAT_FALSE: Param = (&["latitudeoffalseorigin"], 8821, Kind::Angle);
const LON_FALSE: Param = (&["longitudeoffalseorigin"], 8822, Kind::Angle);
const SP1: Param = (&["latitudeof1ststandardparallel"], 8823, Kind::Angle);
const SP2: Param = (&["latitudeof2ndstandardparallel"], 8824, Kind::Angle);
const FE_FALSE: Param = (&["eastingatfalseorigin"], 8826, Kind::Length);
const FN_FALSE: Param = (&["northingatfalseorigin"], 8827, Kind::Length);
const LAT_TS: Param = (&["latitudeofstandardparallel"], 8832, Kind::Angle);
const LON_ORIGIN: Param = (&["longitudeoforigin"], 8833, Kind::Angle);

const CO_LAT_CONE: Param = (&["colatitudeofconeaxis"], 1036, Kind::Angle);
const LAT_PSEUDO_SP: Param = (&["latitudeofpseudostandardparallel"], 8818, Kind::Angle);
const K_PSEUDO_SP: Param = (&["scalefactoronpseudostandardparallel"], 8819, Kind::Scale);

/// Los parámetros de PROJ sin código EPSG: códigos descendiendo desde `u32::MAX`, que no coinciden
/// con ningún `ID`.
const SAT_HEIGHT: Param = (&["satelliteheight"], u32::MAX, Kind::Length);
const LAT_P1: Param = (&["latitudeof1stpoint"], u32::MAX - 1, Kind::Angle);
const LON_P1: Param = (&["longitudeof1stpoint"], u32::MAX - 2, Kind::Angle);
const LAT_P2: Param = (&["latitudeof2ndpoint"], u32::MAX - 3, Kind::Angle);
const LON_P2: Param = (&["longitudeof2ndpoint"], u32::MAX - 4, Kind::Angle);
const LAT_TRUE_SCALE: Param = (&["latitudeoftruescale"], u32::MAX - 5, Kind::Angle);

const PARAMS: [Param; 29] = [
    SAT_HEIGHT,
    LAT_P1,
    LON_P1,
    LAT_P2,
    LON_P2,
    LAT_TRUE_SCALE,
    LAT_NAT,
    LON_NAT,
    K_NAT,
    FE,
    FN,
    LAT_CENTRE,
    LON_CENTRE,
    AZIMUTH,
    GAMMA,
    K_CENTRE,
    FE_CENTRE,
    FN_CENTRE,
    LAT_FALSE,
    LON_FALSE,
    SP1,
    SP2,
    FE_FALSE,
    FN_FALSE,
    LAT_TS,
    LON_ORIGIN,
    CO_LAT_CONE,
    LAT_PSEUDO_SP,
    K_PSEUDO_SP,
];

/// Los parámetros de una conversión, en grados, metros y unidad, y cuáles se usaron.
#[derive(Default)]
struct Params {
    values: Vec<(u32, String, f64)>,
    used: std::cell::RefCell<Vec<u32>>,
}

impl Params {
    fn read(conv: &Node, ang: f64, to_meter: f64) -> Result<Self> {
        let mut values = Vec::new();
        for n in conv.children("PARAMETER") {
            let name = n.name().unwrap_or("");
            let key = norm(name);
            let param = PARAMS
                .iter()
                .find(|(names, code, _)| names.contains(&key.as_str()) || n.epsg() == Some(*code))
                .ok_or_else(|| bad(&format!("parameter '{name}'")))?;
            let v = n
                .num(1)
                .ok_or_else(|| bad(&format!("parameter '{name}' without a value")))?;
            let unit = ["ANGLEUNIT", "LENGTHUNIT", "SCALEUNIT", "UNIT"]
                .iter()
                .find_map(|k| n.child(k))
                .and_then(|u| u.num(1));
            let v = match param.2 {
                Kind::Angle => v * unit.map(degrees).unwrap_or(ang),
                Kind::Length => v * unit.unwrap_or(to_meter),
                Kind::Scale => v * unit.unwrap_or(1.0),
            };
            values.push((param.1, name.to_string(), v));
        }
        // El este y el norte en el origen falso son el falso este y el falso norte.
        for v in &mut values {
            if v.0 == FE_FALSE.1 {
                v.0 = FE.1;
            } else if v.0 == FN_FALSE.1 {
                v.0 = FN.1;
            }
        }
        Ok(Self {
            values,
            used: Default::default(),
        })
    }

    /// El valor de un parámetro, marcado como usado.
    fn get(&self, p: Param) -> Option<f64> {
        let v = self.values.iter().find(|v| v.0 == p.1)?;
        self.used.borrow_mut().push(p.1);
        Some(v.2)
    }

    /// El nombre de un parámetro que ningún brazo de método usó, con un valor que importa (no 0).
    fn unused(&self) -> Option<&str> {
        let used = self.used.borrow();
        self.values
            .iter()
            .find(|v| !used.contains(&v.0) && v.2 != 0.0)
            .map(|v| v.1.as_str())
    }
}

/// Los 3 o 7 parámetros `+towgs84` de la transformación a WGS 84 de un `BOUNDCRS`: `None` cuando es
/// a otro sistema o no desplaza.
fn towgs84(root: &Node) -> Result<Option<Vec<f64>>> {
    let target = root.child("TARGETCRS").and_then(first_node);
    let to_wgs84 = target.is_some_and(|t| {
        t.epsg() == Some(4326)
            || ["DATUM", "ENSEMBLE"]
                .iter()
                .filter_map(|k| t.child(k))
                .any(|d| norm(d.name().unwrap_or("")).starts_with("worldgeodeticsystem1984"))
    });
    let Some(t) = root.child("ABRIDGEDTRANSFORMATION") else {
        return Ok(None);
    };
    // Una transformación a otro sistema no tiene `+towgs84`: se conserva el sistema de origen y su
    // datum se identifica como sin BOUNDCRS (GDAL también lee el sistema de origen de un BOUNDCRS
    // así).
    if !to_wgs84 {
        return Ok(None);
    }
    let method = t
        .child("METHOD")
        .map(|m| (norm(m.name().unwrap_or("")), m.epsg()))
        .unwrap_or_default();
    // Lo mismo ocurre con la que no es una transformación de Helmert de 3 o 7 parámetros (la de una
    // rejilla, NTv2 o NADCON, cuyo archivo es un PARAMETERFILE): traslaciones geocéntricas, vector
    // de posición y marco de coordenadas, en los dominios geog2D, geocéntrico y geog3D.
    let helmert = [
        "geocentrictranslations",
        "positionvector",
        "coordinateframe",
    ]
    .iter()
    .any(|m| method.0.starts_with(m))
        || matches!(
            method.1,
            Some(9603 | 1031 | 1035 | 9606 | 1033 | 1037 | 9607 | 1032 | 1038)
        );
    if !helmert {
        return Ok(None);
    }
    // Las rotaciones del marco de coordenadas giran al revés que el `+towgs84` de PROJ (vector de
    // posición).
    let frame =
        method.0.contains("coordinateframe") || matches!(method.1, Some(9607 | 1032 | 1038));
    let mut v = [0.0; 7];
    let mut seven = false;
    for p in t.children("PARAMETER") {
        let name = norm(p.name().unwrap_or(""));
        let x = p
            .num(1)
            .ok_or_else(|| bad("a transformation parameter without a value"))?;
        let unit = ["ANGLEUNIT", "LENGTHUNIT", "SCALEUNIT", "UNIT"]
            .iter()
            .find_map(|k| p.child(k))
            .and_then(|u| u.num(1));
        let code = p.epsg();
        let i = match (name.as_str(), code) {
            ("xaxistranslation", _) | (_, Some(8605)) => 0,
            ("yaxistranslation", _) | (_, Some(8606)) => 1,
            ("zaxistranslation", _) | (_, Some(8607)) => 2,
            ("xaxisrotation", _) | (_, Some(8608)) => 3,
            ("yaxisrotation", _) | (_, Some(8609)) => 4,
            ("zaxisrotation", _) | (_, Some(8610)) => 5,
            ("scaledifference", _) | (_, Some(8611)) => 6,
            _ => {
                return Err(bad(&format!(
                    "transformation parameter '{}'",
                    p.name().unwrap_or("")
                )));
            }
        };
        v[i] = match i {
            0..=2 => x * unit.unwrap_or(1.0),
            // Segundos de arco y partes por millón.
            3..=5 => {
                x * unit.map(|r| r.to_degrees() * 3600.0).unwrap_or(1.0)
                    * if frame { -1.0 } else { 1.0 }
            }
            // Una diferencia de escala en su unidad o, sin ella, como la razón que escribe PROJ
            // (1,0000067 para 6,7 ppm): cualquier otro valor sin unidad es ambiguo.
            _ => match unit {
                Some(u) => x * u * 1e6,
                None if (x - 1.0).abs() < 0.01 => (x - 1.0) * 1e6,
                None => return Err(bad("a scale difference without its unit")),
            },
        };
        seven |= i >= 3;
    }
    if v.iter().all(|x| *x == 0.0) {
        return Ok(None);
    }
    Ok(Some(if seven { v.to_vec() } else { v[..3].to_vec() }))
}
