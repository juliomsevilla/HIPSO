//! Cómo mueve una transformación los puntos entre dos datums, como lo hace PROJ (y por tanto GDAL):
//! a partir de las transformaciones que GDAL lista para el par (reordenadas según el área de
//! interés de los sistemas) o, cuando la base de datos de PROJ no tiene ninguna entre los dos
//! datums, encadenando dos de las que registra (concatenaciones de varias entre ellas) a través de
//! sistemas y datums intermedios, tal como `proj_create_operations` de PROJ las encuentra y ordena;
//! después se elige para cada punto la más precisa cuya área lo contiene o, si no, la primera que
//! no necesita rejilla (`proj_trans` de PROJ); una lista de una sola se usa para todos los puntos.

use super::datum::{self, Op, Point, Step};
use super::def::Def;
use super::epsg;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// Una transformación y el sentido en que se ejecuta.
#[derive(Debug, Clone)]
pub(crate) struct Leg {
    pub op: Arc<Op>,
    pub reverse: bool,
}

/// Dos transformaciones a través de un datum intermedio, con el área, la precisión y los
/// indicadores por los que PROJ ordena la cadena.
#[derive(Debug)]
struct Chain {
    legs: [Leg; 2],
    rank: Op,
}

#[derive(Debug)]
enum Kind {
    /// Mismo datum: sin desplazamiento.
    Same,
    /// La lista de transformaciones de GDAL para el par: la mejor para un punto o, si no, la
    /// primera.
    List(Vec<Leg>),
    /// Cadenas a través de datums intermedios, en el orden de PROJ, y después la aproximación de
    /// PROJ (sin desplazamiento, que contiene todos los puntos) cuando la lista: la mejor para un
    /// punto o, si no, la aproximación o, si no, la primera cadena que no necesita rejilla.
    Chains {
        chains: Vec<Chain>,
        ballpark: Option<Op>,
    },
    /// Sistemas sin sistema geográfico EPSG: a través de WGS 84 con las transformaciones de cada
    /// lado.
    Hub,
}

/// Áreas en coordenadas de origen, por transformación (dos cajas a ambos lados del antimeridiano).
type Boxes = Vec<Vec<[f64; 4]>>;

/// La parte de datum de una transformación entre dos sistemas.
#[derive(Debug)]
pub(crate) struct Route {
    kind: Kind,
    /// Las áreas de las transformaciones en coordenadas de origen, calculadas la primera vez que
    /// hacen falta: por tramo de `List` o por `Chain`.
    boxes: OnceLock<Boxes>,
    /// Lo mismo, para las transformaciones del origen y del destino de `Hub`.
    hub_boxes: OnceLock<(Boxes, Boxes)>,
    /// Las transformaciones del destino a WGS 84, para `Hub`.
    dst_ops: Vec<Arc<Op>>,
    /// Los dos datums son realizaciones oficiales uno del otro (no existe ni hace falta ningún
    /// desplazamiento).
    coincident: bool,
}

/// Datums que las autoridades definen como coincidentes: ETRS89 y REGCAN95 (Canarias).
fn coincident(a: &Def, b: &Def) -> bool {
    const ETRS89: u32 = 6258;
    const REGCAN95: u32 = 1035;
    matches!(
        (a.datum_code(), b.datum_code()),
        (Some(ETRS89), Some(REGCAN95)) | (Some(REGCAN95), Some(ETRS89))
    )
}

fn legs(ops: &[Arc<Op>], reverse: bool) -> Vec<Leg> {
    ops.iter()
        .map(|op| Leg {
            op: op.clone(),
            reverse,
        })
        .collect()
}

/// Si dos áreas se cortan, como `GeographicBoundingBox::intersects` de PROJ.
fn intersects(a: &[f64; 4], b: &[f64; 4]) -> bool {
    let [w, s, e, n] = *a;
    let [ow, os, oe, on] = *b;
    if n < os || s > on {
        return false;
    }
    if w == -180.0 && e == 180.0 && ow > oe {
        return true;
    }
    if ow == -180.0 && oe == 180.0 && w > e {
        return true;
    }
    if w <= e {
        if ow <= oe {
            return w.max(ow) < e.min(oe);
        }
        if ow > 180.0 || oe < -180.0 {
            return false;
        }
        return intersects(a, &[ow, os, 180.0, on]) || intersects(a, &[-180.0, os, oe, on]);
    }
    if ow <= oe {
        return intersects(b, a);
    }
    true
}

/// Si el área `a` contiene el área `b`, como `GeographicBoundingBox::contains` de PROJ.
fn contains(a: &[f64; 4], b: &[f64; 4]) -> bool {
    let [w, s, e, n] = *a;
    let [ow, os, oe, on] = *b;
    if !(s <= os && n >= on) {
        return false;
    }
    if w == -180.0 && e == 180.0 {
        return ow != oe;
    }
    if ow == -180.0 && oe == 180.0 {
        return false;
    }
    if w < e {
        return ow < oe && w <= ow && e >= oe;
    }
    if ow < oe {
        ow >= w || oe <= e
    } else {
        w <= ow && e >= oe
    }
}

/// La intersección de dos áreas, como `GeographicBoundingBox::intersection` de PROJ: `None` cuando
/// no se solapan; la parte mayor cuando tiene dos a ambos lados del antimeridiano.
fn intersection(a: &[f64; 4], b: &[f64; 4]) -> Option<[f64; 4]> {
    let [w, s, e, n] = *a;
    let [ow, os, oe, on] = *b;
    if n < os || s > on {
        return None;
    }
    let (s2, n2) = (s.max(os), n.min(on));
    if w == -180.0 && e == 180.0 && ow > oe {
        return Some([ow, s2, oe, n2]);
    }
    if ow == -180.0 && oe == 180.0 && w > e {
        return Some([w, s2, e, n2]);
    }
    if w <= e {
        if ow <= oe {
            let (rw, re) = (w.max(ow), e.min(oe));
            return (rw < re).then_some([rw, s2, re, n2]);
        }
        if ow > 180.0 || oe < -180.0 {
            return None;
        }
        return match (
            intersection(a, &[ow, os, 180.0, on]),
            intersection(a, &[-180.0, os, oe, on]),
        ) {
            (Some(x), Some(y)) => Some(if x[2] - x[0] > y[2] - y[0] { x } else { y }),
            (x, y) => x.or(y),
        };
    }
    if ow <= oe {
        return intersection(b, a);
    }
    Some([w.max(ow), s2, e.min(oe), n2])
}

/// La medida del tamaño de un área en PROJ.
fn pseudo_area(a: &[f64; 4]) -> f64 {
    let [w, s, mut e, n] = *a;
    if w > e {
        e += 360.0;
    }
    (e - w) * (n.to_radians().sin() - s.to_radians().sin())
}

/// El área que deben cortar las transformaciones entre dos sistemas: la menor de sus áreas de uso,
/// como `SourceTargetCRSExtentUse::SMALLEST` de PROJ.
fn area_of_interest(src: &Def, dst: &Def) -> Option<[f64; 4]> {
    smaller(src.area(), dst.area())
}

/// La menor de dos áreas, como las mide PROJ.
fn smaller(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<[f64; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if pseudo_area(&a) < pseudo_area(&b) {
            a
        } else {
            b
        }),
        (a, b) => a.or(b),
    }
}

