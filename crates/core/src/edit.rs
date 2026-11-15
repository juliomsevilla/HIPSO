//! Edición de los puntos de una nube como elementos en un SIG: se seleccionan con regiones
//! (rectángulo, polígono, círculo, lazo; en planta o en un perfil) y con atributos, se combinan las
//! selecciones (nueva, añadir, restar, intersecar, invertir) y luego se eliminan o se les cambia la
//! clase, con deshacer y rehacer.
//!
//! Las ediciones son reglas, no puntos reescritos. El visor las aplica de inmediato a los puntos
//! diezmados que muestra; al guardar se aplican las mismas reglas a todos los puntos del archivo en
//! una pasada en flujo, de modo que el resultado es exacto sea cual sea el diezmado y la memoria se
//! mantiene plana.

use std::path::Path;

use rayon::prelude::*;

use crate::classes::ClassSet;
use crate::las::{self, Reader, Writer};
use crate::vector::Polygon;
use crate::view::{Orbit, PointStore, ProfileBox, Projector, return_bit};
use crate::{Error, Result};

/// Dónde mira una operación de selección.
#[derive(Clone, Debug)]
pub enum Region {
    /// La nube entera (selección solo por atributos).
    All,
    /// Un polígono en coordenadas del mapa, sea cual sea la elevación.
    Plan(Polygon),
    /// Un polígono en un perfil: x es la distancia a lo largo de la caja, y la elevación; solo
    /// cuentan los puntos dentro de la caja.
    Profile { bx: ProfileBox, poly: Polygon },
    /// Una forma dibujada en la vista 3D, sea lo que sea lo que haya delante o detrás de los
    /// puntos.
    View(ViewCut),
    /// Un trazo de pincel en coordenadas del mapa, sea cual sea la elevación.
    PlanBrush(Brush),
    /// Un trazo de pincel en un perfil (distancia a lo largo de la caja, elevación); solo cuentan
    /// los puntos dentro de la caja.
    ProfileBrush { bx: ProfileBox, brush: Brush },
    /// Todo lo que está por encima (o por debajo) de una línea dibujada en un perfil, a lo largo de
    /// la línea; solo cuentan los puntos dentro de la caja.
    Side {
        bx: ProfileBox,
        a: [f64; 2],
        b: [f64; 2],
        above: bool,
    },
}

/// Un trazo de pincel: los puntos a menos de `half` de un camino (pincel redondo) o dentro de un
/// cuadrado de lado `2 half` arrastrado a lo largo de él (pincel cuadrado). El camino se estampa
/// cada medio radio de pincel y las estampas se guardan en una rejilla, de modo que preguntar si un
/// punto está pintado no depende de lo largo que sea el trazo, y el visor y el archivo guardado dan
/// la misma respuesta.
#[derive(Clone, Debug)]
pub struct Brush {
    pub half: f64,
    pub square: bool,
    stamps: Vec<[f64; 2]>,
    cells: std::collections::HashMap<(i64, i64), Vec<u32>>,
}

