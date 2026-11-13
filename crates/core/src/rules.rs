//! Classification by rules: the cheap criteria that say a lot. A rule changes the class of the
//! points that are of certain classes and meet every condition it names:
//!
//! * the **echo** (`S` single, `F` first of several, `I` intermediate, `L` last of several; or
//!   return numbers `1` to `9`): first returns of pulses that gave several are canopy and edges,
//!   the last ones are what reached the ground;
//! * the **scan angle**: points scanned at a steep angle are less accurate;
//! * the **height above the ground** (`height`), measured over the mean of the ground points in
//!   cells of 1 m: a rule `from=1;height=0.5..2;to=4` is TerraScan's classification by height from
//!   ground, and any strata can be made with several;
//! * the **intensity** (`intensity`);
//! * the shape of the **neighbourhood** within `radius` (default 1 m): `slope` (degrees of the
//!   local plane from the horizontal), `planarity`, `linearity`, `sphericity`, `roughness` (m) and
//!   `neighbours` (how many points), the geometric features of [`crate::features`].
//!
//! Rules are applied in order, so a later one sees what an earlier one made. A rule that asks for
//! neighbourhood features makes them for the points that pass its other conditions only, and the
//! pass costs nothing when no rule asks.

use crate::classes::ClassSet;
use crate::features::{self, Features};
use crate::filter::{ClassSel, ReturnKind, ReturnSel};
use crate::las::Point;
use crate::{Error, Result};

/// The class of ground, as the ground filters label it.
const GROUND: u8 = 2;

/// Cell of the ground surface heights are measured over (m).
const SURFACE_CELL: f64 = 1.0;

/// A closed range of values; an open end is infinite.
pub type Range = (f64, f64);

