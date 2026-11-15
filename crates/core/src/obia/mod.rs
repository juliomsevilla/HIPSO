//! Análisis basado en objetos de productos LiDAR: las celdas de varias capas corregistradas (altura
//! de copas, intensidad, retornos…) se agrupan en objetos, cada objeto recibe atributos, y los
//! objetos se clasifican y se escriben como polígonos.
//!
//! Todo funciona sobre un retículo compartido por todas las capas (las mismas celdas, celda a
//! celda), es lineal en el número de celdas y guarda el resultado de cada etapa en vectores simples
//! indexados por número de objeto, de modo que ninguna etapa reserva memoria por celda ni por
//! objeto.
//!
//! Los métodos siguen la literatura: la fusión multirresolución de Baatz y Schäpe (2000, en su
//! forma de mejor ajuste mutuo local), los bosques aleatorios de Breiman (2001) y las medidas de
//! exactitud de clasificación de Congalton (1991).

pub mod attributes;
pub mod classify;
pub mod job;
pub mod layers;
pub mod segment;
pub mod strata;
pub mod vectorize;

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

use crate::grid::Grid;
use crate::{Error, Result};

/// Las celdas que comparte toda capa de un análisis: orientado al norte, cuadradas, la fila 0 la
/// más al norte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lattice {
    /// Borde oeste.
    pub x0: f64,
    /// Borde sur.
    pub y0: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
}

impl Lattice {
    pub fn of(g: &Grid) -> Self {
        Self {
            x0: g.x0,
            y0: g.y0,
            cell: g.cell,
            cols: g.cols,
            rows: g.rows,
        }
    }

    pub fn len(&self) -> usize {
        self.cols * self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Si dos retículos son las mismas celdas (hasta una millonésima de celda).
    pub fn same(&self, o: &Self) -> bool {
        self.cols == o.cols
            && self.rows == o.rows
            && (self.cell - o.cell).abs() <= self.cell * 1e-9
            && (self.x0 - o.x0).abs() <= self.cell * 1e-6
            && (self.y0 - o.y0).abs() <= self.cell * 1e-6
    }

    /// El índice de la celda que contiene un punto, si está dentro.
    #[inline]
    pub fn index(&self, x: f64, y: f64) -> Option<usize> {
        let c = ((x - self.x0) / self.cell).floor();
        let r = ((y - self.y0) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        Some((self.rows - 1 - r as usize) * self.cols + c as usize)
    }

    /// Las coordenadas del mapa de la esquina `(col, row)` del retículo (la esquina noroeste de la
    /// celda; `col` puede llegar hasta `cols` y `row` hasta `rows`).
    #[inline]
    pub fn corner(&self, col: usize, row: usize) -> [f64; 2] {
        [
            self.x0 + col as f64 * self.cell,
            self.y0 + (self.rows - row) as f64 * self.cell,
        ]
    }

    /// Una rejilla sobre este retículo que contiene `data`.
    pub fn grid(&self, data: Vec<f32>) -> Grid {
        Grid {
            x0: self.x0,
            y0: self.y0,
            cell: self.cell,
            cols: self.cols,
            rows: self.rows,
            data,
        }
    }
}

/// Una banda de valores con nombre sobre el retículo; NaN marca una celda sin datos.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: String,
    pub data: Vec<f32>,
}

/// Las capas de un análisis, todas sobre un retículo.
#[derive(Clone, Debug)]
pub struct LayerStack {
    pub lat: Lattice,
    pub epsg: Option<u16>,
    pub layers: Vec<Layer>,
}

impl LayerStack {
    pub fn new(lat: Lattice, epsg: Option<u16>) -> Self {
        Self {
            lat,
            epsg,
            layers: Vec::new(),
        }
    }

