//! El tipo de un campo de una capa vectorial: texto, número entero, decimal, sí/no, fecha o
//! hipervínculo. Un valor se guarda como texto (como lo muestra la tabla) y como número (para
//! consultas, ordenación y símbolos graduados); el tipo dice cómo se comprueba y se escribe un
//! valor tipado y cómo lo lee una consulta.
//!
//! Texto canónico de cada tipo: enteros y decimales con punto; sí/no como `Sí` / `No` (número 1 /
//! 0); fechas como `AAAA-MM-DD` (número: días desde 1970-01-01); texto e hipervínculos tal como
//! están.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FieldType {
    #[default]
    Text,
    Integer,
    Real,
    Bool,
    Date,
    /// Una dirección web o una ruta: texto que la tabla permite abrir.
    Link,
}

impl FieldType {
    pub const ALL: [FieldType; 6] = [
        FieldType::Text,
        FieldType::Integer,
        FieldType::Real,
        FieldType::Bool,
        FieldType::Date,
        FieldType::Link,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FieldType::Text => "Texto",
            FieldType::Integer => "Número entero",
            FieldType::Real => "Número decimal",
            FieldType::Bool => "Sí / No",
            FieldType::Date => "Fecha",
            FieldType::Link => "Hipervínculo",
        }
    }

    /// Lo que acepta el tipo, para la ayuda de los diálogos.
    pub fn help(self) -> &'static str {
        match self {
            FieldType::Text => "Cualquier texto.",
            FieldType::Integer => "Números sin decimales (por ejemplo 12 o -3).",
            FieldType::Real => "Números con decimales (se admite la coma o el punto).",
            FieldType::Bool => "Sí o No (también se entiende sí/no, s/n, true/false, 1/0).",
            FieldType::Date => "Fechas como 2025-03-31 (también 31/03/2025).",
            FieldType::Link => "Una dirección web (https://…) o la ruta de un archivo.",
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(self, FieldType::Integer | FieldType::Real)
    }

    /// El tipo que tiene una columna por sus valores: todo valor no vacío un número entero, un
    /// decimal o ninguno (texto); una columna vacía es texto.
    pub fn infer<'a>(values: impl Iterator<Item = &'a str>) -> FieldType {
        let (mut any, mut int) = (false, true);
        for t in values {
            let t = t.trim();
            if t.is_empty() {
                continue;
            }
            any = true;
            match t.parse::<f64>() {
                Ok(v) if v.is_finite() => {
                    if int && (v.fract() != 0.0 || v.abs() > 9e15 || t.contains(['.', 'e', 'E'])) {
                        int = false;
                    }
                }
                _ => return FieldType::Text,
            }
        }
        match (any, int) {
            (false, _) => FieldType::Text,
            (true, true) => FieldType::Integer,
            (true, false) => FieldType::Real,
        }
    }

    /// Un valor tipado como el texto canónico y el número en que cuenta (NaN cuando no tiene). Un
    /// valor vacío es válido para todo tipo. El error dice qué se esperaba.
    pub fn parse(self, raw: &str) -> Result<(String, f64), String> {
        let t = raw.trim();
        if t.is_empty() {
            return Ok((String::new(), f64::NAN));
        }
        let bad = |what: &str| Err(format!("«{t}» no es {what}"));
        match self {
            FieldType::Text | FieldType::Link => Ok((raw.to_string(), f64::NAN)),
            FieldType::Integer => match t.replace(',', ".").parse::<f64>() {
                Ok(v) if v.is_finite() && v.fract() == 0.0 && v.abs() < 9e15 => {
                    Ok((format!("{}", v as i64), v))
                }
                _ => bad("un número entero"),
            },
            FieldType::Real => match t.replace(',', ".").parse::<f64>() {
                Ok(v) if v.is_finite() => Ok((crate::vecops::nice(v), v)),
                _ => bad("un número"),
            },
            FieldType::Bool => match t.to_lowercase().as_str() {
                "sí" | "si" | "s" | "yes" | "y" | "true" | "t" | "1" | "verdadero" => {
                    Ok(("Sí".into(), 1.0))
                }
                "no" | "n" | "false" | "f" | "0" | "falso" => Ok(("No".into(), 0.0)),
                _ => bad("Sí ni No"),
            },
            FieldType::Date => match parse_date(t) {
                Some(d) => Ok((format_date(d), d)),
                None => bad("una fecha (por ejemplo 2025-03-31)"),
            },
        }
    }
}

/// Días desde 1970-01-01 de una fecha civil (gregoriano proléptico; algoritmo de Hinnant).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// La fecha civil `(año, mes, día)` de un recuento de días.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Una fecha como días desde 1970-01-01: `2025-03-31`, `2025/03/31`, `31/03/2025`, `31-03-2025`,
/// con una hora opcional detrás (se ignora).
pub fn parse_date(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = s.split(['T', ' ']).next().unwrap_or(s);
    let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n: Vec<i64> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    let (y, m, d) = if parts[0].len() == 4 {
        (n[0], n[1], n[2])
    } else if parts[2].len() == 4 {
        (n[2], n[1], n[0])
    } else {
        return None;
    };
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let days = days_from_civil(y, m, d);
    // el día debe existir en ese mes (no hay 31 de febrero)
    (civil_from_days(days) == (y, m, d)).then_some(days as f64)
}

