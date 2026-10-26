//! A base map under the layers: web map tiles (OpenStreetMap by default) fetched in the
//! background, kept on disk for a week, and drawn moved into the work's coordinate system.
//!
//! Tiles come from the providers' servers, so each has terms to keep: a program name in the
//! request, at most two downloads at a time, tiles reused from the cache rather than asked for
//! again, and the credit shown on the map. Nothing is fetched while the base map is off.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect};
use rustlidar_core::reproject::{self, Transformer};
use rustlidar_core::tilemap::{self, HALF_WORLD};
use rustlidar_core::view::Plan;

/// A source of tiles.
pub struct Provider {
    pub name: &'static str,
    /// The address of a tile, with `{z}`, `{x}` and `{y}`.
    pub url: &'static str,
    /// The credit its terms ask to show.
    pub credit: &'static str,
    pub max_zoom: u8,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        name: "OpenStreetMap (mundial)",
        url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
        credit: "© colaboradores de OpenStreetMap",
        max_zoom: 19,
    },
    Provider {
        name: "Ortofoto PNOA, IGN (España)",
        url: "https://www.ign.es/wmts/pnoa-ma?request=getTile&layer=OI.OrthoimageCoverage&TileMatrixSet=GoogleMapsCompatible&TileMatrix={z}&TileCol={x}&TileRow={y}&format=image/jpeg",
        credit: "Ortofotos PNOA y Sentinel-2 © IGN, CC BY 4.0 scne.es",
        max_zoom: 19,
    },
    Provider {
        name: "Mapa base, IGN (España)",
        url: "https://www.ign.es/wmts/ign-base?request=getTile&layer=IGNBaseTodo&TileMatrixSet=GoogleMapsCompatible&TileMatrix={z}&TileCol={x}&TileRow={y}&format=image/jpeg",
        credit: "Mapa base © IGN, CC BY 4.0 scne.es",
        max_zoom: 19,
    },
];

/// The index of the provider whose address the user types.
pub const CUSTOM: usize = PROVIDERS.len();

const USER_AGENT: &str = concat!(
    "EasyLiDAR/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/juliomsevilla/EasyLiDAR)"
);

/// How long a tile on disk is used without asking the server again.
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
/// Downloads at the same time: the most the OpenStreetMap tile policy allows.
const WORKERS: usize = 2;
/// Tiles drawn at most: a view that would need more is drawn at a coarser zoom.
const MAX_TILES: usize = 60;
/// Textures kept.
const KEEP_TEXTURES: usize = 320;

type Key = (u64, u8, u32, u32);

struct Job {
    key: Key,
    url: String,
    cache: Option<PathBuf>,
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<Job>,
    /// Tiles a worker has taken and not finished.
    started: HashSet<Key>,
}

struct Queue {
    state: Mutex<Shared>,
    wake: Condvar,
}

struct Done {
    key: Key,
    image: Result<([usize; 2], Vec<u8>), String>,
}

enum Slot {
    Ready { tex: egui::TextureHandle, used: u64 },
    Failed(Instant),
}

pub struct Basemap {
    pub on: bool,
    pub provider: usize,
    pub custom: String,
    tiles: HashMap<Key, Slot>,
    queue: Arc<Queue>,
    done: Receiver<Done>,
    /// The system the transformers below were made for, and Web Mercator to it.
    mapped: Option<(u16, Arc<Transformer>, Arc<Transformer>)>,
    frame: u64,
}

impl Basemap {
    pub fn new(ctx: &egui::Context) -> Self {
        let queue = Arc::new(Queue {
            state: Mutex::new(Shared::default()),
            wake: Condvar::new(),
        });
        let (tx, done) = channel();
        for _ in 0..WORKERS {
            let (q, tx, ctx) = (queue.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || worker(q, tx, ctx));
        }
        Self {
            on: false,
            provider: 0,
            custom: String::new(),
            tiles: HashMap::new(),
            queue,
            done,
            mapped: None,
            frame: 0,
        }
    }

    fn template(&self) -> Option<(&str, &str, u8)> {
        match PROVIDERS.get(self.provider) {
            Some(p) => Some((p.url, p.credit, p.max_zoom)),
            None => {
                let t = self.custom.trim();
                (t.contains("{z}") && t.contains("{x}") && t.contains("{y}")).then_some((
                    t,
                    "mapa base de un servidor propio",
                    19,
                ))
            }
        }
    }

    pub fn credit(&self) -> Option<&str> {
        self.template().map(|t| t.1)
    }

    fn id(template: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        template.hash(&mut h);
        h.finish()
    }

    /// Takes the tiles the workers finished: a texture each.
    fn collect(&mut self, ctx: &egui::Context) {
        for _ in 0..12 {
            let Ok(d) = self.done.try_recv() else { return };
            let name = format!("tile-{}-{}-{}", d.key.1, d.key.2, d.key.3);
            let slot = match d.image {
                Ok((size, rgba)) => Slot::Ready {
                    tex: ctx.load_texture(
                        name,
                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                        egui::TextureOptions::LINEAR,
                    ),
                    used: self.frame,
                },
                Err(_) => Slot::Failed(Instant::now()),
            };
            self.tiles.insert(d.key, slot);
        }
        ctx.request_repaint();
    }

