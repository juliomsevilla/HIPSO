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
use crate::view::{Orbit, PointStore, ProfileBox, Projector, return_bit};
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
    /// A shape drawn on the 3D view, whatever lies in front of or behind the points.
    View(ViewCut),
}

/// A polygon drawn in pixels on a 3D view, with the camera that showed it and, optionally, the
/// range of distances from the camera plane that count (a slab through the scene). It is
/// geometric, so it picks the same points in the decimated viewer and in the whole file.
#[derive(Clone, Debug)]
pub struct ViewCut {
    cam: Projector,
    poly: Polygon,
    depth: Option<(f64, f64)>,
}

impl ViewCut {
    pub fn new(cam: &Orbit, w: usize, h: usize, poly: Polygon, depth: Option<(f64, f64)>) -> Self {
        Self {
            cam: cam.projector(w, h),
            poly,
            depth,
        }
    }

    fn contains(&self, p: &Sample) -> bool {
        match self.cam.apply([p.x, p.y, p.z]) {
            Some((x, y, d)) => {
                self.depth.is_none_or(|(lo, hi)| d >= lo && d <= hi) && self.poly.contains(x, y)
            }
            None => false,
        }
    }
}

/// The attributes of a point a selection can be made on (the histograms of the editor).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrKind {
    Z,
    Intensity,
    ReturnNumber,
    NumReturns,
    Class,
    /// Flight line (point source ID).
    Source,
    /// Scan angle in degrees.
    ScanAngle,
}

impl AttrKind {
    pub const ALL: [AttrKind; 7] = [
        AttrKind::Z,
        AttrKind::Intensity,
        AttrKind::ReturnNumber,
        AttrKind::NumReturns,
        AttrKind::Class,
        AttrKind::Source,
        AttrKind::ScanAngle,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AttrKind::Z => "Elevación (Z)",
            AttrKind::Intensity => "Intensidad",
            AttrKind::ReturnNumber => "Nº de retorno",
            AttrKind::NumReturns => "Nº de retornos",
            AttrKind::Class => "Clase",
            AttrKind::Source => "Pasada (ID de fuente)",
            AttrKind::ScanAngle => "Ángulo de barrido (°)",
        }
    }

    /// Whole-number attributes: a histogram gets one bar per value.
    pub fn is_integer(self) -> bool {
        !matches!(self, AttrKind::Z | AttrKind::ScanAngle)
    }

    pub fn value(self, p: &Sample) -> f64 {
        match self {
            AttrKind::Z => p.z,
            AttrKind::Intensity => f64::from(p.intensity),
            AttrKind::ReturnNumber => f64::from(p.ret & 15),
            AttrKind::NumReturns => f64::from(p.ret >> 4),
            AttrKind::Class => f64::from(p.class),
            AttrKind::Source => f64::from(p.src),
            AttrKind::ScanAngle => f64::from(p.scan),
        }
    }
}

/// A point passes when the attribute lies in any of the intervals `[lo, hi)` (the bars picked
/// in a histogram, or the tails of its odd values).
#[derive(Clone, Debug, PartialEq)]
pub struct RangeSel {
    pub kind: AttrKind,
    pub bins: Vec<(f64, f64)>,
}

/// What a point must be to be picked, besides lying in the region.
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub classes: ClassSet,
    /// Bits: 1 first, 2 intermediate, 4 last, 8 single returns.
    pub returns: u8,
    pub z: Option<(f64, f64)>,
    pub intensity: Option<(u16, u16)>,
    /// Every one of these must hold (the histogram selections).
    pub ranges: Vec<RangeSel>,
}

