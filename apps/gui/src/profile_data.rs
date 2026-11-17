//! Los puntos que dibuja el perfil. La planta y la vista 3D muestran una copia adelgazada de las
//! nubes grandes, y un perfil corta una rebanada fina de ella, así que quedarían pocos puntos. El
//! perfil lee en cambio de los archivos los puntos de la región alrededor de su caja, a toda la
//! densidad (hasta un tope de memoria), en segundo plano; hasta que llegan dibuja los puntos que ya
//! están en memoria.

use super::*;
use hipso_core::edit;
use std::sync::mpsc::TryRecvError;

/// Bytes que ocupa un punto en el almacén.
const POINT_BYTES: u64 = 24;

/// Máximo de puntos que guarda el perfil en memoria: una octava parte de la memoria del equipo (al
/// menos 16 millones, unos 350 MB). Más allá se adelgaza la franja, lo que solo ocurre cuando la
/// propia caja contiene más que eso.
fn cap() -> usize {
    let mem = hipso_core::copc_cache::total_memory().unwrap_or(8 << 30);
    ((mem / 8 / POINT_BYTES) as usize).clamp(16_000_000, 200_000_000)
}

type Loaded = Result<(u64, [f64; 4], ProfileBox, PointStore), String>;

pub struct ProfJob {
    rx: Receiver<Loaded>,
}

pub struct ProfData {
    clouds_key: u64,
    region: [f64; 4],
    /// La franja de la que se leyeron los puntos; los de fuera no están en memoria.
    slab: ProfileBox,
    raw: Arc<PointStore>,
    /// `raw` con las ediciones y la selección de la sesión, y para qué se hicieron.
    view: Option<((u64, u64, bool), Arc<PointStore>)>,
}