/// Si PROJ considera una transformación entre dos sistemas: su área debe cortar las áreas de ambos
/// sistemas (como consulta PROJ su base de datos) y el área de interés.
fn relevant(op: &Op, src: &Def, dst: &Def, aoi: Option<&[f64; 4]>) -> bool {
    op.ballpark
        || [src.area().as_ref(), dst.area().as_ref(), aoi]
            .into_iter()
            .flatten()
            .all(|a| intersects(&op.bounds, a))
}

impl Route {
    /// La ruta de `src` a `dst`.
    pub fn new(src: &Def, dst: &Def) -> Self {
        let aoi = area_of_interest(src, dst);
        let keep = |op: &Op| relevant(op, src, dst, aoi.as_ref());
        // Entre sistemas proyectados, PROJ concatena las transformaciones con las proyecciones, lo
        // que da una concatenación cuya precisión y área registra su base de datos para sus pasos
        // (NAD83 a NAD83(NSRS2007) (NADCON5, Alaska) entre British Columbia Albers y UTM).
        let concatenated = !src.is_geographic() || !dst.is_geographic();
        let list = |ops: &[Arc<Op>], reverse: bool| -> Kind {
            Kind::List(
                legs(ops, reverse)
                    .into_iter()
                    .filter(|l| keep(&l.op))
                    .map(|l| {
                        let flat = concatenated
                            && (l.op.chain_accuracy != l.op.accuracy
                                || l.op.chain_bounds != l.op.bounds);
                        if flat {
                            Leg {
                                op: Arc::new(l.op.as_concatenated()),
                                reverse: l.reverse,
                            }
                        } else {
                            l
                        }
                    })
                    .collect(),
            )
        };
        let kind = if src.same_datum(dst) {
            Kind::Same
        } else if dst.is_wgs84() {
            list(src.ops(), false)
        } else if src.is_wgs84() {
            list(dst.ops(), true)
        } else {
            match (src.geog(), dst.geog()) {
                (Some(a), Some(b)) => match epsg::pair(a, b) {
                    // PROJ busca transformaciones a través de otros datums cuando su base de datos
                    // no tiene ninguna utilizable entre los dos para las áreas de los sistemas
                    // (NAD83(CSRS) a NAD83 entre sistemas árticos, fuera del área marítima de su
                    // única transformación, o con las rejillas ausentes).
                    Some((ops, reverse)) if ops.iter().any(|o| !o.ballpark && keep(o)) => {
                        let mut kind = list(&ops, reverse);
                        // El orden de GDAL es el de los dos sistemas geográficos: entre sistemas de
                        // otra área de interés, PROJ ordena las transformaciones para la suya.
                        let area = |code: u32| epsg::get(code).and_then(|e| e.area);
                        if let Kind::List(l) = &mut kind
                            && aoi != smaller(area(a), area(b))
                        {
                            // Ejecutada a la inversa, una transformación tiene el nombre de la
                            // inversa.
                            let key = |leg: &Leg| {
                                let name = step_names(&leg.op, leg.reverse).join(" + ");
                                SortKey::new(
                                    &leg.op,
                                    name,
                                    pipeline_steps(&[(&leg.op, leg.reverse)]),
                                    aoi.as_ref(),
                                )
                            };
                            let mut keyed: Vec<(SortKey, Leg)> =
                                l.drain(..).map(|leg| (key(&leg), leg)).collect();
                            keyed.sort_by(|x, y| x.0.cmp(&y.0));
                            l.extend(keyed.into_iter().map(|(_, leg)| leg));
                        }
                        kind
                    }
                    _ => chains(a, b, src, dst, aoi.as_ref()),
                },
                _ => Kind::Hub,
            }
        };
        Self {
            kind,
            boxes: OnceLock::new(),
            hub_boxes: OnceLock::new(),
            dst_ops: dst.ops().to_vec(),
            coincident: coincident(src, dst),
        }
    }