impl Default for Attr {
    fn default() -> Self {
        Self {
            classes: ClassSet::all(),
            returns: 15,
            z: None,
            intensity: None,
            ranges: Vec::new(),
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
    /// A step the user has switched off stays in the history but does nothing: the others
    /// are applied as if it had never been made.
    pub enabled: bool,
}

impl Edit {
    pub fn new(sel: Vec<SelOp>, action: Action, label: String) -> Self {
        Self {
            sel,
            action,
            label,
            enabled: true,
        }
    }
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
    pub src: u16,
    /// Scan angle in degrees.
    pub scan: f32,
}

impl SelOp {
    fn matches(&self, p: &Sample) -> bool {
        let a = &self.attr;
        if !a.classes.contains(p.class)
            || a.returns & return_bit(p.ret) == 0
            || a.z.is_some_and(|(lo, hi)| p.z < lo || p.z > hi)
            || a.intensity
                .is_some_and(|(lo, hi)| p.intensity < lo || p.intensity > hi)
            || a.ranges.iter().any(|r| {
                let v = r.kind.value(p);
                !r.bins.iter().any(|(lo, hi)| v >= *lo && v < *hi)
            })
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
            Region::View(cut) => cut.contains(p),
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
        src: s.src.get(i).copied().unwrap_or(0),
        scan: s.scan.get(i).map_or(0.0, |v| f32::from(*v) / 10.0),
    }
}

/// The value of an attribute of point `i` of the store.
pub fn value_of(s: &PointStore, i: usize, kind: AttrKind) -> f64 {
    kind.value(&sample_of(s, i))
}

/// Whether a selection looks at fields that are read from the record beyond the point.
fn needs_extra(ops: &[SelOp]) -> bool {
    ops.iter().any(|o| !o.attr.ranges.is_empty())
}

/// The values of a decoded record; the source and scan angle only when a rule needs them.
fn sample_of_record(r: &Reader, rec: &[u8], p: &las::Point, extra: bool) -> Sample {
    let (src, scan) = if extra {
        let e = r.decode_extra(rec);
        (e.source_id, e.scan_angle)
    } else {
        (0, 0.0)
    };
    Sample {
        x: p.x,
        y: p.y,
        z: p.z,
        class: p.class,
        intensity: p.intensity,
        ret: p.return_number | (p.number_of_returns << 4),
        src,
        scan,
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
        if !e.enabled {
            return 0;
        }
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

    /// Switches the edit `k` off or on again, leaving the others as they are; whether it
    /// exists. A later edit that selects by class sees the points as they now are.
    pub fn toggle(&mut self, s: &mut PointStore, k: usize) -> bool {
        let Some(e) = self.edits.get_mut(k) else {
            return false;
        };
        e.enabled = !e.enabled;
        self.replay(s);
        true
    }

    /// Applies the edits that count to another store of the same clouds (a region read at
    /// full density), so it shows what the store of the session shows.
    pub fn apply_to(&self, s: &mut PointStore) {
        for e in &self.edits {
            Self::run(s, e);
        }
    }

    /// The edits that count when saving.
    pub fn active(&self) -> Vec<Edit> {
        self.edits.iter().filter(|e| e.enabled).cloned().collect()
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
    let active: Vec<Edit> = edits.iter().filter(|e| e.enabled).cloned().collect();
    let edits = &active[..];
    if input == output {
        return Err(Error::Param(
            "la nube editada debe ir a un archivo nuevo, no sobre el original".into(),
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
                "el formato de punto {f} no admite clases por encima de 31: convierte antes la nube"
            ))
        })?),
        _ => None,
    };
    let extra = edits.iter().any(|e| needs_extra(&e.sel));
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
                let mut s = sample_of_record(&r, rec, &p, extra);
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
        return Err(Error::Param(
            "la selección debe ir a un archivo nuevo".into(),
        ));
    }
    crate::paths::ensure_parent(output)?;
    let r = Reader::open(input)?;
    let mut w = Writer::create(output, &r)?;
    let rec_len = usize::from(r.header.record_len);
    let extra = needs_extra(ops);
    let mut n = 0u64;
    r.scan_raw(|bytes| {
        let recs: Vec<&[u8]> = bytes.chunks_exact(rec_len).collect();
        let keep: Vec<bool> = recs
            .par_iter()
            .map(|rec| {
                r.decode(rec)
                    .is_some_and(|p| selected(ops, &sample_of_record(&r, rec, &p, extra)))
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
        let both = [near.clone(), op(Mode::Intersect, Region::All, veg.clone())];
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
    fn an_edit_can_be_switched_off_without_touching_the_others() {
        let mut s = store();
        let mut ses = Session::new(&s);
        let by_class = |c: u8| {
            vec![op(
                Mode::New,
                Region::All,
                Attr {
                    classes: ClassSet::from_bits(1 << c),
                    ..Attr::default()
                },
            )]
        };
        ses.apply(
            &mut s,
            Edit::new(by_class(5), Action::Reclassify(7), "veg a ruido".into()),
        );
        ses.apply(
            &mut s,
            Edit::new(by_class(1), Action::Delete, "borrar".into()),
        );
        assert_eq!(s.class, [2, 7, 7, 1, 2]);
        assert_eq!(s.deleted, [false, false, false, true, false]);
        // the first goes off: vegetation is back, the deletion stays
        assert!(ses.toggle(&mut s, 0));
        assert_eq!(s.class, [2, 5, 5, 1, 2]);
        assert_eq!(s.deleted, [false, false, false, true, false]);
        assert_eq!(ses.active().len(), 1);
        assert!(ses.toggle(&mut s, 0));
        assert_eq!(s.class, [2, 7, 7, 1, 2]);
        assert!(!ses.toggle(&mut s, 9));
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
                    label: "ruido".into(),
                    enabled: true
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
                    label: "x".into(),
                    enabled: true
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
                enabled: true,
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

    /// A LAS 1.2 file of point format 1 with (scan angle in degrees, point source id) per point.
    fn las_with(points: &[(i8, u16)]) -> Vec<u8> {
        let mut h = vec![0u8; 227];
        h[..4].copy_from_slice(b"LASF");
        h[24] = 1;
        h[25] = 2;
        h[94..96].copy_from_slice(&227u16.to_le_bytes());
        h[96..100].copy_from_slice(&227u32.to_le_bytes());
        h[104] = 1;
        h[105..107].copy_from_slice(&28u16.to_le_bytes());
        h[107..111].copy_from_slice(&(points.len() as u32).to_le_bytes());
        for (at, v) in [(131, 0.01), (139, 0.01), (147, 0.01)] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for (at, v) in [
            (179, 10.0),
            (187, 0.0),
            (195, 10.0),
            (203, 0.0),
            (211, 10.0),
            (219, 0.0),
        ] {
            h[at..at + 8].copy_from_slice(&f64::to_le_bytes(v));
        }
        for (k, &(scan, src)) in points.iter().enumerate() {
            let mut rec = [0u8; 28];
            rec[..4].copy_from_slice(&(100 * k as i32).to_le_bytes());
            rec[14] = 0x09;
            rec[15] = 1;
            rec[16] = scan as u8;
            rec[18..20].copy_from_slice(&src.to_le_bytes());
            h.extend_from_slice(&rec);
        }
        h
    }

    #[test]
    fn saving_applies_range_rules_on_fields_read_from_the_record() -> Result<()> {
        // scan angles 0, 5, -4, 35, -40, 10 from passes 1, 1, 2, 1, 3, 2
        let bytes = las_with(&[(0, 1), (5, 1), (-4, 2), (35, 1), (-40, 3), (10, 2)]);
        let dir = std::env::temp_dir();
        let id = std::process::id();
        let (input, out) = (
            dir.join(format!("rustlidar_edit_in_{id}.las")),
            dir.join(format!("rustlidar_edit_out_{id}.las")),
        );
        std::fs::write(&input, &bytes)?;
        let rule = |kind, bins: Vec<(f64, f64)>| {
            vec![SelOp {
                mode: Mode::New,
                region: Region::All,
                attr: Attr {
                    ranges: vec![RangeSel { kind, bins }],
                    ..Attr::default()
                },
            }]
        };
        // the odd angles: beyond ±20°
        let steep = rule(
            AttrKind::ScanAngle,
            vec![(f64::NEG_INFINITY, -20.0), (20.0, f64::INFINITY)],
        );
        let kept = extract_selected(&input, &steep, &out)?;
        let edit = Edit {
            sel: steep,
            action: Action::Delete,
            label: "barrido".into(),
            enabled: true,
        };
        let del = save(&input, &out, std::slice::from_ref(&edit))?;
        // pass 3 only
        let only = Edit {
            sel: rule(AttrKind::Source, vec![(3.0, 4.0)]),
            action: Action::Reclassify(7),
            label: "pasada".into(),
            enabled: true,
        };
        let re = save(&input, &out, std::slice::from_ref(&only))?;
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&out);
        assert_eq!(kept, 2);
        assert_eq!((del.read, del.deleted, del.written), (6, 2, 4));
        assert_eq!((re.deleted, re.reclassified), (0, 1));
        Ok(())
    }

    #[test]
    fn ranges_pick_the_values_inside_any_interval_of_every_attribute() {
        let s = store();
        let by = |kind, bins: Vec<(f64, f64)>| {
            vec![op(
                Mode::New,
                Region::All,
                Attr {
                    ranges: vec![RangeSel { kind, bins }],
                    ..Attr::default()
                },
            )]
        };
        // z = 1, 9, 3, 20, 1: the bar [0, 2) holds two points
        assert_eq!(
            selection(&s, &by(AttrKind::Z, vec![(0.0, 2.0)])),
            [true, false, false, false, true]
        );
        // the tails of the odd values: below 0.5 or above 10
        let tails = vec![(f64::NEG_INFINITY, 0.5), (10.0, f64::INFINITY)];
        assert_eq!(
            selection(&s, &by(AttrKind::Z, tails)),
            [false, false, false, true, false]
        );
        // classes 2, 5, 5, 1, 2: bar [5, 6)
        assert_eq!(
            selection(&s, &by(AttrKind::Class, vec![(5.0, 6.0)])),
            [false, true, true, false, false]
        );
        // two attributes must both hold
        let both = vec![op(
            Mode::New,
            Region::All,
            Attr {
                ranges: vec![
                    RangeSel {
                        kind: AttrKind::Z,
                        bins: vec![(0.0, 5.0)],
                    },
                    RangeSel {
                        kind: AttrKind::Class,
                        bins: vec![(2.0, 3.0)],
                    },
                ],
                ..Attr::default()
            },
        )];
        assert_eq!(selection(&s, &both), [true, false, false, false, true]);
    }

    #[test]
    fn view_cuts_pick_by_screen_shape_and_depth_slab() {
        let s = store();
        let cam = Orbit {
            target: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            dist: 20.0,
            exaggeration: 1.0,
        };
        let cut = |poly, depth| {
            vec![op(
                Mode::New,
                Region::View(ViewCut::new(&cam, 100, 100, poly, depth)),
                Attr::default(),
            )]
        };
        // a rectangle of a few pixels around where the first point lands picks only it
        let (px, py) = cam.project(s.xyz(0), 100, 100).unwrap_or_default();
        let small = rectangle([px - 1.0, py - 1.0], [px + 1.0, py + 1.0]);
        assert_eq!(
            selection(&s, &cut(small, None)),
            [true, false, false, false, false]
        );
        // looking north the depth is 20 + y: a slab keeps the far points (y 2 and 8)
        let all = rectangle([-1e4, -1e4], [1e4, 1e4]);
        assert_eq!(
            selection(&s, &cut(all.clone(), Some((21.5, 30.0)))),
            [false, true, true, false, false]
        );
        assert_eq!(selection(&s, &cut(all, None)), [true; 5]);
    }
}