    /// Draws the tiles under the view; asks for the ones that are missing. `project` is the
    /// EPSG code of the work's system. Returns what to tell the user when nothing can be drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        plan: &Plan,
        rect: Rect,
        w: usize,
        h: usize,
        project: Option<u16>,
    ) -> Option<String> {
        if !self.on {
            return None;
        }
        self.frame += 1;
        let Some(project) = project else {
            return Some(
                "Mapa base: la nube no declara su sistema de coordenadas (asígnalo con «Asignar proyección»)"
                    .into(),
            );
        };
        let Some((template, _, max_zoom)) = self.template().map(|(a, b, c)| (a.to_string(), b, c))
        else {
            return Some("Mapa base: escribe una dirección con {z}, {x} e {y}".into());
        };
        if self.mapped.as_ref().is_none_or(|m| m.0 != project) {
            let pair = reproject::transformer(3857, project)
                .and_then(|a| reproject::transformer(project, 3857).map(|b| (a, b)));
            match pair {
                Ok((a, b)) => self.mapped = Some((project, Arc::new(a), Arc::new(b))),
                Err(e) => return Some(format!("Mapa base: no se puede pasar a {project}: {e}")),
            }
        }
        let (_, from_map, to_map) = self.mapped.clone()?;
        self.collect(ctx);
        // what the view shows, in Web Mercator metres
        let (lo, hi) = (
            plan.to_map(0.0, h as f64, w, h),
            plan.to_map(w as f64, 0.0, w, h),
        );
        let Ok(b) = to_map.transform_bounds((lo.0, lo.1, hi.0, hi.1)) else {
            return Some("Mapa base: esta zona queda fuera del mapa mundial".into());
        };
        let bounds = [
            b.0.max(-HALF_WORLD),
            b.1.max(-HALF_WORLD),
            b.2.min(HALF_WORLD),
            b.3.min(HALF_WORLD),
        ];
        if !(bounds[2] > bounds[0] && bounds[3] > bounds[1]) {
            return Some("Mapa base: esta zona queda fuera del mapa mundial".into());
        }
        let mut z = tilemap::zoom_for((bounds[2] - bounds[0]) / w.max(1) as f64, max_zoom);
        let mut tiles = tilemap::tiles_in(bounds, z);
        while tiles.len() > MAX_TILES && z > 0 {
            z -= 1;
            tiles = tilemap::tiles_in(bounds, z);
        }
        let pid = Self::id(&template);
        let tint = Color32::WHITE;
        let centre = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0];
        let mut missing: Vec<(f64, Key)> = Vec::new();
        for (z, x, y) in &tiles {
            let key = (pid, *z, *x, *y);
            let e = tilemap::extent(*z, *x, *y);
            match self.tiles.get_mut(&key) {
                Some(Slot::Ready { tex, used }) => {
                    *used = self.frame;
                    // the tile curves as the two systems differ: more cells where it is big
                    crate::warp::paint_warped(
                        painter,
                        tex.id(),
                        e,
                        &from_map,
                        plan,
                        rect,
                        w,
                        h,
                        tint,
                        10,
                    );
                }
                Some(Slot::Failed(at)) if at.elapsed() < Duration::from_secs(30) => {}
                _ => {
                    let d = (e[0] + e[2]) / 2.0 - centre[0];
                    let f = (e[1] + e[3]) / 2.0 - centre[1];
                    missing.push((d * d + f * f, key));
                }
            }
        }
        if !missing.is_empty() {
            missing.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Ok(mut s) = self.queue.state.lock() {
                s.queue.clear();
                for (_, key) in missing {
                    if !s.started.contains(&key) {
                        s.queue.push_back(Job {
                            key,
                            url: tilemap::url(&template, key.1, key.2, key.3),
                            cache: cache_file(pid, key),
                        });
                    }
                }
            }
            self.queue.wake.notify_all();
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // keep memory bounded: drop the textures least recently on screen
        if self.tiles.len() > KEEP_TEXTURES {
            let mut used: Vec<(u64, Key)> = self
                .tiles
                .iter()
                .filter_map(|(k, s)| match s {
                    Slot::Ready { used, .. } => Some((*used, *k)),
                    Slot::Failed(_) => None,
                })
                .collect();
            used.sort();
            for (_, k) in used.into_iter().take(self.tiles.len() - KEEP_TEXTURES + 40) {
                self.tiles.remove(&k);
            }
        }
        // the credit the provider asks for
        if let Some(credit) = self.credit() {
            let font = FontId::proportional(11.0);
            let galley = painter.layout_no_wrap(credit.to_string(), font.clone(), Color32::BLACK);
            let r = Rect::from_min_size(
                Pos2::new(
                    rect.right() - galley.size().x - 10.0,
                    rect.bottom() - galley.size().y - 6.0,
                ),
                galley.size() + egui::vec2(8.0, 3.0),
            );
            painter.rect_filled(r, 2.0, Color32::from_white_alpha(200));
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                credit,
                font,
                Color32::BLACK,
            );
        }
        None
    }
}

