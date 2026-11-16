//! Sistemas de referencia de coordenadas y transformaciones entre ellos.
//!
//! Las coordenadas van siempre en el orden tradicional de los SIG: primero x/este/longitud y luego
//! y/norte/latitud, sea cual sea el orden de ejes que defina la autoridad del sistema.
//!
//! ```no_run
//! use hipso_crs::{SpatialRef, Transformer};
//!
//! let nad27 = SpatialRef::parse("EPSG:4267")?;
//! let utm = SpatialRef::parse("+proj=utm +zone=13 +datum=WGS84")?;
//! println!("{} -> {}", nad27.label(), utm.label());
//! let t = Transformer::new(&nad27, &utm)?;
//! let (x, y) = t.transform(-105.5, 39.5)?;
//! let bounds = t.transform_bounds((-106.0, 39.0, -105.0, 40.0))?;
//! println!("{}", utm.to_wkt().unwrap_or_default());
//! # Ok::<(), hipso_crs::Error>(())
//! ```
//!
//! Las rejillas de ajuste de datum (NTv2, GeoTIFF) y los modelos de geoide se usan cuando se
//! encuentran, como los usa GDAL (véanse [`register_grid`] y los directorios de rejillas y el
//! acceso a la red descritos en la [documentación de CRS](https://opengeos.github.io/terrio/crs/)).
//! Los sistemas compuestos (como `EPSG:4326+5773`) y los geográficos 3D transforman las alturas con
//! [`Transformer::transform_3d`]; las transformaciones dependientes del tiempo entre sistemas
//! dinámicos toman la época de coordenadas de [`TransformOptions`]:
//!
//! ```no_run
//! use hipso_crs::{SpatialRef, TransformOptions, Transformer};
//!
//! // De alturas elipsoidales (WGS 84) a alturas EGM96.
//! let t = Transformer::new(&SpatialRef::parse("EPSG:4979")?, &SpatialRef::parse("EPSG:4326+5773")?)?;
//! let (lon, lat, h) = t.transform_3d(-100.0, 40.0, 100.0)?;
//! // De coordenadas ITRF2014 en la época 2010.0 a GDA2020.
//! let opts = TransformOptions { source_coordinate_epoch: Some(2010.0), ..Default::default() };
//! let t = Transformer::with_options(&SpatialRef::parse("EPSG:7912")?, &SpatialRef::parse("EPSG:7843")?, &opts)?;
//! let (lon, lat, h) = t.transform_3d(135.0, -25.0, 0.0)?;
//! # Ok::<(), hipso_crs::Error>(())
//! ```

use crate::{Error, Result};
use serde::Serialize;
use wbprojection as wbp;

mod datum;
mod def;
mod epsg;
mod grid;
mod math;
mod params;
pub mod policy;
mod projections;
mod rotation;
mod route;
mod wkt1;
mod wkt1_write;
mod wkt2;

pub(crate) use def::Def;
pub use grid::{
    GridOptions, register_grid, reset_grid_cache, set_grid_options, set_grid_search_paths,
    set_network_enabled,
};
pub(crate) use wkt1_write::{describe, epsg_ellipsoid, from_proj_on};

/// Empieza a cargar en segundo plano la tabla EPSG de terrio, para trabajos que la necesitarán una
/// vez que hayan leído su entrada.
pub(crate) fn prefetch() {
    epsg::prefetch();
}

/// El sistema geográfico 2D de EPSG sobre un datum geodésico de EPSG con elipsoide (parámetros PROJ
/// como `+ellps=GRS80` o `+a=6378137 +rf=298.257222101`) y el meridiano origen de Greenwich, el
/// código vigente más bajo: el sistema geográfico de las GeoKeys que dan el datum (NAD83(HARN),
/// 6152) pero no el sistema geográfico, tal como lo identifica GDAL (EPSG:4152). `None` cuando el
/// datum no tiene ninguno.
pub(crate) fn geographic_on_datum(datum: u32, ellipsoid: &str) -> Option<u32> {
    let own = Def::from_proj(&format!("+proj=longlat {ellipsoid} +no_defs")).ok()?;
    epsg::geographic_on(datum).iter().copied().find(|code| {
        epsg::get(*code).is_some_and(|e| {
            !e.three_d
                && !e.proj.contains("+pm=")
                && Def::from_proj(&e.proj).is_ok_and(|d| {
                    (own.ell.a - d.ell.a).abs() < 1e-3 && (own.ell.es - d.ell.es).abs() < 1e-9
                })
        })
    })
}

/// El datum de EPSG de un sistema geográfico de EPSG de la tabla de terrio. `None` para otro
/// código.
pub(crate) fn geographic_datum(geog: u32) -> Option<u32> {
    epsg::get(geog).map(|e| e.datum).filter(|d| *d != 0)
}

/// El WKT que escribe GDAL para un código EPSG, a partir de la tabla EPSG de terrio.
pub(crate) fn epsg_wkt(code: u32) -> Option<&'static str> {
    epsg::get(code).map(|e| e.wkt.as_str())
}

/// Los sistemas de ESRI cuya proyección calcula terrio de otro modo que PROJ (en más de un
/// milímetro dentro de su área de uso, como indica `scripts/compare_esri_with_proj.py` con PROJ
/// 9.8), rechazados en lugar de transformados mal: ninguno desde que la proyección Cassini de
/// terrio sigue a la de PROJ también lejos de su meridiano central (World Cassini, ESRI:54028).
pub(crate) const ESRI_UNLIKE_PROJ: &[u32] = &[];

/// Si un código tomado por código EPSG es en realidad de la autoridad ESRI de PROJ, fuera del
/// registro EPSG (wbprojection guarda 54008, 54009 y 54030 de ESRI como códigos EPSG).
pub(crate) fn is_esri_code(code: u32) -> bool {
    epsg::get(code).is_none()
        && epsg::compound(code).is_none()
        && epsg::vertical(code).is_none()
        && epsg::esri(code).is_some()
}

/// Un sistema de referencia de coordenadas, conocido por un código EPSG (o ESRI), una definición
/// WKT, una cadena PROJ o una combinación de ellos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpatialRef {
    /// El código EPSG, cuando el sistema lo tiene (o se identificó como que lo tiene).
    pub epsg: Option<u32>,
    /// La definición WKT.
    pub wkt: Option<String>,
    /// La definición como cadena PROJ.
    pub proj4: Option<String>,
    /// El código de la autoridad ESRI de PROJ (`ESRI:54009`), cuando el sistema lo tiene (o se
    /// identificó como que lo tiene) en lugar de un código EPSG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub esri: Option<u32>,
}

impl SpatialRef {
    /// Un sistema a partir de un código EPSG. Falla cuando el código no está en el registro EPSG
    /// que lleva terrio. Los códigos ESRI que wbprojection guarda como códigos EPSG (54008, 54009 y
    /// 54030) dan esos sistemas ESRI; los demás códigos ESRI no son códigos EPSG, como en PROJ.
    pub fn from_epsg(code: u32) -> Result<Self> {
        let wkt = match epsg::get(code) {
            Some(e) => e.wkt.clone(),
            None => match (epsg::compound(code), epsg::vertical(code)) {
                (Some(c), _) => c.wkt.clone(),
                (None, Some(v)) => v.wkt.clone(),
                _ => {
                    let w = wbp::to_ogc_wkt(code)
                        .map_err(|_| Error::Crs(format!("unknown EPSG code {code}")))?;
                    if epsg::esri(code).is_some() {
                        return Self::from_esri(code);
                    }
                    w
                }
            },
        };
        Ok(Self {
            epsg: Some(code),
            wkt: Some(wkt),
            ..Default::default()
        })
    }

    /// Un sistema a partir de un código de la autoridad ESRI de PROJ, como 54009 (World Mollweide),
    /// con el WKT que escribe GDAL para él. Falla cuando terrio no conoce el código o no puede
    /// calcular su proyección.
    pub fn from_esri(code: u32) -> Result<Self> {
        let e = epsg::esri(code).ok_or_else(|| Error::Crs(format!("unknown ESRI code {code}")))?;
        if ESRI_UNLIKE_PROJ.contains(&code) {
            return Err(Error::Crs(format!(
                "ESRI:{code} ({}) is not supported: terrio does not compute its projection as PROJ does",
                e.name
            )));
        }
        Def::from_proj(&e.proj).map_err(|err| {
            Error::Crs(format!("ESRI:{code} ({}) is not supported: {err}", e.name))
        })?;
        Ok(Self {
            esri: Some(code),
            wkt: Some(e.wkt.clone()),
            ..Default::default()
        })
    }

    /// Un sistema a partir de una definición WKT (WKT1 o WKT2). El código EPSG (o, si no, el ESRI)
    /// se identifica cuando es posible.
    pub fn from_wkt(wkt: &str) -> Self {
        // Un WKT 1 ampliado con su cadena PROJ (`EXTENSION["PROJ4", ...]`, como escribe GDAL los
        // sistemas que WKT 1 no puede describir) es esa cadena PROJ, como la lee GDAL.
        if let Some(proj4) = proj4_extension(wkt)
            && let Ok(own) = Def::from_proj(&proj4)
        {
            // Su código EPSG (o ESRI), declarado o identificado, cuando su definición convierte
            // como la cadena PROJ, sobre su elipsoide y su datum: la extensión gana a un código que
            // no lo hace, como a los nodos. Una cadena PROJ sin ajuste de datum está sobre WGS 84
            // para un código sobre el propio WGS 84 (el WKT de EPSG:3857 da su datum como una
            // rejilla nula). Un código declarado que terrio no conoce no convierte como nada y se
            // descarta.
            let fits = |code: SpatialRef| {
                Def::new(&code).is_ok_and(|r| {
                    let unshifted =
                        own.ops().is_empty() && own.datum_code().is_none() && r.is_wgs84();
                    let ellipsoid =
                        (own.ell.a - r.ell.a).abs() < 1e-3 && (own.ell.es - r.ell.es).abs() < 1e-9;
                    own.same_conversion(&r) && ellipsoid && (unshifted || own.same_datum(&r))
                })
            };
            let epsg = declared_epsg(wkt)
                .or_else(|| identify_epsg(wkt))
                .filter(|code| {
                    fits(SpatialRef {
                        epsg: Some(*code),
                        ..Default::default()
                    })
                });
            let mut s = Self {
                epsg,
                wkt: Some(wkt.to_string()),
                proj4: Some(proj4),
                esri: None,
            }
            .with_esri();
            if let Some(code) = s.esri
                && !fits(SpatialRef {
                    esri: Some(code),
                    ..Default::default()
                })
            {
                s.esri = None;
            }
            return s;
        }
        let epsg = if def::split_compound(wkt).is_some() {
            declared_epsg(wkt)
        } else {
            identify_epsg(wkt)
        };
        Self {
            epsg,
            wkt: Some(wkt.to_string()),
            ..Default::default()
        }
        .with_esri()
    }

    /// El sistema con un código ESRI en lugar de un código que el registro EPSG de terrio no guarda
    /// (el registro identifica 54008, 54009 y 54030 de ESRI como códigos EPSG), o identificado a
    /// partir del WKT cuando no hay código EPSG: el código ESRI que declara o el del sistema ESRI
    /// del mismo nombre cuya definición convierte las coordenadas como el WKT, como los identifica
    /// GDAL.
    pub(crate) fn with_esri(mut self) -> Self {
        // Un código ESRI declarado gana a un código EPSG identificado (ESRI:102100 es la definición
        // de EPSG:3857), como lo conserva GDAL.
        if let Some(code) = self
            .wkt
            .as_deref()
            .and_then(|w| declared_code(w, "ESRI"))
            .filter(|c| epsg::esri(*c).is_some())
        {
            self.epsg = None;
            self.esri = Some(code);
            return self;
        }
        if let Some(code) = self.epsg
            && is_esri_code(code)
        {
            self.epsg = None;
            if self.wkt.is_none() {
                self.esri = Some(code);
            }
        }
        if self.epsg.is_none()
            && self.esri.is_none()
            && let Some(w) = &self.wkt
        {
            self.esri = identify_esri(w);
        }
        self
    }