fn within(r: Option<Range>, v: f64) -> bool {
    r.is_none_or(|(lo, hi)| v >= lo && v <= hi)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassRule {
    /// Only points of these classes are looked at. Noise (7, 18) is left alone by default.
    pub from: ClassSet,
    /// Which echoes of the pulse.
    pub returns: ReturnSel,
    /// Absolute scan angle (degrees) from and to, both included; None: any angle.
    pub angle: Option<(f64, f64)>,
    /// Height above the ground (m); a point with no ground around it never matches.
    pub height: Option<Range>,
    pub intensity: Option<Range>,
    /// Slope of the local plane (degrees from the horizontal).
    pub slope: Option<Range>,
    pub planarity: Option<Range>,
    pub linearity: Option<Range>,
    pub sphericity: Option<Range>,
    /// Standard deviation of the distance to the local plane (m).
    pub roughness: Option<Range>,
    /// Points within `radius`, the point itself included.
    pub neighbours: Option<Range>,
    /// Radius of the neighbourhood the shape conditions look at (m).
    pub radius: f64,
    pub to: u8,
}

impl ClassRule {
    /// A rule that sends every point that is not noise to class `to`.
    pub fn new(to: u8) -> Self {
        let mut from = ClassSet::all();
        from.set(7, false);
        from.set(18, false);
        Self {
            from,
            returns: ReturnSel::all(),
            angle: None,
            height: None,
            intensity: None,
            slope: None,
            planarity: None,
            linearity: None,
            sphericity: None,
            roughness: None,
            neighbours: None,
            radius: 1.0,
            to,
        }
    }

    /// Whether the rule takes a point that is now of class `class`, by class, echo and angle.
    pub fn takes(&self, p: &Point, class: u8) -> bool {
        self.from.contains(class) && self.echo_and_angle(p)
    }

    fn echo_and_angle(&self, p: &Point) -> bool {
        self.returns.allows(p)
            && self.angle.is_none_or(|(lo, hi)| {
                let a = f64::from(p.scan_angle.abs());
                a >= lo && a <= hi
            })
    }

    /// The conditions that do not depend on the class or on the neighbourhood: echo, angle,
    /// height above the ground (NaN: unknown) and intensity.
    fn cheap(&self, p: &Point, height: f64) -> bool {
        self.echo_and_angle(p)
            && (self.height.is_none() || (!height.is_nan() && within(self.height, height)))
            && within(self.intensity, f64::from(p.intensity))
    }

    /// Whether the rule looks at the shape of the neighbourhood.
    pub fn needs_features(&self) -> bool {
        self.slope.is_some()
            || self.planarity.is_some()
            || self.linearity.is_some()
            || self.sphericity.is_some()
            || self.roughness.is_some()
            || self.neighbours.is_some()
    }

    fn shape(&self, f: Option<&Features>) -> bool {
        if !self.needs_features() {
            return true;
        }
        let Some(f) = f else { return false };
        within(self.slope, f64::from(f.slope))
            && within(self.planarity, f64::from(f.planarity))
            && within(self.linearity, f64::from(f.linearity))
            && within(self.sphericity, f64::from(f.sphericity))
            && within(self.roughness, f64::from(f.roughness))
            && within(self.neighbours, f64::from(f.count))
    }

    /// Reads a rule from text: `to=12` and any of `from=1,2` (or `~7`), `returns=F` (`S`, `F`,
    /// `I`, `L`, `A`, or numbers: `FL` is first or last of several), `angle=25..90` (or
    /// `angle=25` for 25 and over), `height=0.5..2` (m above the ground; also `intensity`,
    /// `slope`, `planarity`, `linearity`, `sphericity`, `roughness` and `neighbours`, with the
    /// same ranges: `..2` is up to 2, `0.5` or `0.5..` is 0.5 and over) and `radius=1` (m, for the
    /// shape conditions), separated by `;`.
    pub fn parse(text: &str) -> Result<Self> {
        let mut rule = Self::new(0);
        let mut has_to = false;
        for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                Error::Param(format!(
                    "la parte de la regla '{part}' debe ser nombre=valor"
                ))
            })?;
            let value = value.trim();
            match key.trim() {
                "to" => {
                    rule.to = value.parse().map_err(|_| {
                        Error::Param(format!("la clase '{value}' no es un número de 0 a 255"))
                    })?;
                    has_to = true;
                }
                "from" => rule.from = ClassSel::parse(value)?.set,
                "returns" | "echo" => rule.returns = parse_returns(value)?,
                "angle" => {
                    rule.angle = Some(match parse_range("angle", value)? {
                        (lo, hi) if hi.is_infinite() => (lo, 180.0),
                        r => r,
                    });
                }
                "height" => rule.height = Some(parse_range("height", value)?),
                "intensity" => rule.intensity = Some(parse_range("intensity", value)?),
                "slope" => rule.slope = Some(parse_range("slope", value)?),
                "planarity" => rule.planarity = Some(parse_range("planarity", value)?),
                "linearity" => rule.linearity = Some(parse_range("linearity", value)?),
                "sphericity" => rule.sphericity = Some(parse_range("sphericity", value)?),
                "roughness" => rule.roughness = Some(parse_range("roughness", value)?),
                "neighbours" | "neighbors" => {
                    rule.neighbours = Some(parse_range("neighbours", value)?);
                }
                "radius" => {
                    rule.radius = num("radius", value)?;
                    if !(rule.radius > 0.0) {
                        return Err(Error::Param("el radio debe ser positivo".into()));
                    }
                }
                other => {
                    return Err(Error::Param(format!(
                        "parte de regla desconocida '{other}': usa to, from, returns, angle, height, \
                         intensity, slope, planarity, linearity, sphericity, roughness, neighbours o radius"
                    )));
                }
            }
        }
        if !has_to {
            return Err(Error::Param(
                "una regla necesita la clase de destino: to=12".into(),
            ));
        }
        Ok(rule)
    }

    /// The rule written the way `parse` reads it back (and `--rule` takes it), so that a recipe
    /// can hold it.
    pub fn to_arg(&self) -> String {
        let mut parts = vec![format!("to={}", self.to)];
        let mut default_from = ClassSet::all();
        default_from.set(7, false);
        default_from.set(18, false);
        if self.from != default_from {
            let list: Vec<String> = (0..=255u8)
                .filter(|c| self.from.contains(*c))
                .map(|c| c.to_string())
                .collect();
            parts.push(format!("from={}", list.join(",")));
        }
        if self.returns != ReturnSel::all() {
            let mut r = String::new();
            for (kind, letter) in [
                (ReturnKind::Single, 'S'),
                (ReturnKind::First, 'F'),
                (ReturnKind::Intermediate, 'I'),
                (ReturnKind::Last, 'L'),
            ] {
                if self.returns.kinds[kind.index()] {
                    r.push(letter);
                }
            }
            for n in 1..=9u16 {
                if self.returns.numbers & (1 << n) != 0 {
                    r.push(char::from(b'0' + n as u8));
                }
            }
            parts.push(format!("returns={r}"));
        }
        if let Some((lo, hi)) = self.angle {
            parts.push(if hi >= 180.0 {
                format!("angle={lo}")
            } else {
                format!("angle={lo}..{hi}")
            });
        }
        for (name, r) in [
            ("height", self.height),
            ("intensity", self.intensity),
            ("slope", self.slope),
            ("planarity", self.planarity),
            ("linearity", self.linearity),
            ("sphericity", self.sphericity),
            ("roughness", self.roughness),
            ("neighbours", self.neighbours),
        ] {
            if let Some((lo, hi)) = r {
                let lo = if lo.is_finite() { lo.to_string() } else { String::new() };
                let hi = if hi.is_finite() { hi.to_string() } else { String::new() };
                parts.push(format!("{name}={lo}..{hi}"));
            }
        }
        if self.needs_features() && self.radius != 1.0 {
            parts.push(format!("radius={}", self.radius));
        }
        parts.join(";")
    }

    /// The rule as `parse` reads it, to show it.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if let Some((lo, hi)) = self.angle {
            parts.push(if hi >= 180.0 {
                format!("angle ≥ {lo}°")
            } else {
                format!("angle {lo}° a {hi}°")
            });
        }
        for (name, r) in [
            ("height", self.height),
            ("intensity", self.intensity),
            ("slope", self.slope),
            ("planarity", self.planarity),
            ("linearity", self.linearity),
            ("sphericity", self.sphericity),
            ("roughness", self.roughness),
            ("neighbours", self.neighbours),
        ] {
            if let Some((lo, hi)) = r {
                parts.push(match (lo.is_finite(), hi.is_finite()) {
                    (true, true) => format!("{name} {lo} a {hi}"),
                    (true, false) => format!("{name} ≥ {lo}"),
                    (false, true) => format!("{name} ≤ {hi}"),
                    (false, false) => name.to_string(),
                });
            }
        }
        parts.push(format!("→ clase {}", self.to));
        parts.join(" ")
    }
}