/// Si la región en memoria (`region` y el uno de cada `stride` puntos que conserva) contiene la
/// caja `need`, y si se adelgazó para ajustarse al tope mientras la caja es ahora mucho menor: los
/// puntos de una caja pequeña están todos ahí para leerlos, `wanted` es la región que leer para
/// ella.
fn region_state(held: Option<([f64; 4], u64)>, need: [f64; 4], wanted: [f64; 4]) -> (bool, bool) {
    let area = |r: &[f64; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let Some((r, stride)) = held else {
        return (false, false);
    };
    let covered = r[0] <= need[0] && r[1] <= need[1] && r[2] >= need[2] && r[3] >= need[3];
    (covered, stride > 1 && area(&r) > 1.5 * area(&wanted))
}

/// La caja ensanchada `pad` por cada lado: la franja de la nube que el perfil guarda en memoria.
fn slab_of(bx: &ProfileBox, pad: f64) -> ProfileBox {
    let l = bx.length().max(1e-9);
    let (ux, uy) = ((bx.b[0] - bx.a[0]) / l, (bx.b[1] - bx.a[1]) / l);
    ProfileBox {
        a: [bx.a[0] - ux * pad, bx.a[1] - uy * pad],
        b: [bx.b[0] + ux * pad, bx.b[1] + uy * pad],
        width: bx.width + 2.0 * pad,
    }
}

/// Si la franja `held` contiene toda la caja `bx`.
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

/// El rectángulo del mapa que contiene la caja.
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
    /// El almacén que dibuja el perfil: la región a toda la densidad cuando está en memoria o, si
    /// no, el almacén de las vistas. Empieza a leer la región cuando la caja sale de la que está en
    /// memoria.
    pub fn profile_store(
        &mut self,
        ctx: &egui::Context,
        bx: &ProfileBox,
    ) -> Option<Arc<PointStore>> {
        let base = self.store.clone()?;
        if !self.prof_full || base.stride <= 1 {
            return Some(base);
        }
        // mientras se cargan otras nubes, la lista de nubes ya es la nueva pero los puntos y las copias
        // de detalle son aún los de la anterior: leer ahora daría los puntos de las nubes viejas
        // atribuidos a las nuevas
        if self.loading.is_some() || base.files.len() != self.clouds.len() {
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
                            if store.stride > 1 {
                                self.log(format!(
                                    "Perfil: la caja tiene más puntos de los que caben en memoria; se dibuja 1 de cada {}.",
                                    store.stride
                                ));
                            }
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
        // un margen, para que mover la caja unas anchuras no vuelva a leer un margen de tres
        // anchuras, de modo que mover la caja unos pasos no vuelve a leer; menos cuando la nube es
        // tan densa que el margen solo no cabría en memoria
        let area = if base.area > 0.0 {
            base.area
        } else {
            ((base.bounds[3] - base.bounds[0]) * (base.bounds[4] - base.bounds[1])).max(1.0)
        };
        let density = base.total as f64 / area.max(1.0);
        let cap = cap();
        let pad = [3.0, 2.0, 1.0, 0.5, 0.0]
            .into_iter()
            .map(|k| k * bx.width)
            .find(|pad| density * (bx.length() + 2.0 * pad) * (bx.width + 2.0 * pad) <= cap as f64)
            .unwrap_or(0.0);
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
            // las copias COPC dan los nodos que toca la región; un LAZ normal se recorre entero
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
                    view::load_slab(&files, region, Some(&slab), cap, &like)
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
        // al colorear por altura sobre el suelo, el perfil necesita la altura de sus propios puntos
        // (los de toda la franja a toda su densidad), no la de los de la planta
        let want_hag = self.style.color_by == ColorBy::HeightAboveGround;
        let stamp = (self.rev, self.ed.sel_rev, want_hag);
        let session = self.ed.session.as_ref();
        let edited = session.is_some_and(|s| s.edits.iter().any(|e| e.enabled));
        let data = self.prof_data.as_mut()?;
        if !edited && self.ed.ops.is_empty() && self.ed.focus.is_none() {
            // hag_rev distingue «no calculada» de «calculada y sin suelo»: no se reintenta cada fotograma
            if want_hag
                && data.raw.hag_rev == 0
                && let Some(r) = Arc::get_mut(&mut data.raw)
            {
                r.hag_rev = 1;
                r.compute_hag(1.0);
            }
            return Some(data.raw.clone());
        }
        if data.view.as_ref().is_none_or(|v| v.0 != stamp) {
            // las operaciones se copian solo aquí, cuando el perfil se rehace, y no en cada fotograma
            let ops = crate::editing::with_focus(&self.ed.ops, &self.ed.focus);
            let mut s = (*data.raw).clone();
            if let Some(f) = &self.ed.focus {
                let inner = &f.inner;
                // el foco se mide con los puntos tal como salen del archivo
                s.outside = edit::outside_focus(&s, inner);
            }
            if let Some(ses) = session {
                ses.apply_to(&mut s);
            }
            if want_hag {
                s.compute_hag(1.0);
            }
            s.selected = if ops.is_empty() {
                Vec::new()
            } else {
                edit::selection(&s, &ops)
            };
            data.view = Some((stamp, Arc::new(s)));
        }
        data.view.as_ref().map(|v| v.1.clone())
    }

    /// Si se está leyendo la región del perfil.
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
        // a cuatro anchuras de distancia está fuera del relleno de tres
        assert!(!slab_holds(&held, &bx.shifted(40.0)));
    }

    #[test]
    fn a_thinned_region_is_read_again_when_the_box_gets_small() {
        let huge = [0.0, 0.0, 30_000.0, 30_000.0];
        let small = [10_000.0, 10_000.0, 10_600.0, 10_300.0];
        let wanted = [9_997.0, 9_997.0, 10_603.0, 10_303.0];
        // adelgazada en el tope, y la caja es una astilla de ella: se vuelve a leer
        assert_eq!(region_state(Some((huge, 56)), small, wanted), (true, true));
        // todos los puntos conservados (stride 1): nada que ganar
        assert_eq!(region_state(Some((huge, 1)), small, wanted), (true, false));
        // una región ya del tamaño de la caja se queda, adelgazada o no
        assert_eq!(
            region_state(Some((wanted, 3)), small, wanted),
            (true, false)
        );
        // nada en memoria, o una caja fuera de ella
        assert_eq!(region_state(None, small, wanted), (false, false));
        assert_eq!(region_state(Some((small, 1)), huge, huge), (false, false));
    }
}