    /// La precisión (metros) de la transformación que [`Route::apply`] elige para un punto, como la
    /// suma PROJ a la precisión de una transformación vertical con la que la encadena: 0 entre
    /// sistemas de un datum, negativa cuando se desconoce (una aproximación o ninguna
    /// transformación para el punto).
    pub fn accuracy_at(&self, src: &Def, x: f64, y: f64, p: Point) -> f64 {
        let (dlon, dlat) = src
            .source_degrees(x, y)
            .unwrap_or((p.lon.to_degrees(), p.lat.to_degrees()));
        let wrapped = if (-180.0..=180.0).contains(&dlon) {
            dlon
        } else {
            (dlon + 180.0).rem_euclid(360.0) - 180.0
        };
        let inside = |op: &Op, boxes: Option<&Vec<[f64; 4]>>| match boxes {
            Some(b) if !b.is_empty() => b
                .iter()
                .any(|b| x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]),
            _ => op.contains(dlon, dlat) || op.contains(wrapped, dlat),
        };
        let first_without_grids = |ops: &[&Op]| {
            ops.iter()
                .find(|o| !o.needs_grids())
                .map_or(-1.0, |o| o.accuracy)
        };
        match &self.kind {
            Kind::Same => 0.0,
            Kind::List(list) if list.is_empty() => -1.0,
            Kind::List(list) => {
                let boxes = self
                    .boxes
                    .get_or_init(|| list.iter().map(|l| src.areas(&l.op)).collect());
                let ops: Vec<&Op> = list.iter().map(|l| l.op.as_ref()).collect();
                datum::pick_index(&ops, &[], |op, i| inside(op, boxes.get(i)))
                    .map_or_else(|| first_without_grids(&ops), |i| ops[i].accuracy)
            }
            Kind::Chains { chains, .. } if chains.is_empty() => -1.0,
            Kind::Chains { chains, ballpark } => {
                if chains.len() == 1 && ballpark.is_none() {
                    return chains[0].rank.accuracy;
                }
                let boxes = self
                    .boxes
                    .get_or_init(|| chains.iter().map(|c| src.areas(&c.rank)).collect());
                let ranks: Vec<&Op> = chains
                    .iter()
                    .map(|c| &c.rank)
                    .chain(ballpark.as_ref())
                    .collect();
                match datum::pick_index(&ranks, &[], |op, i| {
                    op.ballpark || inside(op, boxes.get(i))
                }) {
                    Some(i) => ranks[i].accuracy,
                    None => first_without_grids(&ranks),
                }
            }
            Kind::Hub => -1.0,
        }
    }

    /// Mueve un punto geodésico (radianes) del datum de origen al del destino. `(x, y)` son sus
    /// coordenadas de origen, que eligen la transformación para un origen proyectado como en PROJ.
    /// Las alturas pasan por la transformación cuando `three_d` (entre sistemas 3D). `None` cuando
    /// ninguna transformación tiene éxito.
    pub fn apply(&self, src: &Def, x: f64, y: f64, p: Point, three_d: bool) -> Option<Point> {
        if !self.coincident {
            return self.apply_raw(src, x, y, p, three_d);
        }
        // Los datums que coinciden no necesitan desplazamiento: el «sin desplazamiento» del motor
        // es exacto, no una aproximación.
        let before = super::policy::peek();
        let r = self.apply_raw(src, x, y, p, three_d);
        super::policy::restore_ballpark(before);
        r
    }

    fn apply_raw(&self, src: &Def, x: f64, y: f64, p: Point, three_d: bool) -> Option<Point> {
        // PROJ compara las coordenadas de origen con las áreas: para un origen geográfico, su
        // longitud y latitud tal como se dan, en grados.
        let (dlon, dlat) = src
            .source_degrees(x, y)
            .unwrap_or((p.lon.to_degrees(), p.lat.to_degrees()));
        // Las longitudes fuera de [-180, 180] se prueban también envueltas en ese intervalo, como
        // hace PROJ.
        let wrapped = if (-180.0..=180.0).contains(&dlon) {
            dlon
        } else {
            (dlon + 180.0).rem_euclid(360.0) - 180.0
        };
        let inside = |op: &Op, boxes: Option<&Vec<[f64; 4]>>| match boxes {
            Some(b) if !b.is_empty() => b
                .iter()
                .any(|b| x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]),
            _ => op.contains(dlon, dlat) || op.contains(wrapped, dlat),
        };
        match &self.kind {
            Kind::Same => Some(p),
            // Ninguna transformación (un sistema de datum desconocido, definido solo por su
            // elipsoide): sin desplazamiento, la transformación aproximada de PROJ.
            Kind::List(list) if list.is_empty() => {
                super::policy::note(super::policy::Approximation::NoDatumShift);
                Some(p)
            }
            Kind::List(list) => {
                let boxes = self
                    .boxes
                    .get_or_init(|| list.iter().map(|l| src.areas(&l.op)).collect());
                let ops: Vec<&Op> = list.iter().map(|l| l.op.as_ref()).collect();
                datum::transform_with(
                    &ops,
                    |op, i| inside(op, boxes.get(i)),
                    |i| datum::run_op(&list[i].op, p, list[i].reverse, three_d),
                )
            }
            // Solo la aproximación de PROJ: sin desplazamiento.
            Kind::Chains { chains, .. } if chains.is_empty() => {
                super::policy::note(super::policy::Approximation::NoDatumShift);
                Some(p)
            }
            Kind::Chains { chains, ballpark } => {
                // Las alturas pasan por las dos transformaciones, como en la tubería de PROJ para
                // la cadena (la conversión a coordenadas geodésicas y de vuelta entre ellas se
                // anula), y se restauran después de ellas cuando la cadena las conserva.
                let run = |c: &Chain| {
                    let [l1, l2] = &c.legs;
                    let r = datum::run(&l1.op.steps, p, l1.reverse, true)
                        .and_then(|q| datum::run(&l2.op.steps, q, l2.reverse, true))?;
                    Some(if three_d || !c.rank.keeps_height {
                        r
                    } else {
                        Point { h: p.h, ..r }
                    })
                };
                // Se usa una sola operación para todos los puntos, sea cual sea su área, como hace
                // PROJ.
                if chains.len() == 1 && ballpark.is_none() {
                    return run(&chains[0]);
                }
                let boxes = self
                    .boxes
                    .get_or_init(|| chains.iter().map(|c| src.areas(&c.rank)).collect());
                let ranks: Vec<&Op> = chains
                    .iter()
                    .map(|c| &c.rank)
                    .chain(ballpark.as_ref())
                    .collect();
                let mut excluded = Vec::new();
                for _ in 0..3 {
                    let Some(i) = datum::pick_index(&ranks, &excluded, |op, i| {
                        op.ballpark || inside(op, boxes.get(i))
                    }) else {
                        break;
                    };
                    // La aproximación (más precisa que las cadenas entre sistemas que solo difieren
                    // por sus meridianos origen): sin desplazamiento.
                    let Some(chain) = chains.get(i) else {
                        super::policy::note(super::policy::Approximation::NoDatumShift);
                        return Some(p);
                    };
                    if let Some(r) = run(chain) {
                        return Some(r);
                    }
                    super::policy::note(super::policy::Approximation::NextBestOperation);
                    excluded.push(i);
                }
                if ballpark.is_some() {
                    // La aproximación contiene todos los puntos: sin desplazamiento.
                    super::policy::note(super::policy::Approximation::NoDatumShift);
                    return Some(p);
                }
                // Ninguna transformación para el punto: la primera que no necesita rejilla, como
                // `proj_trans` de PROJ.
                let first = chains.iter().find(|c| !c.rank.needs_grids())?;
                run(first)
            }
            Kind::Hub => {
                let (sb, db) = self.hub_boxes.get_or_init(|| {
                    let areas = |ops: &[Arc<Op>]| ops.iter().map(|o| src.areas(o)).collect();
                    (areas(src_ops(src)), areas(&self.dst_ops))
                });
                // Solo transformaciones simples, como las encadena PROJ.
                let single = |op: &Op| !op.ballpark && !op.concatenated;
                let s = datum::pick_in(src_ops(src), |op, i| single(op) && inside(op, sb.get(i)));
                let d = datum::pick_in(&self.dst_ops, |op, i| single(op) && inside(op, db.get(i)));
                match (s, d) {
                    (Some(s), Some(d)) => {
                        // Las alturas pasan por las dos, como en la tubería de PROJ (véanse las
                        // cadenas).
                        let r = datum::run(&s.steps, p, false, true)
                            .and_then(|q| datum::run(&d.steps, q, true, true))?;
                        Some(if three_d || !(s.keeps_height && d.keeps_height) {
                            r
                        } else {
                            Point { h: p.h, ..r }
                        })
                    }
                    // Ninguna transformación (o un datum desconocido): sin desplazamiento, como la
                    // aproximación de PROJ.
                    _ => {
                        super::policy::note(super::policy::Approximation::NoDatumShift);
                        Some(p)
                    }
                }
            }
        }
    }
}

fn src_ops(src: &Def) -> &[Arc<Op>] {
    src.ops()
}

/// De qué se compone una cadena, antes de que PROJ ordene las cadenas: sus dos transformaciones
/// registradas en los sentidos en que se ejecutan y los nombres de las conversiones con las que
/// PROJ las enmarca.
struct Found {
    legs: [(usize, bool); 2],
    /// Conversiones antes de la primera transformación, entre ellas y después de la segunda.
    conversions: [Option<String>; 3],
}

impl Found {
    /// La cadena ejecutada a la inversa.
    fn reversed(self) -> Self {
        let [(i1, r1), (i2, r2)] = self.legs;
        // La inversa de una conversión, salvo un cambio de orden de ejes, que es su propia inversa.
        let back = |c: Option<String>| {
            c.map(|n| {
                if n.starts_with("axis order change") {
                    n
                } else {
                    inverse_name(&n)
                }
            })
        };
        let [c0, c1, c2] = self.conversions;
        Found {
            legs: [(i2, !r2), (i1, !r1)],
            conversions: [back(c2), back(c1), back(c0)],
        }
    }
}