    /// Un sistema compuesto a partir de los códigos EPSG de un sistema horizontal y uno vertical,
    /// como GDAL construye `EPSG:4326+5773`.
    pub fn compound(horizontal: u32, vertical: u32) -> Result<Self> {
        let h = Self::from_epsg(horizontal)?;
        let v = epsg::vertical(vertical)
            .ok_or_else(|| Error::Crs(format!("EPSG:{vertical} is not a vertical CRS")))?;
        let name = format!("{} + {}", h.name().unwrap_or_default(), v.name);
        let wkt = format!(
            "COMPD_CS[\"{name}\",{},{}]",
            h.to_wkt().unwrap_or_default(),
            v.wkt
        );
        Ok(Self {
            wkt: Some(wkt),
            ..Default::default()
        })
    }

    /// Un sistema a partir de una cadena PROJ como `+proj=utm +zone=33 +datum=WGS84`.
    pub fn from_proj4(proj4: &str) -> Result<Self> {
        let p = params::Params::parse(proj4);
        if let Some(code) = p.str("init").and_then(|i| {
            i.to_ascii_lowercase()
                .strip_prefix("epsg:")
                .and_then(|c| c.parse().ok())
        }) {
            return Self::from_epsg(code);
        }
        // Comprueba que terrio puede transformar con él.
        Def::from_proj(proj4)?;
        Ok(Self {
            wkt: proj_to_wkt(proj4),
            proj4: Some(proj4.to_string()),
            ..Default::default()
        })
    }

    /// Analiza un sistema dado por el usuario: `EPSG:4326`, `4326`, `urn:ogc:def:crs:EPSG::4326`,
    /// una URL OGC, `OGC:CRS84`, `WGS84`, `ESRI:54009` (o `urn:ogc:def:crs:ESRI::54009`), una
    /// definición WKT, una cadena PROJ o la ruta de un archivo que contenga una de ellas (como el
    /// `.prj` de un shapefile).
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(Error::Crs("empty definition".into()));
        }
        let upper = s.to_ascii_uppercase();
        if matches!(
            upper.as_str(),
            "WGS84" | "WGS 84" | "OGC:CRS84" | "CRS84" | "URN:OGC:DEF:CRS:OGC:1.3:CRS84"
        ) || upper.ends_with("/OGC/1.3/CRS84")
        {
            return Self::from_epsg(4326);
        }
        if s.starts_with('+') {
            return Self::from_proj4(s);
        }
        // `EPSG:4326+5773`: un sistema horizontal y uno vertical.
        if let Some(rest) = upper.strip_prefix("EPSG:")
            && let Some((h, v)) = rest.split_once('+')
            && let (Ok(h), Ok(v)) = (h.trim().parse::<u32>(), v.trim().parse::<u32>())
        {
            return Self::compound(h, v);
        }
        if looks_like_wkt(s) {
            return Ok(Self::from_wkt(s));
        }
        if let Some(code) = esri_reference(&upper) {
            return Self::from_esri(code);
        }
        if let Some(code) = wbp::epsg_from_srs_reference(s) {
            return Self::from_epsg(code);
        }
        #[cfg(not(target_family = "wasm"))]
        if let Ok(text) = std::fs::read_to_string(s) {
            return Self::parse(&text);
        }
        Err(Error::Crs(format!("cannot parse '{s}'")))
    }

    /// El WKT del sistema geográfico en el que se basa este sistema (el propio si es geográfico):
    /// de la tabla EPSG para un sistema EPSG, si no del nodo `GEOGCS` de su WKT 1. `None` cuando no
    /// se conoce ninguno.
    pub(crate) fn geographic_wkt(&self) -> Option<String> {
        if let Some(e) = self
            .epsg
            .and_then(epsg::get)
            .or_else(|| self.esri.and_then(epsg::esri))
            && let Some(g) = epsg::get(e.geog)
        {
            return Some(g.wkt.clone());
        }
        let wkt = self.to_wkt()?;
        let start = wkt.find("GEOGCS[")?;
        let mut depth = 0usize;
        for (i, c) in wkt[start..].char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(wkt[start..=start + i].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Si no se sabe nada del sistema.
    pub fn is_unknown(&self) -> bool {
        self.epsg.is_none() && self.esri.is_none() && self.wkt.is_none() && self.proj4.is_none()
    }

    /// La autoridad y el código del sistema: `("EPSG", 4326)` o `("ESRI", 54009)` para un sistema
    /// de la autoridad ESRI de PROJ.
    pub fn authority(&self) -> Option<(&'static str, u32)> {
        match (self.epsg, self.esri) {
            (Some(code), _) => Some(("EPSG", code)),
            (None, Some(code)) => Some(("ESRI", code)),
            (None, None) => None,
        }
    }

    /// El código de autoridad como lo escribe GDAL: `EPSG:4326` o `ESRI:54009`.
    pub fn authority_code(&self) -> Option<String> {
        self.authority().map(|(a, code)| format!("{a}:{code}"))
    }

    /// La definición WKT, construida a partir del código EPSG (o ESRI) o de la cadena PROJ cuando
    /// no hay WKT guardado.
    pub fn to_wkt(&self) -> Option<String> {
        if let Some(w) = &self.wkt {
            return Some(w.clone());
        }
        if let Some(e) = self.esri.and_then(epsg::esri) {
            return Some(e.wkt.clone());
        }
        if let Some(code) = self.epsg {
            return epsg::get(code)
                .map(|e| e.wkt.clone())
                .or_else(|| epsg::compound(code).map(|c| c.wkt.clone()))
                .or_else(|| epsg::vertical(code).map(|v| v.wkt.clone()))
                .or_else(|| wbp::to_ogc_wkt(code).ok());
        }
        self.proj4.as_deref().and_then(proj_to_wkt)
    }

    /// El nombre del sistema, como `WGS 84 / UTM zone 33N`.
    pub fn name(&self) -> Option<String> {
        // Primero el nombre del registro: un WKT guardado puede llevar el nombre de otro proveedor
        // (`.prj` de ESRI).
        if let Some(e) = self.epsg.and_then(epsg::get) {
            return Some(e.name.clone());
        }
        if let Some(c) = self.epsg.and_then(epsg::compound) {
            return Some(c.name.clone());
        }
        if let Some(v) = self.epsg.and_then(epsg::vertical) {
            return Some(v.name.clone());
        }
        if let Some(e) = self.esri.and_then(epsg::esri) {
            return Some(e.name.clone());
        }
        let registry = self.epsg.and_then(|code| wbp::to_ogc_wkt(code).ok());
        registry
            .as_deref()
            .and_then(wkt_name)
            .or_else(|| self.to_wkt().as_deref().and_then(wkt_name))
            .or_else(|| {
                self.epsg
                    .and_then(wbp::epsg::epsg_info)
                    .map(|i| i.name.to_string())
            })
    }

    /// Una etiqueta corta: `EPSG:4326` (o `ESRI:54009`) cuando se conoce el código y, si no, el
    /// nombre.
    pub fn label(&self) -> String {
        match (self.authority_code(), self.name()) {
            (Some(code), _) => code,
            (None, Some(name)) => name,
            (None, None) => "unknown".into(),
        }
    }

    /// Metros por unidad de las coordenadas del sistema, como los cuenta GDAL para los conjuntos de
    /// matrices de teselas: la unidad lineal de un sistema proyectado, la longitud de un grado
    /// sobre el semieje mayor en uno geográfico (1 cuando se desconoce).
    pub(crate) fn metres_per_unit(&self) -> f64 {
        let wkt = self.to_wkt().unwrap_or_default();
        if self.is_geographic() {
            return semi_major_axis(&wkt).unwrap_or(6_378_137.0) * std::f64::consts::PI / 180.0;
        }
        // El UNIT de WKT 1 o el LENGTHUNIT de WKT 2 (del sistema o, si no, de su primer eje).
        wkt1::parse(&wkt)
            .and_then(|root| {
                let unit = root.child("UNIT").or_else(|| root.child("LENGTHUNIT"));
                let unit =
                    unit.or_else(|| root.children("AXIS").find_map(|a| a.child("LENGTHUNIT")));
                unit.and_then(|u| u.num(1))
            })
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0)
    }

    /// Si es un sistema geográfico (longitud/latitud).
    pub fn is_geographic(&self) -> bool {
        match Def::new(self) {
            Ok(d) => d.is_geographic(),
            Err(_) => self.to_wbp().map(|c| c.is_geographic()).unwrap_or(false),
        }
    }

    /// Si es una rejilla de polo girado (`+proj=ob_tran` de PROJ, `rotated_latitude_longitude` de
    /// CF): sus coordenadas están en grados pero no son longitudes y latitudes, así que no es
    /// geográfico.
    pub fn is_rotated_pole(&self) -> bool {
        self.epsg.is_none() && Def::new(self).is_ok_and(|d| d.is_rotated_pole())
    }

    /// Si las coordenadas están en grados: un sistema geográfico o una rejilla de polo girado.
    pub(crate) fn is_angular(&self) -> bool {
        self.is_geographic() || self.is_rotated_pole()
    }

    /// Si ambos describen el mismo sistema, hasta donde se puede saber.
    pub fn same_as(&self, other: &SpatialRef) -> bool {
        match (self.authority(), other.authority()) {
            (Some(a), Some(b)) => a == b,
            _ => self.to_wkt().is_some() && self.to_wkt() == other.to_wkt(),
        }
    }

    /// Convierte al sistema del motor de proyecciones.
    pub(crate) fn to_wbp(&self) -> Result<wbp::Crs> {
        if let Some(code) = self.epsg
            && let Ok(c) = wbp::Crs::from_epsg(code)
        {
            return Ok(c);
        }
        // wbprojection leería una rejilla de polo girado como su sistema geográfico base.
        if self.is_rotated_pole() {
            return Err(Error::Crs(
                "a rotated-pole CRS has no wbprojection equivalent".into(),
            ));
        }
        // La cadena PROJ sobre su WKT, que wbprojection lee con menos parámetros.
        if let Some(p) = &self.proj4
            && let Ok(c) = wbp::from_proj_string(&mercator_lat_ts_as_k(p))
        {
            return Ok(c);
        }
        if let Some(w) = &self.wkt {
            return Ok(wbp::Crs::from_wkt(w)?);
        }
        Err(Error::Crs("the CRS is unknown".into()))
    }
}

/// El WKT 1 de una cadena PROJ, como lo escribe GDAL (véase [`wkt1_write`]).
fn proj_to_wkt(p: &str) -> Option<String> {
    wkt1_write::from_proj(p)
}

/// La cadena PROJ de una definición WKT tal como la lee terrio: su extensión PROJ4 o, si no, su
/// conversión desde WKT 1 o WKT 2. `None` para un WKT que terrio lee de otro modo (mediante
/// wbprojection).
pub(crate) fn wkt_proj(wkt: &str) -> Option<String> {
    if let Some(p) = proj4_extension(wkt) {
        return Some(p);
    }
    let root = wkt1::parse(wkt)?;
    let c = if wkt2::handles(&root) {
        wkt2::to_proj(&root).ok()?
    } else {
        wkt1::to_proj(&root)?
    };
    Some(c.proj)
}

/// La cadena PROJ del `EXTENSION["PROJ4", ...]` raíz de un WKT 1.
fn proj4_extension(wkt: &str) -> Option<String> {
    let root = wkt1::parse(wkt)?;
    let ext = root
        .children("EXTENSION")
        .find(|n| n.name().is_some_and(|k| k.eq_ignore_ascii_case("PROJ4")))?;
    match ext.args.get(1)? {
        wkt1::Arg::Str(p) if p.trim_start().starts_with('+') => Some(p.trim().to_string()),
        _ => None,
    }
}

