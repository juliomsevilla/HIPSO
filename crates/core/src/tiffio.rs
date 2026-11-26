//! Lo común a los lectores de GeoTIFF: cuánta memoria puede pedir un archivo y dónde está la esquina
//! de su primer píxel.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use tiff::decoder::{Decoder, Limits};
use tiff::tags::Tag;

use crate::{Error, Result};

/// Lo que se puede descomprimir de cualquier archivo, por pequeño que sea.
const FLOOR: u64 = 256 << 20;
/// La mayor compresión creíble: Deflate y LZW no pasan de unas 1400 veces.
const RATIO: u64 = 1500;
/// El tope absoluto: las celdas de la mayor rejilla admitida en ocho bytes.
const CEILING: u64 = 8 << 32;

/// Los límites de un archivo de `len` bytes. Una cabecera dañada que declare una imagen enorme da un
/// error en lugar de una reserva de memoria que cierra el programa.
pub fn limits(len: u64) -> Limits {
    let bytes = len.saturating_mul(RATIO).clamp(FLOOR, CEILING);
    let bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
    let mut l = Limits::default();
    l.decoding_buffer_size = bytes;
    l.intermediate_buffer_size = bytes;
    // las tablas de teselas ocupan lo que ocupen en el archivo, nunca mucho más que él
    l.ifd_value_size = usize::try_from(len.max(64 << 20)).unwrap_or(usize::MAX);
    l
}

/// Un decodificador de `path` con los límites que corresponden a su tamaño.
pub fn open(path: &Path) -> Result<Decoder<BufReader<File>>> {
    let file = crate::open_file(path)?;
    let len = file.metadata()?.len();
    Ok(Decoder::new(BufReader::new(file))
        .map_err(|e| Error::Format(format!("{}: {e}", path.display())))?
        .with_limits(limits(len)))
}

/// La esquina superior izquierda de la imagen a partir de su punto de anclaje. Con el tipo de ráster
/// «punto» (GeoKey 1025 igual a 2) el anclaje es el centro del primer píxel.
pub fn corner<R: Read + Seek>(
    d: &mut Decoder<R>,
    tx: f64,
    ty: f64,
    sx: f64,
    sy: f64,
) -> (f64, f64) {
    let point = d
        .get_tag_u32_vec(Tag::GeoKeyDirectoryTag)
        .ok()
        .is_some_and(|k| {
            k.get(4..).is_some_and(|k| {
                k.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|e| e[0] == 1025 && e[1] == 0 && e[3] == 2)
            })
        });
    if point {
        (tx - sx / 2.0, ty + sy / 2.0)
    } else {
        (tx, ty)
    }
}

#[cfg(test)]
mod tests {
    use tiff::encoder::{TiffEncoder, colortype::Gray32Float};

    use super::*;
    use crate::grid::Grid;

    /// Un GeoTIFF de 2 x 2 con celdas de 1 m y el tipo de ráster dado.
    fn write(path: &Path, kind: u16) -> Result<()> {
        let bad = |e: tiff::TiffError| Error::Format(e.to_string());
        let mut enc = TiffEncoder::new(File::create(path)?).map_err(bad)?;
        let mut img = enc.new_image::<Gray32Float>(2, 2).map_err(bad)?;
        let e = img.encoder();
        e.write_tag(Tag::ModelPixelScaleTag, &[1.0, 1.0, 0.0][..])
            .map_err(bad)?;
        e.write_tag(
            Tag::ModelTiepointTag,
            &[0.0, 0.0, 0.0, 500000.0, 4000000.0, 0.0][..],
        )
        .map_err(bad)?;
        e.write_tag(
            Tag::GeoKeyDirectoryTag,
            &[1u16, 1, 0, 1, 1025, 0, 1, kind][..],
        )
        .map_err(bad)?;
        img.write_data(&[1.0f32, 2.0, 3.0, 4.0]).map_err(bad)
    }

    #[test]
    fn a_point_raster_is_anchored_at_the_pixel_centre() -> Result<()> {
        for (kind, x0, y0) in [(1, 500000.0, 3999998.0), (2, 499999.5, 3999998.5)] {
            let p = std::env::temp_dir()
                .join(format!("hipso_tiffio_{kind}_{}.tif", std::process::id()));
            write(&p, kind)?;
            let g = Grid::read_tiff(&p)?;
            assert_eq!((g.x0, g.y0), (x0, y0));
            assert_eq!(Grid::read_extent(&p)?.0[0], x0);
            assert_eq!(crate::raster::read_image(&p)?.x0, x0);
            assert_eq!(crate::pyramid::Pyramid::open(&p)?.extent()[0], x0);
            std::fs::remove_file(&p)?;
        }
        Ok(())
    }

    #[test]
    fn a_small_file_cannot_ask_for_a_huge_image() {
        assert_eq!(limits(200).decoding_buffer_size as u64, FLOOR);
        assert_eq!(
            limits(1 << 20).decoding_buffer_size as u64,
            (1 << 20) * RATIO
        );
        assert_eq!(limits(u64::MAX).decoding_buffer_size as u64, CEILING);
    }
}