/// El nombre que da PROJ a la operación entre dos sistemas geodésicos de un datum, tal como lo
/// construye su `buildOpName` (con el tipo de cada sistema cuando sus nombres son iguales): una
/// conversión a coordenadas geocéntricas o desde ellas, un cambio de orden de ejes entre sistemas
/// de un tipo o, si no, un desplazamiento nulo (que PROJ deja fuera de las cadenas de su pivote de
/// datum).
fn conversion_name(from: &epsg::Geodetic, to: &epsg::Geodetic) -> String {
    use epsg::GeodeticKind::*;
    let qualifier = |g: &epsg::Geodetic| match g.kind {
        Geographic2D => " (geog2D)",
        Geographic3D => " (geog3D)",
        Geocentric => " (geocentric)",
    };
    match (from.kind, to.kind) {
        (Geographic2D, Geographic2D) => return "axis order change (2D)".to_string(),
        (Geographic3D, Geographic3D) => {
            return "axis order change (geographic3D horizontal)".to_string();
        }
        _ => {}
    }
    let (q1, q2) = if from.name == to.name {
        (qualifier(from), qualifier(to))
    } else {
        ("", "")
    };
    let kind = if from.kind == Geocentric || to.kind == Geocentric {
        "Conversion"
    } else {
        NULL_OFFSET
    };
    format!("{kind} from {}{q1} to {}{q2}", from.name, to.name)
}

/// El nombre de las transformaciones nulas de PROJ entre sistemas geográficos de un datum.
const NULL_OFFSET: &str = "Null geographic offset";

/// Las cadenas de dos transformaciones del sistema geográfico `a` al `b` a través de datums
/// intermedios, como las lista `proj_create_operations` de PROJ para los sistemas `src` y `dst`
/// cuando su base de datos no tiene ninguna transformación entre los dos datums, ordenadas como las
/// ordena PROJ:
///
/// - primero las que pasan por un sistema intermedio al que la base de datos de PROJ registra
///   transformaciones desde sistemas de ambos datums (`createFromCRSCodesWithIntermediates`),
///   probando por orden de código los sistemas geográficos 2D, 3D y geocéntricos de los dos datums
///   (su pivote de datum) y conservando las del primer par de sistemas que tiene alguna;
/// - después, cuando eso no encuentra ninguna, o solo cadenas por las transformaciones nulas de
///   PROJ entre el conjunto de WGS 84 y sus realizaciones, o solo cadenas sobre menos de la mitad
///   del área común de los sistemas, o solo dependientes del tiempo entre sistemas estáticos, las
///   que pasan por un datum intermedio, sean cuales sean sus sistemas
///   (`createBetweenGeodeticCRSWithDatumBasedIntermediates`).
///
/// Se dejan fuera las transformaciones que la base de datos de PROJ sustituye por otra que también
/// encuentra, y aquellas cuyas rejillas faltan. PROJ termina la lista con su aproximación (sin
/// desplazamiento), salvo que una cadena que no necesita rejilla contenga el área de interés.
fn chains(a: u32, b: u32, src: &Def, dst: &Def, aoi: Option<&[f64; 4]>) -> Kind {
    let reg = epsg::registry();
    let (Some(ga), Some(gb)) = (reg.geodetic.get(&a), reg.geodetic.get(&b)) else {
        return Kind::Chains {
            chains: Vec::new(),
            ballpark: Some(ballpark_op(src, dst)),
        };
    };
    let extents: Vec<[f64; 4]> = [src.area(), dst.area()]
        .into_iter()
        .flatten()
        .filter(|e| *e != [-180.0, -90.0, 180.0, 90.0])
        .collect();
    // PROJ busca las transformaciones hacia un sistema proyectado desde él y las ejecuta a la
    // inversa: las que encuentra son las cadenas de `b` a `a` (las ordena una vez invertidas).
    let reverse = !dst.is_geographic();
    let (ga, gb) = if reverse { (gb, ga) } else { (ga, gb) };
    let mut found = crs_code_chains(reg, ga, gb, &extents);
    let ensemble_only = !found.is_empty()
        && found
            .iter()
            .all(|f| f.legs.iter().any(|&(i, _)| reg.ops[i].ensemble));
    let time_only = !found.is_empty()
        && !src.is_dynamic()
        && !dst.is_dynamic()
        && found
            .iter()
            .all(|f| f.legs.iter().any(|&(i, _)| reg.ops[i].time_dependent));
    let small = too_small(reg, &found, src.area(), dst.area());
    if found.is_empty() || ensemble_only || time_only || small {
        // Tras las cadenas sobre un área demasiado pequeña, solo las de otros nombres.
        let known: Vec<String> = if small {
            found.iter().map(|f| found_name(reg, f)).collect()
        } else {
            Vec::new()
        };
        for f in datum_chains(reg, ga, gb, &extents) {
            if !small || !known.contains(&found_name(reg, &f)) {
                found.push(f);
            }
        }
    }
    if reverse {
        found = found.into_iter().map(Found::reversed).collect();
    }
    // Las cadenas, cada una con aquello por lo que PROJ la ordena.
    let mut out: Vec<(SortKey, Chain)> = Vec::new();
    for f in found {
        let [(i1, r1), (i2, r2)] = f.legs;
        let (o1, o2) = (&reg.ops[i1].op, &reg.ops[i2].op);
        let Some(bounds) = intersection(&o1.chain_bounds, &o2.chain_bounds) else {
            continue;
        };
        if aoi.is_some_and(|a| !intersects(&bounds, a)) {
            continue;
        }
        // El área común tiene el nombre (y por tanto los indicadores) del tramo cuya área es.
        let named = if contains(&o1.chain_bounds, &o2.chain_bounds) {
            Some(o2)
        } else if contains(&o2.chain_bounds, &o1.chain_bounds) {
            Some(o1)
        } else {
            None
        };
        let (a1, a2) = (o1.chain_accuracy, o2.chain_accuracy);
        let accuracy = if a1 >= 0.0 && a2 >= 0.0 {
            a1 + a2
        } else {
            -1.0
        };
        let name = found_name(reg, &f);
        let rank = Op {
            bounds,
            accuracy,
            chain_accuracy: accuracy,
            chain_bounds: bounds,
            offshore: named.is_some_and(|o| o.offshore),
            // Una intersección de áreas que no es la de ninguno de los tramos no tiene nombre, que
            // PROJ toma por un área desconocida.
            unknown_area: named.is_none_or(|o| o.unknown_area),
            ballpark: false,
            concatenated: true,
            between_3d: o1.between_3d,
            geocentric: o1.geocentric || o2.geocentric,
            priority: o1.priority || o2.priority || datum::is_priority(&name),
            keeps_height: o1.keeps_height && o2.keeps_height,
            steps: o1.steps.iter().chain(&o2.steps).cloned().collect(),
            name,
        };
        let legs = [
            Leg {
                op: o1.clone(),
                reverse: r1,
            },
            Leg {
                op: o2.clone(),
                reverse: r2,
            },
        ];
        let key = SortKey::new(
            &rank,
            rank.name.clone(),
            pipeline_steps(&[(o1, r1), (o2, r2)]),
            aoi,
        );
        out.push((key, Chain { legs, rank }));
    }
    out.sort_by(|x, y| x.0.cmp(&y.0));
    // PROJ descarta su aproximación cuando una cadena que no necesita rejilla contiene el área de
    // interés, incluso una que luego descarta él mismo.
    let ballpark = !aoi.is_some_and(|a| {
        out.iter()
            .any(|(_, c)| !c.rank.needs_grids() && contains(&c.rank.bounds, a))
    });
    let chains = remove_uninteresting(out.into_iter().map(|(_, c)| c).collect());
    Kind::Chains {
        chains,
        ballpark: ballpark.then(|| ballpark_op(src, dst)),
    }
}