/// Una cadena PROJ cuyo paralelo estándar de Mercator (`+lat_ts`) se da como la escala en el
/// ecuador que implica (`+k`): el Mercator de wbprojection no tiene paralelo estándar.
fn mercator_lat_ts_as_k(p: &str) -> std::borrow::Cow<'_, str> {
    let tokens: Vec<&str> = p.split_whitespace().collect();
    let lat_ts = tokens
        .iter()
        .find_map(|t| t.strip_prefix("+lat_ts="))
        .and_then(|v| v.parse::<f64>().ok());
    let (true, Some(ts)) = (tokens.contains(&"+proj=merc"), lat_ts) else {
        return p.into();
    };
    let Ok(crs) = wbp::from_proj_string(p) else {
        return p.into();
    };
    let (sin, e2) = (ts.to_radians().sin(), crs.datum.ellipsoid.e2);
    let k = ts.to_radians().cos() / (1.0 - e2 * sin * sin).sqrt();
    let kept = tokens
        .iter()
        .filter(|t| !t.starts_with("+lat_ts=") && !t.starts_with("+k=") && !t.starts_with("+k_0="));
    let mut out: Vec<String> = kept.map(|t| t.to_string()).collect();
    out.push(format!("+k={k}"));
    out.join(" ").into()
}

impl std::fmt::Display for SpatialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// Realizaciones de datum, que una definición WKT nombra cuando se refiere a ellas.
const REALIZATIONS: &[&str] = &[
    "2011", "HARN", "NSRS", "CSRS", "CORS", "MA11", "PA11", "NAD83(", "ITRF", "GDA2020", "ETRF",
];

/// El semieje mayor del primer `SPHEROID` o `ELLIPSOID` de una definición WKT.
fn semi_major_axis(wkt: &str) -> Option<f64> {
    let upper = wkt.to_ascii_uppercase();
    let start = upper
        .find("SPHEROID[")
        .or_else(|| upper.find("ELLIPSOID["))?;
    let body = &wkt[start..];
    let rest = &body[body.find('[')? + 1..];
    // Se salta el nombre entrecomillado y lee el número que va después.
    let after_name = rest
        .strip_prefix('"')
        .and_then(|r| r.find('"').map(|i| &r[i + 1..]))?;
    after_name
        .trim_start_matches([',', ' '])
        .split([',', ']'])
        .next()?
        .trim()
        .parse()
        .ok()
}

/// Los datums EPSG que es el datum geodésico de una definición WKT: el que declara su `AUTHORITY`
/// (WKT 1) o `ID` (WKT 2) o, si no, aquellos con cuyo nombre se le llama (`D_European_1950` es
/// ED50). Vacío para un datum con otro nombre (`Unknown based on Bessel 1841 ellipsoid`), como en
/// GDAL.
fn wkt_datums(wkt: &str) -> Vec<u32> {
    wkt_datums_and_others(wkt).0
}

/// Los datums EPSG del datum geodésico de una definición WKT ([`wkt_datums`]) y los demás datums
/// cuyos sistemas escribe ESRI con su nombre (ETRS89-NOR para `D_ETRS_1989`).
fn wkt_datums_and_others(wkt: &str) -> (Vec<u32>, &'static [u32]) {
    fn find(n: &wkt1::Node) -> Option<&wkt1::Node> {
        const DATUMS: [&str; 5] = ["DATUM", "GEODETICDATUM", "TRF", "ENSEMBLE", "DATUMENSEMBLE"];
        if DATUMS.iter().any(|k| n.key.eq_ignore_ascii_case(k)) {
            return Some(n);
        }
        n.args.iter().find_map(|a| match a {
            wkt1::Arg::Node(c) => find(c),
            _ => None,
        })
    }
    let Some(datum) = wkt1::parse(wkt).and_then(|root| find(&root).cloned()) else {
        return (Vec::new(), &[]);
    };
    if let Some(code) = datum.epsg() {
        return (vec![code], &[]);
    }
    let name = datum.name().unwrap_or_default();
    (epsg::datums_named(name), epsg::datums_written_as(name))
}

/// Identifica el código EPSG de una definición WKT, como lo hace GDAL: un sistema sobre el datum
/// del WKT (por su código o nombre) con la misma conversión, unidades y elipsoide. Un datum sin
/// código que terrio no conoce por el nombre no tiene código, sea cual sea su elipsoide (#125).
/// Entre los sistemas de la tabla de terrio que coinciden, el que se llama como el sistema del WKT,
/// luego el que tiene su orden de ejes y luego el código más bajo; para otros sistemas, el registro
/// de wbprojection, donde entre coincidencias igual de buenas se prefiere el datum base a sus
/// realizaciones posteriores (NAD83 sobre NAD83(HARN) y NAD83(2011)) salvo que el WKT nombre una, y
/// luego el código más bajo.
pub(crate) fn identify_epsg(wkt: &str) -> Option<u32> {
    if let Some(code) = declared_epsg(wkt) {
        return Some(code);
    }
    let (datums, others) = wkt_datums_and_others(wkt);
    // Sin datum conocido (o sin nodo de datum, o WKT que el lector no puede analizar): ningún
    // código, y a propósito no la conjetura imprecisa del registro (#125).
    if datums.is_empty() {
        return None;
    }
    let own = Def::wkt_conversion(wkt);
    // La misma conversión sobre el mismo elipsoide.
    let same = |own: &Def, code: u32| {
        let registered = SpatialRef {
            epsg: Some(code),
            ..Default::default()
        };
        Def::new(&registered).is_ok_and(|d| {
            own.same_conversion(&d)
                && (own.ell.a - d.ell.a).abs() < 1e-3
                && (own.ell.es - d.ell.es).abs() < 1e-9
        })
    };
    // Entre los sistemas que coinciden, el que se llama como el sistema del WKT (`RGF93 v2
    // (lon-lat)`, sin contar el prefijo `GCS_` de ESRI) o, si no, el datum base sobre sus
    // realizaciones posteriores (`NAD83`, que EPSG lista también como alias de NAD83(HARN)) salvo
    // que el WKT nombre una, o si no el código más bajo.
    let key = |n: &str| epsg::datum_key(n.strip_prefix("GCS_").unwrap_or(n));
    let name = wkt_name(wkt).map(|n| key(&n));
    let named = |c: u32| epsg::get(c).is_some_and(|e| Some(key(&e.name)) == name);
    let upper = wkt.to_ascii_uppercase();
    let wkt_names_realization = REALIZATIONS.iter().any(|r| upper.contains(r));
    let realization = |code: u32| {
        let name = wbp::epsg::epsg_info(code)
            .map(|i| i.name.to_ascii_uppercase())
            .unwrap_or_default();
        !wkt_names_realization && REALIZATIONS.iter().any(|r| name.contains(r))
    };
    // Un sistema de la tabla sobre estos datums con la conversión del WKT, como lo encuentra GDAL.
    let search = |datums: &[u32]| {
        let own = own.as_ref()?;
        let geogs = datums.iter().flat_map(|d| epsg::geographic_on(*d)).copied();
        if own.is_geographic() {
            return geogs
                .filter(|c| same(own, *c))
                .min_by_key(|c| (!named(*c), realization(*c), *c));
        }
        let method = conversion_method(wkt)?;
        let found = geogs
            .flat_map(epsg::projected_on)
            .filter(|(c, m)| same_method(m, &method) && same(own, *c))
            .map(|(c, _)| *c);
        // Si no, el que tiene el orden de ejes del WKT (ETRS89 / UTM zone 32N en lugar de su
        // variante N-E para el WKT de ESRI, sin ejes).
        let own_north_first = north_first(wkt);
        let axes = |c: u32| epsg::get(c).is_some_and(|e| north_first(&e.wkt) != own_north_first);
        found.min_by_key(|c| (!named(*c), axes(*c), realization(*c), *c))
    };
    // Sobre el datum que nombra el WKT o, si no, sobre aquellos cuyos sistemas escribe ESRI con su
    // nombre: GDAL identifica el ETRS_1989_NTM_Zone_18 de ESRI sobre D_ETRS_1989 por su nombre como
    // EPSG:5118, sobre ETRS89-NOR.
    if let Some(code) = search(&datums).or_else(|| search(others)) {
        return Some(code);
    }
    // Un sistema vigente sobre el datum del WKT. Su datum: su código en la tabla o, si no, el de su
    // WKT (sistemas WKT 2 de la tabla, sobre conjuntos de datums, y los que solo tiene el
    // registro). Un código obsoleto tiene en la tabla el WKT de su sustituto.
    let on_datum = |code: u32| {
        let theirs = match epsg::get(code) {
            Some(e) if declared_epsg(&e.wkt).is_some_and(|c| c != code) => return false,
            Some(e) if e.datum != 0 => vec![e.datum],
            Some(e) => wkt_datums(&e.wkt),
            None => wbp::to_ogc_wkt(code)
                .map(|w| wkt_datums(&w))
                .unwrap_or_default(),
        };
        theirs.iter().any(|d| datums.contains(d))
    };
    // Si no, el registro de wbprojection (proyecciones que terrio no puede comparar, sistemas que
    // su tabla no tiene).
    let report = wbp::identify_epsg_from_wkt_report(wkt, wbp::EpsgIdentifyPolicy::Lenient)?;
    // El registro coincide con imprecisión: se conserva un código solo cuando su definición
    // convierte las coordenadas como el WKT, sobre su datum (GDAL deja un sistema personalizado sin
    // código).
    let fits = |code: u32| {
        if !on_datum(code) {
            return false;
        }
        // Una proyección que terrio no puede comparar: al menos el mismo elipsoide (una proyección
        // sinusoidal sobre la esfera de MODIS no es la World Sinusoidal de ESRI sobre WGS 84).
        let Some(own) = &own else {
            let candidate = wbp::to_ogc_wkt(code).ok();
            return match (
                semi_major_axis(wkt),
                candidate.as_deref().and_then(semi_major_axis),
            ) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-3,
                _ => true,
            };
        };
        same(own, code)
    };
    // El registro puede tomar el código de un datum o de un meridiano origen por el del sistema:
    // entonces se hace coincidir el WKT solo por sus parámetros.
    if report.used_embedded_epsg
        && !report
            .resolved_code
            .is_some_and(|c| wbp::to_ogc_wkt(c).is_ok())
    {
        let stripped = strip_authorities(wkt);
        return if stripped != wkt {
            identify_epsg(&stripped)
        } else {
            None
        };
    }
    if report.used_embedded_epsg || !report.ambiguous {
        return report.resolved_code.filter(|c| fits(*c));
    }
    let best = report.top_candidates.first()?.total_score;
    report
        .top_candidates
        .iter()
        .filter(|c| c.total_score >= best && fits(c.code))
        .min_by_key(|c| (realization(c.code), c.code))
        .map(|c| c.code)
        .or(report.resolved_code.filter(|c| fits(*c)))
}

/// El método de proyección (`+proj=tmerc`) de la conversión de una definición WKT.
fn conversion_method(wkt: &str) -> Option<String> {
    let root = wkt1::parse(wkt)?;
    let c = if wkt2::handles(&root) {
        wkt2::to_proj(&root).ok()?
    } else {
        wkt1::to_proj(&root)?
    };
    c.proj.split_whitespace().next().map(str::to_string)
}

/// Si el primer eje del sistema de coordenadas de una definición WKT apunta al norte o al sur
/// (norte, este) en lugar de al este (o no se da).
fn north_first(wkt: &str) -> bool {
    let first =
        wkt1::parse(wkt).and_then(|root| root.child("AXIS").and_then(|a| a.args.get(1).cloned()));
    matches!(first, Some(wkt1::Arg::Ident(d)) if ["north", "south"].iter().any(|n| d.eq_ignore_ascii_case(n)))
}

/// Si dos métodos `+proj=` son el mismo (UTM es una Mercator transversa).
fn same_method(a: &str, b: &str) -> bool {
    let m = |s: &str| {
        if s == "+proj=utm" {
            "+proj=tmerc".to_string()
        } else {
            s.to_string()
        }
    };
    m(a) == m(b)
}

/// Una definición WKT sin sus nodos `AUTHORITY[...]`.
fn strip_authorities(wkt: &str) -> String {
    let mut out = wkt.to_string();
    while let Some(start) = out.find(",AUTHORITY[") {
        let Some(len) = out[start..].find(']') else {
            break;
        };
        out.replace_range(start..start + len + 1, "");
    }
    out
}

/// El código EPSG que una definición WKT declara para sí misma: el `AUTHORITY["EPSG","4269"]` del
/// WKT 1 o el `ID["EPSG",4269]` del WKT 2 que cierra su nodo más externo, no los de su datum o sus
/// unidades.
fn declared_epsg(wkt: &str) -> Option<u32> {
    declared_code(wkt, "EPSG")
}

