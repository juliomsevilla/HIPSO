//! Zooming into COPC clouds: the points the viewer keeps are an even sample of the whole cloud,
//! too few for a small window, so the window is read again from the files at full detail in the
//! background, and drawn instead while it covers the view.

use super::*;

/// Whether the window `a` holds the window `b`.
pub fn contains(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3]
}

/// About how many points the detail of a window may hold.
const BUDGET: usize = 3_000_000;
/// The share of the cloud's area a window may have to be worth reading again.
const MAX_SHARE: f64 = 0.5;

impl App {
    /// Keeps the detail of the view `vw` (min x, min y, max x, max y): collects what the
    /// background read has made, and starts another when the view has left what is held.
    pub fn update_detail(&mut self, ctx: &egui::Context, vw: [f64; 4], interactive: bool) {
        if let Some((rx, win)) = &self.detail_job
            && let Ok(r) = rx.try_recv()
        {
            let win = *win;
            self.detail_job = None;
            match r {
                Ok(Some(s)) if !s.is_empty() => {
                    let spacing = Style::fit(&s).spacing;
                    self.detail_id += 1;
                    self.detail = Some(Detail {
                        window: win,
                        store: Arc::new(s),
                        spacing,
                        id: self.detail_id,
                    });
                }
                Ok(_) => {}
                Err(e) => self.log(format!("No se pudo leer el detalle de la ventana: {e}")),
            }
        }
        let Some(base) = &self.store else { return };
        let b = base.bounds;
        let share = (vw[2] - vw[0]) * (vw[3] - vw[1]) / ((b[3] - b[0]) * (b[4] - b[1])).max(1e-9);
        if !self.copc_only || base.stride <= 1 || self.loading.is_some() || share > MAX_SHARE {
            // nothing to gain: the cloud is whole, or the view takes in most of it
            self.detail = None;
            return;
        }
        let covered = self.detail.as_ref().is_some_and(|d| contains(d.window, vw));
        if covered || interactive || self.detail_job.is_some() {
            return;
        }
        // a quarter more on each side, so that small moves stay inside
        let (mx, my) = ((vw[2] - vw[0]) * 0.25, (vw[3] - vw[1]) * 0.25);
        let window = [vw[0] - mx, vw[1] - my, vw[2] + mx, vw[3] + my];
        let (paths, origin, files) = (self.clouds.clone(), base.origin, base.files.clone());
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| {
                rustlidar_core::copc_detail::load_detail(&paths, window, BUDGET, origin, &files)
            })
            .map_err(|_| "error interno".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.detail_job = Some((rx, window));
    }
}