impl Brush {
    pub fn new(path: &[[f64; 2]], half: f64, square: bool) -> Self {
        let half = half.max(f64::MIN_POSITIVE);
        let step = half * 0.5;
        let mut stamps: Vec<[f64; 2]> = Vec::new();
        if let Some(first) = path.first() {
            stamps.push(*first);
        }
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let n = (len / step).ceil().max(1.0) as usize;
            for k in 1..=n {
                let t = k as f64 / n as f64;
                stamps.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        let mut cells: std::collections::HashMap<(i64, i64), Vec<u32>> = Default::default();
        for (i, st) in stamps.iter().enumerate() {
            cells
                .entry(Self::cell_of(st[0], st[1], half))
                .or_default()
                .push(i as u32);
        }
        Self {
            half,
            square,
            stamps,
            cells,
        }
    }

    fn cell_of(x: f64, y: f64, half: f64) -> (i64, i64) {
        (
            (x / (2.0 * half)).floor() as i64,
            (y / (2.0 * half)).floor() as i64,
        )
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        let (cx, cy) = Self::cell_of(x, y, self.half);
        let h2 = self.half * self.half;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = self.cells.get(&(cx + dx, cy + dy)) else {
                    continue;
                };
                for &i in list {
                    let st = self.stamps[i as usize];
                    let (ex, ey) = ((x - st[0]).abs(), (y - st[1]).abs());
                    let inside = if self.square {
                        ex <= self.half && ey <= self.half
                    } else {
                        ex * ex + ey * ey <= h2
                    };
                    if inside {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Si `(u, z)` está por encima (o por debajo) del segmento de `a` a `b`, dentro del alcance del
/// segmento a lo largo de `u`.
pub fn beside_line(a: [f64; 2], b: [f64; 2], above: bool, u: f64, z: f64) -> bool {
    let (u0, u1) = (a[0].min(b[0]), a[0].max(b[0]));
    if u < u0 || u > u1 {
        return false;
    }
    let zl = if (b[0] - a[0]).abs() < 1e-12 {
        // una línea vertical: el alcance es un punto, y también lo es lo que hay encima de ella
        a[1].max(b[1])
    } else {
        a[1] + (b[1] - a[1]) * (u - a[0]) / (b[0] - a[0])
    };
    if above { z > zl } else { z < zl }
}

/// Un polígono dibujado en píxeles en una vista 3D, con la cámara que lo mostró y, opcionalmente,
/// el rango de distancias al plano de la cámara que cuentan (una rebanada a través de la escena).
/// Es geométrico, así que elige los mismos puntos en el visor diezmado y en el archivo entero.
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

/// Los atributos de un punto sobre los que se puede hacer una selección (los histogramas del
/// editor).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrKind {
    Z,
    Intensity,
    ReturnNumber,
    NumReturns,
    Class,
    /// Pasada (ID de fuente del punto).
    Source,
    /// Ángulo de barrido en grados.
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

    /// Atributos de números enteros: un histograma recibe una barra por valor.
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

/// Un punto pasa cuando el atributo está en alguno de los intervalos `[lo, hi)` (las barras
/// elegidas en un histograma o las colas de sus valores atípicos).
#[derive(Clone, Debug, PartialEq)]
pub struct RangeSel {
    pub kind: AttrKind,
    pub bins: Vec<(f64, f64)>,
}

/// Lo que debe ser un punto para ser elegido, además de estar en la región.
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub classes: ClassSet,
    /// Bits: se dibujan los retornos 1 primero, 2 intermedio, 4 último, 8 único.
    pub returns: u8,
    pub z: Option<(f64, f64)>,
    pub intensity: Option<(u16, u16)>,
    /// Debe cumplirse cada una de estas (las selecciones de histograma).
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
    /// Invierte la selección; la región y los atributos se ignoran.
    Invert,
}

/// Un paso de la construcción de una selección.
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

/// Una selección y lo que se hace con ella.
#[derive(Clone, Debug)]
pub struct Edit {
    pub sel: Vec<SelOp>,
    pub action: Action,
    pub label: String,
    /// Un paso que el usuario ha desactivado se queda en el historial pero no hace nada: los demás
    /// se aplican como si nunca se hubiera hecho.
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

/// Los valores de un punto que mira una selección.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub class: u8,
    pub intensity: u16,
    /// Número de retorno en el cuarteto bajo y número de retornos en el alto.
    pub ret: u8,
    pub src: u16,
    /// Ángulo de barrido en grados.
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
            Region::PlanBrush(brush) => brush.contains(p.x, p.y),
            Region::ProfileBrush { bx, brush } => {
                let (u, t) = bx.local(p.x, p.y);
                u >= 0.0 && u <= bx.length() && t.abs() <= bx.width / 2.0 && brush.contains(u, p.z)
            }
            Region::Side { bx, a, b, above } => {
                let (u, t) = bx.local(p.x, p.y);
                u >= 0.0
                    && u <= bx.length()
                    && t.abs() <= bx.width / 2.0
                    && beside_line(*a, *b, *above, u, p.z)
            }
        }
    }
}

/// Si la selección construida por `ops` incluye el punto.
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

/// El valor de un atributo del punto `i` del almacén.
pub fn value_of(s: &PointStore, i: usize, kind: AttrKind) -> f64 {
    kind.value(&sample_of(s, i))
}

/// Si una selección mira campos que se leen del registro más allá del punto.
fn needs_extra(ops: &[SelOp]) -> bool {
    ops.iter().any(|o| !o.attr.ranges.is_empty())
}

/// Los valores de un registro decodificado; la fuente y el ángulo de barrido solo cuando una regla
/// los necesita.
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

/// La selección que hace `ops` sobre los puntos del almacén (los puntos eliminados nunca se
/// seleccionan).
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

/// Un rectángulo como polígono.
pub fn rectangle(a: [f64; 2], b: [f64; 2]) -> Polygon {
    Polygon::new(vec![vec![
        [a[0], a[1]],
        [b[0], a[1]],
        [b[0], b[1]],
        [a[0], b[1]],
    ]])
}

/// La sesión de edición sobre los puntos del visor: las clases originales, las ediciones hechas y
/// las deshechas.
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

    /// Aplica una edición al almacén; cuántos puntos mostrados tocó.
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

    /// Desactiva o vuelve a activar la edición `k`, dejando las demás como están; si existe. Una
    /// edición posterior que selecciona por clase ve los puntos tal como están ahora.
    pub fn toggle(&mut self, s: &mut PointStore, k: usize) -> bool {
        let Some(e) = self.edits.get_mut(k) else {
            return false;
        };
        e.enabled = !e.enabled;
        self.replay(s);
        true
    }

    /// Aplica las ediciones que cuentan a otro almacén de las mismas nubes (una región leída con
    /// toda la densidad), de modo que muestre lo que muestra el almacén de la sesión.
    pub fn apply_to(&self, s: &mut PointStore) {
        for e in &self.edits {
            Self::run(s, e);
        }
    }

    /// Las ediciones que cuentan al guardar.
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

    /// De vuelta al archivo tal como se leyó.
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

/// Aplica las ediciones a todos los puntos de `input` y escribe el resto en `output` (LAS, o LAZ
/// según la extensión), en una pasada en flujo con lotes procesados en todos los núcleos.
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
    // las clases por encima de 31 necesitan un byte de clase completo: los archivos antiguos se
    // escriben como LAS 1.4
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
        // por registro: eliminado, o su clase nueva cuando se cambió
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

/// Escribe en `output` solo los puntos de `input` que elige la selección `ops` (un LAS o LAZ nuevo,
/// en una pasada en flujo); cuántos. Las herramientas de proceso se ejecutan sobre esto cuando hay
/// una selección activa.
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
        // dentro del rectángulo y de clase 5
        let both = [near.clone(), op(Mode::Intersect, Region::All, veg.clone())];
        assert_eq!(selection(&s, &both), [false, true, false, false, false]);
        // toda la vegetación menos el rectángulo
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
        // la primera se apaga: la vegetación vuelve, la eliminación se queda
        assert!(ses.toggle(&mut s, 0));
        assert_eq!(s.class, [2, 5, 5, 1, 2]);
        assert_eq!(s.deleted, [false, false, false, true, false]);
        assert_eq!(ses.active().len(), 1);
        assert!(ses.toggle(&mut s, 0));
        assert_eq!(s.class, [2, 7, 7, 1, 2]);
        assert!(!ses.toggle(&mut s, 9));
    }