/// El código de una autoridad que una definición WKT declara para sí misma, como [`declared_epsg`].
fn declared_code(wkt: &str, authority: &str) -> Option<u32> {
    let body = wkt.trim_end().strip_suffix(']')?.trim_end();
    // El último nodo hijo tiene que ser el propio identificador, no un nodo que contenga uno.
    let body = body.strip_suffix(']')?;
    if body.ends_with(']') {
        return None;
    }
    let start = body.rfind('[')?;
    let keyword = body[..start]
        .trim_end()
        .rsplit([',', '['])
        .next()?
        .trim()
        .to_ascii_uppercase();
    if keyword != "AUTHORITY" && keyword != "ID" {
        return None;
    }
    let mut parts = body[start + 1..]
        .split(',')
        .map(|p| p.trim().trim_matches('"'));
    if !parts.next()?.eq_ignore_ascii_case(authority) {
        return None;
    }
    parts.next()?.parse().ok()
}

/// El código ESRI de una definición WKT: el que declara (`AUTHORITY["ESRI","54009"]`) o el del
/// sistema ESRI del mismo nombre (las cadenas PE de ESRI de GDAL no llevan ninguno) cuya definición
/// convierte las coordenadas como el WKT, sobre el mismo elipsoide.
fn identify_esri(wkt: &str) -> Option<u32> {
    if let Some(code) = declared_code(wkt, "ESRI") {
        return epsg::esri(code).is_some().then_some(code);
    }
    let name = wkt_name(wkt)?;
    let candidates = epsg::esri_named(&name);
    if candidates.is_empty() {
        return None;
    }
    // La conversión del WKT, mediante wbprojection para las proyecciones que el lector de WKT de
    // terrio no sabe asignar; un nombre por sí solo no identifica un sistema.
    let own = Def::wkt_conversion(wkt).or_else(|| {
        Def::new(&SpatialRef {
            wkt: Some(wkt.to_string()),
            ..Default::default()
        })
        .ok()
    })?;
    let a = semi_major_axis(wkt);
    candidates.into_iter().find(|code| {
        let Some(e) = epsg::esri(*code) else {
            return false;
        };
        let Ok(registered) = Def::from_proj(&e.proj) else {
            return false;
        };
        let same_ellipsoid = a.is_none_or(|a| (a - registered.ell.a).abs() < 1e-3);
        same_ellipsoid && own.same_conversion(&registered)
    })
}

/// El código ESRI de una referencia como `ESRI:54009` (sea cual sea su capitalización, como la lee
/// PROJ), `urn:ogc:def:crs:ESRI::54009` o `http://www.opengis.net/def/crs/ESRI/0/54009`, dado en
/// mayúsculas.
fn esri_reference(upper: &str) -> Option<u32> {
    let code = upper
        .strip_prefix("ESRI:")
        .or_else(|| {
            upper
                .strip_prefix("URN:OGC:DEF:CRS:ESRI:")
                .map(|r| r.rsplit(':').next().unwrap_or(r))
        })
        .or_else(|| {
            upper
                .split_once("/DEF/CRS/ESRI/")
                .map(|(_, r)| r.rsplit('/').next().unwrap_or(r))
        })?;
    code.trim().parse().ok()
}

fn looks_like_wkt(s: &str) -> bool {
    let head: String = s
        .chars()
        .take_while(|c| *c != '[' && *c != '(')
        .collect::<String>()
        .trim()
        .to_ascii_uppercase();
    matches!(
        head.as_str(),
        "GEOGCS"
            | "PROJCS"
            | "GEOCCS"
            | "COMPD_CS"
            | "GEOGCRS"
            | "GEOGRAPHICCRS"
            | "PROJCRS"
            | "PROJECTEDCRS"
            | "GEODCRS"
            | "GEODETICCRS"
            | "BOUNDCRS"
            | "COMPOUNDCRS"
            | "VERTCRS"
            | "VERT_CS"
    )
}

/// El nombre en el nodo más externo de una definición WKT.
fn wkt_name(wkt: &str) -> Option<String> {
    let start = wkt.find('"')? + 1;
    let end = start + wkt[start..].find('"')?;
    let name = &wkt[start..end];
    // Las definiciones generadas añaden el código: `WGS 84 / UTM zone 33N (EPSG:32633)`.
    let name = match name.rfind(" (EPSG:") {
        Some(i) if name.ends_with(')') => &name[..i],
        _ => name,
    };
    Some(name.to_string())
}

/// Opciones de un [`Transformer`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransformOptions {
    /// La época de coordenadas de las coordenadas de origen, como año decimal (`2010.5`), para un
    /// sistema de origen dinámico (`-s_coord_epoch` de GDAL).
    pub source_coordinate_epoch: Option<f64>,
    /// La época de coordenadas de las coordenadas de destino, para un sistema de destino dinámico
    /// (`-t_coord_epoch` de GDAL).
    pub target_coordinate_epoch: Option<f64>,
    /// Las coordenadas de origen llevan una altura elipsoidal (para un sistema sin parte vertical).
    pub source_ellipsoidal_height: bool,
    /// Las coordenadas de destino deben llevar una altura elipsoidal.
    pub target_ellipsoidal_height: bool,
}

/// Transforma coordenadas de un sistema a otro, como lo hace GDAL: unidades lineales y angulares,
/// orden de ejes y meridianos origen como los definen los sistemas y, cuando los datums difieren,
/// la transformación de datum que GDAL (PROJ) elige para cada punto entre las que lista para el
/// par, o encadenando a través de un datum intermedio como WGS 84. Las rejillas de ajuste de datum
/// se usan cuando se encuentran, como las usa GDAL; las alturas de los sistemas geográficos 3D y
/// compuestos pasan por modelos de geoide; las transformaciones dependientes del tiempo usan la
/// época de coordenadas (véase [`TransformOptions`]).
pub struct Transformer {
    src: Def,
    dst: Def,
    identity: bool,
    route: route::Route,
    /// El tiempo de las transformaciones dependientes del tiempo, como lo fija GDAL a partir de las
    /// épocas de coordenadas.
    time: Option<f64>,
    /// La transformación de alturas, entre sistemas que tienen ambos alturas.
    heights: Option<Box<Heights>>,
    /// Una operación de movimiento de puntos que mueve coordenadas de un datum entre dos épocas de
    /// coordenadas (las rejillas de velocidades de Canadá de NAD83(CSRS), NKG_RF17vel), como la
    /// aplica PROJ.
    motion: Option<Vec<datum::Step>>,
    /// Las transformaciones que PROJ lista entre los sistemas verticales de los sistemas compuestos
    /// (desplazamientos, rejillas VERTCON...), que se usan en lugar de pasar por alturas
    /// elipsoidales cuando las hay, cada una con la ruta horizontal desde el origen hasta el
    /// sistema geográfico en el que interpola, cuando este está sobre otro datum.
    vertical_ops: Vec<(std::sync::Arc<datum::Op>, Option<route::Route>)>,
    /// El sistema horizontal del origen, del que parten las rutas de `vertical_ops`.
    src_horizontal: Def,
    /// Si los sistemas verticales comparten datum (las alturas solo cambian de unidad).
    same_vertical_datum: bool,
}

/// Un lado de una transformación de alturas: el sistema y los sistemas geográficos 3D de sus
/// modelos de geoide.
struct Side {
    /// El propio sistema y luego los sistemas geográficos de sus modelos de geoide.
    defs: Vec<Def>,
    /// Los modelos de geoide (de alturas elipsoidales a las alturas del sistema), por índice en
    /// `defs`.
    models: Vec<(usize, Vec<std::sync::Arc<datum::Op>>)>,
    /// Rutas horizontales entre el sistema y cada uno de `defs` (desde el sistema para el lado de
    /// origen, hacia él para el de destino).
    routes: Vec<Option<route::Route>>,
}

/// Cómo se transforman las alturas: por alturas elipsoidales, desde el sistema de origen (o uno de
/// los sistemas de sus modelos de geoide) hasta el sistema de destino (o uno de los de sus modelos
/// de geoide).
struct Heights {
    src: Side,
    dst: Side,
    /// Rutas entre los sistemas elipsoidales de ambos lados, construidas la primera vez que se
    /// usan.
    middle:
        std::sync::Mutex<std::collections::HashMap<(usize, usize), std::sync::Arc<route::Route>>>,
}

impl Side {
    fn new(def: &Def, source: bool) -> Self {
        let mut s = Side {
            defs: vec![def.clone()],
            models: Vec::new(),
            routes: vec![None],
        };
        let Some(v) = def.vertical() else { return s };
        let horizontal = def.horizontal();
        // Primero los modelos sobre el datum del propio sistema, como los prefiere PROJ.
        let mut models: Vec<&(Option<u32>, Vec<std::sync::Arc<datum::Op>>)> =
            v.models.iter().collect();
        let own = |g: &Option<u32>| match g {
            None => true,
            Some(g) => epsg::get(*g).is_some_and(|e| {
                Some(e.geog) == def.geog() || (e.datum != 0 && Some(e.datum) == def.datum_code())
            }),
        };
        models.sort_by_key(|(g, _)| !own(g));
        for (g, ops) in models {
            if own(g) {
                s.models.push((0, ops.clone()));
                continue;
            }
            let Some(gd) = g.and_then(def_of) else {
                continue;
            };
            let r = if source {
                route::Route::new(&horizontal, &gd)
            } else {
                route::Route::new(&gd, &horizontal)
            };
            s.defs.push(gd);
            s.routes.push(Some(r));
            s.models.push((s.defs.len() - 1, ops.clone()));
        }
        s
    }
}

/// La operación de movimiento de puntos de un datum durante un intervalo de tiempo (años), de
/// `crates/terrio/data/point_motion.tsv` (`scripts/gen_point_motion.py`).
fn point_motion(datum: u32, dt: f64) -> Option<Vec<datum::Step>> {
    static TABLE: &str = include_str!("../data/point_motion.tsv");
    let line = TABLE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find(|l| l.split('\t').next().and_then(|c| c.parse::<u32>().ok()) == Some(datum))?;
    let steps = line.split('\t').nth(2)?.replace("{dt}", &format!("{dt:?}"));
    datum::parse_steps(&steps)
}

/// La definición de un sistema geográfico 3D de EPSG.
fn def_of(code: u32) -> Option<Def> {
    Def::new(&SpatialRef {
        epsg: Some(code),
        ..Default::default()
    })
    .ok()
}

impl Heights {
    fn middle(&self, i: usize, j: usize) -> std::sync::Arc<route::Route> {
        let mut m = self.middle.lock().unwrap_or_else(|e| e.into_inner());
        m.entry((i, j))
            .or_insert_with(|| {
                std::sync::Arc::new(route::Route::new(&self.src.defs[i], &self.dst.defs[j]))
            })
            .clone()
    }
}

/// Elige y ejecuta un modelo de geoide para un punto (grados para las áreas), `reverse` de alturas
/// a alturas elipsoidales.
fn run_model(
    ops: &[std::sync::Arc<datum::Op>],
    p: datum::Point,
    reverse: bool,
) -> Option<datum::Point> {
    let (lon, lat) = (p.lon.to_degrees(), p.lat.to_degrees());
    datum::transform_with(
        ops,
        |op, _| op.contains(lon, lat),
        |i| datum::run_op(&ops[i], p, reverse, true),
    )
}

impl Transformer {
    /// Un transformador de `src` a `dst`.
    pub fn new(src: &SpatialRef, dst: &SpatialRef) -> Result<Self> {
        Self::with_options(src, dst, &TransformOptions::default())
    }

