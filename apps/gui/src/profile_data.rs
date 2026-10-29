//! The points the profile draws. The plan and the 3D view show a thinned copy of big clouds,
//! and a profile cuts a thin slice of it, so few points would be left. The profile instead
//! reads from the files the points of the region around its box, at full density (up to a
//! memory cap), in the background; until they arrive it draws the points already in memory.

use super::*;
use rustlidar_core::edit;
use std::sync::mpsc::TryRecvError;

/// Most points the profile region keeps in memory (about 350 MB). Past it the region is thinned,
/// which only happens when the box itself holds more.
const CAP: usize = 16_000_000;

type Loaded = Result<(u64, [f64; 4], ProfileBox, PointStore), String>;

pub struct ProfJob {
    rx: Receiver<Loaded>,
}

pub struct ProfData {
    clouds_key: u64,
    region: [f64; 4],
    /// The strip the points were read from; those outside it are not in memory.
    slab: ProfileBox,
    raw: Arc<PointStore>,
    /// `raw` with the edits and the selection of the session, and what they were made for.
    view: Option<((u64, u64), Arc<PointStore>)>,
}

/// Whether the region in memory (`region` and the one in every `stride` points it keeps) holds
/// the box `need`, and whether it was thinned to fit the cap while the box now is much smaller:
/// the points of a small box are all there to be read, `wanted` is the region to read for it.
fn region_state(held: Option<([f64; 4], u64)>, need: [f64; 4], wanted: [f64; 4]) -> (bool, bool) {
    let area = |r: &[f64; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let Some((r, stride)) = held else {
        return (false, false);
    };
    let covered = r[0] <= need[0] && r[1] <= need[1] && r[2] >= need[2] && r[3] >= need[3];
    (covered, stride > 1 && area(&r) > 1.5 * area(&wanted))
}

/// The box widened by `pad` on every side: the strip of the cloud the profile keeps in memory.
fn slab_of(bx: &ProfileBox, pad: f64) -> ProfileBox {
    let l = bx.length().max(1e-9);
    let (ux, uy) = ((bx.b[0] - bx.a[0]) / l, (bx.b[1] - bx.a[1]) / l);
    ProfileBox {
        a: [bx.a[0] - ux * pad, bx.a[1] - uy * pad],
        b: [bx.b[0] + ux * pad, bx.b[1] + uy * pad],
        width: bx.width + 2.0 * pad,
    }
}

/// Whether the strip `held` holds the whole box `bx`.
fn slab_holds(held: &ProfileBox, bx: &ProfileBox) -> bool {
    let l = bx.length().max(1e-9);
    let h = bx.width / 2.0;
    let (nx, ny) = (-(bx.b[1] - bx.a[1]) / l * h, (bx.b[0] - bx.a[0]) / l * h);
    let len = held.length();
    [
        [bx.a[0] + nx, bx.a[1] + ny],
        [bx.a[0] - nx, bx.a[1] - ny],
        [bx.b[0] + nx, bx.b[1] + ny],
        [bx.b[0] - nx, bx.b[1] - ny],
    ]
    .iter()
    .all(|c| {
        let (u, t) = held.local(c[0], c[1]);
        u >= -1e-6 && u <= len + 1e-6 && t.abs() <= held.width / 2.0 + 1e-6
    })
}

/// The map rectangle that holds the box.
fn bbox(bx: &ProfileBox) -> [f64; 4] {
    let l = bx.length().max(1e-9);
    let h = bx.width / 2.0;
    let (nx, ny) = (-(bx.b[1] - bx.a[1]) / l * h, (bx.b[0] - bx.a[0]) / l * h);
    let xs = [bx.a[0] + nx, bx.a[0] - nx, bx.b[0] + nx, bx.b[0] - nx];
    let ys = [bx.a[1] + ny, bx.a[1] - ny, bx.b[1] + ny, bx.b[1] - ny];
    let min = |v: &[f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    [min(&xs), min(&ys), max(&xs), max(&ys)]
}

impl App {
    /// The store the profile draws: the region at full density when it is in memory, else the
    /// store of the views. Starts reading the region when the box leaves the one in memory.
    pub fn profile_store(
        &mut self,
        ctx: &egui::Context,
        bx: &ProfileBox,
    ) -> Option<Arc<PointStore>> {
        let base = self.store.clone()?;
        if !self.prof_full || base.stride <= 1 {
            return Some(base);
        }
        let key = hash_of(&self.clouds);
        if self.prof_data.as_ref().is_some_and(|d| d.clouds_key != key) {
            self.prof_data = None;
        }
        if self.prof_failed.is_some_and(|k| k != key) {
            self.prof_failed = None;
        }
        if let Some(job) = &self.prof_job {
            match job.rx.try_recv() {
                Ok(r) => {
                    self.prof_job = None;
                    match r {
                        Ok((k, region, slab, store)) if k == key => {
                            self.prof_data = Some(ProfData {
                                clouds_key: k,
                                region,
                                slab,
                                raw: Arc::new(store),
                                view: None,
                            });
                            self.tex_prof = None;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            self.log(format!(
                                "Perfil: no se pudieron leer todos los puntos ({e}); se usan los de pantalla."
                            ));
                            self.prof_failed = Some(key);
                        }
                    }
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
                Err(TryRecvError::Disconnected) => self.prof_job = None,
            }
        }
        let need = bbox(bx);
        // a margin, so moving the box a few widths does not read again
        // a margin of three widths, so that moving the box a few steps does not read again; less
        // when the cloud is so dense that the margin alone would not fit in memory
        let area = if base.area > 0.0 {
            base.area
        } else {
            ((base.bounds[3] - base.bounds[0]) * (base.bounds[4] - base.bounds[1])).max(1.0)
        };
        let density = base.total as f64 / area.max(1.0);
        let pad = [3.0, 2.0, 1.0, 0.5]
            .into_iter()
            .map(|k| k * bx.width)
            .find(|pad| density * (bx.length() + 2.0 * pad) * (bx.width + 2.0 * pad) <= CAP as f64)
            .unwrap_or(bx.width * 0.5);
        let slab = slab_of(bx, pad);
        let wanted = bbox(&slab);
        let (covered, thinned) = region_state(
            self.prof_data.as_ref().map(|d| (d.region, d.raw.stride)),
            need,
            wanted,
        );
        let covered = covered
            && self
                .prof_data
                .as_ref()
                .is_some_and(|d| slab_holds(&d.slab, bx));
        if (!covered || thinned) && self.prof_job.is_none() && self.prof_failed.is_none() {
            let region = wanted;
            // the COPC copies give the nodes the region touches; a plain LAZ is scanned whole
            let files = if self.detail_src.is_empty() {
                self.clouds.clone()
            } else {
                self.detail_src.clone()
            };
            let like = PointStore {
                origin: base.origin,
                files: base.files.clone(),
                total: base.total,
                prj: base.prj.clone(),
                ..PointStore::default()
            };
            let (tx, rx) = channel();
            let c = ctx.clone();
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(|| {
                    view::load_slab(&files, region, Some(&slab), CAP, &like)
                })
                .map_err(|_| "error interno al leer".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()))
                .map(|s| (key, region, slab, s));
                let _ = tx.send(r);
                c.request_repaint();
            });
            self.prof_job = Some(ProfJob { rx });
        }
        if !covered {
            return Some(base);
        }
        let stamp = (self.rev, self.ed.sel_rev);
        let ops = &self.ed.ops;
        let session = self.ed.session.as_ref();
        let edited = session.is_some_and(|s| s.edits.iter().any(|e| e.enabled));
        let data = self.prof_data.as_mut()?;
        if !edited && ops.is_empty() {
            return Some(data.raw.clone());
        }
        if data.view.as_ref().is_none_or(|v| v.0 != stamp) {
            let mut s = (*data.raw).clone();
            if let Some(ses) = session {
                ses.apply_to(&mut s);
            }
            s.selected = if ops.is_empty() {
                Vec::new()
            } else {
                edit::selection(&s, ops)
            };
            data.view = Some((stamp, Arc::new(s)));
        }
        data.view.as_ref().map(|v| v.1.clone())
    }

    /// Whether the region of the profile is being read.
    pub fn profile_loading(&self) -> bool {
        self.prof_job.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_holds_the_box_and_a_few_shifts_of_it() {
        let bx = ProfileBox {
            a: [0.0, 0.0],
            b: [100.0, 100.0],
            width: 10.0,
        };
        let held = slab_of(&bx, 30.0);
        assert!(slab_holds(&held, &bx));
        assert!(slab_holds(&held, &bx.shifted(10.0)));
        assert!(slab_holds(&held, &bx.shifted(-20.0)));
        // four widths away is outside the pad of three
        assert!(!slab_holds(&held, &bx.shifted(40.0)));
    }

    #[test]
    fn a_thinned_region_is_read_again_when_the_box_gets_small() {
        let huge = [0.0, 0.0, 30_000.0, 30_000.0];
        let small = [10_000.0, 10_000.0, 10_600.0, 10_300.0];
        let wanted = [9_997.0, 9_997.0, 10_603.0, 10_303.0];
        // thinned at the cap, and the box is a sliver of it: read again
        assert_eq!(region_state(Some((huge, 56)), small, wanted), (true, true));
        // all points kept (stride 1): nothing to gain
        assert_eq!(region_state(Some((huge, 1)), small, wanted), (true, false));
        // a region already about the size of the box stays, thinned or not
        assert_eq!(
            region_state(Some((wanted, 3)), small, wanted),
            (true, false)
        );
        // nothing in memory, or a box outside it
        assert_eq!(region_state(None, small, wanted), (false, false));
        assert_eq!(region_state(Some((small, 1)), huge, huge), (false, false));
    }
}
