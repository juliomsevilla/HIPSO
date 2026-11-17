//! Zoom sobre nubes COPC: los puntos que guarda el visor son una muestra uniforme de toda la nube,
//! demasiado pocos para una ventana pequeña, así que la ventana se vuelve a leer de los archivos
//! con todo el detalle en segundo plano y se dibuja en su lugar mientras cubre la vista.

use super::*;

/// Si la ventana `a` contiene la ventana `b`.
pub fn contains(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3]
}

/// Aproximadamente cuántos puntos puede contener el detalle de una ventana.
const BUDGET: usize = 3_000_000;
/// La parte del área de la nube que puede tener una ventana para que merezca la pena volver a
/// leerla.
const MAX_SHARE: f64 = 0.5;

impl App {
    /// Halla o hace el COPC de cada nube en segundo plano, para poder leer ventanas de él con todo
    /// el detalle; solo cuando el visor guarda una muestra (`sampled`) y las nubes son LAS/LAZ.
    pub fn start_cache(&mut self, ctx: &egui::Context, sampled: bool) {
        let las = |p: &PathBuf| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("laz") || e.eq_ignore_ascii_case("las"))
        };
        if !sampled || self.clouds.is_empty() || !self.clouds.iter().all(las) {
            return;
        }
        let clouds = self.clouds.clone();
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                let copies = clouds
                    .iter()
                    .map(|p| hipso_core::copc_cache::ensure(p))
                    .collect::<Result<Vec<_>, _>>();
                if let Ok(c) = &copies {
                    hipso_core::copc_cache::prune(hipso_core::copc_cache::limit(), c);
                }
                copies
            })
            .map_err(|_| "error interno".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.cache_job = Some(rx);
        self.log("Preparando el detalle al acercar (copia COPC en la caché)…".to_string());
    }

    /// Mantiene el detalle de la vista `vw` (x mínima, y mínima, x máxima, y máxima): recoge lo que
    /// ha hecho la lectura en segundo plano y empieza otra cuando la vista ha salido de lo que se
    /// guarda.
    pub fn update_detail(&mut self, ctx: &egui::Context, vw: [f64; 4], interactive: bool) {
        let px = 1.0 / self.plan.scale.max(1e-9);
        self.step_detail(ctx, vw, px, interactive);
        self.mark_detail_selection();
    }

    /// Marca en los puntos de la ventana de detalle los que la selección actual alcanza: la
    /// ventana es otra lectura de la nube y no hereda la selección del almacén base.
    fn mark_detail_selection(&mut self) {
        let (rev, erev) = (self.ed.sel_rev, self.ed.edit_rev);
        for d in [self.detail.as_mut(), self.detail3.as_mut()].into_iter().flatten() {
            let stale = d.edit_rev != erev;
            if d.sel_rev == rev && !stale {
                continue;
            }
            let st = Arc::make_mut(&mut d.store);
            if stale {
                // la ventana se leyó del archivo sin editar: se le aplican las ediciones de la
                // sesión para que muestre lo mismo que el almacén base
                d.edit_rev = erev;
                st.class.clone_from(&d.orig_class);
                st.deleted.clear();
                if let Some(ses) = &self.ed.session {
                    ses.apply_to(st);
                }
                self.detail_id += 1;
                d.id = self.detail_id; // renueva la textura
            }
            d.sel_rev = rev;
            st.selected = hipso_core::edit::selection(st, &self.ed.ops);
        }
    }

    /// Lo mismo para la vista 3D: `vw` es el terreno alrededor del objetivo que abarca la cámara,
    /// `px` el tamaño de un píxel allí.
    pub fn update_detail3(
        &mut self,
        ctx: &egui::Context,
        vw: [f64; 4],
        px: f64,
        interactive: bool,
    ) {
        std::mem::swap(&mut self.detail, &mut self.detail3);
        std::mem::swap(&mut self.detail_job, &mut self.detail3_job);
        self.step_detail(ctx, vw, px, interactive);
        std::mem::swap(&mut self.detail, &mut self.detail3);
        std::mem::swap(&mut self.detail_job, &mut self.detail3_job);
        self.mark_detail_selection();
    }

    fn step_detail(&mut self, ctx: &egui::Context, vw: [f64; 4], px: f64, interactive: bool) {
        if let Some((rx, win, px)) = &self.detail_job
            && let Ok(r) = rx.try_recv()
        {
            let (win, px) = (*win, *px);
            self.detail_job = None;
            match r {
                Ok(Some((s, level_spacing))) if !s.is_empty() => {
                    let spacing = Style::fit(&s).spacing;
                    self.detail_id += 1;
                    self.detail = Some(Detail {
                        window: win,
                        px,
                        orig_class: s.class.clone(),
                        edit_rev: u64::MAX,
                        store: Arc::new(s),
                        spacing,
                        level_spacing,
                        id: self.detail_id,
                        sel_rev: u64::MAX,
                    });
                }
                Ok(_) => {}
                Err(e) => self.log(format!("No se pudo leer el detalle de la ventana: {e}")),
            }
        }
        if let Some(rx) = &self.cache_job
            && let Ok(r) = rx.try_recv()
        {
            self.cache_job = None;
            match r {
                Ok(v) => {
                    self.detail_src = v;
                    self.log("Detalle al acercar listo.".to_string());
                    ctx.request_repaint();
                }
                Err(e) => self.log(format!("No se pudo preparar el detalle al acercar: {e}")),
            }
        }
        let Some(base) = &self.store else { return };
        let b = base.bounds;
        let share = (vw[2] - vw[0]) * (vw[3] - vw[1]) / ((b[3] - b[0]) * (b[4] - b[1])).max(1e-9);
        if self.detail_src.is_empty()
            || base.stride <= 1
            || self.loading.is_some()
            || share > MAX_SHARE
        {
            // nada que ganar: la nube está entera, o la vista abarca la mayor parte de ella
            self.detail = None;
            return;
        }
        // la ventana se vuelve a leer cuando la vista se ha acercado mucho
        let covered = self
            .detail
            .as_ref()
            .is_some_and(|d| contains(d.window, vw) && d.px <= px * 1.8);
        if covered || interactive || self.detail_job.is_some() {
            return;
        }
        // una cuarta parte más por cada lado, para que los movimientos pequeños queden dentro
        let (mx, my) = ((vw[2] - vw[0]) * 0.25, (vw[3] - vw[1]) * 0.25);
        let window = [vw[0] - mx, vw[1] - my, vw[2] + mx, vw[3] + my];
        let (paths, origin, files) = (self.detail_src.clone(), base.origin, base.files.clone());
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                hipso_core::copc_detail::load_detail(&paths, window, BUDGET, px, origin, &files)
            })
            .map_err(|_| "error interno".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.detail_job = Some((rx, window, px));
    }
}
