//! «Detalle por vista» del 3D: en vez de leer una ventana del mapa a un nivel único, lee de la copia
//! COPC los nodos que ve la cámara (los cercanos con todo su detalle, los lejanos en niveles
//! gruesos) hasta un presupuesto de puntos. Es opcional y pensado para nubes terrestres densas.

use super::*;
use hipso_core::copc_view::ViewLoader;
use std::sync::Mutex;

/// Puntos que puede leer la vista.
pub const VIEW_BUDGET_M: u64 = 12;

/// El presupuesto, en puntos: el de la casilla (millones) o, para pruebas, `HIPSO_VIEW_BUDGET`.
fn budget(millones: u64) -> u64 {
    std::env::var("HIPSO_VIEW_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(millones.max(1) * 1_000_000)
}
/// Un nodo se refina mientras se ve más grande que esto, en píxeles.
const MIN_NODE_PX: f64 = 50.0;
/// Ventana que lo abarca todo: el almacén base no se dibuja mientras manda el detalle por vista.
pub const EVERYTHING: [f64; 4] = [-1e18, -1e18, 1e18, 1e18];

/// El estado del detalle por vista.
#[derive(Default)]
pub struct ViewDetail {
    pub on: bool,
    /// Presupuesto de puntos, en millones (0: el de por defecto).
    pub budget_m: u64,
    src: Vec<PathBuf>,
    loader: Option<Arc<Mutex<ViewLoader>>>,
    opening: Option<Receiver<Result<Option<ViewLoader>, String>>>,
    job: Option<(Receiver<Result<PointStore, String>>, u64)>,
    pub data: Option<(Arc<PointStore>, u64, u64)>,
    id: u64,
}

impl App {
    /// Mantiene el detalle por vista de la cámara actual. Devuelve `true` si hay un almacén listo
    /// para dibujar.
    pub fn update_view_detail(&mut self, ctx: &egui::Context, w: usize, h: usize, interactive: bool) {
        if std::env::var_os("HIPSO_VIEW_DEBUG").is_some() {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(60) {
                eprintln!(
                    "[vista] estado: on={} src={} detail_src={} loader={} abriendo={} trabajo={} interactivo={}",
                    self.vdet.on,
                    self.vdet.src.len(),
                    self.detail_src.len(),
                    self.vdet.loader.is_some(),
                    self.vdet.opening.is_some(),
                    self.vdet.job.is_some(),
                    interactive
                );
            }
        }
        self.poll_cache(ctx);
        let vd = &mut self.vdet;
        if !vd.on {
            return;
        }
        if vd.src != self.detail_src {
            // otras nubes: se empieza de nuevo
            *vd = ViewDetail {
                on: true,
                src: self.detail_src.clone(),
                ..ViewDetail::default()
            };
        }
        let Some(base) = &self.store else { return };
        if let Some(rx) = &vd.opening
            && let Ok(r) = rx.try_recv()
        {
            vd.opening = None;
            match r {
                Ok(Some(l)) => {
                    vd.loader = Some(Arc::new(Mutex::new(l)));
                    ctx.request_repaint();
                }
                Ok(None) => {
                    vd.on = false;
                    self.log("El detalle por vista necesita nubes COPC.".to_string());
                    return;
                }
                Err(e) => {
                    vd.on = false;
                    self.log(format!("No se pudo preparar el detalle por vista: {e}"));
                    return;
                }
            }
        }
        if vd.loader.is_none() && vd.opening.is_none() {
            if self.detail_src.is_empty() || base.stride <= 1 {
                return;
            }
            let (paths, origin, files) = (self.detail_src.clone(), base.origin, base.files.clone());
            let (tx, rx) = channel();
            let c = ctx.clone();
            std::thread::spawn(move || {
                let r = std::panic::catch_unwind(|| ViewLoader::open(&paths, origin, &files))
                    .map_err(|_| "error interno".to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()));
                let _ = tx.send(r);
                c.request_repaint();
            });
            vd.opening = Some(rx);
            self.log("Preparando el detalle por vista (índice de la nube)…".to_string());
            return;
        }
        if let Some((rx, key)) = &vd.job
            && let Ok(r) = rx.try_recv()
        {
            let key = *key;
            vd.job = None;
            match r {
                Ok(s) => {
                    if std::env::var_os("HIPSO_VIEW_DEBUG").is_some() {
                        let mut por_nivel = [0usize; 24];
                        for l in &s.lvl {
                            por_nivel[usize::from(*l).min(23)] += 1;
                        }
                        eprintln!("[vista] {} puntos por nivel {:?}", s.len(), por_nivel);
                    }
                    vd.id += 1;
                    vd.data = Some((Arc::new(s), vd.id, key));
                    ctx.request_repaint();
                }
                Err(e) => {
                    vd.on = false;
                    self.log(format!("No se pudo leer el detalle por vista: {e}"));
                    return;
                }
            }
        }
        let o = self.orbit;
        let key = hash_of((
            w,
            h,
            o.yaw.to_bits(),
            o.pitch.to_bits(),
            o.dist.to_bits(),
            o.exaggeration.to_bits(),
            o.target.map(f64::to_bits),
            vd.budget_m,
        ));
        let current = vd.data.as_ref().is_some_and(|d| d.2 == key);
        if current || vd.job.is_some() || interactive {
            return;
        }
        let Some(loader) = vd.loader.clone() else { return };
        let millones = if vd.budget_m == 0 { VIEW_BUDGET_M } else { vd.budget_m };
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| match loader.lock() {
                Ok(mut l) => {
                    let t0 = Instant::now();
                    let r = l
                        .load(&o, w, h, budget(millones), MIN_NODE_PX)
                        .map_err(|e| e.to_string());
                    if std::env::var_os("HIPSO_VIEW_DEBUG").is_some() {
                        eprintln!("[vista] lectura en {:.2} s", t0.elapsed().as_secs_f64());
                    }
                    r
                }
                Err(_) => Err("error interno".to_string()),
            })
            .unwrap_or_else(|_| Err("error interno".to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        vd.job = Some((rx, key));
    }
}