/// Las cadenas que conserva PROJ una vez ordenadas: sin una que no aporte nada sobre la anterior
/// conservada (de la misma área, igual o menos precisa, con más transformaciones;
/// `removeUninterestingOps` de PROJ), ni una que ejecute los mismos pasos sobre la misma área que
/// otra anterior (`removeDuplicateOps`).
fn remove_uninteresting(chains: Vec<Chain>) -> Vec<Chain> {
    // Las transformaciones de una cadena, como las cuenta PROJ: sus pasos salvo las conversiones.
    let count = |c: &Chain| {
        c.rank
            .name
            .split(" + ")
            .filter(|n| !n.starts_with("Conversion from ") && !n.starts_with("axis order change"))
            .count()
    };
    let mut kept: Vec<Chain> = Vec::new();
    for c in chains {
        if let Some(last) = kept.last() {
            let (accuracy, last_accuracy) = (c.rank.accuracy, last.rank.accuracy);
            if ((accuracy >= last_accuracy && last_accuracy >= 0.0)
                || (accuracy < 0.0 && last_accuracy >= 0.0))
                && c.rank.bounds == last.rank.bounds
                && count(&c) > count(last)
            {
                continue;
            }
        }
        kept.push(c);
    }
    let mut out: Vec<Chain> = Vec::new();
    for c in kept {
        let same = |o: &Chain| {
            o.rank.bounds == c.rank.bounds
                && o.legs
                    .iter()
                    .zip(&c.legs)
                    .all(|(x, y)| x.reverse == y.reverse && x.op.steps == y.op.steps)
        };
        if !out.iter().any(same) {
            out.push(c);
        }
    }
    out
}

/// El nombre que da PROJ a una cadena: los nombres de sus conversiones y transformaciones (los de
/// los pasos de una concatenación registrada), sin las conversiones a un sistema intermedio y de
/// vuelta.
fn found_name(reg: &epsg::Registry, f: &Found) -> String {
    let [(i1, r1), (i2, r2)] = f.legs;
    let mut names: Vec<String> = Vec::new();
    let mut push = |more: Vec<String>| {
        for n in more {
            if names.last().is_some_and(|last| inverse_name(last) == n) {
                names.pop();
            } else {
                names.push(n);
            }
        }
    };
    push(f.conversions[0].iter().cloned().collect());
    push(step_names(&reg.ops[i1].op, r1));
    push(f.conversions[1].iter().cloned().collect());
    push(step_names(&reg.ops[i2].op, r2));
    push(f.conversions[2].iter().cloned().collect());
    names.join(" + ")
}

/// Si toda cadena cubre como mucho la mitad del área común de los sistemas, como lo mide
/// `hasTooSmallAreas` de PROJ (para menos de 10 cadenas).
fn too_small(
    reg: &epsg::Registry,
    found: &[Found],
    e1: Option<[f64; 4]>,
    e2: Option<[f64; 4]>,
) -> bool {
    if found.is_empty() || found.len() >= 10 {
        return false;
    }
    let area = |b: Option<[f64; 4]>| b.map_or(0.0, |b| pseudo_area(&b));
    let target = match (e1, e2) {
        (Some(x), Some(y)) => area(extent_intersection(&x, &y)),
        (x, y) => area(x.or(y)),
    };
    if target <= 0.0 {
        return false;
    }
    !found.iter().any(|f| {
        let [(i1, _), (i2, _)] = f.legs;
        let Some(op) = intersection(&reg.ops[i1].op.chain_bounds, &reg.ops[i2].op.chain_bounds)
        else {
            return false;
        };
        let covered = match (e1, e2) {
            (Some(x), Some(y)) => {
                let (p, q) = (extent_intersection(&op, &x), extent_intersection(&op, &y));
                area(p) + area(q)
                    - p.zip(q)
                        .map_or(0.0, |(p, q)| area(extent_intersection(&p, &q)))
            }
            (x, y) => x
                .or(y)
                .map_or(pseudo_area(&op), |e| area(extent_intersection(&op, &e))),
        };
        covered > 0.5 * target
    })
}

/// La intersección de dos extensiones como la calcula `Extent::intersection` de PROJ: la una cuando
/// contiene a la otra o, si no, la intersección de sus áreas.
fn extent_intersection(a: &[f64; 4], b: &[f64; 4]) -> Option<[f64; 4]> {
    if contains(a, b) {
        return Some(*b);
    }
    if contains(b, a) {
        return Some(*a);
    }
    intersection(a, b)
}

/// Si PROJ rechaza un sistema intermedio entre dos sistemas: una realización de ITRF fuera de los
/// años de dos realizaciones de ETRF, NAD83 o WGS 84 entre dos realizaciones de NAD83(CSRS).
fn rejected_pivot(from: &str, to: &str, pivot: &str) -> bool {
    let year = |name: &str, prefix: &str| {
        let digits: String = name
            .strip_prefix(prefix)?
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let v: i32 = digits.parse().ok()?;
        (v > 0).then_some(if (80..=99).contains(&v) { v + 1900 } else { v })
    };
    if from.starts_with("ETRF")
        && to.starts_with("ETRF")
        && let (Some(s), Some(t), Some(p)) =
            (year(from, "ETRF"), year(to, "ETRF"), year(pivot, "ITRF"))
        && (p < s.min(t) || p > s.max(t))
    {
        return true;
    }
    from.starts_with("NAD83(CSRS)")
        && to.starts_with("NAD83(CSRS)")
        && matches!(pivot, "NAD83" | "WGS 84")
}

/// Si una concatenación registrada pasa por uno de dos sistemas (`useIrrelevantPivot` de PROJ).
fn irrelevant(reg: &epsg::Registry, i: usize, a: u32, b: u32) -> bool {
    reg.ops[i].via.iter().any(|&c| c == a || c == b)
}

/// Las transformaciones de una lista que otras de la lista sustituyen: aquellas de las que una
/// sustituta está en la lista cuando `any`, o si no aquellas de las que lo están todas (PROJ
/// consulta una transformación una vez por sustituta y descarta las filas cuya sustituta
/// encuentra).
fn superseded(
    reg: &epsg::Registry,
    ids: impl Iterator<Item = usize> + Clone,
    any: bool,
) -> HashSet<usize> {
    let keys: HashSet<&str> = ids.clone().map(|i| reg.ops[i].key.as_str()).collect();
    let found = |r: &String| keys.contains(r.as_str());
    ids.filter(|&i| {
        let r = &reg.ops[i].replaced_by;
        !r.is_empty()
            && if any {
                r.iter().any(found)
            } else {
                r.iter().all(found)
            }
    })
    .collect()
}