    /// Un transformador de `src` a `dst` con opciones, como las épocas de coordenadas de las
    /// transformaciones dependientes del tiempo.
    ///
    /// Como hace GDAL, se usa la época de origen cuando el sistema de origen es dinámico y el de
    /// destino estático, la de destino cuando el de destino es dinámico y el de origen estático, y
    /// la de origen cuando ambos son dinámicos y se dan las dos épocas.
    pub fn with_options(
        src: &SpatialRef,
        dst: &SpatialRef,
        opts: &TransformOptions,
    ) -> Result<Self> {
        crate::spain::init();
        let (mut s, mut d) = (Def::new(src)?, Def::new(dst)?);
        if opts.source_ellipsoidal_height && s.vertical().is_none() {
            s = s.with_ellipsoidal_height();
        }
        if opts.target_ellipsoidal_height && d.vertical().is_none() {
            d = d.with_ellipsoidal_height();
        }
        let time = match (
            s.is_dynamic(),
            d.is_dynamic(),
            opts.source_coordinate_epoch,
            opts.target_coordinate_epoch,
        ) {
            (true, false, Some(e), _) => Some(e),
            (false, true, _, Some(e)) => Some(e),
            (true, true, Some(e), Some(_)) => Some(e),
            _ => None,
        };
        let route = route::Route::new(&s, &d);
        let motion = match (
            s.datum_code(),
            opts.source_coordinate_epoch,
            opts.target_coordinate_epoch,
        ) {
            // Entre sistemas geográficos del datum, como lo aplica PROJ (no a los proyectados).
            (Some(datum), Some(from), Some(to))
                if s.datum_code() == d.datum_code()
                    && from != to
                    && s.is_geographic()
                    && d.is_geographic() =>
            {
                point_motion(datum, to - from)
            }
            _ => None,
        };
        let heights = (s.has_height() && d.has_height()).then(|| {
            Box::new(Heights {
                src: Side::new(&s, true),
                dst: Side::new(&d, false),
                middle: std::sync::Mutex::new(std::collections::HashMap::new()),
            })
        });
        let codes = (
            s.vertical().and_then(|v| v.code),
            d.vertical().and_then(|v| v.code),
        );
        let (vertical_ops, same_vertical_datum) = match codes {
            (Some(a), Some(b)) if a != b => {
                let datum = |c: u32| epsg::vertical(c).map(|v| v.datum).filter(|d| *d != 0);
                let same = datum(a).is_some() && datum(a) == datum(b);
                let ops = if same {
                    Vec::new()
                } else {
                    epsg::vertical_transformations(a, b)
                };
                let horizontal = s.horizontal();
                let ops = ops
                    .into_iter()
                    .map(|v| {
                        // Sobre el sistema geográfico 2D de uno 3D (ETRS89 para EPSG:4937).
                        let route = v
                            .interpolation
                            .map(|g| epsg::get(g).map_or(g, |e| e.geog))
                            .and_then(def_of)
                            .filter(|g| {
                                g.datum_code().is_none()
                                    || g.datum_code() != horizontal.datum_code()
                            })
                            .map(|g| route::Route::new(&horizontal, &g));
                        (v.op, route)
                    })
                    .collect();
                (ops, same)
            }
            _ => (Vec::new(), false),
        };
        let src_horizontal = s.horizontal();
        let identity = src.same_as(dst) && motion.is_none();
        Ok(Self {
            identity,
            src: s,
            dst: d,
            route,
            time,
            heights,
            motion,
            vertical_ops,
            same_vertical_datum,
            src_horizontal,
        })
    }

    /// Transforma un punto.
    pub fn transform(&self, x: f64, y: f64) -> Result<(f64, f64)> {
        if self.identity {
            return Ok((x, y));
        }
        let (x, y, _) = self.transform_2d(x, y, 0.0, self.time)?;
        Ok((x, y))
    }

    /// La transformación 2D de un punto, conservando su altura (o, en las transformaciones que PROJ
    /// ejecuta sin conservar las alturas, transformándola).
    fn transform_2d(&self, x: f64, y: f64, z: f64, t: Option<f64>) -> Result<(f64, f64, f64)> {
        let fail = || Error::Crs(format!("cannot transform ({x}, {y})"));
        let (lon, lat) = self.src.unproject(x, y).ok_or_else(fail)?;
        let p = datum::Point { lon, lat, h: z, t };
        let p = match &self.motion {
            Some(steps) => datum::run(steps, p, false, true).ok_or_else(|| {
                let grids: Vec<String> = steps.iter().flat_map(|s| s.grid_names()).map(|(g, _)| g).collect();
                Error::Crs(format!(
                    "cannot move ({x}, {y}) between coordinate epochs: outside the velocity grid, or the grid ({}) is not available",
                    grids.join(", ")
                ))
            })?,
            None => p,
        };
        let q = self
            .route
            .apply(&self.src, x, y, p, false)
            .ok_or_else(fail)?;
        let (tx, ty) = self.dst.project(q.lon, q.lat).ok_or_else(fail)?;
        Ok((tx, ty, q.h))
    }

    /// Transforma un punto con altura. Entre sistemas que tienen ambos alturas (geográficos 3D y
    /// compuestos), las alturas pasan por la transformación de datum y los modelos de geoide como
    /// en GDAL; si no, se conservan, como las conserva GDAL entre sistemas 2D.
    pub fn transform_3d(&self, x: f64, y: f64, z: f64) -> Result<(f64, f64, f64)> {
        self.transform_at(x, y, z, self.time)
    }

    /// Transforma un punto con altura bajo una política: el resultado y las aproximaciones por las
    /// que pasó (vacío cuando salió de la mejor operación). Un punto cuyas aproximaciones rechaza
    /// la política es un error, nunca un valor.
    pub fn transform_checked(
        &self,
        x: f64,
        y: f64,
        z: f64,
        policy: policy::Policy,
    ) -> Result<((f64, f64, f64), policy::Approximations)> {
        let _ = policy::take();
        let r = self.transform_3d(x, y, z);
        let notes = policy::take();
        let v = r?;
        if let Some(a) = notes.refused_by(policy) {
            return Err(Error::Policy(format!(
                "({x}, {y}): {}; la política estricta lo rechaza (HIPSO_ALLOW_BALLPARK, HIPSO_ONLY_BEST)",
                a.describe()
            )));
        }
        Ok((v, notes))
    }

    /// Transforma un punto con altura en un instante (un año decimal), para las transformaciones
    /// dependientes del tiempo; el instante sustituye a la época de coordenadas de las opciones.
    pub fn transform_4d(&self, x: f64, y: f64, z: f64, t: f64) -> Result<(f64, f64, f64)> {
        self.transform_at(x, y, z, Some(t))
    }

    fn transform_at(&self, x: f64, y: f64, z: f64, t: Option<f64>) -> Result<(f64, f64, f64)> {
        if self.identity {
            return Ok((x, y, z));
        }
        let Some(h) = &self.heights else {
            return self.transform_2d(x, y, z, t);
        };
        // Un movimiento de puntos entre épocas mueve las alturas elipsoidales con la posición.
        if self.motion.is_some() && self.src.vertical().is_none() && self.dst.vertical().is_none() {
            return self.transform_2d(x, y, z, t);
        }
        // Sistemas compuestos con el mismo sistema vertical, o sistemas verticales de un mismo
        // datum: las alturas solo cambian de unidad.
        if let (Some(a), Some(b)) = (self.src.vertical(), self.dst.vertical())
            && a.code.is_some()
            && (a.code == b.code || self.same_vertical_datum)
        {
            let (tx, ty, _) = self.transform_2d(x, y, 0.0, t)?;
            return Ok((tx, ty, b.height_of(a.to_metres(z))));
        }
        // Una transformación entre los sistemas verticales (como la prefiere PROJ a los modelos de
        // geoide).
        if let (Some(a), Some(b), false) = (
            self.src.vertical(),
            self.dst.vertical(),
            self.vertical_ops.is_empty(),
        ) {
            let (tx, ty, _) = self.transform_2d(x, y, 0.0, t)?;
            let (lon, lat) = self
                .src
                .unproject(x, y)
                .ok_or_else(|| Error::Crs(format!("cannot transform ({x}, {y}, {z})")))?;
            // En metros, en la dirección del eje del origen: los pasos convierten las alturas en
            // profundidades.
            let p = datum::Point {
                lon,
                lat,
                h: z * a.to_meter,
                t,
            };
            return Ok((
                tx,
                ty,
                match self.vertical_height(x, y, p) {
                    Some(h) => h / b.to_meter,
                    // No se aplica ninguna: las alturas se conservan, como en la aproximación de
                    // PROJ.
                    None => {
                        policy::note(policy::Approximation::HeightKept);
                        b.height_of(a.to_metres(z))
                    }
                },
            ));
        }
        let fail = || Error::Crs(format!("cannot transform ({x}, {y}, {z})"));
        let (lon, lat) = self.src.unproject(x, y).ok_or_else(fail)?;
        let p0 = datum::Point {
            lon,
            lat,
            h: self.src.vertical().map_or(z, |v| v.to_metres(z)),
            t,
        };
        // A alturas elipsoidales, sobre el sistema de origen o el de un modelo de geoide.
        let mut start = None;
        for (i, ops) in &h.src.models {
            let q = match &h.src.routes[*i] {
                Some(r) => r.apply(&self.src, x, y, p0, false),
                None => Some(p0),
            };
            if let Some(r) = q.and_then(|q| run_model(ops, q, true)) {
                start = Some((*i, r));
                break;
            }
        }
        // Sin modelo de geoide: las alturas se conservan (la transformación vertical aproximada de
        // PROJ).
        if start.is_none() && self.src.vertical().is_some() {
            policy::note(policy::Approximation::HeightKept);
        }
        if start.is_none() && self.src.vertical().is_some_and(|v| !v.ballpark) {
            return Err(fail());
        }
        let (i, p) = start.unwrap_or((0, p0));
        let (sx, sy) = if i == 0 {
            (x, y)
        } else {
            (p.lon.to_degrees(), p.lat.to_degrees())
        };
        // De alturas elipsoidales al destino.
        for (j, ops) in &h.dst.models {
            let Some(q) = h.middle(i, *j).apply(&h.src.defs[i], sx, sy, p, true) else {
                continue;
            };
            let Some(r) = run_model(ops, q, false) else {
                continue;
            };
            let r = match &h.dst.routes[*j] {
                Some(route) => route
                    .apply(
                        &h.dst.defs[*j],
                        r.lon.to_degrees(),
                        r.lat.to_degrees(),
                        r,
                        false,
                    )
                    .ok_or_else(fail)?,
                None => r,
            };
            let (tx, ty) = self.dst.project(r.lon, r.lat).ok_or_else(fail)?;
            return Ok((
                tx,
                ty,
                self.dst.vertical().map_or(r.h, |v| v.height_of(r.h)),
            ));
        }
        if self.dst.vertical().is_some() {
            policy::note(policy::Approximation::HeightKept);
        }
        if self.dst.vertical().is_some_and(|v| !v.ballpark) {
            return Err(fail());
        }
        let q = h
            .middle(i, 0)
            .apply(&h.src.defs[i], sx, sy, p, true)
            .ok_or_else(fail)?;
        let (tx, ty) = self.dst.project(q.lon, q.lat).ok_or_else(fail)?;
        Ok((
            tx,
            ty,
            self.dst.vertical().map_or(q.h, |v| v.height_of(q.h)),
        ))
    }

