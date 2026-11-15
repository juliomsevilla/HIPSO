//! Elipsoides, unidades y las funciones auxiliares que comparten las proyecciones cartográficas,
//! según las definiciones y los algoritmos de PROJ (<https://proj.org>, licencia MIT); el aviso de
//! PROJ está en `NOTICE`.

// Las constantes son las de PROJ, dígito por dígito.
#![allow(clippy::excessive_precision)]

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

/// Un elipsoide normalizado a un semieje mayor de 1, como lo usan las proyecciones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ell {
    /// El semieje mayor en metros.
    pub a: f64,
    /// La excentricidad al cuadrado.
    pub es: f64,
    /// La excentricidad.
    pub e: f64,
    /// `1 - es`.
    pub one_es: f64,
}

impl Ell {
    /// Un elipsoide a partir de su semieje mayor y el inverso del aplanamiento (0 para una esfera).
    pub fn from_a_rf(a: f64, rf: f64) -> Self {
        let f = if rf == 0.0 { 0.0 } else { 1.0 / rf };
        Self::from_a_es(a, 2.0 * f - f * f)
    }

    /// Un elipsoide a partir de sus semiejes mayor y menor.
    pub fn from_a_b(a: f64, b: f64) -> Self {
        Self::from_a_es(a, 1.0 - (b * b) / (a * a))
    }

    /// Un elipsoide a partir de su semieje mayor y su excentricidad al cuadrado.
    pub fn from_a_es(a: f64, es: f64) -> Self {
        let es = if es.abs() < 1e-15 { 0.0 } else { es };
        Self {
            a,
            es,
            e: es.sqrt(),
            one_es: 1.0 - es,
        }
    }

    /// WGS 84.
    pub fn wgs84() -> Self {
        Self::from_a_rf(6378137.0, 298.257223563)
    }

    /// Uno de los elipsoides con nombre de PROJ (`+ellps=`).
    pub fn named(name: &str) -> Option<Self> {
        // (nombre, a, rf o b cuando es negativo).
        const ELLIPSOIDS: &[(&str, f64, f64)] = &[
            ("MERIT", 6378137.0, 298.257),
            ("SGS85", 6378136.0, 298.257),
            ("GRS80", 6378137.0, 298.257222101),
            ("IAU76", 6378140.0, 298.257),
            ("airy", 6377563.396, -6356256.910),
            ("APL4.9", 6378137.0, 298.25),
            ("NWL9D", 6378145.0, 298.25),
            ("mod_airy", 6377340.189, -6356034.446),
            ("andrae", 6377104.43, 300.0),
            ("danish", 6377019.2563, 300.0),
            ("aust_SA", 6378160.0, 298.25),
            ("GRS67", 6378160.0, 298.2471674270),
            ("GSK2011", 6378136.5, 298.2564151),
            ("bessel", 6377397.155, 299.1528128),
            ("bess_nam", 6377483.865, 299.1528128),
            ("clrk66", 6378206.4, -6356583.8),
            ("clrk80", 6378249.145, 293.4663),
            ("clrk80ign", 6378249.2, 293.4660212936269),
            ("CPM", 6375738.7, 334.29),
            ("delmbr", 6376428.0, 311.5),
            ("engelis", 6378136.05, 298.2566),
            ("evrst30", 6377276.345, 300.8017),
            ("evrst48", 6377304.063, 300.8017),
            ("evrst56", 6377301.243, 300.8017),
            ("evrst69", 6377295.664, 300.8017),
            ("evrstSS", 6377298.556, 300.8017),
            ("fschr60", 6378166.0, 298.3),
            ("fschr60m", 6378155.0, 298.3),
            ("fschr68", 6378150.0, 298.3),
            ("helmert", 6378200.0, 298.3),
            ("hough", 6378270.0, 297.0),
            ("intl", 6378388.0, 297.0),
            ("krass", 6378245.0, 298.3),
            ("kaula", 6378163.0, 298.24),
            ("lerch", 6378139.0, 298.257),
            ("mprts", 6397300.0, 191.0),
            ("new_intl", 6378157.5, -6356772.2),
            ("plessis", 6376523.0, -6355863.0),
            ("PZ90", 6378136.0, 298.25784),
            ("SEasia", 6378155.0, -6356773.3205),
            ("walbeck", 6376896.0, -6355834.8467),
            ("WGS60", 6378165.0, 298.3),
            ("WGS66", 6378145.0, 298.25),
            ("WGS72", 6378135.0, 298.26),
            ("WGS84", 6378137.0, 298.257223563),
            ("sphere", 6370997.0, 0.0),
        ];
        let &(_, a, v) = ELLIPSOIDS.iter().find(|e| e.0 == name)?;
        Some(if v < 0.0 {
            Self::from_a_b(a, -v)
        } else {
            Self::from_a_rf(a, v)
        })
    }
}