fn num(what: &str, t: &str) -> Result<f64> {
    t.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| Error::Param(format!("{what}: '{t}' no es un número")))
}

/// `a..b`, `a..` or `a` (a and over), `..b` (up to b).
fn parse_range(what: &str, text: &str) -> Result<Range> {
    if text.trim().is_empty() {
        return Err(Error::Param(format!("{what}: falta el valor")));
    }
    let (lo, hi) = match text.split_once("..") {
        Some((lo, hi)) => (lo.trim(), hi.trim()),
        None => (text.trim(), ""),
    };
    let lo = if lo.is_empty() {
        f64::NEG_INFINITY
    } else {
        num(what, lo)?
    };
    let hi = if hi.is_empty() {
        f64::INFINITY
    } else {
        num(what, hi)?
    };
    if lo > hi {
        return Err(Error::Param(format!(
            "{what}: el límite inferior {lo} supera al superior {hi}"
        )));
    }
    Ok((lo, hi))
}

/// The echoes of a rule from text: `S`, `F`, `I`, `L`, `A` or return numbers `1` to `9`.
fn parse_returns(text: &str) -> Result<ReturnSel> {
    let mut sel = ReturnSel {
        kinds: [false; 4],
        numbers: 0,
    };
    for c in text.trim().chars() {
        match c.to_ascii_uppercase() {
            'A' => sel.kinds = [true; 4],
            'S' => sel.kinds[ReturnKind::Single.index()] = true,
            'F' => sel.kinds[ReturnKind::First.index()] = true,
            'I' => sel.kinds[ReturnKind::Intermediate.index()] = true,
            'L' => sel.kinds[ReturnKind::Last.index()] = true,
            d @ '1'..='9' => sel.numbers |= 1 << (d as u8 - b'0'),
            ',' | ' ' => {}
            other => {
                return Err(Error::Param(format!(
                    "'{other}' no es un eco: usa S, F, I, L, A o de 1 a 9"
                )));
            }
        }
    }
    Ok(sel)
}

/// The command-line arguments a recipe file stands for: one or more per line, separated by
/// spaces (a value with spaces goes in double quotes); empty lines and lines starting with `#`
/// are skipped. A recipe is a classification written down once and run on any number of sheets:
/// `hipso classify sheet.laz --recipe forest.txt`.
pub fn recipe_args(text: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (mut cur, mut quoted, mut started) = (String::new(), false, false);
        for c in line.chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                c if c.is_whitespace() && !quoted => {
                    if started {
                        args.push(std::mem::take(&mut cur));
                        started = false;
                    }
                }
                c => {
                    cur.push(c);
                    started = true;
                }
            }
        }
        if quoted {
            return Err(Error::Param(format!(
                "receta, línea {}: comillas sin cerrar",
                n + 1
            )));
        }
        if started {
            args.push(cur);
        }
    }
    Ok(args)
}