/// Where a tile is kept: the user's cache folder of the system.
fn cache_file(provider: u64, key: Key) -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?
    };
    Some(
        base.join("EasyLiDAR")
            .join("tiles")
            .join(format!("{provider:016x}"))
            .join(key.1.to_string())
            .join(key.2.to_string())
            .join(format!("{}.tile", key.3)),
    )
}

fn worker(queue: Arc<Queue>, tx: Sender<Done>, ctx: egui::Context) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    loop {
        let job = {
            let Ok(mut s) = queue.state.lock() else {
                return;
            };
            loop {
                if let Some(j) = s.queue.pop_front() {
                    s.started.insert(j.key);
                    break j;
                }
                s = match queue.wake.wait(s) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let image = fetch(&agent, &job).and_then(|bytes| decode(&bytes));
        if let Ok(mut s) = queue.state.lock() {
            s.started.remove(&job.key);
        }
        if tx
            .send(Done {
                key: job.key,
                image,
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();
    }
}

/// The bytes of a tile: from the disk if they are fresh, else from the server (and kept).
fn fetch(agent: &ureq::Agent, job: &Job) -> Result<Vec<u8>, String> {
    let age = |p: &PathBuf| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
    };
    if let Some(p) = &job.cache
        && age(p).is_some_and(|a| a < KEEP)
        && let Ok(b) = std::fs::read(p)
    {
        return Ok(b);
    }
    let got = agent
        .get(&job.url)
        .call()
        .map_err(|e| e.to_string())
        .and_then(|mut r| r.body_mut().read_to_vec().map_err(|e| e.to_string()));
    match got {
        Ok(bytes) => {
            if let Some(p) = &job.cache
                && let Some(dir) = p.parent()
                && std::fs::create_dir_all(dir).is_ok()
            {
                let _ = std::fs::write(p, &bytes);
            }
            Ok(bytes)
        }
        // without a connection an old tile is better than none
        Err(e) => job
            .cache
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .ok_or(e),
    }
}

/// A PNG or JPEG tile as RGBA.
pub fn decode(bytes: &[u8]) -> Result<([usize; 2], Vec<u8>), String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let size = reader.output_buffer_size().ok_or("PNG demasiado grande")?;
        let mut buf = vec![0u8; size];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let (w, h) = (info.width as usize, info.height as usize);
        let px = &buf[..info.buffer_size()];
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => px.to_vec(),
            png::ColorType::Rgb => px
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[1], c[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => px
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|c| [c[0], c[0], c[0], c[1]])
                .collect(),
            png::ColorType::Grayscale => px.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
            png::ColorType::Indexed => return Err("PNG indexado sin expandir".into()),
        };
        if rgba.len() != w * h * 4 {
            return Err("PNG con un tamaño que no cuadra".into());
        }
        return Ok(([w, h], rgba));
    }
    let mut dec = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(bytes));
    let px = dec.decode().map_err(|e| format!("{e:?}"))?;
    let info = dec.info().ok_or("JPEG sin cabecera")?;
    let (w, h) = (usize::from(info.width), usize::from(info.height));
    let rgba: Vec<u8> = match px.len() {
        n if n == w * h * 3 => px
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        n if n == w * h => px.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        _ => return Err("JPEG con un tamaño que no cuadra".into()),
    };
    Ok(([w, h], rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_tile_decodes_to_rgba() -> Result<(), String> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30])
                .map_err(|e| e.to_string())?;
        }
        let (size, rgba) = decode(&out)?;
        assert_eq!(size, [2, 2]);
        assert_eq!(&rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(&rgba[12..], &[10, 20, 30, 255]);
        assert!(decode(b"not an image").is_err());
        Ok(())
    }

    #[test]
    fn urls_and_cache_paths_are_per_provider() {
        let a = Basemap::id("https://a/{z}/{x}/{y}");
        let b = Basemap::id("https://b/{z}/{x}/{y}");
        assert_ne!(a, b);
        if let (Some(p), Some(q)) = (cache_file(a, (a, 3, 4, 5)), cache_file(b, (b, 3, 4, 5))) {
            assert_ne!(p, q);
            assert!(p.ends_with("3/4/5.tile"));
        }
    }

    /// Needs internet: the real servers answer with images this program can read.
    #[test]
    #[ignore]
    fn real_tiles_download_and_decode() -> Result<(), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        for p in PROVIDERS {
            let job = Job {
                key: (0, 12, 2005, 1544),
                url: tilemap::url(p.url, 12, 2005, 1544),
                cache: None,
            };
            let ([w, h], rgba) = decode(&fetch(&agent, &job)?)?;
            assert_eq!((w, h), (256, 256), "{}", p.name);
            // a map is not one flat colour
            let first = &rgba[..4];
            assert!(
                rgba.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|c| c.as_slice() != first),
                "{}",
                p.name
            );
        }
        Ok(())
    }
}
