//! Escritura de JSON para el asistente: el lector de `hipso_core::json` no escribe, y aquí hace
//! falta para las peticiones a los servicios, la configuración y las conversaciones.

use hipso_core::json::{Json, quote};

pub fn dump(j: &Json) -> String {
    match j {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Num(n) => {
            // JSON no tiene `NaN` ni infinitos
            if !n.is_finite() {
                "null".into()
            } else if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        Json::Str(s) => quote(s),
        Json::Arr(a) => format!("[{}]", a.iter().map(dump).collect::<Vec<_>>().join(",")),
        Json::Obj(m) => format!(
            "{{{}}}",
            m.iter().map(|(k, v)| format!("{}:{}", quote(k), dump(v))).collect::<Vec<_>>().join(",")
        ),
    }
}

/// Un objeto con los miembros en el orden dado.
pub fn obj(members: Vec<(&str, Json)>) -> Json {
    Json::Obj(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

pub fn s(text: impl Into<String>) -> Json {
    Json::Str(text.into())
}

pub fn n(v: f64) -> Json {
    Json::Num(v)
}

pub fn arr(items: Vec<Json>) -> Json {
    Json::Arr(items)
}

/// El texto de un miembro, o vacío.
pub fn text(j: &Json, key: &str) -> String {
    j.get(key).and_then(Json::as_str).unwrap_or_default().to_string()
}

/// El número de un miembro.
pub fn num(j: &Json, key: &str) -> Option<f64> {
    j.get(key).and_then(Json::as_f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() -> hipso_core::Result<()> {
        let j = obj(vec![
            ("a", s("x \"q\"\n")),
            ("n", n(2.0)),
            ("f", n(0.25)),
            ("l", arr(vec![Json::Bool(true), Json::Null, s("é")])),
        ]);
        let back = Json::parse(&dump(&j))?;
        assert_eq!(back, j);
        assert_eq!(dump(&n(3.0)), "3");
        assert_eq!(dump(&n(f64::NAN)), "null");
        assert_eq!(dump(&n(f64::INFINITY)), "null");
        Ok(())
    }
}
