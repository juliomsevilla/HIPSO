//! Editing the points of a cloud like features in a GIS: select them with regions (rectangle,
//! polygon, circle, lasso; in plan or in a profile) and attributes, combine selections (new, add,
//! subtract, intersect, invert), then delete them or change their class, with undo and redo.
//!
//! Edits are rules, not rewritten points. The viewer applies them at once to the decimated points
//! it shows; saving applies the same rules to every point of the file in one streaming pass, so
//! the result is exact whatever the decimation and memory stays flat.

use std::path::Path;

use rayon::prelude::*;

use crate::classes::ClassSet;
use crate::las::{self, Reader, Writer};
use crate::vector::Polygon;
use crate::view::{PointStore, ProfileBox, return_bit};
use crate::{Error, Result};

/// Where a selection operation looks.
#[derive(Clone, Debug)]
pub enum Region {
    /// The whole cloud (selection by attributes alone).
    All,
    /// A polygon in map coordinates, whatever the elevation.
    Plan(Polygon),
    /// A polygon in a profile: x is the distance along the box, y the elevation; only points
    /// inside the box count.
    Profile { bx: ProfileBox, poly: Polygon },
}

/// What a point must be to be picked, besides lying in the region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attr {
    pub classes: ClassSet,
    /// Bits: 1 first, 2 intermediate, 4 last, 8 single returns.
    pub returns: u8,
    pub z: Option<(f64, f64)>,
    pub intensity: Option<(u16, u16)>,
}