    /// La altura de un punto (metros, en la dirección del eje del origen) tras la transformación
    /// entre los sistemas verticales que PROJ elige para él: la más precisa cuya área contiene el
    /// punto, contando en una transformación que interpola en el sistema geográfico de otro datum
    /// la precisión de la transformación horizontal hasta él en ambos sentidos (como la encadena
    /// PROJ); la siguiente cuando falla (fuera de su rejilla), hasta tres. `None` cuando no se
    /// aplica ninguna.
    fn vertical_height(&self, x: f64, y: f64, p: datum::Point) -> Option<f64> {
        let horizontal = &self.src_horizontal;
        let (dlon, dlat) = (p.lon.to_degrees(), p.lat.to_degrees());
        let mut candidates: Vec<(usize, f64)> = self
            .vertical_ops
            .iter()
            .enumerate()
            .filter(|(_, (op, _))| op.contains(dlon, dlat))
            .map(|(i, (op, route))| {
                let accuracy = match route {
                    None => op.accuracy,
                    Some(r) => match r.accuracy_at(horizontal, x, y, p) {
                        h if h >= 0.0 && op.accuracy >= 0.0 => op.accuracy + 2.0 * h,
                        _ => -1.0,
                    },
                };
                (i, accuracy)
            })
            .collect();
        // PROJ ordena las transformaciones encadenadas por sus precisiones (las conocidas primero),
        // luego elige entre ellas en ese orden (`pj_get_suggested_operation`) y después la
        // siguiente mejor cuando una falla.
        candidates.sort_by(|a, b| {
            (a.1 < 0.0)
                .cmp(&(b.1 < 0.0))
                .then(a.1.max(0.0).total_cmp(&b.1.max(0.0)))
        });
        let mut excluded: Vec<usize> = Vec::new();
        for _ in 0..3 {
            let mut best: Option<(usize, f64)> = None;
            for &(i, accuracy) in candidates.iter().filter(|(i, _)| !excluded.contains(i)) {
                let op = &self.vertical_ops[i].0;
                if best.is_none_or(|(b, ba)| {
                    datum::better_by(op, accuracy, &self.vertical_ops[b].0, ba)
                }) {
                    best = Some((i, accuracy));
                }
            }
            let (i, _) = best?;
            let (op, route) = &self.vertical_ops[i];
            // La posición del punto en el sistema de interpolación; su altura se queda como está.
            let q = match route {
                Some(r) => r
                    .apply(horizontal, x, y, p, false)
                    .map(|q| datum::Point { h: p.h, ..q }),
                None => Some(p),
            };
            if let Some(h) = q
                .and_then(|q| datum::run_op(op, q, false, true))
                .map(|r| r.h)
            {
                return Some(h);
            }
            excluded.push(i);
        }
        None
    }