/// Applies the rules, in order, to the points of a tile. `class` holds the class a step has
/// already decided (None: the point keeps its own); `ground` follows it. Returns, for each point,
/// whether a rule changed its class.
pub fn apply(
    pts: &[Point],
    class: &mut [Option<u8>],
    ground: &mut [bool],
    rules: &[ClassRule],
) -> Vec<bool> {
    let mut changed = vec![false; pts.len()];
    if rules.is_empty() {
        return changed;
    }
    // heights above the ground, once, from the ground the steps before the rules found
    let height: Vec<f32> = if rules.iter().any(|r| r.height.is_some()) {
        match crate::extras::ground_surface(pts, ground, SURFACE_CELL) {
            Some(s) => pts
                .iter()
                .map(|q| s.sample(q.x, q.y).map_or(f32::NAN, |g| (q.z - f64::from(g)) as f32))
                .collect(),
            None => vec![f32::NAN; pts.len()],
        }
    } else {
        Vec::new()
    };
    let h = |k: usize| height.get(k).map_or(f64::NAN, |v| f64::from(*v));
    // shape features, one pass per radius, for the points some rule of that radius may take
    let mut radii: Vec<f64> = Vec::new();
    for r in rules.iter().filter(|r| r.needs_features()) {
        if !radii.contains(&r.radius) {
            radii.push(r.radius);
        }
    }
    let feats: Vec<Vec<Option<Features>>> = radii
        .iter()
        .map(|&radius| {
            let wanted: Vec<bool> = pts
                .iter()
                .enumerate()
                .map(|(k, q)| {
                    !q.is_noise()
                        && rules
                            .iter()
                            .any(|r| r.needs_features() && r.radius == radius && r.cheap(q, h(k)))
                })
                .collect();
            features::compute(pts, &wanted, radius)
        })
        .collect();
    let feature_set = |r: &ClassRule| radii.iter().position(|x| *x == r.radius);
    for (k, q) in pts.iter().enumerate() {
        // points set aside (overlap) are left as they are
        if q.withheld {
            continue;
        }
        let mut now = class.get(k).copied().flatten().unwrap_or(q.class);
        for r in rules {
            if now == r.to || !r.from.contains(now) || !r.cheap(q, h(k)) {
                continue;
            }
            let f = feature_set(r)
                .filter(|_| r.needs_features())
                .and_then(|i| feats[i][k].as_ref());
            if r.shape(f) {
                now = r.to;
                changed[k] = true;
            }
        }
        if changed[k] {
            if let Some(c) = class.get_mut(k) {
                *c = Some(now);
            }
            if let Some(g) = ground.get_mut(k) {
                *g = now == GROUND;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(class: u8, r: u8, n: u8, angle: f32) -> Point {
        Point {
            class,
            return_number: r,
            number_of_returns: n,
            scan_angle: angle,
            ..Default::default()
        }
    }

    #[test]
    fn a_rule_takes_by_class_echo_and_absolute_angle() -> Result<()> {
        // first returns of several, at 25° or more to either side, to class 12
        let rule = ClassRule::parse("from=1,2;returns=F;angle=25..90;to=12")?;
        assert!(rule.takes(&pt(1, 1, 3, 30.0), 1));
        assert!(rule.takes(&pt(2, 1, 3, -30.0), 2));
        // the last of several, a single return, class 5, or a gentle angle: not taken
        assert!(!rule.takes(&pt(1, 3, 3, 30.0), 1));
        assert!(
            !rule.takes(&pt(1, 1, 1, 30.0), 1),
            "F is the first of several only"
        );
        assert!(ClassRule::parse("returns=SL;to=5")?.takes(&pt(1, 1, 1, 0.0), 1));
        assert!(!rule.takes(&pt(5, 1, 3, 30.0), 5));
        assert!(!rule.takes(&pt(1, 1, 3, 10.0), 1));
        // an angle with no upper limit
        assert!(ClassRule::parse("angle=25;to=1")?.takes(&pt(2, 1, 1, -80.0), 2));
        Ok(())
    }

    #[test]
    fn noise_is_left_alone_unless_the_rule_says_otherwise() -> Result<()> {
        let rule = ClassRule::new(1);
        assert!(!rule.takes(&pt(7, 1, 1, 0.0), 7) && !rule.takes(&pt(18, 1, 1, 0.0), 18));
        assert!(ClassRule::parse("from=7;to=1")?.takes(&pt(7, 1, 1, 0.0), 7));
        Ok(())
    }

    #[test]
    fn bad_rules_say_what_is_wrong() {
        for bad in ["from=1", "to=300", "angle=x;to=1", "colour=3;to=1", "to"] {
            assert!(
                matches!(ClassRule::parse(bad), Err(Error::Param(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn rules_apply_in_order_and_ground_follows() -> Result<()> {
        let pts = [
            pt(2, 1, 1, 40.0), // ground at a steep angle
            pt(2, 1, 1, 5.0),  // ground at a gentle angle
            pt(1, 1, 3, 5.0),  // first of several
            pt(7, 1, 1, 40.0), // noise
        ];
        let rules = [
            ClassRule::parse("angle=30;to=12")?,
            // sees what the rule before made: class 12 points that are first returns go to 5
            ClassRule::parse("from=12;returns=S;to=5")?,
            ClassRule::parse("from=1;returns=F;to=5")?,
        ];
        let mut class = vec![None; 4];
        let mut ground = vec![true, true, false, false];
        let changed = apply(&pts, &mut class, &mut ground, &rules);
        assert_eq!(class, [Some(5), None, Some(5), None]);
        assert_eq!(changed, [true, false, true, false]);
        // the steep ground point is no longer ground
        assert_eq!(ground, [false, true, false, false]);
        Ok(())
    }

    fn at(class: u8, x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            class,
            return_number: 1,
            number_of_returns: 1,
            ..Default::default()
        }
    }

    #[test]
    fn ranges_read_open_and_closed_ends() -> Result<()> {
        let r = ClassRule::parse("height=0.5..2;intensity=..100;slope=45;to=4")?;
        assert_eq!(r.height, Some((0.5, 2.0)));
        assert_eq!(r.intensity, Some((f64::NEG_INFINITY, 100.0)));
        assert_eq!(r.slope, Some((45.0, f64::INFINITY)));
        assert!(r.needs_features());
        assert!(!ClassRule::parse("height=1;to=4")?.needs_features());
        assert_eq!(ClassRule::parse("radius=2.5;slope=10;to=4")?.radius, 2.5);
        for bad in ["height=3..1;to=1", "height=x;to=1", "radius=0;to=1", "slope=;to=1"] {
            assert!(
                matches!(ClassRule::parse(bad), Err(Error::Param(_))),
                "{bad}"
            );
        }
        // the old angle syntax still reads the same
        assert_eq!(ClassRule::parse("angle=25;to=1")?.angle, Some((25.0, 180.0)));
        assert_eq!(ClassRule::parse("angle=25..90;to=1")?.angle, Some((25.0, 90.0)));
        let text = ClassRule::parse("height=0.5..2;slope=45;to=4")?.text();
        assert!(text.contains("height 0.5 a 2") && text.contains("slope ≥ 45"), "{text}");
        Ok(())
    }

    #[test]
    fn height_rules_make_strata_over_the_ground() -> Result<()> {
        // a flat ground on a 1 m lattice, and points 0.3, 1, 3 m above it
        let mut pts = Vec::new();
        for i in 0..10 {
            for j in 0..10 {
                pts.push(at(2, f64::from(i), f64::from(j), 100.0));
            }
        }
        pts.push(at(1, 5.0, 5.0, 100.3));
        pts.push(at(1, 5.0, 5.0, 101.0));
        pts.push(at(1, 5.0, 5.0, 103.0));
        let n = pts.len();
        let rules = [
            ClassRule::parse("from=1;height=..0.5;to=3")?,
            ClassRule::parse("from=1;height=0.5..2;to=4")?,
            ClassRule::parse("from=1;height=2;to=5")?,
        ];
        let mut class = vec![None; n];
        let mut ground: Vec<bool> = pts.iter().map(|q| q.class == 2).collect();
        let changed = apply(&pts, &mut class, &mut ground, &rules);
        assert_eq!(&class[100..], [Some(3), Some(4), Some(5)]);
        // the ground is left alone
        assert!(class[..100].iter().all(Option::is_none));
        assert_eq!(changed.iter().filter(|c| **c).count(), 3);
        Ok(())
    }

    #[test]
    fn height_rules_ignore_points_with_no_ground_known() -> Result<()> {
        let pts = [at(1, 0.0, 0.0, 5.0)];
        let mut class = vec![None];
        let mut ground = vec![false];
        let rules = [ClassRule::parse("height=0..10;to=5")?];
        assert_eq!(apply(&pts, &mut class, &mut ground, &rules), [false]);
        Ok(())
    }

    #[test]
    fn shape_rules_tell_a_wall_from_the_ground() -> Result<()> {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                let (u, v) = (f64::from(i) * 0.1, f64::from(j) * 0.1);
                pts.push(at(1, u, v, 0.0)); // flat
                pts.push(at(1, u + 20.0, 0.0, v)); // wall
            }
        }
        let n = pts.len();
        let rules = [
            ClassRule::parse("slope=70;planarity=0.8;radius=0.5;to=6")?,
            ClassRule::parse("slope=..10;planarity=0.8;radius=0.5;to=2")?,
        ];
        let mut class = vec![None; n];
        let mut ground = vec![false; n];
        apply(&pts, &mut class, &mut ground, &rules);
        let wall = (0..n).filter(|k| pts[*k].x >= 20.0);
        let flat = (0..n).filter(|k| pts[*k].x < 20.0);
        let (mut w6, mut wn, mut f2, mut fn_) = (0, 0, 0, 0);
        for k in wall.clone() {
            wn += 1;
            w6 += usize::from(class[k] == Some(6));
        }
        for k in flat {
            fn_ += 1;
            f2 += usize::from(class[k] == Some(2));
        }
        // away from the edges, where the neighbourhood is whole, the shape decides
        assert!(w6 * 10 >= wn * 7, "wall points taken: {w6} of {wn}");
        assert!(f2 * 10 >= fn_ * 7, "flat points taken: {f2} of {fn_}");
        assert!(ground.iter().filter(|g| **g).count() == f2);
        Ok(())
    }

    #[test]
    fn a_recipe_is_the_arguments_of_its_lines() -> Result<()> {
        let text = "# forest\n--ground smrf\n\n  --rule \"from=1;height=..0.5;to=3\"  \n--rule from=1;height=2;to=5\n--vegetation # no\n";
        assert_eq!(
            recipe_args(text)?,
            [
                "--ground",
                "smrf",
                "--rule",
                "from=1;height=..0.5;to=3",
                "--rule",
                "from=1;height=2;to=5",
                "--vegetation",
                "#",
                "no"
            ]
        );
        assert!(recipe_args("--rule \"to=1").is_err());
        assert!(recipe_args("# nothing\n\n")?.is_empty());
        Ok(())
    }

    #[test]
    fn a_rule_written_as_an_argument_reads_back_the_same() -> Result<()> {
        for text in [
            "to=5",
            "from=1,2;returns=F;angle=25..90;to=12",
            "from=1;height=..0.5;to=3",
            "from=1;height=0.5..2;intensity=10..200;to=4",
            "height=2..;slope=60..;planarity=0.7..;radius=2.5;to=6",
            "returns=SL3;angle=30;neighbours=4..;roughness=..0.2;linearity=0.1..0.9;sphericity=..0.5;to=14",
        ] {
            let rule = ClassRule::parse(text)?;
            assert_eq!(ClassRule::parse(&rule.to_arg())?, rule, "{text} -> {}", rule.to_arg());
        }
        Ok(())
    }

    #[test]
    fn high_noise_can_be_told_from_low_noise_with_a_rule() -> Result<()> {
        // ground at 100 m on a 1 m lattice; noise 30 m above it and 8 m below it
        let mut pts = Vec::new();
        for i in 0..10 {
            for j in 0..10 {
                pts.push(at(2, f64::from(i), f64::from(j), 100.0));
            }
        }
        pts.push(at(7, 5.0, 5.0, 130.0));
        pts.push(at(7, 5.0, 5.0, 92.0));
        let mut ground: Vec<bool> = pts.iter().map(|q| q.class == 2).collect();
        let mut class = vec![None; pts.len()];
        // ASPRS: 7 is low noise and 18 high noise
        let rules = [ClassRule::parse("from=7;height=5..;to=18")?];
        apply(&pts, &mut class, &mut ground, &rules);
        assert_eq!(&class[100..], [Some(18), None]);
        Ok(())
    }
}
