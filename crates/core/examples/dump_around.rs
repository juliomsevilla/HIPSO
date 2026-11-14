//! Writes the points around a class: `dump_around FILE OUT CLASS [--half 60]`. Takes the points
//! within `--half` m (plan) of the centre of the points of that class, with SMRF's ground height:
//! `x y z class height` per line, to look at them.

use hipso_core::las::{Points, Reader};
use hipso_core::smrf::{self, SmrfParams};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (file, out, class) = (
        a.first().ok_or("FILE")?,
        a.get(1).ok_or("OUT")?,
        a.get(2).ok_or("CLASS")?.parse::<u8>()?,
    );
    let half: f64 = a.get(4).map_or(Ok(60.0), |v| v.parse())?;
    let r = Reader::open(std::path::Path::new(file))?;
    let mut pts = Vec::new();
    r.scan(|q| pts.push(*q))?;
    let w = Points::new(pts);
    let c: Vec<_> = w.pts.iter().filter(|q| q.class == class).collect();
    if c.is_empty() {
        return Err("no points of that class".into());
    }
    let (cx, cy) = (
        c.iter().map(|q| q.x).sum::<f64>() / c.len() as f64,
        c.iter().map(|q| q.y).sum::<f64>() / c.len() as f64,
    );
    let ground = smrf::ground(&w.pts, &SmrfParams::default());
    let surf = hipso_core::extras::ground_surface_pub(&w.pts, &ground, 1.0);
    let mut text = String::new();
    use std::fmt::Write as _;
    let stride: usize = a.get(5).map_or(Ok(1), |v| v.parse())?;
    for (idx, q) in w.pts.iter().enumerate() {
        if idx % stride == 0 && (q.x - cx).abs() <= half && (q.y - cy).abs() <= half {
            let h = surf
                .as_ref()
                .and_then(|g| g.sample(q.x, q.y))
                .map_or(f64::NAN, |g| q.z - f64::from(g));
            let _ = writeln!(text, "{:.2} {:.2} {:.2} {} {:.2} {} {} {} {}", q.x, q.y, q.z, q.class, h, q.return_number, q.number_of_returns, q.intensity, u8::from(ground[idx]));
        }
    }
    std::fs::write(out, text)?;
    Ok(())
}