/// El tamaño en metros de una de las unidades lineales con nombre de PROJ (`+units=`).
pub(crate) fn unit_to_meter(name: &str) -> Option<f64> {
    Some(match name {
        "m" => 1.0,
        "km" => 1000.0,
        "dm" => 0.1,
        "cm" => 0.01,
        "mm" => 0.001,
        "kmi" => 1852.0,
        "in" => 0.0254,
        "ft" => 0.3048,
        "yd" => 0.9144,
        "mi" => 1609.344,
        "fath" => 1.8288,
        "ch" => 20.1168,
        "link" => 0.201168,
        "us-in" => 1.0 / 39.37,
        "us-ft" => 1200.0 / 3937.0,
        "us-yd" => 3600.0 / 3937.0,
        "us-ch" => 79200.0 / 3937.0,
        "us-mi" => 6336000.0 / 3937.0,
        "ind-yd" => 0.91439523,
        "ind-ft" => 0.30479841,
        "ind-ch" => 20.11669506,
        _ => return None,
    })
}

/// La longitud de uno de los meridianos origen con nombre de PROJ (`+pm=`), en grados.
pub(crate) fn prime_meridian(name: &str) -> Option<f64> {
    Some(match name {
        "greenwich" => 0.0,
        "lisbon" => -9.131906111111,
        "paris" => 2.337229166667,
        "bogota" => -74.080916666667,
        "madrid" => -3.687938888889,
        "rome" => 12.452333333333,
        "bern" => 7.439583333333,
        "jakarta" => 106.807719444444,
        "ferro" => -17.666666666667,
        "brussels" => 4.367975,
        "stockholm" => 18.058277777778,
        "athens" => 23.7163375,
        "oslo" => 10.722916666667,
        "copenhagen" => 12.57788,
        _ => return None,
    })
}

/// Envuelve una longitud en radianes en [-π, π].
pub(crate) fn adjlon(lon: f64) -> f64 {
    if lon.abs() <= PI + 1e-12 {
        return lon;
    }
    let l = (lon + PI).rem_euclid(2.0 * PI) - PI;
    if l == -PI && lon > 0.0 { PI } else { l }
}

/// `asin` limitado a [-1, 1].
pub(crate) fn aasin(v: f64) -> f64 {
    v.clamp(-1.0, 1.0).asin()
}

/// `pj_tsfn` de PROJ: la función t de la latitud conforme.
pub(crate) fn tsfn(phi: f64, sinphi: f64, e: f64) -> f64 {
    let c = e * sinphi;
    (0.5 * (FRAC_PI_2 - phi)).tan() / ((1.0 - c) / (1.0 + c)).powf(0.5 * e)
}

/// PROJ's `pj_msfn`: cos φ / sqrt(1 - e² sin² φ).
pub(crate) fn msfn(sinphi: f64, cosphi: f64, es: f64) -> f64 {
    cosphi / (1.0 - es * sinphi * sinphi).sqrt()
}

/// `pj_qsfn` de PROJ, para latitudes auténticas.
pub(crate) fn qsfn(sinphi: f64, e: f64, one_es: f64) -> f64 {
    if e >= 1e-7 {
        let con = e * sinphi;
        let div1 = 1.0 - con * con;
        let div2 = 1.0 + con;
        one_es * (sinphi / div1 - (0.5 / e) * ((1.0 - con) / div2).ln())
    } else {
        sinphi + sinphi
    }
}

/// tan φ a partir de sinh ψ (ψ la latitud isométrica), por el método de Newton (`pj_sinhpsi2tanphi`
/// de PROJ, según Karney).
pub(crate) fn sinhpsi2tanphi(taup: f64, e: f64) -> f64 {
    let rooteps = f64::EPSILON.sqrt();
    let tol = rooteps / 10.0;
    let tmax = 2.0 / rooteps;
    let e2m = 1.0 - e * e;
    let stol = tol * taup.abs().max(1.0);
    let mut tau = if taup.abs() > 70.0 {
        taup * (e * e.atanh()).exp()
    } else {
        taup / e2m
    };
    if tau.abs() >= tmax {
        return tau;
    }
    for _ in 0..5 {
        let tau1 = (1.0 + tau * tau).sqrt();
        let sig = (e * (e * tau / tau1).atanh()).sinh();
        let taupa = (1.0 + sig * sig).sqrt() * tau - sig * tau1;
        let dtau =
            (taup - taupa) * (1.0 + e2m * tau * tau) / (e2m * tau1 * (1.0 + taupa * taupa).sqrt());
        tau += dtau;
        if dtau.abs() < stol {
            break;
        }
    }
    tau
}

/// `pj_phi2` de PROJ: la latitud a partir de la función conforme t.
pub(crate) fn phi2(ts: f64, e: f64) -> f64 {
    sinhpsi2tanphi((1.0 / ts - ts) / 2.0, e).atan()
}