/// El pivote de datum de PROJ: las cadenas a través de un sistema intermedio entre los sistemas
/// geodésicos de los datums de `a` y `b`, por orden de código: primero las de sus nombres y de un
/// mismo tipo, luego las de otros nombres y después las de sus nombres y de otros tipos; las del
/// primer par de sistemas que tiene alguna (y también las de los pares siguientes mientras toda
/// cadena encontrada use una de las transformaciones propias de PROJ).
fn crs_code_chains(
    reg: &epsg::Registry,
    a: &epsg::Geodetic,
    b: &epsg::Geodetic,
    extents: &[[f64; 4]],
) -> Vec<Found> {
    let candidates = |g: &epsg::Geodetic| -> Vec<&epsg::Geodetic> {
        reg.by_datum
            .get(&g.datum)
            .into_iter()
            .flatten()
            .filter_map(|c| reg.geodetic.get(c))
            .collect()
    };
    let (ca, cb) = (candidates(a), candidates(b));
    let mut found: Vec<Found> = Vec::new();
    let proj_only = |found: &[Found]| {
        found.iter().all(|f| {
            f.legs
                .iter()
                .any(|&(i, _)| reg.ops[i].key.starts_with("PROJ:"))
        })
    };
    for same_kind in [true, false] {
        // De los nombres de `a` y `b`, de un mismo tipo (luego de otros tipos); después de otros
        // nombres.
        let pairs = ca
            .iter()
            .flat_map(|x| cb.iter().map(move |y| (*x, *y)))
            .filter(|(x, y)| {
                x.name == a.name && y.name == b.name && (x.kind == y.kind) == same_kind
            })
            .chain(
                ca.iter()
                    .flat_map(|x| cb.iter().map(move |y| (*x, *y)))
                    .filter(|(x, y)| x.name != a.name || y.name != b.name),
            );
        for (x, y) in pairs {
            let more = crs_code_pair(reg, x, y, extents);
            if more.is_empty() {
                continue;
            }
            // Las operaciones de `a` a `x` y de `y` a `b`, sin las nulas.
            let framing = |from: &epsg::Geodetic, to: &epsg::Geodetic| {
                (from.code != to.code)
                    .then(|| conversion_name(from, to))
                    .filter(|n| !n.starts_with(NULL_OFFSET))
            };
            let (first, last) = (framing(a, x), framing(y, b));
            found.extend(more.into_iter().map(|legs| Found {
                legs,
                conversions: [first.clone(), None, last.clone()],
            }));
            if !proj_only(&found) {
                return found;
            }
        }
    }
    found
}

/// Las cadenas de dos transformaciones que la base de datos de PROJ registra del sistema `x` a un
/// sistema intermedio y de este al sistema `y` (`createFromCRSCodesWithIntermediates` de PROJ): a
/// través de un sistema geográfico de un datum publicado desde el más antiguo de los suyos (o de
/// ninguno, como WGS 84) entre sistemas geográficos, a través de cualquier sistema en los demás
/// casos; con áreas que se cortan entre sí y con las de los sistemas; sin las sustituidas por otra
/// del mismo sentido encontrada.
fn crs_code_pair(
    reg: &epsg::Registry,
    x: &epsg::Geodetic,
    y: &epsg::Geodetic,
    extents: &[[f64; 4]],
) -> Vec<[(usize, bool); 2]> {
    use epsg::GeodeticKind::*;
    let geographic = x.kind != Geocentric && y.kind != Geocentric;
    let oldest = if geographic {
        epsg::datum_date(x.datum)
            .zip(epsg::datum_date(y.datum))
            .map(|(p, q)| p.min(q))
    } else {
        None
    };
    let fits = |i: usize| extents.iter().all(|e| intersects(&reg.ops[i].op.bounds, e));
    let (Some(from_x), Some(from_y)) = (reg.at.get(&x.code), reg.at.get(&y.code)) else {
        return Vec::new();
    };
    // El otro extremo de una transformación desde o hacia un sistema, y si se ejecuta a la inversa
    // para salir del sistema.
    let other = |i: usize, code: u32| {
        let r = &reg.ops[i];
        if r.source == code {
            (r.target, false)
        } else {
            (r.source, true)
        }
    };
    let mut out = Vec::new();
    // Las cuatro consultas de PROJ, por los sentidos de las dos transformaciones, cada una sin las
    // sustituidas por otra que encuentra.
    for (rev1, rev2) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut rows: Vec<(usize, usize, u32)> = Vec::new();
        for &i in from_x {
            let (pivot, r1) = other(i, x.code);
            if r1 != rev1 || !fits(i) {
                continue;
            }
            let Some(p) = reg.geodetic.get(&pivot) else {
                continue;
            };
            if geographic
                && (p.kind == Geocentric
                    || oldest.is_some_and(|oldest| {
                        epsg::datum_date(p.datum).is_some_and(|d| d < oldest)
                    }))
            {
                continue;
            }
            for &j in from_y {
                // La segunda va del pivote a `y`: a la inversa cuando está registrada desde `y`.
                let (back, r2) = other(j, y.code);
                if back != pivot
                    || r2 == rev2
                    || i == j
                    || !fits(j)
                    || !intersects(&reg.ops[i].op.bounds, &reg.ops[j].op.bounds)
                {
                    continue;
                }
                rows.push((i, j, pivot));
            }
        }
        let gone1 = superseded(reg, rows.iter().map(|r| r.0), false);
        let gone2 = superseded(reg, rows.iter().map(|r| r.1), false);
        for (i, j, pivot) in rows {
            let name = reg.geodetic.get(&pivot).map_or("", |p| p.name.as_str());
            // Después aquellas cuyas rejillas faltan.
            if gone1.contains(&i)
                || gone2.contains(&j)
                || !reg.ops[i].op.available()
                || !reg.ops[j].op.available()
                || irrelevant(reg, i, x.code, y.code)
                || irrelevant(reg, j, x.code, y.code)
                || rejected_pivot(&x.name, &y.name, name)
            {
                continue;
            }
            out.push([(i, rev1), (j, rev2)]);
        }
    }
    out
}

