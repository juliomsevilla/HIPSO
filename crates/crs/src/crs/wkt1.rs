//! Un lector de WKT 1 (OGC y ESRI) que convierte la definición de un sistema en una cadena PROJ,
//! como hace GDAL, con sus unidades lineales y angulares.

/// A WKT node: `KEYWORD[arg, arg, ...]`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Arg {
    Str(String),
    Num(f64),
    Ident(String),
    Node(Node),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub key: String,
    pub args: Vec<Arg>,
}

impl Node {
    /// El primer nodo hijo con una palabra clave.
    pub fn child(&self, key: &str) -> Option<&Node> {
        self.args.iter().find_map(|a| match a {
            Arg::Node(n) if n.key.eq_ignore_ascii_case(key) => Some(n),
            _ => None,
        })
    }

    /// Los nodos hijos con una palabra clave.
    pub fn children<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.args.iter().filter_map(move |a| match a {
            Arg::Node(n) if n.key.eq_ignore_ascii_case(key) => Some(n),
            _ => None,
        })
    }

    /// El nombre del nodo (su primer argumento, cuando es una cadena).
    pub fn name(&self) -> Option<&str> {
        match self.args.first() {
            Some(Arg::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// El `i`-ésimo argumento como número.
    pub fn num(&self, i: usize) -> Option<f64> {
        match self.args.get(i) {
            Some(Arg::Num(v)) => Some(*v),
            Some(Arg::Str(s)) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// El código EPSG del `AUTHORITY` o `ID` del nodo.
    pub fn epsg(&self) -> Option<u32> {
        let n = self.child("AUTHORITY").or_else(|| self.child("ID"))?;
        if !n.name()?.eq_ignore_ascii_case("EPSG") {
            return None;
        }
        match n.args.get(1)? {
            Arg::Num(v) => Some(*v as u32),
            Arg::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
}

/// Analiza WKT y lo convierte en su árbol.
pub(crate) fn parse(s: &str) -> Option<Node> {
    let b = s.as_bytes();
    let mut i = 0;
    let n = node(b, &mut i)?;
    Some(n)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn node(b: &[u8], i: &mut usize) -> Option<Node> {
    skip_ws(b, i);
    let start = *i;
    while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_') {
        *i += 1;
    }
    let key = std::str::from_utf8(&b[start..*i]).ok()?.to_string();
    skip_ws(b, i);
    if *i >= b.len() || !(b[*i] == b'[' || b[*i] == b'(') {
        return None;
    }
    let close = if b[*i] == b'[' { b']' } else { b')' };
    *i += 1;
    let mut args = Vec::new();
    loop {
        skip_ws(b, i);
        if *i >= b.len() {
            return None;
        }
        if b[*i] == close {
            *i += 1;
            break;
        }
        if b[*i] == b',' {
            *i += 1;
            continue;
        }
        if b[*i] == b'"' {
            *i += 1;
            let mut s = Vec::new();
            while *i < b.len() {
                if b[*i] == b'"' {
                    if *i + 1 < b.len() && b[*i + 1] == b'"' {
                        s.push(b'"');
                        *i += 2;
                        continue;
                    }
                    break;
                }
                s.push(b[*i]);
                *i += 1;
            }
            *i += 1;
            args.push(Arg::Str(String::from_utf8_lossy(&s).into_owned()));
            continue;
        }
        // Un número, un identificador o un nodo anidado.
        let start = *i;
        while *i < b.len() && !matches!(b[*i], b',' | b'[' | b'(' | b']' | b')') {
            *i += 1;
        }
        let tok = std::str::from_utf8(&b[start..*i]).ok()?.trim();
        if *i < b.len() && (b[*i] == b'[' || b[*i] == b'(') {
            *i = start;
            args.push(Arg::Node(node(b, i)?));
        } else if let Ok(v) = tok.parse::<f64>() {
            args.push(Arg::Num(v));
        } else {
            args.push(Arg::Ident(tok.to_string()));
        }
    }
    Some(Node { key, args })
}

pub(crate) fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Un sistema WKT 1 como cadena PROJ, con el código de su sistema geográfico cuando lo declara.
pub(crate) struct Converted {
    pub proj: String,
    /// El nodo `GEOGCS`, para identificar el datum.
    pub geogcs: Node,
    /// El sistema geográfico EPSG, cuando el lector lo conoce.
    pub geog: Option<u32>,
    /// Un `GEOGCS` de WKT 1 con el que identificar el datum, cuando el WKT no tiene uno propio (WKT
    /// 2).
    pub geogcs_wkt1: Option<String>,
}

/// Convierte un `GEOGCS` o `PROJCS` de WKT 1 (variante ESRI u OGC) en una cadena PROJ. `None` para
/// otros tipos de sistema y proyecciones que terrio no conoce.
pub(crate) fn to_proj(root: &Node) -> Option<Converted> {
    let key = root.key.to_ascii_uppercase();
    let geogcs = match key.as_str() {
        "GEOGCS" => root,
        "PROJCS" => root.child("GEOGCS")?,
        _ => return None,
    };
    let datum = geogcs.child("DATUM")?;
    let sph = datum
        .child("SPHEROID")
        .or_else(|| datum.child("ELLIPSOID"))?;
    let a = sph.num(1)?;
    let rf = sph.num(2)?;
    // Unidad angular del sistema geográfico, en grados.
    let ang = geogcs
        .child("UNIT")
        .and_then(|u| u.num(1))
        .map(|r| r.to_degrees())
        .unwrap_or(1.0);
    // WKT redondea el tamaño de un grado (0,0174532925199433) y de un grado centesimal.
    let ang = [1.0, 0.9]
        .into_iter()
        .find(|u| (ang - u).abs() < 1e-9)
        .unwrap_or(ang);
    let pm = geogcs.child("PRIMEM").and_then(|p| p.num(1)).unwrap_or(0.0) * ang;
    let mut out = Vec::new();
    let mut tail = vec![format!("+a={a}"), format!("+rf={rf}")];
    if rf == 0.0 {
        tail[1] = format!("+b={a}");
    }
    if pm != 0.0 {
        tail.push(format!("+pm={pm}"));
    }
    if let Some(t) = datum.child("TOWGS84") {
        let v: Vec<String> = (0..t.args.len())
            .filter_map(|i| t.num(i))
            .map(|v| v.to_string())
            .collect();
        if !v.is_empty() {
            tail.push(format!("+towgs84={}", v.join(",")));
        }
    }
    if key == "GEOGCS" {
        out.push("+proj=longlat".to_string());
        if (ang - 1.0).abs() > 1e-12 {
            tail.push(format!("+to_deg={ang}"));
        }
        out.extend(tail);
        return Some(Converted {
            proj: out.join(" "),
            geogcs: geogcs.clone(),
            geog: None,
            geogcs_wkt1: None,
        });
    }

    let to_meter = root
        .children("UNIT")
        .last()
        .and_then(|u| u.num(1))
        .unwrap_or(1.0);
    let params: Vec<(String, f64)> = root
        .children("PARAMETER")
        .filter_map(|p| Some((norm(p.name()?), p.num(1)?)))
        .collect();
    let get = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| params.iter().find(|(k, _)| k == n).map(|(_, v)| *v))
    };
    let angle = |names: &[&str]| get(names).map(|v| v * ang);
    let lon_0 = angle(&[
        "centralmeridian",
        "longitudeofcenter",
        "longitudeoforigin",
        "longitudeofnaturalorigin",
        "longitudeofcentre",
    ]);
    let lat_0 = angle(&[
        "latitudeoforigin",
        "latitudeofcenter",
        "latitudeofnaturalorigin",
        "latitudeofcentre",
    ]);
    let k = get(&[
        "scalefactor",
        "scalefactoratnaturalorigin",
        "scalefactoroninitialline",
    ]);
    let sp1 = angle(&[
        "standardparallel1",
        "standardparallel",
        "latitudeof1ststandardparallel",
    ]);
    let sp2 = angle(&["standardparallel2", "latitudeof2ndstandardparallel"]);
    let fe = get(&[
        "falseeasting",
        "eastingatfalseorigin",
        "eastingatprojectioncentre",
    ])
    .unwrap_or(0.0)
        * to_meter;
    let fn_ = get(&[
        "falsenorthing",
        "northingatfalseorigin",
        "northingatprojectioncentre",
    ])
    .unwrap_or(0.0)
        * to_meter;
    let azimuth = angle(&["azimuth", "azimuthofinitialline"]);
    let gamma = angle(&["rectifiedgridangle", "angleofrectifiedtoskewedgrid"]);
    let method = norm(root.child("PROJECTION")?.name()?);
    let push = |out: &mut Vec<String>, k: &str, v: Option<f64>| {
        if let Some(v) = v {
            out.push(format!("+{k}={v}"));
        }
    };
    match method.as_str() {
        "transversemercator" | "gausskruger" | "transversemercatorcomplex" => {
            out.push("+proj=tmerc".into());
            push(&mut out, "lat_0", lat_0.or(Some(0.0)));
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "transversemercatorsouthorientated" => {
            out.push("+proj=tmerc +axis=wsu".into());
            push(&mut out, "lat_0", lat_0.or(Some(0.0)));
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "lambertconformalconic1sp" => {
            out.push("+proj=lcc".into());
            push(&mut out, "lat_1", lat_0);
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k_0", k.or(Some(1.0)));
        }
        "lambertconformalconic2sp"
        | "lambertconformalconic"
        | "lambertconformalconic2spbelgium" => {
            out.push("+proj=lcc".into());
            match (sp1, sp2) {
                (Some(s1), Some(s2)) => {
                    push(&mut out, "lat_1", Some(s1));
                    push(&mut out, "lat_2", Some(s2));
                }
                // ESRI's single-parallel form.
                (Some(s1), None) => push(&mut out, "lat_1", Some(s1)),
                _ => push(&mut out, "lat_1", lat_0),
            }
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k_0", k);
        }
        "albersconicequalarea" | "albers" | "albersequalarea" => {
            out.push("+proj=aea".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "lat_1", sp1);
            push(&mut out, "lat_2", sp2);
        }
        "mercator1sp" => {
            out.push("+proj=merc".into());
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "mercator2sp" | "mercator" => {
            out.push("+proj=merc".into());
            push(&mut out, "lon_0", lon_0);
            if sp1.is_some() {
                push(&mut out, "lat_ts", sp1);
            } else {
                push(&mut out, "k", k.or(Some(1.0)));
            }
        }
        "mercatorauxiliarysphere" | "popularvisualisationpseudomercator" => {
            out.push("+proj=merc".into());
            push(&mut out, "lon_0", lon_0);
            // Fórmulas esféricas sobre el semieje mayor.
            tail[1] = format!("+b={a}");
        }
        "polarstereographic" | "polarstereographicvarianta" | "polarstereographicvariantb" => {
            out.push("+proj=stere".into());
            let lat = lat_0.or(sp1).unwrap_or(90.0);
            push(
                &mut out,
                "lat_0",
                Some(if lat < 0.0 { -90.0 } else { 90.0 }),
            );
            if (lat.abs() - 90.0).abs() > 1e-9 {
                push(&mut out, "lat_ts", Some(lat));
            }
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "stereographicnorthpole" | "stereographicsouthpole" => {
            out.push("+proj=stere".into());
            push(
                &mut out,
                "lat_0",
                Some(if method.ends_with("southpole") {
                    -90.0
                } else {
                    90.0
                }),
            );
            push(&mut out, "lat_ts", sp1);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "obliquestereographic" | "doublestereographic" => {
            out.push("+proj=sterea".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "stereographic" => {
            out.push("+proj=stere".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "lambertazimuthalequalarea" => {
            out.push("+proj=laea".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "hotineobliquemercator"
        | "hotineobliquemercatorazimuthnaturalorigin"
        | "hotineobliquemercatorvarianta" => {
            out.push("+proj=omerc +no_uoff".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lonc", lon_0);
            push(&mut out, "alpha", azimuth);
            push(&mut out, "gamma", gamma.or(azimuth));
            push(&mut out, "k", k.or(Some(1.0)));
        }
        // Con un acimut (y un ángulo de rejilla rectificada, que en ESRI es el acimut) de 90°, la
        // Mercator oblicua suiza, como la lee PROJ (y la escribe, a partir de `+proj=somerc`).
        "hotineobliquemercatorazimuthcenter" | "hotineobliquemercatorvariantb"
            if azimuth.is_some_and(|a| (a - 90.0).abs() < 1e-9)
                && gamma.is_none_or(|g| (g - 90.0).abs() < 1e-9) =>
        {
            out.push("+proj=somerc".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k_0", k.or(Some(1.0)));
        }
        "hotineobliquemercatorazimuthcenter"
        | "obliquemercator"
        | "hotineobliquemercatorvariantb" => {
            out.push("+proj=omerc".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lonc", lon_0);
            push(&mut out, "alpha", azimuth);
            push(&mut out, "gamma", gamma.or(azimuth));
            push(&mut out, "k", k.or(Some(1.0)));
        }
        "swissobliquecylindrical"
        | "swissobliquemercator"
        | "hotineobliquemercatorazimuthcenterswiss" => {
            out.push("+proj=somerc".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k_0", k.or(Some(1.0)));
        }
        "krovak" => {
            // Sus ejes: la escala y la rotación del plano de ESRI (-1 y 90° para este-norte) o, si
            // no, los del WKT, este-norte salvo sur y oeste, como los lee PROJ.
            let east_north = match (get(&["xscale"]), get(&["xyplanerotation"])) {
                (Some(sx), Some(rot)) => sx < 0.0 && (rot - 90.0).abs() < 1e-10,
                _ => {
                    let dirs: Vec<String> = root
                        .children("AXIS")
                        .filter_map(|a| match a.args.get(1) {
                            Some(Arg::Ident(d)) => Some(d.to_ascii_lowercase()),
                            _ => None,
                        })
                        .collect();
                    !(dirs.iter().any(|d| d == "south") && dirs.iter().any(|d| d == "west"))
                }
            };
            out.push(
                if east_north {
                    "+proj=krovak"
                } else {
                    "+proj=krovak +axis=swu"
                }
                .into(),
            );
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "alpha", azimuth);
            push(&mut out, "lat_ts", angle(&["pseudostandardparallel1"]));
            push(&mut out, "k", k.or(Some(0.9999)));
        }
        "labordeobliquemercator" => {
            out.push("+proj=labrd".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "azi", azimuth);
            push(&mut out, "k_0", k.or(Some(1.0)));
        }
        "equidistantconic" => {
            out.push("+proj=eqdc".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "lat_1", sp1);
            push(&mut out, "lat_2", sp2);
        }
        "gaussschreibertransversemercator" => {
            out.push("+proj=gstmerc".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
            push(&mut out, "k_0", k.or(Some(1.0)));
        }
        // La latitud de origen de GDAL, el paralelo central de ESRI.
        "loximuthal" => {
            out.push("+proj=loxim".into());
            push(
                &mut out,
                "lat_1",
                lat_0.or_else(|| angle(&["centralparallel"])),
            );
            push(&mut out, "lon_0", lon_0);
        }
        "winkeltripel" | "winkeli" | "winkelii" => {
            let (name, parallel) = match method.as_str() {
                "winkeltripel" => ("wintri", "lat_1"),
                "winkeli" => ("wink1", "lat_ts"),
                _ => ("wink2", "lat_1"),
            };
            out.push(format!("+proj={name}"));
            push(&mut out, parallel, sp1.or(Some(0.0)));
            push(&mut out, "lon_0", lon_0);
        }
        "cassinisoldner" | "cassini" => {
            out.push("+proj=cass".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "polyconic" | "americanpolyconic" => {
            out.push("+proj=poly".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "equirectangular" | "equidistantcylindrical" | "platecarree" => {
            out.push("+proj=eqc".into());
            push(&mut out, "lat_ts", sp1);
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "cylindricalequalarea" | "lambertcylindricalequalarea" => {
            out.push("+proj=cea".into());
            push(&mut out, "lat_ts", sp1);
            push(&mut out, "lon_0", lon_0);
        }
        "azimuthalequidistant" | "modifiedazimuthalequidistant" => {
            out.push("+proj=aeqd".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "newzealandmapgrid" => out.push("+proj=nzmg".into()),
        "equalearth" => {
            out.push("+proj=eqearth".into());
            push(&mut out, "lon_0", lon_0);
        }
        "robinson" => {
            out.push("+proj=robin".into());
            push(&mut out, "lon_0", lon_0);
        }
        // `satellite_height` de GDAL y `Height` de ESRI (la `Option` de ESRI no cambia nada, y su
        // WKT no puede dar un eje de barrido: barrido y). Sin altura, PROJ rechaza el sistema.
        "geostationarysatellite" => {
            out.push("+proj=geos".into());
            push(&mut out, "lon_0", lon_0);
            push(
                &mut out,
                "h",
                get(&["satelliteheight", "height"]).map(|h| h * to_meter),
            );
        }
        "orthographic" => {
            out.push("+proj=ortho".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        // Proyecciones que terrio calcula sobre un elipsoide como PROJ (#126), no a través de
        // wbprojection.
        "sinusoidal" | "millercylindrical" | "vandergrinten" | "vandergrinteni" => {
            let name = match method.as_str() {
                "sinusoidal" => "sinu",
                "millercylindrical" => "mill",
                _ => "vandg",
            };
            out.push(format!("+proj={name}"));
            push(&mut out, "lon_0", lon_0);
        }
        "gnomonic" => {
            out.push("+proj=gnom".into());
            push(&mut out, "lat_0", lat_0);
            push(&mut out, "lon_0", lon_0);
        }
        "bonne" => {
            out.push("+proj=bonne".into());
            push(&mut out, "lat_1", sp1.or(lat_0));
            push(&mut out, "lon_0", lon_0);
        }
        // Proyecciones del mundo que terrio porta de PROJ, como las nombra GDAL (wbprojection las
        // lee como otras).
        "gallstereographic" | "flatpolarquartic" | "werenskioldi" => {
            let name = match method.as_str() {
                "gallstereographic" => "gall",
                "flatpolarquartic" => "mbtfpq",
                _ => "weren",
            };
            out.push(format!("+proj={name}"));
            push(&mut out, "lon_0", lon_0);
        }
        // Proyecciones del mundo de un meridiano central, como las escribe terrio.
        m => {
            let (proj, _) = super::wkt1_write::WORLD
                .iter()
                .find(|(_, w)| norm(w) == m)?;
            out.push(format!("+proj={proj}"));
            push(&mut out, "lon_0", lon_0);
        }
    }
    out.push(format!("+x_0={fe}"));
    out.push(format!("+y_0={fn_}"));
    out.push(format!("+to_meter={to_meter}"));
    out.extend(tail);
    Some(Converted {
        proj: out.join(" "),
        geogcs: geogcs.clone(),
        geog: None,
        geogcs_wkt1: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_esri_feet() {
        let esri = r#"PROJCS["NAD_1983_StatePlane_New_York_Long_Island_FIPS_3104_Feet",GEOGCS["GCS_North_American_1983",DATUM["D_North_American_1983",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",984250.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",-74.0],PARAMETER["Standard_Parallel_1",40.66666666666666],PARAMETER["Standard_Parallel_2",41.03333333333333],PARAMETER["Latitude_Of_Origin",40.16666666666666],UNIT["Foot_US",0.3048006096012192]]"#;
        let c = to_proj(&parse(esri).unwrap()).unwrap();
        assert!(
            c.proj
                .starts_with("+proj=lcc +lat_1=40.66666666666666 +lat_2=41.03333333333333"),
            "{}",
            c.proj
        );
        assert!(
            c.proj.contains("+to_meter=0.3048006096012192"),
            "{}",
            c.proj
        );
        assert_eq!(c.geogcs.name(), Some("GCS_North_American_1983"));
    }
}