    #[test]
    fn a_brush_stroke_picks_what_lies_along_its_path() {
        let s = store();
        // un trazo a lo largo de y = 1 de x = 0 a x = 10, 1,5 m a cada lado: todos menos el punto
        // en (8, 8)
        let path = [[0.0, 1.0], [5.0, 1.0], [10.0, 1.0]];
        let round = Region::PlanBrush(Brush::new(&path, 1.5, false));
        let sel = [op(Mode::New, round, Attr::default())];
        assert_eq!(selection(&s, &sel), [true, true, false, true, true]);
        // un punto justo dentro y justo fuera del extremo redondo de un trazo
        let b = Brush::new(&[[0.0, 0.0]], 1.0, false);
        assert!(b.contains(0.99, 0.0) && !b.contains(1.01, 0.0));
        // el pincel cuadrado llega a sus esquinas, el redondo no
        let sq = Brush::new(&[[0.0, 0.0]], 1.0, true);
        assert!(sq.contains(0.95, 0.95) && !b.contains(0.95, 0.95));
        // un trazo inclinado no tiene huecos a lo largo
        let slant = Brush::new(&[[0.0, 0.0], [10.0, 10.0]], 0.5, false);
        assert!((0..=100).all(|k| slant.contains(k as f64 / 10.0, k as f64 / 10.0)));
    }