/// `AAAA-MM-DD` de un recuento de días.
pub fn format_date(days: f64) -> String {
    let (y, m, d) = civil_from_days(days.round() as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_and_bad_ones_are_refused() {
        assert_eq!(parse_date("1970-01-01"), Some(0.0));
        assert_eq!(parse_date("2025-03-31"), parse_date("31/03/2025"));
        assert_eq!(format_date(parse_date("2024-02-29").unwrap_or(f64::NAN)), "2024-02-29");
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("2025-13-01"), None);
        assert_eq!(parse_date("hola"), None);
        assert!(parse_date("2025-03-01").unwrap_or(0.0) < parse_date("2025-03-02").unwrap_or(0.0));
    }

    #[test]
    fn values_are_checked_by_type() {
        assert_eq!(FieldType::Integer.parse(" 12 "), Ok(("12".into(), 12.0)));
        assert!(FieldType::Integer.parse("1.5").is_err());
        assert_eq!(FieldType::Real.parse("1,5").map(|v| v.1), Ok(1.5));
        assert!(FieldType::Real.parse("abc").is_err());
        assert_eq!(FieldType::Bool.parse("si"), Ok(("Sí".into(), 1.0)));
        assert_eq!(FieldType::Bool.parse("False"), Ok(("No".into(), 0.0)));
        assert!(FieldType::Bool.parse("quizá").is_err());
        assert_eq!(FieldType::Date.parse("31/03/2025").map(|v| v.0), Ok("2025-03-31".into()));
        assert_eq!(FieldType::Text.parse("a b").map(|v| v.0), Ok("a b".into()));
        assert_eq!(FieldType::Date.parse("").map(|v| v.0), Ok(String::new()));
    }

    #[test]
    fn a_column_is_typed_by_its_values() {
        let t = |v: &[&str]| FieldType::infer(v.iter().copied());
        assert_eq!(t(&["1", "2", ""]), FieldType::Integer);
        assert_eq!(t(&["1", "2.5"]), FieldType::Real);
        assert_eq!(t(&["1", "a"]), FieldType::Text);
        assert_eq!(t(&[]), FieldType::Text);
    }

    /// Los tipos que se dieron a una capa sobreviven a escribirla y leerla en cada formato.
    #[test]
    fn types_survive_every_format() -> crate::Result<()> {
        use crate::view::{Feature, GeomKind, VectorData};
        let mut d = VectorData {
            features: vec![
                Feature { parts: vec![vec![[1.0, 2.0]]] },
                Feature { parts: vec![vec![[3.0, 4.0]]] },
            ],
            points: true,
            ..VectorData::default()
        };
        d.text = vec![vec![], vec![]];
        d.num = vec![vec![], vec![]];
        for (n, t) in [
            ("nombre", FieldType::Text),
            ("cuenta", FieldType::Integer),
            ("medida", FieldType::Real),
            ("activo", FieldType::Bool),
            ("fecha", FieldType::Date),
            ("web", FieldType::Link),
            ("codigo", FieldType::Text),
        ] {
            d.add_field(n, t);
        }
        let rows = [
            ["Pino", "3", "1,5", "sí", "31/03/2025", "https://hipso.example/a", "007"],
            ["Haya", "", "2", "no", "", "", "10"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                d.set_value(r, c, v).map_err(crate::Error::Param)?;
            }
        }
        let dir = std::env::temp_dir().join(format!("hipso_types_{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        for file in ["t.shp", "t.gpkg", "t.geojson"] {
            let path = dir.join(file);
            crate::vector::write_layer(&path, &d, GeomKind::Point, None)?;
            let back = crate::view::read_vector(&path)?;
            // por nombre: un GeoPackage da además su columna `fid`
            let got: Vec<FieldType> = d
                .fields
                .iter()
                .map(|n| back.fields.iter().position(|f| f == n).map_or(FieldType::Text, |c| back.field_type(c)))
                .collect();
            let col = |n: &str| back.fields.iter().position(|f| f == n).unwrap_or(0);
            let want: Vec<FieldType> = d.types.clone();
            // GeoJSON no tiene fechas ni enlaces: vuelven como fechas por su forma y como texto
            let ok = if file == "t.geojson" {
                got[1..4] == want[1..4] && got[4] == FieldType::Date
            } else {
                got == want
            };
            assert!(ok, "{file}: {got:?}");
            assert_eq!(back.text[0][col("activo")], "Sí", "{file}");
            assert_eq!(back.text[1][col("activo")], "No", "{file}");
            assert_eq!(back.text[0][col("fecha")], "2025-03-31", "{file}");
            assert_eq!(back.text[0][col("codigo")], "007", "{file}");
            assert_eq!(back.text[0][col("medida")], "1.5", "{file}");
            assert!(back.num[0][col("activo")] == 1.0 && back.num[0][col("fecha")] == d.num[0][4], "{file}");
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
