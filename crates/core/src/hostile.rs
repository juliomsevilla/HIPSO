//! Archivos dañados o fabricados a propósito: cada lector debe contestar con un error, sin abortar
//! por falta de memoria, entrar en pánico ni quedarse colgado.

use std::path::PathBuf;

use crate::{Error, Result};

/// Una carpeta temporal para una prueba, borrada al soltarla.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Result<Self> {
        let d = std::env::temp_dir().join(format!("hipso_hostile_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d)?;
        Ok(Self(d))
    }
    fn file(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let p = self.0.join(name);
        std::fs::write(&p, bytes)?;
        Ok(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Un GeoTIFF de unos 200 bytes que dice medir `side` x `side` en decimales de 64 bits.
fn huge_tiff(side: u32) -> Vec<u8> {
    // (etiqueta, tipo, recuento, valor o desplazamiento); los dobles van tras el directorio
    let n = 12u32;
    let data_at = 8 + 2 + n * 12 + 4;
    let entries: [(u16, u16, u32, u32); 12] = [
        (256, 4, 1, side),
        (257, 4, 1, side),
        (258, 3, 1, 64),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
        (273, 4, 1, 8),
        (277, 3, 1, 1),
        (278, 4, 1, side),
        (279, 4, 1, 8),
        (339, 3, 1, 3),
        (33550, 12, 3, data_at),
        (33922, 12, 6, data_at + 24),
    ];
    let mut f = b"II*\0\x08\0\0\0".to_vec();
    f.extend((n as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        f.extend(tag.to_le_bytes());
        f.extend(kind.to_le_bytes());
        f.extend(count.to_le_bytes());
        f.extend(value.to_le_bytes());
    }
    f.extend(0u32.to_le_bytes());
    for v in [1.0f64, 1.0, 0.0, 0.0, 0.0, 0.0, 500_000.0, 4_000_000.0, 0.0] {
        f.extend(v.to_le_bytes());
    }
    f
}

#[test]
fn a_tiff_that_claims_a_huge_image() -> Result<()> {
    let d = Dir::new("tiff")?;
    for side in [30_000, 100_000, u32::MAX] {
        let p = d.file("a.tif", &huge_tiff(side))?;
        assert!(crate::grid::Grid::read_tiff(&p).is_err(), "{side}");
        assert!(crate::raster::read_image(&p).is_err(), "{side}");
    }
    Ok(())
}

#[test]
fn a_png_that_claims_a_huge_image() -> Result<()> {
    let chunk = |kind: &[u8; 4], body: &[u8]| {
        let mut c = (body.len() as u32).to_be_bytes().to_vec();
        let mut all = kind.to_vec();
        all.extend(body);
        let mut crc = flate2::Crc::new();
        crc.update(&all);
        c.extend(all);
        c.extend(crc.sum().to_be_bytes());
        c
    };
    let mut head = 100_000u32.to_be_bytes().to_vec();
    head.extend(100_000u32.to_be_bytes());
    head.extend([16, 6, 0, 0, 0]);
    let mut f = b"\x89PNG\r\n\x1a\n".to_vec();
    f.extend(chunk(b"IHDR", &head));
    f.extend(chunk(
        b"IDAT",
        &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    f.extend(chunk(b"IEND", &[]));
    let d = Dir::new("png")?;
    let p = d.file("a.png", &f)?;
    d.file("a.pgw", b"1\n0\n0\n-1\n0\n0\n")?;
    assert!(crate::rasterfmt::load(&p).is_err());
    Ok(())
}

#[test]
fn an_envi_header_whose_size_overflows() -> Result<()> {
    let d = Dir::new("envi")?;
    let p = d.file("a.bil", &[0u8; 64])?;
    for (side, offset) in [(1u64 << 32, 0u64), (100_000, 0), (4, u64::MAX)] {
        let text = format!(
            "ENVI\nsamples = {side}\nlines = {side}\nbands = 4\ndata type = 5\ninterleave = bil\nheader offset = {offset}\nmap info = {{UTM, 1, 1, 500000, 4000000, 1, 1, 30, North}}\n"
        );
        d.file("a.hdr", text.as_bytes())?;
        assert!(crate::rasterfmt::load(&p).is_err(), "{side} {offset}");
    }
    Ok(())
}

#[test]
fn a_jp2_box_whose_length_wraps_around() -> Result<()> {
    // la firma y una caja con una longitud extendida que, sumada a su posición, daría 0
    let mut f = vec![0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0d, 0x0a, 0x87, 0x0a];
    f.extend([0, 0, 0, 1]);
    f.extend(b"free");
    f.extend((u64::MAX - 11).to_be_bytes());
    let d = Dir::new("jp2")?;
    let p = d.file("a.jp2", &f)?;
    assert!(crate::jp2::Jp2::open(&p).is_err());
    Ok(())
}

/// Una base de datos de una página de 512 bytes; `page` rellena lo que sigue a la cabecera.
fn sqlite(page: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut f = vec![0u8; 512];
    f[..16].copy_from_slice(b"SQLite format 3\0");
    f[16..18].copy_from_slice(&512u16.to_be_bytes());
    page(&mut f);
    f
}

#[test]
fn a_sqlite_page_that_points_at_itself() -> Result<()> {
    // página interior sin celdas cuyo hijo derecho es ella misma
    let db = crate::sqlite::Db::new(sqlite(|f| {
        f[100] = 0x05;
        f[108..112].copy_from_slice(&1u32.to_be_bytes());
    }))?;
    assert!(
        db.schema()
            .is_err_and(|e| e.to_string().contains("dos veces"))
    );
    Ok(())
}

#[test]
fn a_sqlite_cell_larger_than_the_file() -> Result<()> {
    // una hoja con una celda que declara 2^34 bytes, en una cadena de desbordamiento que se
    // apunta a sí misma
    let db = crate::sqlite::Db::new(sqlite(|f| {
        f[100] = 0x0d;
        f[103..105].copy_from_slice(&1u16.to_be_bytes());
        f[108..110].copy_from_slice(&200u16.to_be_bytes());
        f[200..206].copy_from_slice(&[0xc0, 0x80, 0x80, 0x80, 0x00, 0x01]);
    }))?;
    assert!(
        db.schema()
            .is_err_and(|e| e.to_string().contains("mayor que el archivo"))
    );
    Ok(())
}

#[test]
fn a_spreadsheet_cell_far_beyond_the_last_column() -> Result<()> {
    let xml = br#"<worksheet><sheetData><row r="1"><c r="ZZZZZZZZ1"><v>1</v></c></row></sheetData></worksheet>"#;
    let d = Dir::new("xlsx")?;
    let p = d.file(
        "a.xlsx",
        &crate::kml::zip_one("xl/worksheets/sheet1.xml", xml)?,
    )?;
    assert!(crate::xlsx::read(&p).is_err_and(|e| e.to_string().contains("columnas")));
    Ok(())
}

#[test]
fn a_copc_node_that_cannot_exist() -> Result<()> {
    let d = Dir::new("copc")?;
    let good = d.0.join("good.copc.laz");
    crate::copc::tests::build(&good)?;
    let bytes = std::fs::read(&good)?;
    // la primera entrada de la jerarquía: dónde está, lo dice la información COPC
    let at = u64::from_le_bytes(
        bytes
            .get(375 + 54 + 40..375 + 54 + 48)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| Error::Format("sin jerarquía".into()))?,
    ) as usize;
    // nivel absurdo (en la segunda entrada: la primera es la raíz), demasiados bytes y demasiados
    // puntos
    for (field, value) in [(32, 40u32), (24, u32::MAX / 2), (28, i32::MAX as u32)] {
        let mut bad = bytes.clone();
        bad[at + field..at + field + 4].copy_from_slice(&value.to_le_bytes());
        let p = d.file("bad.copc.laz", &bad)?;
        let c = crate::copc::Copc::open(&p)?.ok_or_else(|| Error::Format("no es COPC".into()))?;
        assert!(c.nodes(None, None).is_err(), "campo {field}");
    }
    Ok(())
}

#[test]
fn a_las_with_huge_records_and_no_points() -> Result<()> {
    let mut h = vec![0u8; 227];
    h[..4].copy_from_slice(b"LASF");
    h[24] = 1;
    h[25] = 2;
    h[94..96].copy_from_slice(&227u16.to_le_bytes());
    h[96..100].copy_from_slice(&227u32.to_le_bytes());
    h[105..107].copy_from_slice(&u16::MAX.to_le_bytes());
    for k in 0..3 {
        h[131 + 8 * k..139 + 8 * k].copy_from_slice(&0.01f64.to_le_bytes());
    }
    let d = Dir::new("las")?;
    let p = d.file("a.las", &h)?;
    assert_eq!(crate::las::Reader::open(&p)?.scan_raw(|_| Ok(()))?, 0);
    // con puntos que no están, un error
    h[107..111].copy_from_slice(&u32::MAX.to_le_bytes());
    let p = d.file("b.las", &h)?;
    assert!(
        crate::las::Reader::open(&p)
            .and_then(|r| r.scan_raw(|_| Ok(())))
            .is_err()
    );
    Ok(())
}