/// Las cadenas de PROJ a través de un datum intermedio
/// (`createBetweenGeodeticCRSWithDatumBasedIntermediates`): transformaciones que la base de datos
/// de PROJ registra entre cualesquiera sistemas del datum de `a` y de un tercer datum, y de ese
/// datum y del de `b`, cuyas áreas se cortan entre sí y con las de los sistemas, sin las
/// sustituidas por otra encontrada ni las que usan un datum publicado antes de 1900 (salvo que lo
/// sean ambos datums), con las conversiones entre los sistemas por los que pasan.
fn datum_chains(
    reg: &epsg::Registry,
    a: &epsg::Geodetic,
    b: &epsg::Geodetic,
    extents: &[[f64; 4]],
) -> Vec<Found> {
    let date = epsg::datum_date;
    let csrs = a.name.starts_with("NAD83(CSRS)") && b.name.starts_with("NAD83(CSRS)");
    let fits = |i: usize| extents.iter().all(|e| intersects(&reg.ops[i].op.bounds, e));
    // Las transformaciones desde o hacia los sistemas de un datum, por el datum de su otro extremo,
    // con si salen del datum a la inversa.
    let side = |datum: u32| -> HashMap<u32, Vec<(usize, bool)>> {
        let mut by: HashMap<u32, Vec<(usize, bool)>> = HashMap::new();
        for code in reg.by_datum.get(&datum).into_iter().flatten() {
            for &i in reg.at.get(code).into_iter().flatten() {
                let r = &reg.ops[i];
                let (other, reverse) = if r.source == *code {
                    (r.target, false)
                } else {
                    (r.source, true)
                };
                let Some(o) = reg.geodetic.get(&other) else {
                    continue;
                };
                if o.datum == datum
                    || (csrs && matches!(o.name.as_str(), "NAD83" | "WGS 84"))
                    || !fits(i)
                {
                    continue;
                }
                let list = by.entry(o.datum).or_default();
                if !list.contains(&(i, reverse)) {
                    list.push((i, reverse));
                }
            }
        }
        by
    };
    let (from_a, from_b) = (side(a.datum), side(b.datum));
    let mut pairs: Vec<((usize, bool), (usize, bool))> = Vec::new();
    let mut pivots: Vec<&u32> = from_a.keys().collect();
    pivots.sort_unstable();
    for pivot in pivots {
        let Some(second) = from_b.get(pivot) else {
            continue;
        };
        for &(i, r1) in &from_a[pivot] {
            if irrelevant(reg, i, a.code, b.code) {
                continue;
            }
            for &(j, r2) in second {
                if intersects(&reg.ops[i].op.bounds, &reg.ops[j].op.bounds)
                    && !irrelevant(reg, j, a.code, b.code)
                {
                    pairs.push(((i, r1), (j, !r2)));
                }
            }
        }
    }
    let gone = superseded(reg, pairs.iter().flat_map(|(x, y)| [x.0, y.0]), true);
    let oldest = match (date(a.datum), date(b.datum)) {
        (s, Some(t)) if s.is_none_or(|s| t < s) => Some(t),
        (s, _) => s,
    };
    let ancient = |code: u32| {
        reg.geodetic
            .get(&code)
            .and_then(|g| date(g.datum))
            .is_some_and(|d| d < "1900-01-01")
            && oldest.is_some_and(|o| o > "1900-01-01")
    };
    let mut out = Vec::new();
    for ((i, r1), (j, r2)) in pairs {
        let (o1, o2) = (&reg.ops[i], &reg.ops[j]);
        if gone.contains(&i)
            || gone.contains(&j)
            || [o1.source, o1.target, o2.source, o2.target]
                .into_iter()
                .any(ancient)
            || !o1.op.available()
            || !o2.op.available()
        {
            continue;
        }
        // Los sistemas de los que parten y a los que llegan las transformaciones, en los sentidos
        // en que se ejecutan.
        let ends = |r: &epsg::Registered, reverse: bool| {
            if reverse {
                (r.target, r.source)
            } else {
                (r.source, r.target)
            }
        };
        let ((s1, t1), (s2, t2)) = (ends(o1, r1), ends(o2, r2));
        let g = |code: u32| reg.geodetic.get(&code);
        let conversion = |from: u32, to: u32| match (g(from), g(to)) {
            (Some(f), Some(t)) if from != to => Some(conversion_name(f, t)),
            _ => None,
        };
        out.push(Found {
            legs: [(i, r1), (j, r2)],
            conversions: [
                conversion(a.code, s1),
                conversion(t1, s2),
                conversion(t2, b.code),
            ],
        });
    }
    out
}

/// La transformación aproximada de PROJ entre dos sistemas, que contiene todos los puntos: de
/// precisión desconocida, pero de 0 m cuando es solo una rotación de longitud (sistemas sobre un
/// mismo elipsoide con distintos meridianos origen, como MGI 1901 y Greek (Athens)), que entonces
/// gana a toda cadena que PROJ lista antes.
fn ballpark_op(src: &Def, dst: &Def) -> Op {
    let accuracy = if src.rotation_only_to(dst) { 0.0 } else { -1.0 };
    Op {
        accuracy,
        chain_accuracy: accuracy,
        ballpark: true,
        ..Op::world(Vec::new())
    }
}

/// Aquello por lo que PROJ ordena una transformación, en su orden: primero la precisión conocida
/// (las desconocidas con rejillas primero), luego el área decreciente dentro del área de interés,
/// luego la precisión, las que no tienen rejillas primero, luego las de menos pasos, luego las de
/// menos pasos de tubería, luego las de nombre más corto y luego las de nombre mayor.
struct SortKey {
    unknown: bool,
    unknown_without_grids: bool,
    area: f64,
    accuracy: f64,
    grids: bool,
    steps: usize,
    pipeline_steps: usize,
    name: String,
}

impl SortKey {
    /// La clave de una transformación con el nombre que le da PROJ y aproximadamente cuántos pasos
    /// tiene su tubería.
    fn new(op: &Op, name: String, pipeline_steps: usize, aoi: Option<&[f64; 4]>) -> Self {
        let area = aoi
            .map_or(Some(op.bounds), |a| intersection(&op.bounds, a))
            .map_or(0.0, |b| pseudo_area(&b));
        let (unknown, grids) = (op.accuracy < 0.0, op.needs_grids());
        Self {
            unknown,
            unknown_without_grids: unknown && !grids,
            area,
            accuracy: if unknown { 0.0 } else { op.accuracy },
            grids,
            steps: name.split(" + ").count(),
            pipeline_steps,
            name,
        }
    }

    /// El orden de dos claves: un orden total.
    fn cmp(&self, other: &Self) -> Ordering {
        self.unknown
            .cmp(&other.unknown)
            .then(self.unknown_without_grids.cmp(&other.unknown_without_grids))
            .then(other.area.total_cmp(&self.area))
            .then(self.accuracy.total_cmp(&other.accuracy))
            .then(self.grids.cmp(&other.grids))
            .then(self.steps.cmp(&other.steps))
            .then(self.pipeline_steps.cmp(&other.pipeline_steps))
            .then(self.name.len().cmp(&other.name.len()))
            .then_with(|| other.name.cmp(&self.name))
    }
}

/// Aproximadamente cuántos pasos tiene la tubería de PROJ para transformaciones ejecutadas una tras
/// otra (cada una en su sentido), de los que PROJ prefiere menos: sus pasos de datum, sin una
/// conversión a coordenadas geocéntricas justo después de su inversa sobre el mismo elipsoide (que
/// la optimización de la tubería de PROJ elimina), con las traslaciones geocéntricas consecutivas
/// fusionadas en una (o eliminadas cuando se anulan), como hace la optimización de PROJ, y los
/// `push v_3` y `pop v_3` alrededor de los pasos geocéntricos.
fn pipeline_steps(legs: &[(&Op, bool)]) -> usize {
    /// Un paso de la tubería: un paso de datum y si se ejecuta invertido, o traslaciones que PROJ
    /// fusionó.
    enum Item<'a> {
        Step(&'a Step, bool),
        Translation([f64; 3]),
    }
    // El elipsoide de una conversión y si parte de coordenadas geocéntricas.
    let cart = |item: &Item| match item {
        Item::Step(Step::Cart { ell, inv }, inverted) => Some((*ell, *inv != *inverted)),
        _ => None,
    };
    // La traslación de un paso de Helmert solo con traslaciones, tal como se ejecuta.
    let translation = |item: &Item| match item {
        Item::Translation(t) => Some(*t),
        Item::Step(
            Step::Helmert {
                t,
                r,
                scale,
                pivot,
                inv,
                rates: None,
            },
            inverted,
        ) if *scale == 1.0
            && *pivot == [0.0; 3]
            && *r == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] =>
        {
            let sign = if inv != inverted { -1.0 } else { 1.0 };
            Some(t.map(|v| v * sign))
        }
        _ => None,
    };
    let mut run: Vec<Item> = Vec::new();
    for &(op, reverse) in legs {
        let steps: Vec<(&Step, bool)> = if reverse {
            op.steps.iter().rev().map(|s| (s, true)).collect()
        } else {
            op.steps.iter().map(|s| (s, false)).collect()
        };
        for (step, inverted) in steps {
            let item = Item::Step(step, inverted);
            if let (Some(last), Some((ell, false))) = (run.last(), cart(&item))
                && cart(last) == Some((ell, true))
            {
                run.pop();
                continue;
            }
            if let (Some(a), Some(b)) = (run.last().and_then(translation), translation(&item)) {
                run.pop();
                let sum = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
                if sum != [0.0; 3] {
                    run.push(Item::Translation(sum));
                }
                continue;
            }
            run.push(item);
        }
    }
    let geocentric = run.iter().any(|item| cart(item).is_some());
    run.len() + if geocentric { 2 } else { 0 }
}