    /// Añade una capa; su longitud debe ser la del retículo.
    pub fn push(&mut self, name: &str, data: Vec<f32>) -> Result<()> {
        if data.len() != self.lat.len() {
            return Err(Error::Param(format!(
                "la capa '{name}' tiene {} celdas y la rejilla {}",
                data.len(),
                self.lat.len()
            )));
        }
        let mut name = name.to_string();
        let base = name.clone();
        let mut n = 2;
        while self.layers.iter().any(|l| l.name == name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        self.layers.push(Layer { name, data });
        Ok(())
    }

    /// Añade un ráster que debe estar sobre el retículo de la pila.
    pub fn push_grid(&mut self, name: &str, g: &Grid) -> Result<()> {
        if !self.lat.same(&Lattice::of(g)) {
            return Err(Error::Param(format!(
                "la capa '{name}' no coincide celda a celda con las demás (mismo origen, tamaño de celda y dimensiones): iguálala antes con Ráster → Igualar rásteres (`hipso align`)"
            )));
        }
        self.push(name, g.data.clone())
    }

    /// Añade cada banda de una imagen multibanda como una capa llamada `<nombre>_b1`,
    /// `<nombre>_b2`…; la imagen debe estar sobre el retículo de la pila.
    pub fn push_image(&mut self, name: &str, img: &crate::raster::Image) -> Result<()> {
        let lat = Lattice {
            x0: img.x0,
            y0: img.y0,
            cell: img.cell,
            cols: img.cols,
            rows: img.rows,
        };
        if !self.lat.same(&lat) {
            return Err(Error::Param(format!(
                "la imagen '{name}' no coincide celda a celda con las demás capas (mismo origen, tamaño de celda y dimensiones): iguálala antes con Ráster → Igualar rásteres (`hipso align`)"
            )));
        }
        for (b, data) in img.bands.iter().enumerate() {
            self.push(&format!("{name}_b{}", b + 1), data.clone())?;
        }
        Ok(())
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.layers
            .iter()
            .position(|l| l.name.eq_ignore_ascii_case(name))
    }
}

/// Una etiqueta por celda: 0 donde no hay objeto, si no el número de objeto `1..=count`.
#[derive(Clone, Debug)]
pub struct Labels {
    pub lat: Lattice,
    pub ids: Vec<u32>,
    pub count: u32,
}

/// Las etiquetas que sitúan un ráster en el mapa, como las escribe [`Grid::write_tiff`].
fn write_int<C>(
    path: &Path,
    lat: &Lattice,
    epsg: Option<u16>,
    data: &[C::Inner],
    nodata: &str,
) -> Result<()>
where
    C: colortype::ColorType,
    [C::Inner]: tiff::encoder::TiffValue,
{
    let tiff_err = |e: tiff::TiffError| Error::Format(format!("TIFF: {e}"));
    let too_big = || Error::Param("la rejilla es demasiado grande para un TIFF".into());
    let w = BufWriter::new(File::create(path)?);
    let mut enc = TiffEncoder::new(w)
        .map_err(tiff_err)?
        .with_compression(Compression::Deflate(DeflateLevel::Balanced));
    let mut img = enc
        .new_image::<C>(
            u32::try_from(lat.cols).map_err(|_| too_big())?,
            u32::try_from(lat.rows).map_err(|_| too_big())?,
        )
        .map_err(tiff_err)?;
    let e = img.encoder();
    e.write_tag(Tag::ModelPixelScaleTag, &[lat.cell, lat.cell, 0.0][..])
        .map_err(tiff_err)?;
    let top = lat.y0 + lat.rows as f64 * lat.cell;
    e.write_tag(
        Tag::ModelTiepointTag,
        &[0.0, 0.0, 0.0, lat.x0, top, 0.0][..],
    )
    .map_err(tiff_err)?;
    let mut keys: Vec<u16> = vec![1, 1, 0, 2, 1024, 0, 1, 1, 1025, 0, 1, 1];
    if let Some(code) = epsg {
        keys[3] = 3;
        keys.extend([3072, 0, 1, code]);
    }
    e.write_tag(Tag::GeoKeyDirectoryTag, &keys[..])
        .map_err(tiff_err)?;
    e.write_tag(Tag::GdalNodata, nodata).map_err(tiff_err)?;
    img.write_data(data).map_err(tiff_err)?;
    Ok(())
}

impl Labels {
    /// Escribe los números de objeto como GeoTIFF entero (0 es «sin objeto»), que conserva exacto
    /// cada número, a diferencia de un ráster decimal más allá de 16 millones de objetos.
    pub fn write_tiff(&self, path: &Path, epsg: Option<u16>) -> Result<()> {
        write_int::<colortype::Gray32>(path, &self.lat, epsg, &self.ids, "0")
    }