    /// Transforma una caja envolvente `(min_x, min_y, max_x, max_y)`, densificando sus bordes para
    /// cubrir los bordes curvos en el sistema de destino. Los puntos que no se transforman se
    /// omiten.
    pub fn transform_bounds(&self, b: (f64, f64, f64, f64)) -> Result<(f64, f64, f64, f64)> {
        const STEPS: usize = 20;
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for i in 0..=STEPS {
            let t = i as f64 / STEPS as f64;
            let x = b.0 + (b.2 - b.0) * t;
            let y = b.1 + (b.3 - b.1) * t;
            for (px, py) in [(x, b.1), (x, b.3), (b.0, y), (b.2, y)] {
                if let Ok((tx, ty)) = self.transform(px, py)
                    && tx.is_finite()
                    && ty.is_finite()
                {
                    x0 = x0.min(tx);
                    y0 = y0.min(ty);
                    x1 = x1.max(tx);
                    y1 = y1.max(ty);
                }
            }
        }
        if !x0.is_finite() {
            return Err(Error::Crs(
                "no point of the bounds could be transformed".into(),
            ));
        }
        Ok((x0, y0, x1, y1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metres_per_unit_of_tile_matrix_sets() {
        assert!((SpatialRef::from_epsg(3857).unwrap().metres_per_unit() - 1.0).abs() < 1e-12);
        assert!(
            (SpatialRef::from_epsg(2263).unwrap().metres_per_unit() - 0.3048006096012192).abs()
                < 1e-12
        );
        let deg = 6_378_137.0 * std::f64::consts::PI / 180.0;
        assert!((SpatialRef::parse("OGC:CRS84").unwrap().metres_per_unit() - deg).abs() < 1e-6);
        // WKT 2: el LENGTHUNIT de los ejes del sistema de coordenadas.
        let wkt2 = r#"PROJCRS["x",BASEGEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563]]],CONVERSION["c",METHOD["Transverse Mercator"],PARAMETER["False easting",500000,LENGTHUNIT["metre",1]]],CS[Cartesian,2],AXIS["easting",east,ORDER[1],LENGTHUNIT["foot",0.3048]],AXIS["northing",north,ORDER[2],LENGTHUNIT["foot",0.3048]]]"#;
        assert!((SpatialRef::from_wkt(wkt2).metres_per_unit() - 0.3048).abs() < 1e-12);
    }

    #[test]
    fn mercator_standard_parallel_survives_wkt() {
        // Sobre otro elipsoide, y con un +k que el paralelo estándar anula. (wbprojection escribe
        // una Mercator esférica como Mercator_Auxiliary_Sphere, sin su escala.)
        let wgs84 = SpatialRef::from_epsg(4326).unwrap();
        for p in [
            "+proj=merc +lat_ts=-41 +lon_0=20 +ellps=intl +units=m",
            "+proj=merc +lat_ts=30 +k=1 +datum=WGS84 +units=m",
        ] {
            let crs = SpatialRef::from_proj4(p).unwrap();
            let from_wkt = SpatialRef::parse(&crs.to_wkt().unwrap()).unwrap();
            let (x, y) = Transformer::new(&wgs84, &crs)
                .unwrap()
                .transform(25.0, -50.0)
                .unwrap();
            let (wx, wy) = Transformer::new(&wgs84, &from_wkt)
                .unwrap()
                .transform(25.0, -50.0)
                .unwrap();
            assert!(
                (x - wx).abs() < 1e-3 && (y - wy).abs() < 1e-3,
                "{p}: ({x}, {y}) vs ({wx}, {wy})"
            );
        }
    }

    #[test]
    fn proj4_extension_describes_the_whole_crs() {
        // GDAL y PROJ leen un WKT 1 con extensión PROJ4 como esa cadena PROJ, digan lo que digan
        // sus nodos: aquí un meridiano central de 10, no -75.
        let wkt = r#"PROJCS["unknown",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",-75],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1],EXTENSION["PROJ4","+proj=tmerc +lon_0=10 +datum=WGS84 +units=m +no_defs"]]"#;
        let own = Def::from_proj("+proj=tmerc +lon_0=10 +datum=WGS84 +units=m +no_defs").unwrap();
        let bare = SpatialRef {
            wkt: Some(wkt.into()),
            ..Default::default()
        };
        for d in [
            Def::new(&SpatialRef::from_wkt(wkt)).unwrap(),
            Def::new(&bare).unwrap(),
            Def::wkt_conversion(wkt).unwrap(),
        ] {
            assert!(d.same_conversion(&own));
        }
        // Sobre un código declarado con el que no convierte (el meridiano central de la zona UTM
        // 18N es -75).
        let declared = format!("{},AUTHORITY[\"EPSG\",\"32618\"]]", &wkt[..wkt.len() - 1]);
        let s = SpatialRef::from_wkt(&declared);
        assert_eq!(s.epsg, None);
        assert!(Def::new(&s).unwrap().same_conversion(&own));
        // Se conserva un código declarado con el que convierte.
        let utm = r#"PROJCS["WGS 84 / UTM zone 18N",GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["central_meridian",-75],UNIT["metre",1],EXTENSION["PROJ4","+proj=utm +zone=18 +datum=WGS84 +units=m +no_defs"],AUTHORITY["EPSG","32618"]]"#;
        assert_eq!(SpatialRef::from_wkt(utm).epsg, Some(32618));
        // Cuyo TOWGS84 no anula el datum de la extensión.
        let shifted = utm.replace(
            "298.257223563]]",
            "298.257223563],TOWGS84[100,0,0,0,0,0,0]]",
        );
        let s = SpatialRef::from_wkt(&shifted);
        let own = Def::from_proj("+proj=utm +zone=18 +datum=WGS84 +units=m +no_defs").unwrap();
        assert!(shifted.contains("TOWGS84") && Def::new(&s).unwrap().same_datum(&own));
        // El WKT de EPSG:3857, cuya extensión da el datum como una rejilla nula.
        assert_eq!(
            SpatialRef::from_wkt(epsg_wkt(3857).unwrap()).epsg,
            Some(3857)
        );
        // Ni uno sobre otro elipsoide.
        assert_eq!(
            SpatialRef::from_wkt(&utm.replace("+datum=WGS84", "+ellps=intl")).epsg,
            None
        );
        let geog = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433],EXTENSION["PROJ4","+proj=longlat +ellps=intl +no_defs"],AUTHORITY["EPSG","4326"]]"#;
        assert_eq!(SpatialRef::from_wkt(geog).epsg, None);
        // Ni un código ESRI declarado con el que no convierte.
        let moll = SpatialRef::from_esri(54009).unwrap().to_wkt().unwrap();
        let i = moll.rfind(",AUTHORITY[").unwrap();
        let other = format!(
            "{},EXTENSION[\"PROJ4\",\"+proj=moll +lon_0=10 +datum=WGS84 +units=m\"]{}",
            &moll[..i],
            &moll[i..]
        );
        assert_eq!(SpatialRef::from_wkt(&other).authority(), None);
        let same = other.replace("+lon_0=10", "+lon_0=0");
        assert_eq!(
            SpatialRef::from_wkt(&same).authority(),
            Some(("ESRI", 54009))
        );
        // Ni un código declarado sobre otro datum que el de la extensión (ED50, que se desplaza).
        let ed50 = utm
            .replace("+zone=18 +datum=WGS84", "+zone=32 +ellps=intl")
            .replace("-75", "9")
            .replace("32618", "23032");
        assert_eq!(SpatialRef::from_wkt(&ed50).epsg, None);
    }

    #[test]
    fn declared_codes_win() {
        let nad83 = r#"GEOGCS["NAD83",DATUM["North_American_Datum_1983",SPHEROID["GRS 1980",6378137,298.257222101,AUTHORITY["EPSG","7019"]],AUTHORITY["EPSG","6269"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]],AXIS["Latitude",NORTH],AXIS["Longitude",EAST],AUTHORITY["EPSG","4269"]]"#;
        assert_eq!(declared_epsg(nad83), Some(4269));
        assert_eq!(SpatialRef::from_wkt(nad83).epsg, Some(4269));
        assert_eq!(
            declared_epsg(r#"GEOGCRS["WGS 84",CS[ellipsoidal,2],ID["EPSG",4326]]"#),
            Some(4326)
        );
        assert_eq!(
            declared_epsg(
                r#"GEOGCS["x",DATUM["d",SPHEROID["s",1,2]],UNIT["degree",0.01745,AUTHORITY["EPSG","9122"]]]"#
            ),
            None
        );
        // Los códigos de un datum o de un meridiano origen no son los del sistema (GDAL escribió
        // este en un GeoPackage, a partir del sistema de un archivo GeoParquet).
        let wgs84 = r#"GEOGCS["WGS 84",DATUM["WGS_1984",SPHEROID["WGS 84",6378137,298.257223563],AUTHORITY["EPSG","6326"]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["Degree",0.0174532925199433],AXIS["Longitude",EAST],AXIS["Latitude",NORTH]]"#;
        assert_eq!(identify_epsg(wgs84), Some(4326));
    }

    #[test]
    fn unknown_datums_are_not_guessed_and_named_ones_are_identified_as_in_gdal() {
        // pyproj.CRS("+proj=tmerc +lon_0=10 +ellps=... +units=m +no_defs").to_wkt("WKT1_GDAL"): un
        // datum sin código ni TOWGS84, que GDAL y PROJ dejan sin desplazar y sin código. (Estos
        // pasaron a ser Korean 1985, LUREF, Pulkovo 1942(58)... y tomaron sus desplazamientos.)
        let wgs84 = SpatialRef::from_epsg(4326).unwrap();
        let ellipsoids = [
            ("bessel", "Bessel 1841", "6377397.155,299.1528128"),
            ("clrk66", "Clarke 1866", "6378206.4,294.978698213898"),
            (
                "intl",
                "International 1924 (Hayford 1909, 1910)",
                "6378388,297",
            ),
            ("airy", "Airy 1830", "6377563.396,299.3249646"),
            ("krass", "Krassovsky, 1942", "6378245,298.3"),
            (
                "GRS80",
                "GRS 1980",
                "6378137,298.257222101,AUTHORITY[\"EPSG\",\"7019\"]",
            ),
        ];
        for (ellps, name, spheroid) in ellipsoids {
            let geogcs = format!(
                r#"GEOGCS["unknown",DATUM["Unknown based on {name} ellipsoid",SPHEROID["{name}",{spheroid}]],PRIMEM["Greenwich",0,AUTHORITY["EPSG","8901"]],UNIT["degree",0.0174532925199433,AUTHORITY["EPSG","9122"]]]"#
            );
            let projcs = format!(
                r#"PROJCS["unknown",{geogcs},PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",0],PARAMETER["central_meridian",10],PARAMETER["scale_factor",1],PARAMETER["false_easting",0],PARAMETER["false_northing",0],UNIT["metre",1,AUTHORITY["EPSG","9001"]],AXIS["Easting",EAST],AXIS["Northing",NORTH]]"#
            );
            let proj = |p: &str| SpatialRef::from_proj4(p).unwrap();
            let pairs = [
                (
                    projcs,
                    proj(&format!(
                        "+proj=tmerc +lon_0=10 +ellps={ellps} +units=m +no_defs"
                    )),
                ),
                (
                    geogcs,
                    proj(&format!("+proj=longlat +ellps={ellps} +no_defs")),
                ),
            ];
            for (wkt, string) in pairs {
                let crs = SpatialRef::from_wkt(&wkt);
                assert_eq!(crs.epsg, None, "{ellps}: {wkt}");
                let (x, y) = Transformer::new(&wgs84, &string)
                    .unwrap()
                    .transform(12.0, 48.0)
                    .unwrap();
                let (wx, wy) = Transformer::new(&wgs84, &crs)
                    .unwrap()
                    .transform(12.0, 48.0)
                    .unwrap();
                // Dentro de 1 mm (grados: 1e-8 es aproximadamente 1 mm).
                let tol = if crs.is_geographic() { 1e-8 } else { 1e-3 };
                assert!(
                    (x - wx).abs() < tol && (y - wy).abs() < tol,
                    "{ellps}: {x} {y} != {wx} {wy}"
                );
            }
        }
        // WKT de ESRI (WKT1_ESRI de pyproj) que nombra un datum por el nombre de ESRI: los códigos
        // que identifican GDAL y pyproj, ED50 en lugar de ED50(ED77) (EPSG:4154), ETRS89 por su
        // alias de ESRI.
        let esri = |code: u32, name: &str, datum: &str, spheroid: &str, rest: &str| {
            let geogcs = format!(
                r#"GEOGCS["GCS_{datum}",DATUM["D_{datum}",SPHEROID[{spheroid}]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#
            );
            let wkt = if name.is_empty() {
                geogcs
            } else {
                format!(r#"PROJCS["{name}",{geogcs},{rest}]"#)
            };
            assert_eq!(SpatialRef::from_wkt(&wkt).epsg, Some(code), "{wkt}");
            wkt
        };
        let utm = |lon: i32| {
            format!(
                r#"PROJECTION["Transverse_Mercator"],PARAMETER["False_Easting",500000.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",{lon}.0],PARAMETER["Scale_Factor",0.9996],PARAMETER["Latitude_Of_Origin",0.0],UNIT["Meter",1.0]"#
            )
        };
        let intl = r#""International_1924",6378388.0,297.0"#;
        let grs80 = r#""GRS_1980",6378137.0,298.257222101"#;
        let ed50 = esri(4230, "", "European_1950", intl, "");
        let ed50_utm = esri(
            23032,
            "ED_1950_UTM_Zone_32N",
            "European_1950",
            intl,
            &utm(9),
        );
        esri(4269, "", "North_American_1983", grs80, "");
        esri(
            26918,
            "NAD_1983_UTM_Zone_18N",
            "North_American_1983",
            grs80,
            &utm(-75),
        );
        esri(4258, "", "ETRS_1989", grs80, "");
        esri(25832, "ETRS_1989_UTM_Zone_32N", "ETRS_1989", grs80, &utm(9));
        // Sobre ETRS89-NOR, que ESRI escribe D_ETRS_1989 (GDAL lo identifica por su nombre de
        // ESRI).
        let ntm = r#"PROJECTION["Transverse_Mercator"],PARAMETER["False_Easting",100000.0],PARAMETER["False_Northing",1000000.0],PARAMETER["Central_Meridian",18.5],PARAMETER["Scale_Factor",1.0],PARAMETER["Latitude_Of_Origin",58.0],UNIT["Meter",1.0]"#;
        esri(5118, "ETRS_1989_NTM_Zone_18", "ETRS_1989", grs80, ntm);
        // Con el desplazamiento de datum de ED50, como EPSG:4230 y EPSG:23032 (unos 100 m en los
        // Países Bajos).
        for (wkt, code, tol) in [(&ed50, 4230, 1e-8), (&ed50_utm, 23032, 1e-3)] {
            let t = Transformer::new(&wgs84, &SpatialRef::from_wkt(wkt)).unwrap();
            let (x, y) = t.transform(5.0, 52.0).unwrap();
            let (ex, ey) = Transformer::new(&wgs84, &SpatialRef::from_epsg(code).unwrap())
                .unwrap()
                .transform(5.0, 52.0)
                .unwrap();
            assert!(
                (x - ex).abs() < tol && (y - ey).abs() < tol,
                "{code}: {x} {y} != {ex} {ey}"
            );
        }
        let (x, _) = Transformer::new(&wgs84, &SpatialRef::from_wkt(&ed50))
            .unwrap()
            .transform(5.0, 52.0)
            .unwrap();
        assert!((x - 5.0).abs() > 5e-4, "{x}");
        // Un datum mal escrito no se conoce por el nombre, así que no tiene código, ni siquiera
        // sobre el elipsoide de ED50 y en su forma ESRI: no se pide al registro que adivine.
        let misspelled = ed50.replace("D_European_1950", "D_Europaen_1950");
        assert_eq!(SpatialRef::from_wkt(&misspelled).epsg, None);
        // Un sistema WKT 2 sobre un conjunto de datums sin ID: se conoce por el nombre del
        // conjunto.
        let wkt2 = r#"GEOGCRS["WGS 84",ENSEMBLE["World Geodetic System 1984 ensemble",MEMBER["World Geodetic System 1984 (G2139)"],ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]],ENSEMBLEACCURACY[2.0]],PRIMEM["Greenwich",0,ANGLEUNIT["degree",0.0174532925199433]],CS[ellipsoidal,2],AXIS["geodetic latitude (Lat)",north,ORDER[1],ANGLEUNIT["degree",0.0174532925199433]],AXIS["geodetic longitude (Lon)",east,ORDER[2],ANGLEUNIT["degree",0.0174532925199433]]]"#;
        assert_eq!(SpatialRef::from_wkt(wkt2).epsg, Some(4326));
    }

    #[test]
    fn conversions_in_other_units_differ() {
        // Nebraska en metros (EPSG:6516) y en pies de agrimensura de EE. UU. (EPSG:6880): las
        // mismas posiciones sobre el terreno, no la misma conversión (identificación y fidelidad de
        // proj_to_wkt).
        let lcc = "+proj=lcc +lat_0=39.8333333333333 +lon_0=-100 +lat_1=43 +lat_2=40 +x_0=500000 +y_0=0 +ellps=GRS80";
        let m = Def::from_proj(&format!("{lcc} +units=m")).unwrap();
        let ft = Def::from_proj(&format!("{lcc} +units=us-ft")).unwrap();
        assert!(m.same_conversion(&Def::from_proj(&format!("{lcc} +units=m")).unwrap()));
        assert!(!m.same_conversion(&ft) && !ft.same_conversion(&m));
    }

    #[test]
    fn datum_names_are_read_from_the_generated_table() {
        // Como los normaliza scripts/gen_datum_names.py.
        assert_eq!(epsg::datum_key("D_European_1950"), "european1950");
        assert_eq!(
            epsg::datum_key("Militar-Geographische Institut"),
            "militargeographischeinstitut"
        );
        assert_eq!(epsg::datum_key("CH1903+"), "ch1903+");
        // Nombres oficiales, de WKT 1 de GDAL, de EPSG y de ESRI.
        for (name, code) in [
            ("European Datum 1950", 6230),
            ("European_Datum_1950", 6230),
            ("ED50", 6230),
            ("D_European_1950", 6230),
            ("D_ETRS_1989", 6258),
            ("WGS_1984", 6326),
            ("D_North_American_1983", 6269),
            ("CH1903+", 6150),
        ] {
            assert_eq!(epsg::datums_named(name), vec![code], "{name}");
        }
        // EPSG lista NAD83 también como alias de NAD83(HARN): la identificación toma el datum base.
        assert_eq!(epsg::datums_named("NAD83"), vec![6152, 6269]);
        let nad83 = r#"GEOGCS["unknown",DATUM["NAD83",SPHEROID["GRS 1980",6378137,298.257222101]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]]"#;
        assert_eq!(identify_epsg(nad83), Some(4269));
        assert!(epsg::datums_named("Unknown based on Bessel 1841 ellipsoid").is_empty());
    }

    #[test]
    fn parses_common_forms() {
        for s in [
            "EPSG:4326",
            "4326",
            "urn:ogc:def:crs:EPSG::4326",
            "http://www.opengis.net/def/crs/EPSG/0/4326",
            "OGC:CRS84",
        ] {
            assert_eq!(SpatialRef::parse(s).unwrap().epsg, Some(4326), "{s}");
        }
        let wkt = wbp::to_ogc_wkt(32633).unwrap();
        assert_eq!(SpatialRef::parse(&wkt).unwrap().epsg, Some(32633));
        // Un .prj de ESRI (distritos de Nueva York): GDAL identifica EPSG:2263.
        let esri = r#"PROJCS["NAD_1983_StatePlane_New_York_Long_Island_FIPS_3104_Feet",GEOGCS["GCS_North_American_1983",DATUM["D_North_American_1983",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",984250.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",-74.0],PARAMETER["Standard_Parallel_1",40.66666666666666],PARAMETER["Standard_Parallel_2",41.03333333333333],PARAMETER["Latitude_Of_Origin",40.16666666666666],UNIT["Foot_US",0.3048006096012192]]"#;
        assert_eq!(SpatialRef::parse(esri).unwrap().epsg, Some(2263));
        assert_eq!(
            SpatialRef::parse("EPSG:32633").unwrap().name().unwrap(),
            "WGS 84 / UTM zone 33N"
        );
    }

    #[test]
    fn nad83_to_wgs84_is_a_null_transformation_as_in_gdal() {
        // echo "-106.001666667 40.001666667" | gdaltransform -s_srs EPSG:4269 -t_srs EPSG:32613
        let t = Transformer::new(
            &SpatialRef::from_epsg(4269).unwrap(),
            &SpatialRef::from_epsg(32613).unwrap(),
        )
        .unwrap();
        let (x, y) = t.transform(-106.001666667, 40.001666667).unwrap();
        assert!((x - 414499.347197613).abs() < 1e-3, "{x}");
        assert!((y - 4428422.65095327).abs() < 1e-3, "{y}");
        // echo "-100 40" | gdaltransform -s_srs EPSG:4269 -t_srs EPSG:4326 -> -100 40
        let t = Transformer::new(
            &SpatialRef::from_epsg(4269).unwrap(),
            &SpatialRef::from_epsg(4326).unwrap(),
        )
        .unwrap();
        assert_eq!(t.transform(-100.0, 40.0).unwrap(), (-100.0, 40.0));
        // ED50 conserva su desplazamiento de datum: gdaltransform da 9.99897347903267
        // 44.9990669286146.
        let t = Transformer::new(
            &SpatialRef::from_epsg(4230).unwrap(),
            &SpatialRef::from_epsg(4326).unwrap(),
        )
        .unwrap();
        let (x, y) = t.transform(10.0, 45.0).unwrap();
        assert!(
            (x - 9.99897347903267).abs() < 1e-4 && (y - 44.9990669286146).abs() < 1e-4,
            "{x} {y}"
        );
    }

    #[test]
    fn transforms_to_web_mercator() {
        let t = Transformer::new(
            &SpatialRef::from_epsg(4326).unwrap(),
            &SpatialRef::from_epsg(3857).unwrap(),
        )
        .unwrap();
        let (x, y) = t.transform(-115.0, 36.0).unwrap();
        assert!((x - -12801741.0).abs() < 1.0, "{x}");
        assert!((y - 4300621.4).abs() < 1.0, "{y}");
    }

    #[test]
    fn matches_gdal_for_national_grids() {
        // gdaltransform -s_srs EPSG:2263 -t_srs EPSG:4326 (orden tradicional de los SIG): pies de
        // agrimensura de EE. UU.
        let t = Transformer::new(
            &SpatialRef::from_epsg(2263).unwrap(),
            &SpatialRef::from_epsg(4326).unwrap(),
        )
        .unwrap();
        let (lon, lat) = t.transform(988000.0, 200000.0).unwrap();
        assert!(
            (lon - -73.98647263351127).abs() < 1e-9 && (lat - 40.71562970330491).abs() < 1e-9,
            "{lon} {lat}"
        );
        // Desde WGS 84, con las transformaciones de datum, los órdenes de ejes y las unidades de
        // GDAL.
        let cases = [
            (27700, -1.5, 52.0, 434423.00723593397, 233624.3747866078),
            (2056, 8.0, 47.0, 2642695.4201549254, 1205590.522287224),
            (21781, 8.0, 47.0, 642695.4201549252, 205590.52228722392),
            (28992, 5.0, 52.0, 128410.08537081013, 445806.50883315015),
            (2154, 2.0, 47.0, 624015.5354680117, 6656012.00965832),
            (5514, 15.0, 50.0, -703011.8819030909, -1058147.274048979),
            (2065, 15.0, 50.0, 1058147.274048981, 703011.881903093),
            (2193, 174.0, -41.0, 1684102.1344457027, 5460761.410740666),
            (27200, 174.0, -41.0, 2594115.185056257, 6022476.222384517),
            (3035, 10.0, 50.0, 4321000.0, 2987510.566964945),
            (3031, 0.0, -75.0, 0.0, 1638783.2384072365),
            (3413, -45.0, 75.0, 0.0, -1633879.4974707854),
            (5070, -96.0, 40.0, 0.0, 1886977.5648988034),
            (3577, 135.0, -25.0, 299362.2208462456, -2706085.979530222),
        ];
        let wgs84 = SpatialRef::from_epsg(4326).unwrap();
        for (code, lon, lat, x, y) in cases {
            let crs = SpatialRef::from_epsg(code).unwrap();
            let (tx, ty) = Transformer::new(&wgs84, &crs)
                .unwrap()
                .transform(lon, lat)
                .unwrap();
            assert!(
                (tx - x).abs() < 1e-3 && (ty - y).abs() < 1e-3,
                "EPSG:{code}: {tx} {ty}"
            );
            let (tlon, tlat) = Transformer::new(&crs, &wgs84)
                .unwrap()
                .transform(x, y)
                .unwrap();
            // Within a millimetre: PROJ inverts Helmert rotations approximately.
            assert!(
                (tlon - lon).abs() < 2e-8 && (tlat - lat).abs() < 2e-8,
                "EPSG:{code} inverse: {tlon} {tlat}"
            );
        }
    }

    #[test]
    fn custom_wkt_units_and_towgs84_as_in_gdal() {
        let wgs84 = SpatialRef::from_epsg(4326).unwrap();
        // Un .prj de ESRI en pies de agrimensura de EE. UU. que no coincide con ningún código EPSG.
        let esri = r#"PROJCS["Custom_Feet_LCC",GEOGCS["GCS_North_American_1983",DATUM["D_North_American_1983",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",1000000.0],PARAMETER["False_Northing",50000.0],PARAMETER["Central_Meridian",-74.1],PARAMETER["Standard_Parallel_1",40.5],PARAMETER["Standard_Parallel_2",41.1],PARAMETER["Latitude_Of_Origin",40.0],UNIT["Foot_US",0.3048006096012192]]"#;
        let crs = SpatialRef::parse(esri).unwrap();
        assert_eq!(crs.epsg, None, "GDAL identifies no EPSG code");
        let (x, y) = Transformer::new(&wgs84, &crs)
            .unwrap()
            .transform(-73.95, 40.75)
            .unwrap();
        assert!(
            (x - 1041560.5074030993).abs() < 1e-3 && (y - 323273.5802968154).abs() < 1e-3,
            "{x} {y}"
        );
        // Un WKT 1 con TOWGS84 y sin código de autoridad.
        let osgb = r#"PROJCS["Custom BNG",GEOGCS["OSGB36",DATUM["OSGB_1936",SPHEROID["Airy 1830",6377563.396,299.3249646],TOWGS84[446.448,-125.157,542.06,0.15,0.247,0.842,-20.489]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["latitude_of_origin",49],PARAMETER["central_meridian",-2],PARAMETER["scale_factor",0.9996012717],PARAMETER["false_easting",400000],PARAMETER["false_northing",-100000],UNIT["metre",1]]"#;
        let crs = SpatialRef::from_wkt(osgb);
        let (x, y) = Transformer::new(&wgs84, &crs)
            .unwrap()
            .transform(-1.5, 52.0)
            .unwrap();
        assert!(
            (x - 434423.00723593397).abs() < 1e-3 && (y - 233624.3747866078).abs() < 1e-3,
            "{x} {y}"
        );
        // Una cadena PROJ en pies.
        let p = SpatialRef::parse("+proj=lcc +lat_0=40.1666666666667 +lon_0=-74 +lat_1=41.0333333333333 +lat_2=40.6666666666667 +x_0=300000 +y_0=0 +datum=NAD83 +units=us-ft").unwrap();
        let (lon, lat) = Transformer::new(&p, &wgs84)
            .unwrap()
            .transform(988000.0, 200000.0)
            .unwrap();
        assert!(
            (lon - -73.98647263351127).abs() < 1e-9 && (lat - 40.71562970330491).abs() < 1e-9,
            "{lon} {lat}"
        );
    }

    /// Un archivo NTv2 de una sola subrejilla sobre 0..1° E, 0..1° N con nodos a 1° de distancia:
    /// desplazamientos de latitud de 1, 2, 3 y 4 segundos de arco (SO, SE, NO, NE) y
    /// desplazamientos de longitud de 2 segundos de arco al este.
    fn tiny_ntv2() -> Vec<u8> {
        let mut b = Vec::new();
        let rec_i = |b: &mut Vec<u8>, name: &str, v: i32| {
            b.extend(format!("{name:<8}").bytes());
            b.extend(v.to_le_bytes());
            b.extend([0u8; 4]);
        };
        let rec_s = |b: &mut Vec<u8>, name: &str, v: &str| {
            b.extend(format!("{name:<8}").bytes());
            b.extend(format!("{v:<8}").bytes());
        };
        let rec_f = |b: &mut Vec<u8>, name: &str, v: f64| {
            b.extend(format!("{name:<8}").bytes());
            b.extend(v.to_le_bytes());
        };
        rec_i(&mut b, "NUM_OREC", 11);
        rec_i(&mut b, "NUM_SREC", 11);
        rec_i(&mut b, "NUM_FILE", 1);
        rec_s(&mut b, "GS_TYPE", "SECONDS");
        for (k, v) in [
            ("VERSION", "NTv2.0"),
            ("SYSTEM_F", "TEST"),
            ("SYSTEM_T", "WGS84"),
        ] {
            rec_s(&mut b, k, v);
        }
        for k in ["MAJOR_F", "MINOR_F", "MAJOR_T", "MINOR_T"] {
            rec_f(&mut b, k, 6378137.0);
        }
        rec_s(&mut b, "SUB_NAME", "TEST");
        rec_s(&mut b, "PARENT", "NONE");
        rec_s(&mut b, "CREATED", "");
        rec_s(&mut b, "UPDATED", "");
        // Latitudes y luego longitudes positivas al oeste, en segundos de arco.
        for (k, v) in [
            ("S_LAT", 0.0),
            ("N_LAT", 3600.0),
            ("E_LONG", -3600.0),
            ("W_LONG", 0.0),
            ("LAT_INC", 3600.0),
            ("LONG_INC", 3600.0),
        ] {
            rec_f(&mut b, k, v);
        }
        rec_i(&mut b, "GS_COUNT", 4);
        // Filas desde el sur, cada una de este a oeste: desplazamientos de latitud y longitud
        // (positivos al oeste) y sus precisiones.
        for lat_shift in [2.0f32, 1.0, 4.0, 3.0] {
            for v in [lat_shift, -2.0, 0.0, 0.0] {
                b.extend(v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn ntv2_grids_shift_as_proj() {
        register_grid("terrio_test_tiny.gsb", tiny_ntv2());
        let src =
            SpatialRef::parse("+proj=longlat +ellps=WGS84 +nadgrids=terrio_test_tiny.gsb").unwrap();
        let wgs84 = SpatialRef::from_epsg(4326).unwrap();
        let t = Transformer::new(&src, &wgs84).unwrap();
        let (lon, lat) = t.transform(0.25, 0.5).unwrap();
        // Interpolación bilineal: 0,375 * 1 + 0,125 * 2 + 0,375 * 3 + 0,125 * 4 = 2,25" (los
        // desplazamientos son de precisión simple en radianes, como en PROJ).
        assert!(
            (lon - (0.25 + 2.0 / 3600.0)).abs() < 1e-10
                && (lat - (0.5 + 2.25 / 3600.0)).abs() < 1e-10,
            "{lon} {lat}"
        );
        let (x, y) = Transformer::new(&wgs84, &src)
            .unwrap()
            .transform(lon, lat)
            .unwrap();
        assert!(
            (x - 0.25).abs() < 1e-12 && (y - 0.5).abs() < 1e-12,
            "{x} {y}"
        );
        // Fuera de la rejilla, la única transformación falla, como en PROJ.
        assert!(t.transform(5.0, 5.0).is_err());
    }

    #[test]
    fn gtx_geoids_shift_heights_as_proj() {
        // Una rejilla GTX de 2 x 2 desde (0°, 0°), a 1° de distancia: 10, 20 (fila sur), 30, 40.
        let mut b = Vec::new();
        for v in [0.0f64, 0.0, 1.0, 1.0] {
            b.extend(v.to_be_bytes());
        }
        for v in [2i32, 2] {
            b.extend(v.to_be_bytes());
        }
        for v in [10.0f32, 20.0, 30.0, 40.0] {
            b.extend(v.to_be_bytes());
        }
        register_grid("terrio_test_tiny.gtx", b);
        let src = SpatialRef::parse(
            "+proj=longlat +datum=WGS84 +geoidgrids=terrio_test_tiny.gtx +vunits=m",
        )
        .unwrap();
        let t = Transformer::new(&src, &SpatialRef::from_epsg(4979).unwrap()).unwrap();
        let (_, _, h) = t.transform_3d(0.25, 0.5, 100.0).unwrap();
        assert!((h - 122.5).abs() < 1e-9, "{h}");
        let (_, _, z) = Transformer::new(&SpatialRef::from_epsg(4979).unwrap(), &src)
            .unwrap()
            .transform_3d(0.25, 0.5, h)
            .unwrap();
        assert!((z - 100.0).abs() < 1e-9, "{z}");
    }

    #[test]
    fn compound_crss_parse_as_in_gdal() {
        let c = SpatialRef::parse("EPSG:4326+5773").unwrap();
        assert_eq!(c.epsg, None);
        assert_eq!(c.name().unwrap(), "WGS 84 + EGM96 height");
        assert!(
            c.to_wkt()
                .unwrap()
                .starts_with("COMPD_CS[\"WGS 84 + EGM96 height\",GEOGCS[\"WGS 84\"")
        );
        assert!(c.is_geographic());
        let k = SpatialRef::from_epsg(5498).unwrap();
        assert_eq!(k.name().unwrap(), "NAD83 + NAVD88 height");
        // Sin rejillas, las alturas entre sistemas con alturas se conservan (la aproximación de
        // PROJ).
        let t = Transformer::new(
            &SpatialRef::from_epsg(9705).unwrap(),
            &SpatialRef::from_epsg(4326).unwrap(),
        )
        .unwrap();
        assert_eq!(
            t.transform_3d(10.0, 50.0, 100.0).unwrap(),
            (10.0, 50.0, 100.0)
        );
    }

    #[test]
    fn names_and_wkt_come_from_the_epsg_table() {
        let s = SpatialRef::from_epsg(2263).unwrap();
        assert_eq!(s.name().unwrap(), "NAD83 / New York Long Island (ftUS)");
        let w = s.to_wkt().unwrap();
        assert!(
            w.contains("UNIT[\"US survey foot\",0.304800609601219")
                && w.ends_with("AUTHORITY[\"EPSG\",\"2263\"]]"),
            "{w}"
        );
        assert_eq!(SpatialRef::from_wkt(&w).epsg, Some(2263));
    }
}