/// Coeficientes de la distancia meridiana (`pj_enfn` de PROJ).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mlfn {
    en: [f64; 5],
    es: f64,
}

impl Mlfn {
    pub fn new(es: f64) -> Self {
        const C00: f64 = 1.0;
        const C02: f64 = 0.25;
        const C04: f64 = 0.046875;
        const C06: f64 = 0.01953125;
        const C08: f64 = 0.01068115234375;
        const C22: f64 = 0.75;
        const C44: f64 = 0.46875;
        const C46: f64 = 0.01302083333333333333;
        const C48: f64 = 0.00712076822916666666;
        const C66: f64 = 0.36458333333333333333;
        const C68: f64 = 0.00569661458333333333;
        const C88: f64 = 0.3076171875;
        let mut en = [0.0; 5];
        en[0] = C00 - es * (C02 + es * (C04 + es * (C06 + es * C08)));
        en[1] = es * (C22 - es * (C04 + es * (C06 + es * C08)));
        let mut t = es * es;
        en[2] = t * (C44 - es * (C46 + es * C48));
        t *= es;
        en[3] = t * (C66 - es * C68);
        en[4] = t * es * C88;
        Self { en, es }
    }

    /// La distancia meridiana desde el ecuador hasta φ, para un semieje mayor de 1.
    pub fn mlfn(&self, phi: f64, sphi: f64, cphi: f64) -> f64 {
        let en = &self.en;
        let cs = cphi * sphi;
        let s2 = sphi * sphi;
        en[0] * phi - cs * (en[1] + s2 * (en[2] + s2 * (en[3] + s2 * en[4])))
    }

    /// La latitud a una distancia meridiana.
    pub fn inv(&self, arg: f64) -> f64 {
        let k = 1.0 / (1.0 - self.es);
        let mut phi = arg;
        for _ in 0..10 {
            let s = phi.sin();
            let t = 1.0 - self.es * s * s;
            let d = (self.mlfn(phi, s, phi.cos()) - arg) * (t * t.sqrt()) * k;
            phi -= d;
            if d.abs() < 1e-11 {
                return phi;
            }
        }
        phi
    }
}

/// Coeficientes que convierten latitudes auténticas en geodésicas, como PROJ desde la 9.7: la serie
/// de Fourier en la latitud auténtica cuyos coeficientes son series de Taylor en el tercer
/// aplanamiento n, hasta orden 6 (Karney, «On auxiliary latitudes», 2024; `pj_auxlat_coeffs`,
/// `C[phi,xi]` de PROJ).
pub(crate) fn authset(es: f64) -> [f64; 6] {
    const C: [f64; 21] = [
        4.0 / 3.0,
        4.0 / 45.0,
        -16.0 / 35.0,
        -2582.0 / 14175.0,
        60136.0 / 467775.0,
        28112932.0 / 212837625.0,
        46.0 / 45.0,
        152.0 / 945.0,
        -11966.0 / 14175.0,
        -21016.0 / 51975.0,
        251310128.0 / 638512875.0,
        3044.0 / 2835.0,
        3802.0 / 14175.0,
        -94388.0 / 66825.0,
        -8797648.0 / 10945935.0,
        6059.0 / 4725.0,
        41072.0 / 93555.0,
        -1472637812.0 / 638512875.0,
        768272.0 / 467775.0,
        455935736.0 / 638512875.0,
        4210684958.0 / 1915538625.0,
    ];
    let b = (1.0 - es).sqrt();
    let n = (1.0 - b) / (1.0 + b);
    let mut apa = [0.0; 6];
    let (mut o, mut d) = (0, n);
    for (l, a) in apa.iter_mut().enumerate() {
        // Un polinomio en n de grado 5 - l, por el método de Horner.
        let p = &C[o..o + 6 - l];
        *a = d * p.iter().rev().fold(0.0, |y, c| y * n + c);
        o += 6 - l;
        d *= n;
    }
    apa
}

/// La latitud geodésica de una latitud auténtica: `beta + Σ apa[k] sin((2k + 2) beta)`, por suma de
/// Clenshaw (`pj_authalic_lat_inverse` de PROJ, para elipsoides de aplanamiento hasta
/// aproximadamente 1/50).
pub(crate) fn authlat(beta: f64, apa: &[f64; 6]) -> f64 {
    let (s, c) = beta.sin_cos();
    let x = 2.0 * (c - s) * (c + s);
    let (mut u0, mut u1) = (0.0, 0.0);
    for a in apa.iter().rev() {
        let t = x * u0 - u1 + a;
        u1 = u0;
        u0 = t;
    }
    beta + 2.0 * s * c * u0
}

/// π/4, para las fórmulas de abajo.
pub(crate) const FORTPI: f64 = FRAC_PI_4;
pub(crate) const HALFPI: f64 = FRAC_PI_2;