    /// Lee un GeoTIFF entero (o decimal de valores enteros) de números de objeto.
    pub fn read_tiff(path: &Path) -> Result<Self> {
        use tiff::decoder::{Decoder, DecodingResult};
        let tiff_err = |e: tiff::TiffError| Error::Format(format!("TIFF: {e}"));
        let mut d = Decoder::new(BufReader::new(File::open(path)?)).map_err(tiff_err)?;
        let (w, h) = d.dimensions().map_err(tiff_err)?;
        let scale = d
            .get_tag_f64_vec(Tag::ModelPixelScaleTag)
            .map_err(tiff_err)?;
        let tie = d.get_tag_f64_vec(Tag::ModelTiepointTag).map_err(tiff_err)?;
        let (Some(&s), Some(&tx), Some(&ty)) = (scale.first(), tie.get(3), tie.get(4)) else {
            return Err(Error::Format(format!(
                "{}: no es un GeoTIFF",
                path.display()
            )));
        };
        let (cols, rows) = (w as usize, h as usize);
        let ids: Vec<u32> = match d.read_image().map_err(tiff_err)? {
            DecodingResult::U8(v) => v.into_iter().map(u32::from).collect(),
            DecodingResult::U16(v) => v.into_iter().map(u32::from).collect(),
            DecodingResult::U32(v) => v,
            DecodingResult::F32(v) => v
                .into_iter()
                .map(|x| {
                    if x.is_finite() && x > 0.0 {
                        x as u32
                    } else {
                        0
                    }
                })
                .collect(),
            _ => {
                return Err(Error::Unsupported(format!(
                    "{}: se esperaba un ráster de números enteros",
                    path.display()
                )));
            }
        };
        if ids.len() != cols * rows {
            return Err(Error::Format(format!(
                "{}: una banda de {cols} x {rows} celdas",
                path.display()
            )));
        }
        let count = ids.iter().copied().max().unwrap_or(0);
        Ok(Self {
            lat: Lattice {
                x0: tx,
                y0: ty - rows as f64 * s,
                cell: s,
                cols,
                rows,
            },
            ids,
            count,
        })
    }
}

/// Escribe una clase por celda como GeoTIFF entero de 16 bits (0 es «sin clase»).
pub fn write_classes_tiff(
    path: &Path,
    lat: &Lattice,
    epsg: Option<u16>,
    classes: &[u16],
) -> Result<()> {
    write_int::<colortype::Gray16>(path, lat, epsg, classes, "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lat() -> Lattice {
        Lattice {
            x0: 100.0,
            y0: 200.0,
            cell: 0.5,
            cols: 4,
            rows: 3,
        }
    }

    #[test]
    fn lattice_indexes_north_up() {
        let l = lat();
        // la celda noroeste es el índice 0, la sureste la última
        assert_eq!(l.index(100.1, 201.4), Some(0));
        assert_eq!(l.index(101.9, 200.1), Some(11));
        assert_eq!(l.index(99.9, 200.1), None);
        assert_eq!(l.corner(0, 0), [100.0, 201.5]);
        assert_eq!(l.corner(4, 3), [102.0, 200.0]);
    }

    #[test]
    fn labels_round_trip_beyond_float_precision() -> Result<()> {
        let l = lat();
        let big = 16_777_217u32;
        let ids: Vec<u32> = (0..12).map(|i| if i == 5 { big } else { i }).collect();
        let labels = Labels {
            lat: l,
            ids: ids.clone(),
            count: big,
        };
        let path = std::env::temp_dir().join(format!("rl_obia_ids_{}.tif", std::process::id()));
        labels.write_tiff(&path, Some(25830))?;
        let back = Labels::read_tiff(&path)?;
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.ids, ids);
        assert!(back.lat.same(&l));
        Ok(())
    }

    #[test]
    fn a_layer_must_match_the_lattice() {
        let mut s = LayerStack::new(lat(), None);
        assert!(s.push("a", vec![0.0; 12]).is_ok());
        assert!(s.push("b", vec![0.0; 11]).is_err());
        assert!(s.push("a", vec![1.0; 12]).is_ok());
        assert_eq!(s.layers[1].name, "a_2");
        let mut other = lat();
        other.x0 += 0.1;
        assert!(s.push_grid("c", &other.grid(vec![0.0; 12])).is_err());
    }
}