/// El nombre que da PROJ a la inversa de una operación simple: los nombres que hace PROJ
/// (`Conversion from A to B`) con sus sistemas intercambiados, los demás con `Inverse of ` añadido
/// o quitado.
fn inverse_name(name: &str) -> String {
    if let Some(forward) = name.strip_prefix("Inverse of ") {
        return forward.to_string();
    }
    for kind in [
        "Conversion",
        "Null geographic offset",
        "Null geocentric translation",
    ] {
        if let Some((from, to)) = name
            .strip_prefix(kind)
            .and_then(|n| n.strip_prefix(" from "))
            .and_then(|n| n.split_once(" to "))
        {
            return format!("{kind} from {to} to {from}");
        }
    }
    format!("Inverse of {name}")
}

/// Los nombres de los pasos de una transformación tal como los nombra PROJ cuando la ejecuta
/// `reverse` o no: los de la inversa son las inversas de los pasos en orden inverso.
fn step_names(op: &Op, reverse: bool) -> Vec<String> {
    let names: Vec<&str> = op.name.split(" + ").filter(|n| !n.is_empty()).collect();
    if reverse {
        names.into_iter().rev().map(inverse_name).collect()
    } else {
        names.into_iter().map(str::to_string).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crs::{SpatialRef, Transformer};

    /// Un punto movido de un sistema EPSG a otro.
    fn moved(src: u32, dst: u32, x: f64, y: f64) -> (f64, f64) {
        let (s, d) = (
            SpatialRef::from_epsg(src).unwrap(),
            SpatialRef::from_epsg(dst).unwrap(),
        );
        Transformer::new(&s, &d).unwrap().transform(x, y).unwrap()
    }

    #[test]
    fn areas_across_the_antimeridian() {
        let russia = [18.92, 39.87, -168.97, 85.19];
        let azores = [-31.34, 39.3, -31.02, 39.77];
        assert_eq!(intersection(&russia, &azores), None);
        assert!(!intersects(&russia, &azores));
        assert_eq!(
            intersection(&russia, &[170.0, 50.0, -170.0, 60.0]),
            Some([170.0, 50.0, -170.0, 60.0])
        );
        assert_eq!(
            intersection(&[-180.0, -90.0, 180.0, 90.0], &russia),
            Some(russia)
        );
        assert_eq!(
            intersection(&[100.0, 40.0, 120.0, 50.0], &russia),
            Some([100.0, 40.0, 120.0, 50.0])
        );
        assert!(
            contains(&russia, &[170.0, 50.0, -170.0, 60.0])
                && contains(&russia, &[30.0, 50.0, 40.0, 60.0])
        );
        assert!(!contains(&russia, &azores) && !contains(&azores, &russia));
        assert!(contains(&[-180.0, -90.0, 180.0, 90.0], &azores));
    }

    #[test]
    fn chains_outside_their_areas_as_proj() {
        // Justo al este del área de Azores Occidental 1939: ninguna cadena contiene el punto y una
        // contiene el área de interés (así que PROJ descarta su aproximación): la primera cadena,
        // como GDAL.
        let (x, y) = moved(4182, 4740, -31.016816380974838, 39.56501297818613);
        assert!(
            (x + 31.021097205869058).abs() < 1e-7 && (y - 39.566344404860935).abs() < 1e-7,
            "{x} {y}"
        );
        // Se usa una sola cadena sea cual sea el punto, como hace GDAL.
        let (x, y) = moved(2999, 9053, 298971.18861002335, 8694000.007133322);
        assert!(
            (x - 43.16443339530448).abs() < 1e-7 && (y + 11.811498334670878).abs() < 1e-7,
            "{x} {y}"
        );
    }

    #[test]
    fn chain_names_as_proj() {
        assert_eq!(
            inverse_name("NTF to WGS 84 (1)"),
            "Inverse of NTF to WGS 84 (1)"
        );
        assert_eq!(
            inverse_name("Inverse of WGS 72BE to WGS 84 (1)"),
            "WGS 72BE to WGS 84 (1)"
        );
        assert_eq!(
            inverse_name("Conversion from A (geog2D) to A (geocentric)"),
            "Conversion from A (geocentric) to A (geog2D)"
        );
        // Las conversiones entre los sistemas de un datum, como las nombra PROJ.
        let reg = epsg::registry();
        let name = |a: u32, b: u32| conversion_name(&reg.geodetic[&a], &reg.geodetic[&b]);
        assert_eq!(
            name(7844, 7842),
            "Conversion from GDA2020 (geog2D) to GDA2020 (geocentric)"
        );
        assert_eq!(
            name(4258, 4937),
            "Null geographic offset from ETRS89 (geog2D) to ETRS89 (geog3D)"
        );
        assert_eq!(name(10310, 10312), "axis order change (2D)");
    }

    #[test]
    fn chains_ordered_as_proj() {
        let close = |(x, y): (f64, f64), (gx, gy): (f64, f64), tol: f64| {
            (x - gx).abs() < tol && (y - gy).abs() < tol
        };
        // GDA94 a través de WGS 84 (G1762) y la concatenación propia de PROJ de las realizaciones
        // de WGS 84.
        let p = moved(10459, 10606, 19600.97347104833, 592323.3389500268);
        assert!(
            close(p, (113.95807615479848, -27.180067590263132), 1e-8),
            "{p:?}"
        );
        // A través de WGS 84 en lugar de WGS 72, de igual precisión y área: primero el nombre
        // mayor.
        let p = moved(27573, 32431, 412711.90358792024, 3155755.5255349036);
        assert!(
            close(p, (259245.25512248135, 4840436.820252412), 1e-3),
            "{p:?}"
        );
        // GDA94 a WGS 84 (1), una transformación nula, hace menos pasos de tubería que (2).
        let p = moved(28351, 4324, 625263.796562132, 8842164.96869988);
        assert!(
            close(p, (124.144357767899, -10.472091951341795), 1e-8),
            "{p:?}"
        );
        // A través de WGS 84 en lugar de ETRS89, de igual precisión y área: primero el nombre más
        // corto.
        let p = moved(2392, 4124, 2500642.641281675, 7632767.385064217);
        assert!(
            close(p, (24.016658156077746, 68.77924926097009), 1e-7),
            "{p:?}"
        );
    }
}