    #[test]
    fn above_and_below_a_line_in_a_profile() {
        let s = store();
        let bx = ProfileBox {
            a: [0.0, 1.0],
            b: [10.0, 1.0],
            width: 2.0,
        };
        // una línea a 5 m desde el principio hasta el final de la caja
        let side = |above| {
            vec![op(
                Mode::New,
                Region::Side {
                    bx,
                    a: [0.0, 5.0],
                    b: [10.0, 5.0],
                    above,
                },
                Attr::default(),
            )]
        };
        // la caja toma los puntos a menos de 1 m del eje (y = 1): todos menos el de y = 8
        let up = selection(&s, &side(true));
        let down = selection(&s, &side(false));
        // un punto está como mucho en uno de los dos, y la caja (y a menos de 1 del eje) los limita
        assert!(up.iter().zip(&down).all(|(a, b)| !(*a && *b)));
        assert_eq!(up, [false, true, false, true, false]);
        assert_eq!(down, [true, false, false, false, true]);
        // solo a lo largo de la línea: una línea que abarca la mitad de la caja deja en paz a la
        // otra mitad
        assert!(beside_line([0.0, 5.0], [5.0, 5.0], true, 3.0, 6.0));
        assert!(!beside_line([0.0, 5.0], [5.0, 5.0], true, 7.0, 6.0));
        // una línea inclinada: por encima es relativo a la línea a esa distancia
        assert!(beside_line([0.0, 0.0], [10.0, 10.0], true, 2.0, 3.0));
        assert!(!beside_line([0.0, 0.0], [10.0, 10.0], true, 8.0, 3.0));
    }

    #[test]
    fn profile_region_uses_distance_and_height() {
        let s = store();
        let bx = ProfileBox {
            a: [0.0, 1.0],
            b: [10.0, 1.0],
            width: 2.0,
        };
        // por encima de 10 m en el perfil: solo el punto en z = 20
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
        // el filtro de clase ve ahora la clase 7: la siguiente selección de clase 5 no encuentra
        // nada
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

    /// Un archivo LAS 1.2 de formato de punto 1 con (ángulo de barrido en grados, identificador de
    /// fuente del punto) por punto.
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
        // ángulos de barrido 0, 5, -4, 35, -40, 10 de las pasadas 1, 1, 2, 1, 3, 2
        let bytes = las_with(&[(0, 1), (5, 1), (-4, 2), (35, 1), (-40, 3), (10, 2)]);
        let dir = std::env::temp_dir();
        let id = std::process::id();
        let (input, out) = (
            dir.join(format!("hipso_edit_in_{id}.las")),
            dir.join(format!("hipso_edit_out_{id}.las")),
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
        // los ángulos atípicos: más allá de ±20°
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
        // solo la pasada 3
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
        // z = 1, 9, 3, 20, 1: la barra [0, 2) contiene dos puntos
        assert_eq!(
            selection(&s, &by(AttrKind::Z, vec![(0.0, 2.0)])),
            [true, false, false, false, true]
        );
        // las colas de los valores atípicos: por debajo de 0,5 o por encima de 10
        let tails = vec![(f64::NEG_INFINITY, 0.5), (10.0, f64::INFINITY)];
        assert_eq!(
            selection(&s, &by(AttrKind::Z, tails)),
            [false, false, false, true, false]
        );
        // clases 2, 5, 5, 1, 2: barra [5, 6)
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
        // un rectángulo de unos pocos píxeles alrededor de donde cae el primer punto solo lo elige
        // a él
        let (px, py) = cam.project(s.xyz(0), 100, 100).unwrap_or_default();
        let small = rectangle([px - 1.0, py - 1.0], [px + 1.0, py + 1.0]);
        assert_eq!(
            selection(&s, &cut(small, None)),
            [true, false, false, false, false]
        );
        // mirando al norte la profundidad es 20 + y: una rebanada conserva los puntos lejanos (y 2
        // y 8)
        let all = rectangle([-1e4, -1e4], [1e4, 1e4]);
        assert_eq!(
            selection(&s, &cut(all.clone(), Some((21.5, 30.0)))),
            [false, true, true, false, false]
        );
        assert_eq!(selection(&s, &cut(all, None)), [true; 5]);
    }
}
