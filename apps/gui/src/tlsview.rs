//! «Ver nubes TLS»: abre en una ventana propia (`hipso-tls`) una nube terrestre densa con todo el
//! detalle al acercarse. La primera vez la nube se convierte, en segundo plano, a un octree que la
//! ventana lee por trozos; la conversión queda en la caché (junto a las copias COPC) y las
//! siguientes veces la ventana se abre al instante.

use super::*;

/// Convierte `src` si hace falta y abre la ventana. Devuelve el mensaje final para el registro.
fn abrir(src: &Path, oscuro: bool, abierta: &AtomicBool, avance: &AtomicU32) -> Result<String, String> {
    use hipso_core::tls_octree as oct;
    let visor = oct::junto_al_programa("hipso-tls")
        .ok_or("Falta hipso-tls junto al programa: la ventana «Ver nubes TLS» no está instalada.")?;
    let destino = oct::convertir(src, &|p| avance.store((p * 100.0) as u32, Ordering::Relaxed))?;
    let titulo = hipso_core::paths::stem(src);
    abierta.store(true, Ordering::Relaxed);
    let mut hijo = std::process::Command::new(visor)
        .arg(&destino)
        .args(["--tema", if oscuro { "oscuro" } else { "claro" }])
        .args(["--titulo", &titulo])
        .spawn()
        .map_err(|e| format!("No se pudo abrir la ventana: {e}"))?;
    let _ = hijo.wait();
    Ok(format!("Cerrada la ventana «Ver nubes TLS» de {titulo}."))
}

impl App {
    /// El botón «Ver nubes TLS»: prepara y abre la primera nube cargada.
    pub fn ver_nubes_tls(&mut self, ctx: &egui::Context) {
        if self.tls_job.is_some() {
            self.log("«Ver nubes TLS» ya se está preparando: espera a que termine (se ve abajo, en la barra de estado).".to_string());
            return;
        }
        let Some(src) = self.clouds.first().cloned() else {
            self.log("«Ver nubes TLS» necesita una nube cargada.".to_string());
            return;
        };
        let oscuro = self.dark;
        let (tx, rx) = channel();
        let c = ctx.clone();
        let abierta = Arc::new(AtomicBool::new(false));
        let ab = abierta.clone();
        let avance = Arc::new(AtomicU32::new(0));
        let av = avance.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| abrir(&src, oscuro, &ab, &av))
                .unwrap_or_else(|_| Err("error interno".to_string()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.tls_job = Some((rx, Instant::now(), abierta, avance));
        self.log("Preparando la nube para «Ver nubes TLS» (la primera vez tarda unos minutos)…".to_string());
    }

    /// Recoge el final del trabajo de «Ver nubes TLS».
    pub fn paso_tls(&mut self) {
        if let Some((rx, _, _, _)) = &self.tls_job
            && let Ok(r) = rx.try_recv()
        {
            self.tls_job = None;
            match r {
                Ok(m) | Err(m) => self.log(m),
            }
        }
    }
}