impl Default for Attr {
    fn default() -> Self {
        Self {
            classes: ClassSet::all(),
            returns: 15,
            z: None,
            intensity: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    New,
    Add,
    Subtract,
    Intersect,
    /// Flips the selection; the region and attributes are ignored.
    Invert,
}

/// One step of building a selection.
#[derive(Clone, Debug)]
pub struct SelOp {
    pub mode: Mode,
    pub region: Region,
    pub attr: Attr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Delete,
    Reclassify(u8),
}

/// A selection and what is done to it.
#[derive(Clone, Debug)]
pub struct Edit {
    pub sel: Vec<SelOp>,
    pub action: Action,
    pub label: String,
}

/// The values of a point a selection looks at.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub class: u8,
    pub intensity: u16,
    /// Return number in the low nibble, number of returns in the high one.
    pub ret: u8,
}

impl SelOp {
    fn matches(&self, p: &Sample) -> bool {
        let a = &self.attr;
        if !a.classes.contains(p.class)
            || a.returns & return_bit(p.ret) == 0
            || a.z.is_some_and(|(lo, hi)| p.z < lo || p.z > hi)
            || a.intensity
                .is_some_and(|(lo, hi)| p.intensity < lo || p.intensity > hi)
        {
            return false;
        }
        match &self.region {
            Region::All => true,
            Region::Plan(poly) => poly.contains(p.x, p.y),
            Region::Profile { bx, poly } => {
                let (u, t) = bx.local(p.x, p.y);
                u >= 0.0 && u <= bx.length() && t.abs() <= bx.width / 2.0 && poly.contains(u, p.z)
            }
        }
    }
}

/// Whether the selection built by `ops` includes the point.
pub fn selected(ops: &[SelOp], p: &Sample) -> bool {
    let mut state = false;
    for op in ops {
        state = match op.mode {
            Mode::Invert => !state,
            Mode::New => op.matches(p),
            Mode::Add => state || op.matches(p),
            Mode::Subtract => state && !op.matches(p),
            Mode::Intersect => state && op.matches(p),
        };
    }
    state
}

fn sample_of(s: &PointStore, i: usize) -> Sample {
    let p = s.xyz(i);
    Sample {
        x: p[0],
        y: p[1],
        z: p[2],
        class: s.class[i],
        intensity: s.intensity[i],
        ret: s.ret[i],
    }
}

/// The selection `ops` makes over the points of the store (deleted points are never selected).
pub fn selection(s: &PointStore, ops: &[SelOp]) -> Vec<bool> {
    if ops.is_empty() {
        return Vec::new();
    }
    (0..s.len())
        .into_par_iter()
        .map(|i| !s.deleted.get(i).copied().unwrap_or(false) && selected(ops, &sample_of(s, i)))
        .collect()
}

/// A regular polygon approximating a circle.
pub fn circle(cx: f64, cy: f64, r: f64) -> Polygon {
    let ring: Vec<[f64; 2]> = (0..72)
        .map(|k| {
            let a = k as f64 * std::f64::consts::TAU / 72.0;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    Polygon::new(vec![ring])
}

/// A rectangle as a polygon.
pub fn rectangle(a: [f64; 2], b: [f64; 2]) -> Polygon {
    Polygon::new(vec![vec![
        [a[0], a[1]],
        [b[0], a[1]],
        [b[0], b[1]],
        [a[0], b[1]],
    ]])
}

/// The editing session over the points in the viewer: the original classes, the edits made and
/// those undone.
#[derive(Clone, Debug, Default)]
pub struct Session {
    orig_class: Vec<u8>,
    pub edits: Vec<Edit>,
    pub undone: Vec<Edit>,
}

impl Session {
    pub fn new(s: &PointStore) -> Self {
        Self {
            orig_class: s.class.clone(),
            edits: Vec::new(),
            undone: Vec::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.edits.is_empty()
    }

    fn run(s: &mut PointStore, e: &Edit) -> usize {
        let sel = selection(s, &e.sel);
        if sel.is_empty() {
            return 0;
        }
        if matches!(e.action, Action::Delete) && s.deleted.is_empty() {
            s.deleted = vec![false; s.len()];
        }
        let mut n = 0;
        for (i, on) in sel.iter().enumerate() {
            if !on {
                continue;
            }
            match e.action {
                Action::Delete => s.deleted[i] = true,
                Action::Reclassify(c) => s.class[i] = c,
            }
            n += 1;
        }
        n
    }

    /// Applies an edit to the store; how many displayed points it touched.
    pub fn apply(&mut self, s: &mut PointStore, e: Edit) -> usize {
        let n = Self::run(s, &e);
        self.edits.push(e);
        self.undone.clear();
        n
    }

    fn replay(&self, s: &mut PointStore) {
        s.class.clone_from(&self.orig_class);
        s.deleted.clear();
        for e in &self.edits {
            Self::run(s, e);
        }
    }

    pub fn undo(&mut self, s: &mut PointStore) -> bool {
        let Some(e) = self.edits.pop() else {
            return false;
        };
        self.undone.push(e);
        self.replay(s);
        true
    }

    pub fn redo(&mut self, s: &mut PointStore) -> bool {
        let Some(e) = self.undone.pop() else {
            return false;
        };
        Self::run(s, &e);
        self.edits.push(e);
        true
    }

    /// Back to the file as it was read.
    pub fn discard(&mut self, s: &mut PointStore) {
        self.edits.clear();
        self.undone.clear();
        self.replay(s);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveReport {
    pub read: u64,
    pub written: u64,
    pub deleted: u64,
    pub reclassified: u64,
}

/// Applies the edits to every point of `input` and writes the rest to `output` (LAS, or LAZ by
/// extension), one streaming pass in batches processed on all cores.
pub fn save(input: &Path, output: &Path, edits: &[Edit]) -> Result<SaveReport> {
    if input == output {
        return Err(Error::Param(
            "the edited cloud must go to a new file, not over the original".into(),
        ));
    }
    crate::paths::ensure_parent(output)?;
    let r = Reader::open(input)?;
    // classes above 31 need a full class byte: legacy files are written as LAS 1.4
    let big = edits
        .iter()
        .any(|e| matches!(e.action, Action::Reclassify(c) if c > 31));
    let upgrade = match (big, r.header.format) {
        (true, f) if f < 6 => Some(las::upgraded_format(f).ok_or_else(|| {
            Error::Unsupported(format!(
                "point format {f} cannot hold classes above 31: convert the cloud first"
            ))
        })?),
        _ => None,
    };
    let (old_format, rec_len) = (r.header.format, usize::from(r.header.record_len));
    let template = match upgrade {
        Some(n) => Reader::open(input)?.upgraded(n),
        None => Reader::open(input)?,
    };
    let mut w = Writer::create(output, &template)?;
    let (format, mut rep) = (template.header.format, SaveReport::default());
    let mut conv = Vec::new();
    r.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        // per record: removed, or its new class when changed
        let out: Vec<(bool, Option<u8>)> = recs
            .par_iter()
            .map(|rec| {
                let Some(p) = r.decode(rec) else {
                    return (false, None);
                };
                let mut s = Sample {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    class: p.class,
                    intensity: p.intensity,
                    ret: p.return_number | (p.number_of_returns << 4),
                };
                let mut changed = false;
                for e in edits {
                    if selected(&e.sel, &s) {
                        match e.action {
                            Action::Delete => return (true, None),
                            Action::Reclassify(c) => {
                                s.class = c;
                                changed = true;
                            }
                        }
                    }
                }
                (false, changed.then_some(s.class))
            })
            .collect();
        let mut buf = vec![0u8; rec_len];
        for (rec, (gone, class)) in recs.iter().zip(out) {
            rep.read += 1;
            if gone {
                rep.deleted += 1;
                continue;
            }
            if let Some(new) = upgrade {
                las::upgrade_record(old_format, new, rec, &mut conv);
                if let Some(c) = class {
                    las::set_class(format, &mut conv, c);
                    rep.reclassified += 1;
                }
                w.write(&conv)?;
            } else if let Some(c) = class {
                buf.copy_from_slice(rec);
                las::set_class(format, &mut buf, c);
                w.write(&buf)?;
                rep.reclassified += 1;
            } else {
                w.write(rec)?;
            }
            rep.written += 1;
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(rep)
}

/// Writes to `output` only the points of `input` the selection `ops` picks (a new LAS or LAZ, one
/// streaming pass); how many. The processing tools run on this when a selection is active.
pub fn extract_selected(input: &Path, ops: &[SelOp], output: &Path) -> Result<u64> {
    if input == output {
        return Err(Error::Param("the selection must go to a new file".into()));
    }
    crate::paths::ensure_parent(output)?;
    let r = Reader::open(input)?;
    let mut w = Writer::create(output, &r)?;
    let rec_len = usize::from(r.header.record_len);
    let mut n = 0u64;
    r.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        let keep: Vec<bool> = recs
            .par_iter()
            .map(|rec| {
                r.decode(rec).is_some_and(|p| {
                    selected(
                        ops,
                        &Sample {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                            class: p.class,
                            intensity: p.intensity,
                            ret: p.return_number | (p.number_of_returns << 4),
                        },
                    )
                })
            })
            .collect();
        for (rec, k) in recs.iter().zip(keep) {
            if k {
                w.write(rec)?;
                n += 1;
            }
        }
        Ok(())
    })?;
    w.finish()?;
    Ok(n)
}

pub fn report_text(r: &SaveReport, output: &Path) -> String {
    format!(
        "{} puntos leídos, {} escritos, {} eliminados, {} reclasificados: {}",
        r.read,
        r.written,
        r.deleted,
        r.reclassified,
        output.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> PointStore {
        let mut s = PointStore {
            stride: 1,
            total: 5,
            origin: [0.0; 3],
            ..PointStore::default()
        };
        for (x, y, z, c) in [
            (1.0f32, 1.0f32, 1.0f32, 2u8),
            (2.0, 2.0, 9.0, 5),
            (8.0, 8.0, 3.0, 5),
            (3.0, 1.0, 20.0, 1),
            (9.0, 1.0, 1.0, 2),
        ] {
            s.x.push(x);
            s.y.push(y);
            s.z.push(z);
            s.class.push(c);
            s.intensity.push(10);
            s.ret.push(0x11);
            s.src.push(0);
            s.file.push(0);
        }
        s
    }

    fn op(mode: Mode, region: Region, attr: Attr) -> SelOp {
        SelOp { mode, region, attr }
    }

    #[test]
    fn selection_modes_combine() {
        let s = store();
        let near = op(
            Mode::New,
            Region::Plan(rectangle([0.0, 0.0], [4.0, 4.0])),
            Attr::default(),
        );
        assert_eq!(
            selection(&s, std::slice::from_ref(&near)),
            [true, true, false, true, false]
        );
        let veg = Attr {
            classes: ClassSet::from_bits(1 << 5),
            ..Attr::default()
        };
        // inside the rectangle and class 5
        let both = [near.clone(), op(Mode::Intersect, Region::All, veg)];
        assert_eq!(selection(&s, &both), [false, true, false, false, false]);
        // all vegetation minus the rectangle
        let rest = [
            op(Mode::New, Region::All, veg),
            op(Mode::Subtract, near.region.clone(), Attr::default()),
        ];
        assert_eq!(selection(&s, &rest), [false, false, true, false, false]);
        let inv = [near, op(Mode::Invert, Region::All, Attr::default())];
        assert_eq!(selection(&s, &inv), [false, false, true, false, true]);
    }

    #[test]
    fn profile_region_uses_distance_and_height() {
        let s = store();
        let bx = ProfileBox {
            a: [0.0, 1.0],
            b: [10.0, 1.0],
            width: 2.0,
        };
        // above 10 m in the profile: only the point at z = 20
        let sel = [op(
            Mode::New,
            Region::Profile {
                bx,
                poly: rectangle([0.0, 10.0], [10.0, 30.0]),
            },
            Attr::default(),
        )];
        assert_eq!(selection(&s, &sel), [false, false, false, true, false]);
    }

    #[test]
    fn edits_undo_and_redo() {
        let mut s = store();
        let mut ses = Session::new(&s);
        let all_veg = vec![op(
            Mode::New,
            Region::All,
            Attr {
                classes: ClassSet::from_bits(1 << 5),
                ..Attr::default()
            },
        )];
        assert_eq!(
            ses.apply(
                &mut s,
                Edit {
                    sel: all_veg.clone(),
                    action: Action::Reclassify(7),
                    label: "ruido".into()
                }
            ),
            2
        );
        assert_eq!(s.class, [2, 7, 7, 1, 2]);
        // the class filter now sees class 7: the next selection of class 5 finds nothing
        assert_eq!(
            ses.apply(
                &mut s,
                Edit {
                    sel: all_veg,
                    action: Action::Delete,
                    label: "x".into()
                }
            ),
            0
        );
        let noise = vec![op(
            Mode::New,
            Region::All,
            Attr {
                classes: ClassSet::from_bits(1 << 7),
                ..Attr::default()
            },
        )];
        ses.apply(
            &mut s,
            Edit {
                sel: noise,
                action: Action::Delete,
                label: "borrar ruido".into(),
            },
        );
        assert_eq!(s.deleted, [false, true, true, false, false]);
        assert!(ses.undo(&mut s));
        assert!(s.deleted.is_empty() || s.deleted.iter().all(|d| !d));
        assert_eq!(s.class, [2, 7, 7, 1, 2]);
        assert!(ses.redo(&mut s));
        assert_eq!(s.deleted, [false, true, true, false, false]);
        ses.discard(&mut s);
        assert_eq!(s.class, [2, 5, 5, 1, 2]);
    }
}
