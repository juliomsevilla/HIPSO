//! Transforms the points of a JSON file (`[{"src","dst","pts":[[x,y,(z)]...]}]`) and prints the
//! results as JSON: the same cases are run through PROJ by `scripts/check_spain_crs.py`.
use rustlidar_crs::{SpatialRef, Transformer};

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let cases: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json");
    let mut out = Vec::new();
    for c in cases.as_array().expect("array") {
        let (s, d) = (
            c["src"].as_str().expect("src"),
            c["dst"].as_str().expect("dst"),
        );
        let t = SpatialRef::parse(s)
            .and_then(|a| SpatialRef::parse(d).and_then(|b| Transformer::new(&a, &b)));
        let res: Vec<serde_json::Value> = match &t {
            Ok(t) => c["pts"]
                .as_array()
                .expect("pts")
                .iter()
                .map(|p| {
                    let v: Vec<f64> = p
                        .as_array()
                        .expect("pt")
                        .iter()
                        .map(|x| x.as_f64().expect("num"))
                        .collect();
                    let r = if v.len() == 3 {
                        t.transform_3d(v[0], v[1], v[2])
                            .map(|(a, b, c)| vec![a, b, c])
                    } else {
                        t.transform(v[0], v[1]).map(|(a, b)| vec![a, b])
                    };
                    r.map_or(serde_json::Value::Null, |r| serde_json::json!(r))
                })
                .collect(),
            Err(e) => vec![serde_json::json!(format!("ERR {e}"))],
        };
        out.push(serde_json::json!({"src": s, "dst": d, "res": res}));
    }
    println!("{}", serde_json::Value::Array(out));
}
