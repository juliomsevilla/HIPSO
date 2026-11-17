//! `hipso`: productos forestales a partir de nubes de puntos LiDAR, desde la línea de comandos.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand, ValueEnum};
mod flow;
use hipso_core::axelsson::AxelssonParams;
use hipso_core::buildings::BuildingParams;
use hipso_core::csf::CsfParams;
use hipso_core::ground::GroundParams;
use hipso_core::mcc::MccParams;
use hipso_core::metrics::MetricParams;
use hipso_core::noise::NoiseParams;
use hipso_core::pipeline::{self, Ground, Noise, Steps, Surface, Tiling};
use hipso_core::pmf::PmfParams;
use hipso_core::rules::ClassRule;
use hipso_core::smrf::SmrfParams;
use hipso_core::trees::{CrownParams, Segmenter, Shape, Smoothing, TopParams};
use hipso_core::{Error, Result, jobs, paths};

#[derive(Parser)]
#[command(
    name = "hipso",
    version,
    about = "Herramientas LiDAR forestales en flujo: clasificación, MDT, modelo de altura de copas, métricas, ráster y vectorial"
)]
struct Cli {
    /// Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la
    /// carpeta de cada entrada
    #[arg(long, global = true)]
    results_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum GroundMethod {
    /// Filtro iterativo de Kraus y Pfeifer (por defecto)
    Kraus,
    /// Multiscale Curvature Classification (Evans & Hudak 2007)
    Mcc,
    /// Cloth Simulation Filter (Zhang et al. 2016)
    Csf,
    /// Progressive Morphological Filter (Zhang et al. 2003)
    Pmf,
    /// Simple Morphological Filter (Pingel et al. 2013)
    Smrf,
    /// Progressive TIN densification (Axelsson 2000)
    Axelsson,
    /// Puntos ya clasificados como suelo (clase 2)
    Classes,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum NoiseMethod {
    /// Puntos aislados en celdas de 3 x 3 x 3
    Isolated,
    /// Eliminación estadística de valores atípicos sobre los k vecinos más próximos
    Sor,
    /// Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido)
    Elm,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ChmMethod {
    /// Punto más alto de cada celda
    Max,
    /// Pila de TIN sin picos (Khosravipour et al., 2014)
    Pitfree,
    /// TIN por el punto más alto de cada celda
    Tin,
    /// Ponderación por el inverso de la distancia del punto más alto de cada celda
    Idw,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ClipModeArg {
    /// Una nube con los puntos de todas las unidades de muestreo
    Merge,
    /// Una nube por unidad de muestreo
    Each,
    /// Sin nubes: solo la capa de unidades de muestreo con sus métricas
    Metrics,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SampleArg {
    /// Parcelas al azar (--count), con una distancia mínima entre centros
    Random,
    /// Parcelas en una cuadrícula regular (--spacing)
    Systematic,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ViewArg {
    /// Planta, vista desde arriba
    Plan,
    #[value(name = "3d")]
    /// Vista 3D, orientada con --yaw y --pitch
    Three,
    /// Perfil de una caja (--profile)
    Profile,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ColorArg {
    /// Elevación, con la rampa azul-rojo
    Elevation,
    /// Elevación, con la rampa de tierra
    Earth,
    /// Clase de cada punto
    Class,
    /// Intensidad
    Intensity,
    /// Número de retorno
    Return,
    /// ID de fuente (la pasada)
    Source,
    /// Archivo de origen
    File,
    /// Color RGB del punto
    Rgb,
    /// Crown number (treeID extra attribute)
    Tree,
    /// Altura sobre el suelo clasificado (clase 2) de la propia nube
    Hag,
    /// Ángulo de barrido, con signo
    ScanAngle,
    /// Color de la clase, más claro o más oscuro según la intensidad
    ClassIntensity,
    /// Rampa de elevación, más clara o más oscura según la intensidad
    ElevationIntensity,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum DupsArg {
    /// No buscar duplicados
    Keep,
    /// Marcarlos como retenidos (withheld), dejando uno de cada conjunto
    Mark,
    /// Quitarlos, dejando uno de cada conjunto
    Delete,
}

/// Los atributos que deben coincidir para que dos puntos sean duplicados.
fn dup_fields(also: &[String]) -> Result<hipso_core::cloudtools::DupFields> {
    let mut f = hipso_core::cloudtools::DupFields::default();
    for a in also {
        match a.as_str() {
            "class" => f.class = true,
            "gps" => f.gps_time = true,
            "intensity" => f.intensity = true,
            "return" => f.return_number = true,
            "count" => f.return_count = true,
            "source" => f.source_id = true,
            o => {
                return Err(hipso_core::Error::Param(format!(
                    "'{o}' no es un atributo: class, gps, intensity, return, count o source"
                )));
            }
        }
    }
    Ok(f)
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ThinArg {
    /// Punto más bajo de cada celda
    Lowest,
    /// Punto más alto de cada celda
    Highest,
    /// Punto más cercano al centro de cada celda
    Nearest,
    /// Punto en el percentil --percentile de las elevaciones de cada celda
    Percentile,
    /// Más cercano al centro de cada celda 3D (--vcell high)
    Voxel,
    /// Uno de cada n puntos (--n)
    Every,
    /// Quita uno de cada n puntos (--n)
    Remove,
    /// Random sample (--fraction)
    Random,
    /// Muestra aleatoria hasta --density puntos por m²
    Density,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum OverlapArg {
    /// El valor del primer ráster de la lista
    First,
    /// El valor del último ráster de la lista
    Last,
    /// La media de los rásteres
    Mean,
    /// El mínimo
    Min,
    /// El máximo
    Max,
    /// La mediana
    Median,
    /// La suma
    Sum,
    /// Cuántos rásteres cubren la celda
    Count,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum TerrainArg {
    /// Pendiente, en grados o en porcentaje (--percent)
    Slope,
    /// Orientación de la ladera, en grados desde el norte
    Aspect,
    /// Sombreado de 0 a 255 con la luz de --azimuth y --altitude
    Hillshade,
    /// Curvatura en planta (1/m)
    PlanCurvature,
    /// Curvatura de perfil (1/m)
    ProfileCurvature,
    /// Curvatura total (1/m)
    Curvature,
    /// Rugosidad: la celda más alta menos la más baja de la ventana de 3×3
    Roughness,
    /// Índice de rugosidad del terreno: media de las diferencias con las ocho vecinas
    Tri,
    /// Posición topográfica: altura menos la media del entorno de radio --radius
    Tpi,
    /// Profundidad de depresiones: lo que hay que rellenar para que el agua salga
    FillDepth,
    /// Acumulación de flujo (D8): superficie que vierte a cada celda, en m²
    FlowAccumulation,
    /// Índice topográfico de humedad ln(a / tan β)
    Twi,
    /// Red de drenaje: las celdas que drenan al menos --threshold m²
    Streams,
    /// Subcuencas: la cuenca de cada tramo de la red de drenaje
    Subbasins,
    /// Cuenca visual de un observador (--observer)
    Viewshed,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum StatArg {
    /// Puntos por m²
    Density,
    /// Número de puntos
    Count,
    /// Elevación media
    MeanZ,
    /// Elevación máxima
    MaxZ,
    /// Elevación mínima
    MinZ,
    /// Diferencia entre la elevación máxima y la mínima
    RangeZ,
    /// Intensidad media
    MeanIntensity,
    /// Intensidad máxima
    MaxIntensity,
}

/// Opciones de selección de puntos comunes a las herramientas que leen la nube entera. Cómo se
/// comprime un ráster escrito por teselas.
#[derive(Args, Clone, Copy)]
struct TileArgs {
    /// Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos
    /// habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits
    /// de 1 o 3 bandas)
    #[arg(long, value_enum, default_value_t = CompressArg::Auto)]
    compress: CompressArg,
    /// Calidad JPEG, de 1 a 100
    #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(1..=100))]
    quality: u8,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum CompressArg {
    /// JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos
    Auto,
    /// Sin pérdidas
    Deflate,
    /// Con pérdidas, para imágenes de 8 bits de una o tres bandas
    Jpeg,
}

impl TileArgs {
    fn compression(self) -> Option<hipso_core::tiled::Compression> {
        use hipso_core::tiled::Compression;
        match self.compress {
            CompressArg::Auto => None,
            CompressArg::Deflate => Some(Compression::Deflate),
            CompressArg::Jpeg => Some(Compression::Jpeg(self.quality)),
        }
    }
}

#[derive(Args)]
struct SelArgs {
    /// Clases que se conservan: 2,5 o ~7,18 para todas menos esas
    #[arg(long)]
    class: Option<String>,
    /// Retornos que se conservan: A, F, L o números (p. ej. 1,2)
    #[arg(long)]
    returns: Option<String>,
    /// Elevación mínima (m) de los puntos que se usan
    #[arg(long, allow_negative_numbers = true)]
    zmin: Option<f64>,
    /// Elevación máxima (m) de los puntos que se usan
    #[arg(long, allow_negative_numbers = true)]
    zmax: Option<f64>,
    /// Conserva un punto de cada tantos
    #[arg(long, default_value_t = 1)]
    decimate: u64,
}

impl SelArgs {
    fn filter(&self) -> Result<hipso_core::filter::Filter> {
        use hipso_core::filter::{ClassSel, Filter, ReturnSel};
        Ok(Filter {
            classes: self
                .class
                .as_deref()
                .map_or(Ok(ClassSel::all()), ClassSel::parse)?,
            returns: self
                .returns
                .as_deref()
                .map_or(Ok(ReturnSel::all()), ReturnSel::parse)?,
            zmin: self.zmin,
            zmax: self.zmax,
            decimate: self.decimate,
            skip_noise: false,
        })
    }
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SegArg {
    /// Crecimiento de regiones desde las cimas (Dalponte y Coomes, 2016)
    Dalponte,
    /// Polígonos de Voronoi acotados por la altura de cada árbol (Silva et al., 2016)
    Silva,
    /// Cuencas (watershed) sobre el CHM invertido
    Watershed,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum FormatArg {
    /// Solo shapefile
    Vector,
    /// Solo tabla de texto (lo mejor para ejecuciones muy grandes)
    Csv,
    /// Both
    Both,
}

#[derive(Clone, Copy, ValueEnum)]
enum FloodRelative {
    /// Sobre la celda más baja del modelo
    Min,
    /// Sobre la elevación de la primera semilla
    Seed,
}

#[derive(Clone, Copy, ValueEnum)]
enum CalcDtype {
    /// Decimal de 32 bits
    F32,
    /// Entero sin signo de 8 bits
    U8,
    /// Entero sin signo de 16 bits
    U16,
}

#[derive(Clone, Copy, ValueEnum)]
enum AlignResample {
    /// Según los datos: vecino más cercano (o moda al reducir) en rásteres de enteros; media si las celdas de salida son mayores que las de entrada; bilineal en los demás casos
    Auto,
    /// Para clases e identificadores
    Nearest,
    /// Interpolación bilineal (superficies continuas)
    Bilinear,
    /// Media de las celdas que cubre cada celda nueva
    Average,
    /// Cubic convolution (smooth surfaces)
    Cubic,
    /// Valor más frecuente (clases, al reducir)
    Mode,
}

#[derive(Clone, Copy, ValueEnum)]
enum ObiaMethod {
    /// Fusión multirresolución de manchas
    Merge,
    /// Celdas conectadas con un valor dentro de un rango
    Mask,
}

/// Los formatos pedidos, o los que implica el nombre de la salida (`.csv`: texto; si no,
/// vectorial).
fn formats_of(arg: Option<FormatArg>, path: &Path) -> paths::Formats {
    match arg {
        Some(FormatArg::Vector) => paths::Formats {
            vector: true,
            csv: false,
        },
        Some(FormatArg::Csv) => paths::Formats {
            vector: false,
            csv: true,
        },
        Some(FormatArg::Both) => paths::Formats {
            vector: true,
            csv: true,
        },
        None => paths::Formats::from_path(path),
    }
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum TransformArg {
    /// Sin transformación
    None,
    /// Logaritmo de la respuesta
    LogResponse,
    /// Logaritmo de la respuesta y de las métricas (una ley de potencias)
    LogBoth,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum CriterionArg {
    /// Criterio de información bayesiano (BIC): castiga más los modelos con muchas métricas
    Bic,
    /// Criterio de Akaike corregido para muestras pequeñas (AICc)
    Aicc,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SmoothArg {
    /// Media de la ventana
    Mean,
    /// Mediana de la ventana
    Median,
    /// Suavizado gaussiano
    Gaussian,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ShapeArg {
    /// Ventana circular
    Circle,
    /// Ventana cuadrada
    Square,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SurfaceMethod {
    /// Elevación media de los puntos de suelo por celda (en flujo, una pasada)
    Mean,
    /// Triangulación de Delaunay (TIN) de los puntos de suelo (por defecto)
    Tin,
    /// Ponderación por el inverso de la distancia de los puntos de suelo más próximos
    Idw,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum DsmArg {
    /// Punto más alto de cada celda (en flujo, una pasada)
    Max,
    /// TIN por el punto más alto de cada celda (por defecto)
    Tin,
    /// Ponderación por el inverso de la distancia de los puntos más altos de las celdas
    Idw,
}

#[derive(Args)]
struct GroundArgs {
    /// Cómo se encuentran los puntos de suelo (classify: kraus; dtm: classes, la clase 2 que ya
    /// trae la nube)
    #[arg(long, value_enum)]
    ground: Option<GroundMethod>,
    /// Kraus-Pfeifer: tamaño de celda de las superficies intermedias del filtro
    #[arg(long, default_value_t = 5.0)]
    filter_cell: f64,
    /// Kraus-Pfeifer: número de iteraciones del filtro
    #[arg(long, default_value_t = 5)]
    iterations: u32,
    /// Kraus-Pfeifer: desplazamiento g de la función de peso (los residuos por debajo pesan 1);
    /// también --kp-g
    #[arg(long, visible_alias = "kp-g", default_value_t = -2.0, allow_negative_numbers = true)]
    g: f64,
    /// Kraus-Pfeifer: anchura w de la función de peso por encima de g (los residuos por encima de g
    /// + w pesan 0); también --kp-w
    #[arg(long, visible_alias = "kp-w", default_value_t = 2.5)]
    w: f64,
    /// Kraus-Pfeifer: pendiente a de la función de peso; también --kp-a
    #[arg(long, visible_alias = "kp-a", default_value_t = 1.0)]
    a: f64,
    /// Kraus-Pfeifer: pendiente b de la función de peso; también --kp-b
    #[arg(long, visible_alias = "kp-b", default_value_t = 4.0)]
    b: f64,
    /// Kraus-Pfeifer: suelo = a esta distancia de la superficie final (por defecto: residuo <= g +
    /// w)
    #[arg(long)]
    tolerance: Option<f64>,
    /// MCC: tamaño de celda del dominio de escala 2 (aproximadamente el espaciado de los puntos)
    #[arg(long, default_value_t = 1.5)]
    mcc_spacing: f64,
    /// MCC: umbral de curvatura (m) por encima del cual un punto deja de ser suelo
    #[arg(long, default_value_t = 0.3)]
    mcc_threshold: f64,
    /// CSF: distancia entre las partículas de la tela
    #[arg(long, default_value_t = 0.5)]
    csf_resolution: f64,
    /// CSF: rigidez, de 1 (terreno escarpado) a 3 (llano)
    #[arg(long, default_value_t = 1)]
    csf_rigidness: u8,
    /// CSF: altura sobre la tela hasta la que los puntos son suelo
    #[arg(long, default_value_t = 0.5)]
    csf_threshold: f64,
    /// PMF: mayor tamaño de ventana (m) de la apertura morfológica progresiva
    #[arg(long, default_value_t = 20.0)]
    pmf_window: f64,
    /// PMF: pendiente del terreno (subida sobre recorrido) que tolera el filtro
    #[arg(long, default_value_t = 1.0)]
    pmf_slope: f64,
    /// SMRF: lado de celda (m) de las superficies que se comparan
    #[arg(long, default_value_t = 1.0)]
    smrf_cell: f64,
    /// SMRF: mayor radio de ventana (mayor que el edificio más grande)
    #[arg(long, default_value_t = 18.0)]
    smrf_window: f64,
    /// SMRF: pendiente más empinada del terreno (subida sobre recorrido)
    #[arg(long, default_value_t = 0.15)]
    smrf_slope: f64,
    /// SMRF: umbral de altura entre un punto y la superficie
    #[arg(long, default_value_t = 0.5)]
    smrf_threshold: f64,
    /// SMRF: se suma al umbral por unidad de pendiente
    #[arg(long, default_value_t = 1.25)]
    smrf_scalar: f64,
    /// Axelsson: lado de las celdas cuyos puntos más bajos siembran el TIN (mayor que cualquier
    /// edificio)
    #[arg(long, default_value_t = 20.0)]
    axelsson_seed_cell: f64,
    /// Axelsson: ángulo máximo (grados) entre un punto y el plano de la triangulación para aceptarlo como suelo
    #[arg(long, default_value_t = 8.0)]
    axelsson_angle: f64,
    /// Axelsson: distancia máxima (m) de un punto al plano de la triangulación para aceptarlo como suelo
    #[arg(long, default_value_t = 1.4)]
    axelsson_dist: f64,
    /// Axelsson: los puntos a esta distancia (m) de un triángulo son suelo sea cual sea el ángulo
    #[arg(long, default_value_t = 0.10)]
    axelsson_flat: f64,
}

fn parse_rule(text: &str) -> std::result::Result<ClassRule, String> {
    ClassRule::parse(text).map_err(|e| e.to_string())
}

#[derive(Args)]
struct StepArgs {
    #[command(flatten)]
    ground: GroundArgs,
    /// Aparta antes que nada, como clase 12 (solape), los puntos de las pasadas que solapan con
    /// otra: donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se la queda
    #[arg(long)]
    overlap: bool,
    /// Solape: lado de celda en metros
    #[arg(long, default_value_t = 5.0)]
    overlap_cell: f64,
    /// Clasifica primero el ruido (clase 7) con este método
    #[arg(long, value_enum)]
    noise: Option<NoiseMethod>,
    /// Ruido (sor): número de vecinos con los que se calcula la distancia media de cada punto
    #[arg(long, default_value_t = 10)]
    sor_k: usize,
    /// Ruido (sor): desviaciones típicas por encima de la distancia media
    #[arg(long, default_value_t = 3.0)]
    sor_m: f64,
    /// Ruido (isolated): tamaño de celda en x e y
    #[arg(long, default_value_t = 4.0)]
    noise_step: f64,
    /// Ruido (isolated): tamaño de celda en z
    #[arg(long, default_value_t = 4.0)]
    noise_step_z: f64,
    /// Ruido (elm): lado de celda en metros
    #[arg(long, default_value_t = 10.0)]
    elm_cell: f64,
    /// Ruido (elm): metros por debajo del siguiente punto de su celda y de los puntos más bajos de
    /// las celdas vecinas que debe estar el punto más bajo de una celda para ser ruido bajo
    #[arg(long, default_value_t = 3.0)]
    elm_threshold: f64,
    /// Ruido: el mayor número de otros puntos alrededor de un punto que aún se llama aislado
    #[arg(long, default_value_t = 5)]
    isolated: u32,
    /// Clasifica la vegetación por altura sobre el suelo: baja (3), media (4) y alta (5)
    #[arg(long)]
    vegetation: bool,
    /// Clasifica los cables (14): líneas casi horizontales muy por encima del suelo
    #[arg(long)]
    wires: bool,
    /// Clasifica los postes y torres (15): estructuras estrechas y aisladas que se elevan del suelo
    #[arg(long)]
    poles: bool,
    /// Cables: altura mínima sobre el suelo (m)
    #[arg(long, default_value_t = 5.0)]
    wire_min_height: f64,
    /// Cables: radio del entorno con el que se mira la forma de cada punto (m; 0: según el espaciado
    /// de los puntos)
    #[arg(long, default_value_t = 0.0)]
    wire_radius: f64,
    /// Cables: linealidad mínima del entorno (0 a 1)
    #[arg(long, default_value_t = 0.7)]
    wire_linearity: f64,
    /// Cables: mayor pendiente de la línea (0 horizontal, 1 vertical)
    #[arg(long, default_value_t = 0.5)]
    wire_max_slope: f64,
    /// Cables: mayor número de puntos alrededor de un punto de cable
    #[arg(long, default_value_t = 1000)]
    wire_max_neighbours: usize,
    /// Cables: mayor anchura del entorno a través de la línea (m; 0: según el radio)
    #[arg(long, default_value_t = 0.0)]
    wire_max_width: f64,
    /// Cables: longitud mínima de una cadena de puntos de cable (m)
    #[arg(long, default_value_t = 10.0)]
    wire_min_length: f64,
    /// Cables: lo recta que debe ser una cadena (longitud sobre anchura)
    #[arg(long, default_value_t = 10.0)]
    wire_min_straightness: f64,
    /// Cables: lado de los vóxeles a los que se reducen los puntos antes de mirar su forma (m)
    #[arg(long, default_value_t = 0.05)]
    wire_voxel: f64,
    /// Cables: mayor hueco (m) que se salva al unir trozos de conductor en fila (0: no se unen)
    #[arg(long, default_value_t = 15.0)]
    wire_link_gap: f64,
    /// Cables: lado (m) de los vóxeles con que se buscan trozos finos y sueltos de conductor (0: no
    /// se buscan)
    #[arg(long, default_value_t = 0.5)]
    wire_piece_voxel: f64,
    /// Cables: longitud mínima de un trozo fino de conductor (m)
    #[arg(long, default_value_t = 1.2)]
    wire_piece_min_length: f64,
    /// Cables: mayor desviación típica (m) de un trozo fino a través de su dirección
    #[arg(long, default_value_t = 0.12)]
    wire_piece_max_width: f64,
    /// Cables: mínimo de puntos de un trozo fino de conductor
    #[arg(long, default_value_t = 6)]
    wire_piece_min_points: usize,
    /// Cables: un trozo fino de conductor cuelga solo: a menos de esta distancia (m) no hay más que
    /// unos pocos vóxeles de otros grupos (0: no se comprueba)
    #[arg(long, default_value_t = 1.0)]
    wire_piece_isolation: f64,
    /// Cables: cuántos vóxeles de otros grupos tolera un trozo fino a esa distancia
    #[arg(long, default_value_t = 12)]
    wire_piece_max_others: usize,
    /// Postes: altura mínima sobre el suelo de los puntos de una estructura (m)
    #[arg(long, default_value_t = 2.0)]
    pole_min_height: f64,
    /// Postes: los puntos más cercanos que esto pertenecen a la misma estructura (m)
    #[arg(long, default_value_t = 1.5)]
    pole_radius: f64,
    /// Postes: mayor anchura de la estructura (m)
    #[arg(long, default_value_t = 8.0)]
    pole_max_width: f64,
    /// Postes: lo mínimo que debe elevarse la estructura (m)
    #[arg(long, default_value_t = 6.0)]
    pole_min_extent: f64,
    /// Postes: longitud (m) de la cabeza de una torre vista como una cruceta aislada (0: no se busca)
    #[arg(long, default_value_t = 12.0)]
    pole_head_length: f64,
    /// Postes: lo que debe elevarse un poste fino (m; 0: no se buscan)
    #[arg(long, default_value_t = 3.0)]
    pole_thin_extent: f64,
    /// Postes: con cables en la tesela, solo cuentan las estructuras a menos de esta distancia (m)
    /// de un cable, y se buscan las torres de la línea (0: no se usan los cables)
    #[arg(long, default_value_t = 10.0)]
    pole_wire_reach: f64,
    /// Clasifica las carreteras (11): suelo liso, llano y oscuro (intensidad baja)
    #[arg(long)]
    roads: bool,
    /// Clasifica el agua (9): regiones extensas y niveladas de suelo oscuro de retorno único
    #[arg(long)]
    water: bool,
    /// Clasifica por reglas, al final y en orden. Se repite para varias reglas. Partes, separadas
    /// por `;`: to=CLASE (obligatoria), from=1,2 (o ~7), returns=F|L|A|1..9, angle=25..90 (grados
    /// absolutos; `angle=25` es 25 o más), height=0.5..2 (m sobre el suelo), intensity=0..100 y la
    /// forma del entorno dentro de radius=1 (m): slope=60.. (grados), planarity, linearity,
    /// sphericity (0 a 1), roughness (m), neighbours (número). Por ejemplo: `angle=25;to=12` o
    /// `from=1;height=0.5..2;to=4`
    #[arg(long = "rule", value_parser = parse_rule)]
    rules: Vec<ClassRule>,
    /// Clasifica edificios (6) y vegetación alta (5), y quita los tejados del suelo
    #[arg(long)]
    buildings: bool,
    /// Lee más de estas opciones de un archivo de texto, una o varias por línea (`#` inicia una
    /// línea de comentario): una clasificación escrita una vez y ejecutada en cualquier hoja o
    /// bloque. Las opciones del archivo van donde está `--recipe`, de modo que no pueden repetirse
    /// en la línea de comandos, salvo `--rule`
    #[arg(long, value_name = "FILE")]
    #[allow(dead_code)] // se lee antes de analizar, en expand_recipes
    recipe: Option<PathBuf>,
    /// Edificios: altura sobre el terreno a partir de la cual los puntos son edificios o árboles
    #[arg(long, default_value_t = 2.5)]
    building_offset: f64,
    /// Edificios: el mayor edificio esperado (m); fija el tamaño de la ventana con la que se separa el suelo de los tejados
    #[arg(long, default_value_t = 60.0)]
    building_window: f64,
    /// Edificios: la menor planta (en unidades cuadradas del mapa) que se acepta como edificio
    #[arg(long, default_value_t = 40.0)]
    building_area: f64,
    /// Lado (m) de las teselas en que se divide cada hoja para clasificarla en paralelo
    #[arg(long, default_value_t = 250.0)]
    tile: f64,
    /// Margen que se lee alrededor de cada tesela (elevado a la mitad de la ventana de edificios al
    /// clasificar edificios)
    #[arg(long, default_value_t = 30.0)]
    buffer: f64,
    /// Hilos (0: uno por núcleo)
    #[arg(long, default_value_t = 0)]
    threads: usize,
}

impl StepArgs {
    /// Lo que distingue los productos de estos ajustes en el nombre de un archivo.
    fn tag(&self) -> String {
        self.tag_with(GroundMethod::Kraus)
    }

    fn tag_with(&self, default: GroundMethod) -> String {
        let g = match self.ground.ground.unwrap_or(default) {
            GroundMethod::Kraus => "",
            GroundMethod::Mcc => "_mcc",
            GroundMethod::Csf => "_csf",
            GroundMethod::Pmf => "_pmf",
            GroundMethod::Smrf => "_smrf",
            GroundMethod::Axelsson => "_axelsson",
            GroundMethod::Classes => "_clase2",
        };
        let n = match self.noise {
            None => "",
            Some(NoiseMethod::Isolated) => "_aislados",
            Some(NoiseMethod::Sor) => "_sor",
            Some(NoiseMethod::Elm) => "_elm",
        };
        format!(
            "{g}{n}{}{}",
            if self.buildings { "_edif" } else { "" },
            self.extras().tag()
        )
    }

    fn steps(&self) -> Steps {
        self.steps_with(GroundMethod::Kraus)
    }

    fn extras(&self) -> hipso_core::extras::Extras {
        use hipso_core::extras::{Extras, PoleParams, RoadParams, VegParams, WaterParams, WireParams};
        Extras {
            vegetation: self.vegetation.then(VegParams::default),
            wires: self.wires.then(|| WireParams {
                min_height: self.wire_min_height,
                radius: self.wire_radius,
                linearity: self.wire_linearity,
                max_slope: self.wire_max_slope,
                max_neighbours: self.wire_max_neighbours,
                max_width: self.wire_max_width,
                min_length: self.wire_min_length,
                min_straightness: self.wire_min_straightness,
                voxel: self.wire_voxel,
                link_gap: self.wire_link_gap,
                piece_voxel: self.wire_piece_voxel,
                piece_min_length: self.wire_piece_min_length,
                piece_max_width: self.wire_piece_max_width,
                piece_min_points: self.wire_piece_min_points,
                piece_isolation: self.wire_piece_isolation,
                piece_max_others: self.wire_piece_max_others,
            }),
            poles: self.poles.then(|| PoleParams {
                min_height: self.pole_min_height,
                radius: self.pole_radius,
                max_width: self.pole_max_width,
                min_extent: self.pole_min_extent,
                head_length: self.pole_head_length,
                thin_extent: self.pole_thin_extent,
                wire_reach: self.pole_wire_reach,
            }),
            roads: self.roads.then(RoadParams::default),
            water: self.water.then(WaterParams::default),
        }
    }

    fn steps_with(&self, default: GroundMethod) -> Steps {
        let g = &self.ground;
        Steps {
            overlap: self.overlap.then_some(self.overlap_cell),
            noise: self.noise.map(|m| match m {
                NoiseMethod::Isolated => Noise::Isolated(NoiseParams {
                    step_xy: self.noise_step,
                    step_z: self.noise_step_z,
                    isolated: self.isolated,
                }),
                NoiseMethod::Sor => Noise::Sor {
                    k: self.sor_k,
                    m: self.sor_m,
                },
                NoiseMethod::Elm => Noise::Elm {
                    cell: self.elm_cell,
                    threshold: self.elm_threshold,
                },
            }),
            ground: match g.ground.unwrap_or(default) {
                GroundMethod::Kraus => Ground::Fusion(self.ground_params()),
                GroundMethod::Mcc => Ground::Mcc(MccParams {
                    spacing: g.mcc_spacing,
                    threshold: g.mcc_threshold,
                }),
                GroundMethod::Csf => Ground::Csf(CsfParams {
                    resolution: g.csf_resolution,
                    rigidness: g.csf_rigidness,
                    threshold: g.csf_threshold,
                    ..Default::default()
                }),
                GroundMethod::Pmf => Ground::Pmf(PmfParams {
                    max_window: g.pmf_window,
                    slope: g.pmf_slope,
                    ..Default::default()
                }),
                GroundMethod::Smrf => Ground::Smrf(SmrfParams {
                    cell: g.smrf_cell,
                    max_window: g.smrf_window,
                    slope: g.smrf_slope,
                    threshold: g.smrf_threshold,
                    scalar: g.smrf_scalar,
                }),
                GroundMethod::Axelsson => Ground::Axelsson(AxelssonParams {
                    seed_cell: g.axelsson_seed_cell,
                    iter_angle: g.axelsson_angle,
                    iter_dist: g.axelsson_dist,
                    flat_tolerance: g.axelsson_flat,
                    ..Default::default()
                }),
                GroundMethod::Classes => Ground::Classes,
            },
            buildings: self.buildings.then(|| BuildingParams {
                offset: self.building_offset,
                window: self.building_window,
                min_area: self.building_area,
                ..Default::default()
            }),
            extras: self.extras(),
            rules: self.rules.clone(),
        }
    }

    fn ground_params(&self) -> GroundParams {
        let g = &self.ground;
        GroundParams {
            filter_cell: g.filter_cell,
            g: g.g,
            w: g.w,
            a: g.a,
            b: g.b,
            iterations: g.iterations,
            tolerance: g.tolerance,
        }
    }

    fn tiling(&self) -> Tiling {
        let buffer = if self.buildings {
            self.buffer.max(self.building_window / 2.0)
        } else {
            self.buffer
        };
        Tiling {
            size: self.tile,
            buffer,
            threads: self.threads,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Informe completo de archivos LAS/LAZ: cabecera, clases, retornos, densidad, avisos
    Info {
        /// Archivos LAS o LAZ de los que se saca el informe
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Revisa una entrega de hojas solo con sus cabeceras (no se lee ningún punto): mismo sistema
    /// de coordenadas, hojas repetidas, solapes, huecos dentro del bloque. Una carpeta equivale a
    /// sus LAS/LAZ.
    Catalog {
        /// Archivos LAS/LAZ (o carpetas con ellos) de la entrega que se revisa
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Cuánto (m) pueden solaparse las hojas o quedarse cortas unas de otras sin que cuente
        #[arg(long, default_value_t = 1.0)]
        tolerance: f64,
    },
    /// Clasifica ruido, suelo y edificios, tesela a tesela en paralelo, y escribe los puntos
    Classify {
        /// Nubes LAS o LAZ que clasificar; cada una es una hoja
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// LAS de salida, o LAZ si termina en .laz; por defecto:
        /// <resultados>/<entrada>_clasif<ajustes>
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que
        /// sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la
        /// propia hoja (0: desactivado)
        #[arg(long, default_value_t = 0.0)]
        neighbors: f64,
        #[command(flatten)]
        steps: StepArgs,
    },
    /// MDT como GeoTIFF, uno por hoja. Por defecto: la clase de suelo (2) que ya trae la nube;
    /// --ground prueba otro método
    Dtm {
        /// Nubes LAS o LAZ de las que se calcula el MDT; cada una da un GeoTIFF
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// GeoTIFF de salida; por defecto: <resultados>/<entrada>_mdt_<celda>m<ajustes>.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tamaño de celda del MDT
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Cómo reciben su elevación las celdas
        #[arg(long, value_enum, default_value_t = SurfaceMethod::Tin)]
        surface: SurfaceMethod,
        /// Suaviza el MDT con una ventana de este tamaño en unidades del mapa (0: nada)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// Suavizado del modelo: media, mediana o gaussiano
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Une además los MDT de las hojas en un mosaico (por defecto cada hoja conserva el suyo)
        #[arg(long)]
        mosaic: bool,
        /// Líneas de ruptura que debe seguir el MDT (shapefile de líneas o polígonos): con
        /// elevaciones (PolylineZ) fijan la altura; sin ellas, toman el suelo más bajo a su lado
        #[arg(long)]
        breaklines: Option<PathBuf>,
        /// Hace el MDT de cada hoja con las hojas que estén a menos de tantos metros de ella,
        /// recortado a su propia extensión: sin costura en los bordes de las hojas (0: cada hoja
        /// por separado)
        #[arg(long, default_value_t = 0.0)]
        neighbors: f64,
        #[command(flatten)]
        steps: StepArgs,
    },
    /// Modelo de altura de copas como GeoTIFF, uno por hoja
    Chm {
        /// Nubes LAS o LAZ de las que se calcula el CHM; cada una da un GeoTIFF
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con
        /// esta misma nube
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// GeoTIFF de salida; por defecto: <resultados>/<entrada>_chm_<celda>m.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tamaño de celda del CHM (m)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Deja vacías las celdas sin puntos (método max)
        #[arg(long)]
        no_fill: bool,
        /// Punto más alto por celda, o pila de TIN sin picos (Khosravipour et al., 2014)
        #[arg(long, value_enum, default_value_t = ChmMethod::Max)]
        method: ChmMethod,
        /// Sin picos: arista de triángulo más larga en las capas por encima de 0
        #[arg(long, default_value_t = 1.0)]
        max_edge: f64,
        /// Sin picos: tamaño de tesela
        #[arg(long, default_value_t = 250.0)]
        tile: f64,
        /// Sin picos: hilos (0: uno por núcleo)
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Suaviza el CHM con una ventana de este tamaño en unidades del mapa (0: nada)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// Suavizado del modelo: media, mediana o gaussiano
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Une además los CHM de las hojas en un mosaico (por defecto cada hoja conserva el suyo)
        #[arg(long)]
        mosaic: bool,
        /// Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que
        /// sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la
        /// propia hoja (0: desactivado)
        #[arg(long, default_value_t = 0.0)]
        neighbors: f64,
    },
    /// Modelo digital de superficie (retornos más altos) como GeoTIFF, uno por hoja
    Dsm {
        /// Nubes LAS o LAZ de las que se calcula el MDS; cada una da un GeoTIFF
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// GeoTIFF de salida (solo una entrada); por defecto:
        /// <resultados>/<entrada>_mds_<celda>m_<método>.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tamaño de celda del MDS (m)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Cómo se calcula la superficie: punto más alto por celda o TIN de los primeros retornos
        #[arg(long, value_enum, default_value_t = DsmArg::Tin)]
        method: DsmArg,
        /// Deja vacías las celdas sin puntos (método max)
        #[arg(long)]
        no_fill: bool,
        /// Tamaño de tesela (métodos tin e idw)
        #[arg(long, default_value_t = 250.0)]
        tile: f64,
        /// Hilos (0: uno por núcleo)
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Suaviza con una ventana de este tamaño en unidades del mapa (0: nada)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// Suavizado del modelo: media, mediana o gaussiano
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que
        /// sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la
        /// propia hoja (0: desactivado)
        #[arg(long, default_value_t = 0.0)]
        neighbors: f64,
    },
    /// Árboles individuales a partir de un modelo de altura de copas: cimas (máximos locales),
    /// copas (Dalponte, Silva o cuenca), polígonos, treeID por punto y comprobación con datos de
    /// campo
    Trees {
        /// Modelo de altura de copas (GeoTIFF o rejilla ASCII)
        chm: PathBuf,
        /// Modelos de copas de las hojas vecinas de esta (se repite para cada una): los árboles se
        /// buscan en esta hoja ampliada con --margin con lo que tienen allí, y solo se conservan
        /// los árboles con su cima en esta hoja, de modo que un árbol en el borde de una hoja se
        /// encuentra una sola vez y entero
        #[arg(long)]
        around: Vec<PathBuf>,
        /// Hasta dónde (m) se mira en las hojas vecinas
        #[arg(long, default_value_t = 30.0)]
        margin: f64,
        /// Cimas de los árboles (.shp, o .csv si se da uno); por defecto:
        /// <resultados>/<chm>_arboles_<método>.shp (nunca un archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Escribe además el ráster de copas (identificadores de árbol)
        #[arg(long)]
        crowns: bool,
        /// Escribe además los polígonos de copa (shapefile)
        #[arg(long)]
        crowns_shp: bool,
        /// Escribe además las cimas de los árboles (shapefile PointZ)
        #[arg(long)]
        tops_shp: bool,
        /// Copia esta nube añadiendo un atributo treeID (LAS/LAZ), en <resultados>/<nube>_treeid
        #[arg(long)]
        cloud: Option<PathBuf>,
        /// Cimas dadas a mano: CSV con líneas x,y (en lugar de buscarlas)
        #[arg(long)]
        tops_file: Option<PathBuf>,
        /// Altura mínima (m) de una cima para contarla como árbol
        #[arg(long, default_value_t = 2.0)]
        hmin: f64,
        /// Diámetro de ventana = a + b h + c h² (unidades del mapa)
        #[arg(long, default_value_t = 3.0)]
        ws_a: f64,
        /// Coeficiente de la altura (b) del diámetro de la ventana de búsqueda de cimas
        #[arg(long, default_value_t = 0.1)]
        ws_b: f64,
        /// Coeficiente del cuadrado de la altura (c) del diámetro de la ventana de búsqueda de cimas
        #[arg(long, default_value_t = 0.0)]
        ws_c: f64,
        /// Forma de la ventana de búsqueda de cimas
        #[arg(long, value_enum, default_value_t = ShapeArg::Circle)]
        shape: ShapeArg,
        /// Método con el que se reparten las copas entre las cimas: Dalponte, Silva o cuenca
        #[arg(long, value_enum, default_value_t = SegArg::Dalponte)]
        method: SegArg,
        /// Dalponte: un píxel se une por encima de esta proporción de la altura de su cima...
        #[arg(long, default_value_t = 0.45)]
        th_seed: f64,
        /// ...y de la altura media de la copa
        #[arg(long, default_value_t = 0.55)]
        th_cr: f64,
        /// Dalponte: alcance de la copa en píxeles
        #[arg(long, default_value_t = 10.0)]
        max_cr: f64,
        /// Silva: radio de la copa como proporción de la altura del árbol
        #[arg(long, default_value_t = 0.6)]
        max_cr_factor: f64,
        /// Silva: descarta los píxeles más bajos que esta proporción de la altura del árbol
        #[arg(long, default_value_t = 0.3)]
        exclusion: f64,
        /// Cuenca: escalón mínimo de altura entre copas
        #[arg(long, default_value_t = 1.0)]
        tolerance: f64,
        /// Cuenca: radio de búsqueda de picos en píxeles
        #[arg(long, default_value_t = 1)]
        ext: usize,
        /// Suaviza antes el CHM con una ventana de este tamaño (unidades del mapa; 0: nada)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// Suavizado del CHM antes de buscar cimas: media, mediana o gaussiano
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Inventario de campo (delimitado por tabuladores, ; o ,, se admiten comas decimales) con
        /// el que comprobar
        #[arg(long)]
        field: Option<PathBuf>,
        /// Columna del inventario de campo con la coordenada X del árbol
        #[arg(long, default_value = "X_arbol")]
        field_x: String,
        /// Columna del inventario de campo con la coordenada Y del árbol
        #[arg(long, default_value = "Y_arbol")]
        field_y: String,
        /// Columna del inventario de campo con la altura del árbol (m)
        #[arg(long, default_value = "Ht")]
        field_h: String,
        /// Columna del inventario de campo con el nombre de la parcela
        #[arg(long, default_value = "Muestra")]
        field_plot: String,
        /// Columna del inventario de campo con la coordenada X del centro de la parcela
        #[arg(long, default_value = "X_Muestra")]
        plot_x: String,
        /// Columna del inventario de campo con la coordenada Y del centro de la parcela
        #[arg(long, default_value = "Y_Muestra")]
        plot_y: String,
        /// Radio de la parcela para la comprobación
        #[arg(long, default_value_t = 10.0)]
        plot_radius: f64,
        /// Formato de salida: shapefile de cimas, tabla de texto o ambos
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// Una detección coincide con un árbol de campo a esta distancia como máximo
        #[arg(long, default_value_t = 3.0)]
        match_dist: f64,
    },
    /// Análisis basado en objetos: agrupa las celdas de varias capas (altura de copas, intensidad,
    /// retornos, rugosidad…) en objetos, los describe y clasifica, y escribe polígonos
    Obia {
        /// Rásteres sobre las mismas celdas (GeoTIFF), el modelo de altura de copas el primero
        #[arg(required = true)]
        rasters: Vec<PathBuf>,
        /// Nubes de puntos de las que derivar capas (véase --cloud-layers)
        #[arg(long, num_args = 1..)]
        cloud: Vec<PathBuf>,
        /// Capas que se hacen con las nubes: intensidad, retornos, suelo, densidad
        #[arg(long, value_delimiter = ',', default_value = "intensidad,retornos")]
        cloud_layers: Vec<String>,
        /// Las celdas vacías de las capas hechas con las nubes toman el valor de la celda más
        /// cercana que lo tenga, hasta tantas celdas de distancia (0: quedan vacías y dejan huecos)
        #[arg(long, default_value_t = 3)]
        cloud_fill: usize,
        /// Promedia esas capas en una ventana de este radio en celdas (0: no)
        #[arg(long, default_value_t = 1)]
        cloud_smooth: usize,
        /// Añade la rugosidad local del primer ráster en una ventana de tantas celdas (0: no)
        #[arg(long, default_value_t = 0)]
        roughness: usize,
        /// Cómo se hacen los objetos
        #[arg(long, value_enum, default_value_t = ObiaMethod::Merge)]
        method: ObiaMethod,
        /// Fusión: las manchas se fusionan mientras el coste sea menor que la escala al cuadrado
        /// (mayor: objetos mayores)
        #[arg(long, default_value_t = 8.0)]
        scale: f64,
        /// Fusión: peso de la forma frente al valor, de 0 a 0,9
        #[arg(long, default_value_t = 0.1)]
        shape: f64,
        /// Fusión: peso de la compacidad frente a la suavidad dentro de la forma, de 0 a 1
        #[arg(long, default_value_t = 0.5)]
        compact: f64,
        /// Fusión: peso de una capa, como capa=peso (0 la deja fuera); se repite para varias
        #[arg(long = "weight")]
        weights: Vec<String>,
        /// Fusión: anchura, en desviaciones típicas, de los intervalos de las manchas iniciales
        #[arg(long, default_value_t = 0.25)]
        init: f64,
        /// Máscara: la capa y el rango de valores que forman un objeto
        #[arg(long)]
        mask_layer: Option<String>,
        /// Máscara: valor mínimo de la capa elegida que forma un objeto
        #[arg(long, default_value_t = f32::NEG_INFINITY, allow_hyphen_values = true)]
        min: f32,
        /// Máscara: valor máximo de la capa elegida que forma un objeto
        #[arg(long, default_value_t = f32::INFINITY)]
        max: f32,
        /// Máscara: las celdas que se tocan por una esquina van juntas
        #[arg(long)]
        diagonal: bool,
        /// Objeto más pequeño, en celdas
        #[arg(long, default_value_t = 10)]
        min_cells: u32,
        /// Clasifica con reglas de un archivo de texto (clase: atributo >= valor y ...)
        #[arg(long, conflicts_with_all = ["samples", "kmeans"])]
        rules: Option<PathBuf>,
        /// Clasifica con un bosque aleatorio entrenado con polígonos de muestra (.shp)
        #[arg(long, conflicts_with = "kmeans")]
        samples: Option<PathBuf>,
        /// Agrupa los objetos en tantos estratos según su parecido (sin reglas ni muestras)
        #[arg(long)]
        kmeans: Option<usize>,
        /// Atributos con los que se hacen los estratos (por defecto: las medias y los percentiles
        /// 95 de las capas, y la cobertura del dosel)
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        /// Une las clases en estratos: las superficies de una clase que se tocan pasan a ser un
        /// polígono
        #[arg(long)]
        strata: bool,
        /// Los estratos más pequeños que tantas celdas toman la clase de sus vecinos
        #[arg(long, default_value_t = 500)]
        strata_min_cells: u32,
        /// Reparte tantas parcelas de muestra entre los estratos (asignación de Neyman)
        #[arg(long, default_value_t = 0)]
        plots: usize,
        /// Altura a partir de la cual una celda cuenta como cubierta por el dosel (atributo
        /// `cobertura`)
        #[arg(long, default_value_t = 2.0)]
        cover_height: f64,
        /// Campo de las muestras que da nombre a la clase
        #[arg(long, default_value = "clase")]
        class_field: String,
        /// Número de árboles del bosque aleatorio
        #[arg(long, default_value_t = 100)]
        trees: usize,
        /// Semilla del bosque aleatorio: la misma semilla da el mismo resultado
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Ruta base de los resultados (.shp o .csv); por defecto:
        /// <resultados>/<ráster>_objetos.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Formato de salida: shapefile de objetos, tabla de texto o ambos
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// Simplifica los polígonos en tantas unidades del mapa (0: contornos exactos)
        #[arg(long, default_value_t = 0.0)]
        simplify: f64,
    },
    /// Convierte una capa vectorial entre shapefile y GeoJSON (según la extensión de la entrada).
    /// El GeoJSON recibe el sistema de coordenadas del shapefile como miembro `crs`
    Geojson {
        /// Un .shp (da un .geojson) o un .geojson (da un .shp)
        input: PathBuf,
        /// Salida; por defecto: <resultados>/<entrada>.geojson o .shp (nunca un archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Cambio forestal entre dos fechas a partir de dos modelos de altura de copas sobre las mismas
    /// celdas: diferencia de altura, crecimiento, pérdida y cambio de cobertura, manchas de dosel
    /// perdido, resumen por rodal y crecimiento de cada árbol
    Change {
        /// Modelo de altura de copas de la primera fecha
        before: PathBuf,
        /// Modelo de altura de copas de la segunda fecha (mismas celdas; véase `align`)
        after: PathBuf,
        /// Ruta base de los resultados; por defecto: <resultados>/<después>_cambio.shp (nunca un
        /// archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Un cambio de altura menor que este es ruido
        #[arg(long, default_value_t = 1.0)]
        min_change: f64,
        /// Altura a partir de la cual una celda cuenta como dosel
        #[arg(long, default_value_t = 2.0)]
        cover_height: f64,
        /// Las manchas de dosel perdido menores que tantas celdas no se comunican
        #[arg(long, default_value_t = 10)]
        min_patch: u32,
        /// Años entre las fechas, para las tasas anuales
        #[arg(long)]
        years: Option<f64>,
        /// Rodales que resumir (.shp o .geojson)
        #[arg(long)]
        stands: Option<PathBuf>,
        /// Campo de los rodales que los nombra
        #[arg(long)]
        stand_field: Option<String>,
        /// Empareja además los árboles de las dos fechas y mide su crecimiento
        #[arg(long)]
        trees: bool,
        /// Árboles: altura mínima (m) de una cima para contarla como árbol
        #[arg(long, default_value_t = 2.0)]
        hmin: f64,
        /// Árboles: diámetro de ventana = a + b h (unidades del mapa)
        #[arg(long, default_value_t = 3.0)]
        ws_a: f64,
        /// Coeficiente de la altura (b) del diámetro de la ventana de búsqueda de cimas
        #[arg(long, default_value_t = 0.1)]
        ws_b: f64,
        /// Árboles: suaviza antes los dos modelos con una ventana de este tamaño (0: nada)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// Árboles: es el mismo árbol cuando están más cerca que esto
        #[arg(long, default_value_t = 3.0)]
        match_dist: f64,
        /// Formato de salida: shapefile, tabla de texto o ambos
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
    },
    /// Importa una nube de puntos de otro formato como LAS/LAZ: PLY (ASCII o binario) o E57
    /// (escáneres terrestres y móviles; cada escaneo se coloca con su pose y su número queda como
    /// identificador de fuente del punto)
    Import {
        /// El archivo que convertir
        input: PathBuf,
        /// .las o .laz de salida; por defecto: <resultados>/<entrada>_ply.laz o _e57.laz (nunca un
        /// archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Código EPSG del sistema de coordenadas de los puntos (PLY y E57 rara vez lo dicen)
        #[arg(long)]
        epsg: Option<u16>,
    },
    /// Escribe las nubes como COPC (Cloud Optimized Point Cloud): un LAZ cuyos bloques son los
    /// nodos de un octree, para visores y servidores que leen solo la parte y el detalle que
    /// necesitan
    Copc {
        /// Nubes LAS o LAZ que convertir a COPC
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Salida (solo una entrada); por defecto: <resultados>/<entrada>.copc.laz
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Colorea los puntos a partir de una imagen del mismo lugar (una ortofoto): cada punto toma el
    /// rojo, el verde y el azul de su píxel. Los formatos sin color se escriben como formato 7
    Colorize {
        /// Nubes LAS o LAZ a las que se da el color de la imagen
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// La imagen (GeoTIFF, JPEG 2000…), en el sistema de coordenadas de las nubes
        #[arg(long)]
        image: PathBuf,
        /// Bandas del rojo, el verde y el azul, desde 1
        #[arg(long, value_delimiter = ',', num_args = 3, default_values_t = [1, 2, 3])]
        bands: Vec<usize>,
        /// Banda del infrarrojo cercano, que se escribe en los formatos 8 y 10
        #[arg(long)]
        nir: Option<usize>,
        /// Los puntos fuera de la imagen conservan su color (por defecto: negro)
        #[arg(long)]
        keep_outside: bool,
        /// Salida (solo una entrada); por defecto: <resultados>/<entrada>_rgb.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Normaliza la intensidad por la distancia al escáner, I · (R / R_ref)^a, a partir de la
    /// trayectoria (tiempo GPS, x, y, z) o de la altitud de vuelo y el ángulo de barrido
    Intensity {
        /// Nubes LAS o LAZ cuya intensidad se normaliza
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Trayectoria del escáner: un archivo de texto con líneas «tiempo x y z»
        #[arg(long, conflicts_with = "altitude")]
        trajectory: Option<PathBuf>,
        /// Altitud del vuelo (m, mismo datum vertical que los puntos), cuando no hay trayectoria
        #[arg(long)]
        altitude: Option<f64>,
        /// Exponente a de la distancia: 2 para superficies (suelo, dosel), alrededor de 3 para
        /// cables
        #[arg(long, default_value_t = 2.0)]
        exponent: f64,
        /// Distancia de referencia R_ref (m); por defecto: la distancia media de cada nube
        #[arg(long)]
        reference: Option<f64>,
        /// Salida (solo una entrada); por defecto: <resultados>/<entrada>_intensidad.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Reescribe un ráster como Cloud Optimized GeoTIFF (teselas y vistas reducidas, con todos los
    /// directorios al principio del archivo); las imágenes de varias bandas se escriben en flujo,
    /// sea cual sea su tamaño
    Cog {
        /// GeoTIFF, JPEG 2000 o rejilla ASCII, de una banda o de varias (ortofotos, imágenes de
        /// satélite)
        input: PathBuf,
        /// Salida; por defecto: <resultados>/<entrada>_cog.tif (nunca un archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Reduce las vistas reducidas tomando una celda en lugar de promediar (clases,
        /// identificadores)
        #[arg(long)]
        nearest: bool,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Corredores de líneas eléctricas: conductores reconstruidos a partir de los puntos de cable
    /// (clase 14), postes (clase 15) y la vegetación que se acerca demasiado o que podría alcanzar
    /// la línea si cayera, con un informe por vano. Se clasifican antes los cables y los postes
    Corridor {
        /// Nubes LAS o LAZ de la línea eléctrica, con los cables (clase 14) y los postes (clase 15) ya clasificados
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con
        /// la primera nube
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Ruta base de los resultados; por defecto: <resultados>/<entrada>_corredor.shp (nunca un
        /// archivo existente)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Clase de los puntos de cable
        #[arg(long, default_value_t = 14)]
        wire_class: u8,
        /// Clase de los puntos de poste
        #[arg(long, default_value_t = 15)]
        pole_class: u8,
        /// A menos de esta distancia de un conductor (en el espacio) hay invasión
        #[arg(long, default_value_t = 3.0)]
        min_distance: f64,
        /// Un árbol cuya altura más este margen alcanza la posición del conductor podría golpearlo
        #[arg(long, default_value_t = 1.0)]
        fall_margin: f64,
        /// La vegetación a más de esta distancia de todo conductor no se mira
        #[arg(long, default_value_t = 30.0)]
        half_width: f64,
        /// Los puntos de cable más cercanos que esto son un solo conductor
        #[arg(long, default_value_t = 2.5)]
        link: f64,
        /// Mayor hueco de puntos de cable que se rellena (los mayores cortan el conductor)
        #[arg(long, default_value_t = 30.0)]
        gap_max: f64,
        /// Menor número de puntos de un conductor
        #[arg(long, default_value_t = 30)]
        min_points: usize,
        /// Conductor más corto (los más cortos son ruido de la clasificación, p. ej. en copas de
        /// árboles)
        #[arg(long, default_value_t = 25.0)]
        min_length: f64,
        /// Altura mínima (m) de un poste o una torre para aceptarlo
        #[arg(long, default_value_t = 6.0)]
        min_pole_height: f64,
        /// Lado de las celdas en las que se agrupa la vegetación
        #[arg(long, default_value_t = 3.0)]
        cell: f64,
        /// La vegetación más baja que esto sobre el suelo se ignora
        #[arg(long, default_value_t = 2.0)]
        min_height: f64,
        /// Clases de vegetación
        #[arg(long, default_value = "3,4,5")]
        vegetation: String,
        /// Un poste a más de esta distancia de un conductor no es uno de sus apoyos
        #[arg(long, default_value_t = 8.0)]
        pole_reach: f64,
        /// Formato de salida: shapefile, tabla de texto o ambos
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
    },
    /// Recorta nubes con polígonos, parcelas o rectángulos
    Clip {
        /// Nubes LAS o LAZ que recortar
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Polígonos (.shp), centros de parcelas (.shp de puntos, con --radius) o una lista
        /// (.csv/.txt)
        #[arg(long)]
        regions: Option<PathBuf>,
        /// Un rectángulo: minx,miny,maxx,maxy (se repite para varios)
        #[arg(long)]
        rect: Vec<String>,
        /// Una parcela circular: x,y,radio (se repite para varias)
        #[arg(long)]
        circle: Vec<String>,
        /// El atributo que da nombre a cada región: el nombre de un campo o su posición contando
        /// desde 1
        #[arg(long)]
        name_field: Option<String>,
        /// Amplía cada región esta distancia en unidades del mapa (metros en un sistema
        /// proyectado); si es negativa, la reduce. Las esquinas quedan redondeadas
        #[arg(long, allow_hyphen_values = true)]
        buffer: Option<f64>,
        /// Divide los elementos de varias partes en una región por parte (llamadas <nombre>_p1,
        /// <nombre>_p2…)
        #[arg(long)]
        split_parts: bool,
        /// Radio de las parcelas dadas como puntos
        #[arg(long)]
        radius: Option<f64>,
        /// Atributo que guarda el radio propio de cada punto (metros)
        #[arg(long)]
        radius_field: Option<String>,
        /// Las parcelas dadas como puntos son cuadradas en lugar de circulares
        #[arg(long)]
        square: bool,
        /// Lado de las parcelas cuadradas, en metros (o los campos que guardan los lados)
        #[arg(long)]
        side: Option<f64>,
        /// Campo de las regiones con el lado X de la parcela cuadrada (m)
        #[arg(long)]
        side_x_field: Option<String>,
        /// Campo de las regiones con el lado Y de la parcela cuadrada (m)
        #[arg(long)]
        side_y_field: Option<String>,
        /// Genera las parcelas en lugar de leerlas: al azar o en una cuadrícula regular
        #[arg(long, value_enum)]
        sample: Option<SampleArg>,
        /// Muestreo aleatorio: número de parcelas
        #[arg(long, default_value_t = 10)]
        count: usize,
        /// Muestreo sistemático: separación entre centros en metros, X o X,Y
        #[arg(long)]
        spacing: Option<String>,
        /// Muestreo aleatorio: distancia mínima entre centros (por defecto: el doble del tamaño de
        /// la parcela)
        #[arg(long)]
        min_dist: Option<f64>,
        /// Semilla del muestreo aleatorio: la misma semilla da las mismas parcelas
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Las parcelas generadas se quedan dentro de estos polígonos (un rodal, por ejemplo)
        #[arg(long)]
        limit: Option<PathBuf>,
        /// Dónde se guarda la capa con las parcelas; por defecto: junto a los resultados
        #[arg(long)]
        layout: Option<PathBuf>,
        /// Conserva solo las regiones cuyo atributo tiene un valor: CAMPO=VALOR
        #[arg(long = "where")]
        where_: Option<String>,
        /// Qué se escribe: una nube con todo (merge), una nube por región (each) o solo la capa con las métricas (metrics)
        #[arg(long, value_enum, default_value_t = ClipModeArg::Merge)]
        mode: ClipModeArg,
        /// Archivo de salida (un nombre base con --mode each); por defecto: junto a los resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Clases que se conservan: 2,5 o ~7,18 para todas menos esas
        #[arg(long)]
        class: Option<String>,
        /// Retornos que se conservan: A, F, L o números (p. ej. 1,2)
        #[arg(long)]
        returns: Option<String>,
        /// Elevación mínima (m) de los puntos que se conservan
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        /// Elevación máxima (m) de los puntos que se conservan
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Modelo del suelo: escribe alturas sobre el suelo en lugar de elevaciones
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Se suma a cada elevación escrita (m)
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        z_offset: f64,
        /// Conserva un punto de cada tantos
        #[arg(long, default_value_t = 1)]
        decimate: u64,
        /// Regiones que se solapan: un punto va solo a la primera
        #[arg(long)]
        exclusive: bool,
        /// Escribe archivos para las regiones sin puntos
        #[arg(long)]
        keep_empty: bool,
        /// Archivos abiertos a la vez con --mode each
        #[arg(long, default_value_t = 64)]
        max_open: usize,
        /// Escribe además las CloudMetrics de cada región en este CSV
        #[arg(long)]
        metrics: Option<PathBuf>,
        /// Métricas: altura de corte para la cobertura
        #[arg(long, default_value_t = 2.0)]
        above: f64,
        /// Formato de la tabla de métricas: shapefile de las regiones, tabla de texto o ambos
        #[arg(long, value_enum)]
        metrics_format: Option<FormatArg>,
        /// Métricas: solo los retornos por encima de esta altura
        #[arg(long, default_value_t = 0.0)]
        minht: f64,
        /// Métricas: las columnas que se escriben, por nombre (`Elev P90,Elev mean,Int mean`); por
        /// defecto: todas
        #[arg(long, value_delimiter = ',')]
        metrics_columns: Vec<String>,
    },
    /// Une, filtra, normaliza o convierte nubes en una pasada (LAS o LAZ según la extensión de la
    /// salida)
    Filter {
        /// Nubes LAS o LAZ que unir, filtrar o convertir
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Salida; por defecto: <resultados>/<entrada>_filtrada.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        sel: SelArgs,
        /// Escribe alturas sobre este modelo del suelo en lugar de elevaciones
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Se suma a cada elevación escrita (m)
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        z_offset: f64,
    },
    /// Adelgaza (diezma) una nube: por celda, por celda 3D, un punto de cada n, al azar o hasta una
    /// densidad. Cada entrada tiene su propio archivo adelgazado salvo que se dé -o (entonces se
    /// unen).
    Thin {
        /// Nubes LAS o LAZ que adelgazar
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Archivo de salida (las entradas se unen); por defecto: un archivo adelgazado por cada entrada
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Cómo se adelgaza la nube: un punto por celda (más bajo, más alto, central o percentil), por celda 3D, uno de cada n, quitando uno de cada n, al azar o hasta una densidad
        #[arg(long, value_enum, default_value_t = ThinArg::Lowest)]
        method: ThinArg,
        /// Tamaño de celda en metros (métodos por celda)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Altura de la celda 3D en metros (vóxel)
        #[arg(long, default_value_t = 1.0)]
        vcell: f64,
        /// Percentil, de 0 a 100, que se conserva en cada celda (50 es la mediana)
        #[arg(long, default_value_t = 50.0)]
        percentile: f64,
        /// Con `every` y `remove`: cada cuántos puntos se conserva (o se quita) uno
        #[arg(long, default_value_t = 2)]
        n: u64,
        /// Proporción de puntos que se conservan, de 0 a 1 (aleatorio)
        #[arg(long, default_value_t = 0.5)]
        fraction: f64,
        /// Puntos por m² que se dejan (densidad)
        #[arg(long, default_value_t = 1.0)]
        density: f64,
        /// Semilla de los métodos aleatorios
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[command(flatten)]
        sel: SelArgs,
    },
    /// Rejilla de una estadística de los puntos (densidad, recuento, elevación, intensidad) como
    /// GeoTIFF
    Raster {
        /// Nubes LAS o LAZ de las que se calcula la rejilla
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// GeoTIFF de salida; por defecto, en la carpeta de resultados con el nombre de la entrada y el estadístico
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tamaño de celda de la rejilla (m)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Estadístico de cada celda: densidad, recuento, elevación media, máxima, mínima o rango, o intensidad media o máxima
        #[arg(long, value_enum, default_value_t = StatArg::Density)]
        stat: StatArg,
        #[command(flatten)]
        sel: SelArgs,
    },
    /// PNG de sombreado de un MDT o un CHM
    Hillshade {
        /// El MDT o el CHM (GeoTIFF)
        input: PathBuf,
        /// PNG de salida; por defecto: <resultados>/<entrada>_sombreado.png
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Dirección de la luz, en grados en el sentido de las agujas del reloj desde el norte
        #[arg(long, default_value_t = 315.0)]
        azimuth: f64,
        /// Altura de la luz sobre el horizonte, en grados
        #[arg(long, default_value_t = 45.0)]
        altitude: f64,
    },
    /// Curvas de nivel de un MDT o un CHM como shapefile de polilíneas 3D
    Contour {
        /// El MDT o el CHM (GeoTIFF)
        input: PathBuf,
        /// Shapefile de salida; por defecto: <resultados>/<entrada>_curvas_<equidistancia>m.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Equidistancia de las curvas de nivel (m)
        #[arg(long, default_value_t = 5.0)]
        interval: f64,
    },
    /// Divide las nubes en teselas cuadradas (con un margen opcional), un archivo por tesela
    Tile {
        /// Nubes LAS o LAZ que dividir en teselas
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Nombre base de las teselas; por defecto: <resultados>/<entrada>_tile.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Lado de cada tesela (m)
        #[arg(long, default_value_t = 1000.0)]
        size: f64,
        /// Margen (m) que cada tesela toma de sus vecinas
        #[arg(long, default_value_t = 0.0)]
        buffer: f64,
    },
    /// Edita puntos como si fueran elementos: borra o reclasifica los puntos dentro de polígonos
    /// que cumplen las pruebas de atributos; escribe un archivo nuevo, nunca el original
    Edit {
        /// Nube LAS o LAZ que editar
        input: PathBuf,
        /// Nube de salida; por defecto, en la carpeta de resultados con el nombre de la entrada
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Polígonos, centros de parcelas (con --radius) o una lista; por defecto: toda la nube
        #[arg(long)]
        regions: Option<PathBuf>,
        /// Radio (m) de las parcelas cuando las regiones son puntos
        #[arg(long)]
        radius: Option<f64>,
        /// Selecciona en cambio los puntos fuera de las regiones
        #[arg(long)]
        outside: bool,
        /// Solo se seleccionan los puntos de estas clases (2,5 o ~7,18)
        #[arg(long)]
        class: Option<String>,
        /// Elevación mínima (m) de los puntos que se seleccionan
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        /// Elevación máxima (m) de los puntos que se seleccionan
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Borra los puntos seleccionados
        #[arg(long, conflicts_with = "set_class")]
        delete: bool,
        /// Cambia los puntos seleccionados a esta clase
        #[arg(long)]
        set_class: Option<u8>,
    },
    /// Aplica a una o más nubes las ediciones guardadas desde el visor (un archivo de ediciones
    /// JSON) y escribe archivos nuevos, nunca el original
    ApplyEdits {
        /// Nubes LAS o LAZ a las que se aplican las ediciones
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Archivo de ediciones, el que escribe el visor con «Guardar ediciones…»
        #[arg(long)]
        edits: PathBuf,
        /// Nube de salida (solo una entrada); por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Da a las nubes sin sistema de coordenadas el de otra nube o un código EPSG
    Crs {
        /// Nubes LAS o LAZ a las que se asigna el sistema de coordenadas
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Una nube que tiene el sistema de coordenadas que copiar
        #[arg(long, conflicts_with = "epsg")]
        from: Option<PathBuf>,
        /// Código EPSG del sistema de coordenadas, p. ej. 25830
        #[arg(long)]
        epsg: Option<u16>,
        /// Sustituye el sistema de coordenadas de nubes que ya tienen uno
        #[arg(long)]
        replace: bool,
        /// Salida (solo una entrada); por defecto: <resultados>/<entrada>_crs.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Mide los troncos de una nube terrestre o móvil a lo largo del tronco y estima el volumen de
    /// cada individuo con la fórmula de Smalian (un shapefile o CSV de árboles, un CSV de trozas y,
    /// opcionalmente, nubes por árbol)
    #[command(alias = "cubicar")]
    StemVolume {
        /// Nubes LAS o LAZ de escáner terrestre; cada una es una unidad de muestreo
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Tabla de árboles (.shp o .csv); por defecto: <resultados>/<entrada>_volumen.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Escribe una tabla de texto además del shapefile
        #[arg(long)]
        csv: bool,
        /// Las z de la nube ya son alturas sobre el suelo
        #[arg(long)]
        normalized: bool,
        /// Altura sobre el suelo (m) donde empieza la primera troza
        #[arg(long, default_value_t = 0.10)]
        base_height: f64,
        /// Diámetro (m) en el que termina el fuste comercial
        #[arg(long, default_value_t = 0.07)]
        top_diameter: f64,
        /// Distancia (m) entre los diámetros medidos: la longitud de una troza
        #[arg(long, default_value_t = 0.5)]
        log_length: f64,
        /// Vóxel (m) al que se reduce la nube
        #[arg(long, default_value_t = 0.03)]
        voxel: f64,
        /// Franja (m sobre el suelo) donde se buscan los troncos
        #[arg(long, default_value_t = 0.7)]
        stripe_low: f64,
        /// Límite superior (m sobre el suelo) de la franja donde se buscan los troncos
        #[arg(long, default_value_t = 3.5)]
        stripe_high: f64,
        /// Mayor diámetro de tronco esperado (m)
        #[arg(long, default_value_t = 1.0)]
        max_diameter: f64,
        /// Escribe además una nube por árbol
        #[arg(long)]
        tree_clouds: bool,
        /// Escribe además la nube con los troncos en la clase 64 y las ramas en la clase 65
        #[arg(long)]
        stem_classes: bool,
        /// Distancia (m) hasta la que un punto va al árbol más cercano en las nubes por árbol
        #[arg(long, default_value_t = 8.0)]
        assign_dist: f64,
        /// Distancia (m) entre las bases de los troncos de una cepa (brotes de monte bajo)
        #[arg(long, default_value_t = 0.6)]
        stool_dist: f64,
        /// Altura (m) del diámetro: los troncos separados a esa altura son brotes, cada uno un
        /// árbol; los troncos que son uno a esa altura y se bifurcan por encima son un solo árbol
        /// bifurcado
        #[arg(long, default_value_t = 1.3)]
        fork_height: f64,
        /// Mayor tramo (m) de un tronco que puede quedar sin medir antes de darlo por perdido
        #[arg(long, default_value_t = 2.0)]
        max_gap: f64,
        /// Altura comercial como proporción (0-1) de la altura del árbol, para estimarla en los
        /// troncos perdidos cuando aún eran gruesos y muy por debajo de la cima (por ejemplo 0,65
        /// para el eucalipto); 0: sin estimación
        #[arg(long, default_value_t = 0.0)]
        lost_top_share: f64,
    },
    /// Modelo de transición (por área): regresión de una variable de campo (volumen, área
    /// basimétrica…) sobre las métricas LiDAR de las parcelas, con validación cruzada dejando una
    /// fuera que repite la elección de métricas, diagnósticos y, dadas las métricas de las celdas,
    /// el mapa de la variable
    Model {
        /// Métricas de las parcelas: la capa (.shp) o la tabla de texto de `clip --metrics`
        #[arg(long)]
        plots: PathBuf,
        /// Tabla de campo, una fila por parcela (.csv, .txt o .xlsx)
        #[arg(long)]
        field: PathBuf,
        /// Columna de la tabla de campo que se estima
        #[arg(long)]
        response: String,
        /// Columna que nombra la parcela en la tabla de métricas
        #[arg(long, default_value = "Muestra")]
        plot_key: String,
        /// Columna que nombra la parcela en la tabla de campo; por defecto: el mismo nombre
        #[arg(long)]
        field_key: Option<String>,
        /// Métricas que se usan, separadas por comas; por defecto: elegidas automáticamente
        #[arg(long, value_delimiter = ',')]
        predictors: Vec<String>,
        /// Transformación antes del ajuste: ninguna, logaritmo de la respuesta, o logaritmo de la respuesta y de las métricas
        #[arg(long, value_enum, default_value_t = TransformArg::None)]
        transform: TransformArg,
        /// Criterio de la elección automática
        #[arg(long, value_enum, default_value_t = CriterionArg::Bic)]
        criterion: CriterionArg,
        /// Máximo de métricas que conserva la elección automática (0: una por cada cinco parcelas,
        /// como mucho cinco)
        #[arg(long, default_value_t = 0)]
        max_vars: usize,
        /// Mayor factor de inflación de la varianza que acepta la elección automática
        #[arg(long, default_value_t = 5.0)]
        max_vif: f64,
        /// Métricas que puede usar la elección automática, separadas por comas; por defecto: todas
        /// las de la tabla
        #[arg(long, value_delimiter = ',')]
        candidates: Vec<String>,
        /// Los candidatos correlacionados más que esto dicen lo mismo: solo se conserva el más
        /// relacionado con la variable (1: se conservan todos)
        #[arg(long, default_value_t = 0.8)]
        max_corr: f64,
        /// Solo el análisis previo al ajuste: normalidad de la variable y cribado de los candidatos
        #[arg(long)]
        analyze: bool,
        /// Ofrece también las métricas de intensidad a la elección automática (por defecto se dejan
        /// fuera: la intensidad de la mayoría de los vuelos no está calibrada entre pasadas)
        #[arg(long)]
        with_intensity: bool,
        /// Métricas de las celdas (la tabla de texto de `metrics`; una por hoja): escribe el mapa
        #[arg(long, num_args = 1..)]
        cells: Vec<PathBuf>,
        /// El mapa (GeoTIFF); por defecto: junto a la tabla de parcelas
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Escribe también la semianchura del intervalo de predicción del 95 %
        #[arg(long)]
        interval: bool,
        /// Estima también las celdas fuera del rango de las parcelas
        #[arg(long)]
        extrapolate: bool,
        /// Código EPSG del mapa; por defecto: el de la capa de la tabla de celdas
        #[arg(long)]
        epsg: Option<u16>,
    },
    /// Imagen multibanda a partir de rásteres: cada banda de cada entrada pasa a ser una banda de
    /// la salida
    Stack {
        /// Rásteres que apilar; sus bandas se colocan en el orden de la lista
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Ráster multibanda de salida
        #[arg(short, long)]
        output: PathBuf,
        /// Tamaño de píxel de la salida; por defecto: el de la entrada más fina
        #[arg(long)]
        cell: Option<f64>,
        /// Valor de las entradas que significa sin datos
        #[arg(long)]
        src_nodata: Option<f32>,
        /// Valor que se escribe donde no hay datos
        #[arg(long, default_value_t = 0.0)]
        nodata: f32,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Altura de cada punto sobre el suelo, a partir de los puntos de suelo que ya tiene la nube
    /// (un TIN con ellos): se escribe como el atributo adicional HeightAboveGround o, con
    /// --replace-z, en lugar de la elevación
    Hag {
        /// Nubes LAS o LAZ con el suelo ya clasificado (clase 2)
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Salida (solo una entrada); por defecto: <resultados>/<entrada>_alturas.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Celda de la superficie del suelo
        #[arg(long, default_value_t = 0.5)]
        cell: f64,
        /// Clase de los puntos de suelo
        #[arg(long, default_value_t = 2)]
        ground_class: u8,
        /// Escribe la altura como elevación (una nube normalizada) en lugar de como atributo
        #[arg(long)]
        replace_z: bool,
    },
    /// Lleva una nube (LAS/LAZ) o un ráster a otro sistema de coordenadas, p. ej. de ED50 a ETRS89
    /// con la rejilla del IGN español. Las alturas se conservan salvo que --vertical-out pida otra
    /// referencia vertical (p. ej. de elipsoidal a ortométrica con el modelo de geoide)
    Reproject {
        /// La nube (LAS/LAZ) o el ráster que reproyectar
        input: PathBuf,
        /// Archivo de salida (LAS/LAZ, o GeoTIFF para rásteres)
        #[arg(short, long)]
        output: PathBuf,
        /// Código EPSG del sistema que se escribe, p. ej. 25830 (no hace falta con --reference)
        #[arg(long, required_unless_present = "reference")]
        to: Option<u16>,
        /// Código EPSG en el que está la entrada, cuando no lo declara
        #[arg(long)]
        from: Option<u16>,
        /// Rásteres: toma el sistema, la extensión, el origen y la celda de este ráster, de modo
        /// que el resultado coincida celda a celda con él
        #[arg(long, conflicts_with_all = ["to", "cell", "vertical_out"])]
        reference: Option<PathBuf>,
        /// Rásteres: cómo se trasladan los valores (auto: las clases conservan sus valores; si no,
        /// la media cuando las celdas crecen y bilineal en caso contrario)
        #[arg(long, value_enum, default_value_t = AlignResample::Auto)]
        resample: AlignResample,
        /// Rásteres: celda del resultado (por defecto: la de la entrada o la equivalente en el
        /// sistema nuevo)
        #[arg(long)]
        cell: Option<f64>,
        /// Lo que contienen las celdas vacías de un ráster cuando se lleva sobre una referencia
        #[arg(long, default_value_t = -9999.0, allow_hyphen_values = true)]
        nodata: f32,
        #[command(flatten)]
        tiles: TileArgs,
        /// Referencia vertical de las alturas de la entrada: `ellipsoidal` o un código EPSG (p. ej.
        /// 5782), cuando el archivo no la declara. Hace falta para cambiar las alturas con
        /// --vertical-out
        #[arg(long)]
        vertical_in: Option<String>,
        /// Referencia vertical que dar a las alturas (`ellipsoidal` o un código EPSG). Sin ella,
        /// las alturas no se tocan
        #[arg(long)]
        vertical_out: Option<String>,
        /// Acepta aproximaciones: sin ajuste de datum, alturas conservadas donde ningún geoide
        /// cubre el punto (por defecto: las rechaza; también HIPSO_ALLOW_BALLPARK=YES)
        #[arg(long)]
        allow_ballpark: bool,
        /// Acepta una operación inferior cuando la mejor no puede ejecutarse, p. ej. por falta de
        /// una rejilla (por defecto: la rechaza; también HIPSO_ONLY_BEST=NO)
        #[arg(long)]
        allow_fallback: bool,
    },
    /// Pone rásteres sobre las celdas de un ráster de referencia (mismo origen, celda y tamaño), de
    /// modo que coincidan celda a celda. Escribe <resultados>/<nombre>_alineado.tif para cada uno
    Align {
        /// El ráster cuyas celdas toman todos los demás
        reference: PathBuf,
        /// Rásteres que alinear
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Cómo se trasladan los valores (auto: la media si el ráster es más fino; si no, bilineal)
        #[arg(long, value_enum, default_value_t = AlignResample::Auto)]
        resample: AlignResample,
        /// Lo que contienen las celdas fuera de un ráster
        #[arg(long, default_value_t = -9999.0, allow_hyphen_values = true)]
        nodata: f32,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Mosaico de rásteres con las mismas bandas; --overlap dice qué hacer donde se solapan
    Mosaic {
        /// Rásteres que unir
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Ráster de salida
        #[arg(short, long)]
        output: PathBuf,
        /// Qué valor queda donde los rásteres se solapan: el primero, el último, la media, el mínimo, el máximo…
        #[arg(long, value_enum, default_value_t = OverlapArg::First)]
        overlap: OverlapArg,
        /// Tamaño de píxel de la salida; por defecto: el de la entrada más fina
        #[arg(long)]
        cell: Option<f64>,
        /// Valor de las entradas que significa sin datos
        #[arg(long)]
        src_nodata: Option<f32>,
        /// Valor que se escribe donde no hay datos
        #[arg(long, default_value_t = 0.0)]
        nodata: f32,
        /// La salida cubre solo donde coinciden todas las entradas
        #[arg(long)]
        intersection: bool,
        /// Promedia los píxeles de origen al reducir (por defecto: el más cercano)
        #[arg(long)]
        average: bool,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Análisis de terreno de un MDT (GeoTIFF): pendiente, orientación, sombreado, curvaturas,
    /// rugosidad, TRI, TPI, depresiones, flujo acumulado, índice de humedad, cauces, subcuencas,
    /// cuenca visual
    Terrain {
        /// El MDT (un GeoTIFF en un sistema proyectado, celda en metros)
        input: PathBuf,
        /// Producto que se calcula: pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, flujo, TWI, cauces, subcuencas o cuenca visual
        #[arg(long, value_enum)]
        product: TerrainArg,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_<producto>.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Pendiente en porcentaje en lugar de grados
        #[arg(long)]
        percent: bool,
        /// Ventanas sobre el borde del ráster o una celda vacía (productos 3x3): `nodata` (por
        /// defecto, sin valor), `repeat` (repite las celdas del borde; sin valor alrededor de los
        /// huecos) o `center` (los vecinos que faltan toman la altura del centro)
        #[arg(long, default_value = "nodata")]
        edges: String,
        /// Multiplica las alturas cuando no están en las unidades de la celda
        #[arg(long, default_value_t = 1.0)]
        z_factor: f64,
        /// Sombreado: dirección de la luz en grados en el sentido de las agujas del reloj desde el
        /// norte
        #[arg(long, default_value_t = 315.0)]
        azimuth: f64,
        /// Sombreado: altura de la luz sobre el horizonte en grados
        #[arg(long, default_value_t = 45.0)]
        altitude: f64,
        /// TPI: radio del entorno (unidades del mapa)
        #[arg(long, default_value_t = 10.0)]
        radius: f64,
        /// Cauces y subcuencas: superficie drenada (m²) que forma un cauce
        #[arg(long, default_value_t = 10000.0)]
        threshold: f64,
        /// Cuenca visual: la X y la Y del observador
        #[arg(long, num_args = 2, value_names = ["X", "Y"])]
        observer: Option<Vec<f64>>,
        /// Cuenca visual: altura del ojo sobre el suelo (m)
        #[arg(long, default_value_t = 1.7)]
        eye: f64,
        /// Cuenca visual: altura sobre el suelo que debe verse (m)
        #[arg(long, default_value_t = 0.0)]
        target: f64,
        /// Cuenca visual: hasta dónde mira el observador (unidades del mapa)
        #[arg(long, default_value_t = 1000.0)]
        max_distance: f64,
    },
    /// Simulación de inundación sobre un MDT: el área, el volumen y la profundidad del agua a uno o
    /// varios niveles, desde unas semillas. Con conectividad, un hoyo detrás de una cresta queda
    /// seco
    Flood {
        /// El MDT (un GeoTIFF en un sistema proyectado)
        input: PathBuf,
        /// Una semilla del agua, `x,y`; se repite para varias. Sin semillas, `--border`
        #[arg(long, value_name = "X,Y", num_args = 1)]
        seed: Vec<String>,
        /// El agua viene del borde del modelo (y de junto a las celdas vacías)
        #[arg(long)]
        border: bool,
        /// El modelo de bañera: todo lo que queda bajo el nivel se inunda, esté conectado o no
        #[arg(long, conflicts_with_all = ["seed", "border"])]
        bathtub: bool,
        /// Un nivel del agua, se repite para varios
        #[arg(long, value_name = "Z")]
        level: Vec<f64>,
        /// Una serie de niveles `desde:hasta:paso`
        #[arg(long, value_name = "A:B:STEP")]
        levels: Option<String>,
        /// Los niveles son alturas sobre `min` (la celda más baja) o sobre la `seed`; por defecto:
        /// elevaciones
        #[arg(long, value_enum)]
        relative: Option<FloodRelative>,
        /// Conectividad del agua: con 4 vecinos pasa solo por los lados; con 8, también por las diagonales
        #[arg(long, default_value_t = 8)]
        connectivity: u8,
        /// Una capa de polígonos con la superficie inundada de cada nivel
        #[arg(long)]
        extent: bool,
        /// Sin ráster de profundidad por nivel
        #[arg(long)]
        no_depth: bool,
        /// No escribe la tabla de resultados en CSV
        #[arg(long)]
        no_csv: bool,
        /// Ruta base de los resultados; por defecto: <resultados>/<entrada>_inundacion
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Líneas de cresta o de drenaje trazadas a partir de un MDT, como líneas 3D con orden de
    /// Strahler, longitud y pendiente media
    Ridges {
        /// El MDT (un GeoTIFF en un sistema proyectado)
        input: PathBuf,
        /// Traza las líneas de drenaje en lugar de las crestas
        #[arg(long)]
        drainage: bool,
        /// Superficie drenada que forma una línea (m2)
        #[arg(long, default_value_t = 1000.0)]
        min_area: f64,
        /// Descarta las líneas más cortas que esto (m)
        #[arg(long, default_value_t = 0.0)]
        min_length: f64,
        /// Pasadas de suavizado (Chaikin); 0 conserva las celdas
        #[arg(long, default_value_t = 0)]
        smooth: u8,
        /// Las líneas; por defecto: <resultados>/<entrada>_crestas.shp (o _drenaje)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Ruta de menor coste sobre un MDT, desde un origen hasta un destino con paradas opcionales
    #[command(name = "leastcost")]
    LeastCost {
        /// El MDT (un GeoTIFF en un sistema proyectado)
        input: PathBuf,
        /// El punto de origen `x,y`, en las coordenadas del MDT
        #[arg(long = "from", value_name = "X,Y")]
        from: String,
        /// El destino `x,y`; se repite con --nearest para ir al más barato de varios
        #[arg(long = "to", value_name = "X,Y", required = true)]
        to: Vec<String>,
        /// Una parada en el camino `x,y`, en orden
        #[arg(long = "via", value_name = "X,Y")]
        via: Vec<String>,
        /// `slope`, `tobler`, `uniform` o `raster:<tif>` (coste por metro en las celdas del MDT)
        #[arg(long, default_value = "slope")]
        model: String,
        /// Modelo de pendiente: exponente y pendiente (%) donde el coste por metro se duplica
        #[arg(long, default_value_t = 2.0)]
        power: f64,
        /// Modelo de pendiente: pendiente (%) a la que el coste por metro se duplica
        #[arg(long, default_value_t = 10.0)]
        reference_slope: f64,
        /// Las celdas más empinadas que esto (%) no se pueden cruzar
        #[arg(long)]
        max_slope: Option<f64>,
        /// Polígonos que no se pueden cruzar
        #[arg(long)]
        barriers: Option<PathBuf>,
        /// Polígonos donde el coste se multiplica por --prefer-factor
        #[arg(long)]
        prefer: Option<PathBuf>,
        /// Factor por el que se multiplica el coste dentro de los polígonos preferidos
        #[arg(long, default_value_t = 0.5)]
        prefer_factor: f64,
        /// Va al más barato de los destinos
        #[arg(long)]
        nearest: bool,
        /// Escribe también un ráster con el coste de llegar a cada celda desde el origen
        #[arg(long)]
        cost_distance: bool,
        /// La ruta; por defecto: <resultados>/<entrada>_ruta.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Volúmenes de acopios: el material sobre una superficie base dentro del contorno de cada
    /// pila. La base se hace para cada pila (un nivel, la altura más baja o mediana de su contorno,
    /// un plano por el contorno, una superficie interpolada a partir de él o un modelo del terreno
    /// del sitio antes)
    Piles {
        /// El modelo de superficie (un GeoTIFF, MDS)
        surface: PathBuf,
        /// Los contornos de las pilas (una capa de polígonos)
        #[arg(long)]
        piles: PathBuf,
        /// `plane`, `lowest`, `median`, `interpolated`, `level:<z>` o `dtm:<archivo>`
        #[arg(long, default_value = "plane")]
        base: String,
        /// Las alturas más cercanas que esto a la base no cuentan (m)
        #[arg(long, default_value_t = 0.0)]
        min_height: f64,
        /// Densidad del material (t/m3): la capa y la tabla reciben una masa
        #[arg(long)]
        density: Option<f64>,
        /// Ruta base de los resultados; por defecto: <resultados>/<superficie>_pilas
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// No escribe la tabla de resultados en CSV
        #[arg(long)]
        no_csv: bool,
        /// Escribe también un ráster con la superficie base
        #[arg(long)]
        base_raster: bool,
        /// Escribe también un ráster con la altura sobre la base
        #[arg(long)]
        height_raster: bool,
    },
    /// Desmonte y terraplén entre un terreno existente y un diseño: dz = diseño - existente;
    /// positivo es terraplén (tierra que añadir), negativo es desmonte (tierra que quitar)
    #[command(name = "cutfill")]
    CutFill {
        /// El terreno existente (un GeoTIFF)
        existing: PathBuf,
        /// El diseño: un archivo ráster, `level:<z>` o `plane:<z0>,<pendiente %>,<orientación
        /// grados>,<x>,<y>` (elevación z0 en x,y, bajando hacia la orientación)
        #[arg(long)]
        design: String,
        /// Polígonos que medir uno a uno (el ráster entero se mide siempre también)
        #[arg(long)]
        regions: Option<PathBuf>,
        /// Las diferencias de hasta este tamaño cuentan como sin cambio (m)
        #[arg(long, default_value_t = 0.05)]
        tolerance: f64,
        /// Ruta base de los resultados; por defecto: <resultados>/<existente>_cutfill
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// No escribe la tabla de resultados en CSV
        #[arg(long)]
        no_csv: bool,
        /// Escribe también un ráster de números enteros: 1 desmonte, 2 terraplén, 0 sin cambio
        #[arg(long)]
        classified: bool,
    },
    /// Calculadora ráster: una expresión sobre rásteres con nombres, celda a celda. Funciones: abs
    /// sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan;
    /// comparaciones, `and`, `or`, `not`, `%`, `^`. `A.2` es la banda 2 de A. Ejemplo: `hipso calc
    /// "if(A>0, A, 0)" A=chm.tif -o out.tif`
    Calc {
        /// La expresión, entre comillas
        expr: String,
        /// Los rásteres, como `NOMBRE=archivo.tif` (`A=chm.tif B=otro.tif`); con uno solo, también
        /// vale un archivo sin nombre y sus bandas son B1, B2…
        #[arg(required = true)]
        inputs: Vec<String>,
        /// El resultado; por defecto: <resultados>/<primer ráster>_calc.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tipo de muestra del resultado
        #[arg(long, value_enum, default_value = "f32")]
        dtype: CalcDtype,
        /// Las vistas reducidas conservan el primer píxel válido en lugar de la media (clases)
        #[arg(long)]
        nearest: bool,
        /// Los rásteres cuyas celdas difieren de las del primero se ponen sobre ellas con este
        /// remuestreo; sin él se rechazan
        #[arg(long, value_enum)]
        align: Option<AlignResample>,
    },
    /// Vectoriza un ráster: las celdas que comparten una clase, caen en un rango o están entre dos
    /// cortes pasan a ser polígonos con el valor, el área y el número de celdas como atributos
    Vectorize {
        /// El ráster (un GeoTIFF)
        input: PathBuf,
        /// Los polígonos: `.shp` o `.gpkg`; por defecto: <resultados>/<entrada>_poligonos.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Agrupa por rango: `bajo,alto`, ambos incluidos; el resto se deja fuera
        #[arg(long, value_delimiter = ',', num_args = 2, conflicts_with_all = ["breaks", "classes"])]
        range: Vec<f64>,
        /// Agrupa por los intervalos entre estos cortes, de menor a mayor: `0,5,10`
        #[arg(long, value_delimiter = ',', conflicts_with = "classes")]
        breaks: Vec<f64>,
        /// Agrupa por clase (valores enteros iguales); es lo habitual
        #[arg(long)]
        classes: bool,
        /// Descarta las partes con menos celdas que esto
        #[arg(long, default_value_t = 1)]
        min_cells: u32,
        /// Simplifica los límites (unidades del mapa); 0 conserva los escalones de las celdas
        #[arg(long, default_value_t = 0.0)]
        simplify: f64,
        /// Las celdas que se tocan por una esquina son una sola parte
        #[arg(long)]
        diagonal: bool,
    },
    /// Charcas: donde queda agua en un MDT. El modelo se rellena hasta que drena, y cada mancha
    /// conectada de agua más profunda que el mínimo es una charca, medida por área, profundidad,
    /// volumen, nivel de desborde y superficie drenada
    Ponds {
        /// El MDT (un GeoTIFF en un sistema proyectado, celda en metros)
        input: PathBuf,
        /// Ruta base de los resultados; por defecto: <resultados>/<entrada>_charcos.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// `depression` (agua retenida hasta el nivel de desborde de cada depresión) o `local`
        /// (puntos bajos frente al modelo suavizado; la pendiente y la superficie drenada se
        /// comprueban celda a celda)
        #[arg(long, default_value = "depression")]
        method: String,
        /// Suaviza antes el MDT (gaussiano, desviación típica en unidades del mapa; 0: no)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        /// El valor que significa sin datos cuando el MDT no declara ninguno
        #[arg(long)]
        nodata: Option<f64>,
        /// El agua menos profunda no es una charca (unidades del MDT)
        #[arg(long, default_value_t = 0.05)]
        min_depth: f64,
        /// Descarta las charcas menores que esto (m²)
        #[arg(long, default_value_t = 0.0)]
        min_area: f64,
        /// Descarta las charcas mayores que esto (m²): los embalses no son charcos
        #[arg(long)]
        max_area: Option<f64>,
        /// Descarta las charcas más profundas que esto (unidades del MDT): los hoyos y las zanjas
        /// no son charcos
        #[arg(long)]
        max_depth: Option<f64>,
        /// Descarta las charcas que retienen menos que esto (m³)
        #[arg(long, default_value_t = 0.0)]
        min_volume: f64,
        /// Descarta las charcas en suelo con una pendiente media mayor que esta (grados)
        #[arg(long)]
        max_slope: Option<f64>,
        /// Descarta las charcas que drenan menos superficie que esta (m²)
        #[arg(long)]
        min_catchment: Option<f64>,
        /// Escribe también la profundidad del agua como ráster
        #[arg(long)]
        depth_raster: bool,
        /// Escribe además un GeoPackage
        #[arg(long)]
        gpkg: bool,
    },
    /// Estadísticas de un ráster (un CHM, un índice, un MDT) dentro de cada polígono. El resultado
    /// es una capa de polígonos (shapefile) con los atributos propios de los polígonos más un campo
    /// nuevo por estadístico: count, min, max, mean, sum, stddev, median. Una celda cuenta cuando
    /// su centro está en el polígono
    Zonal {
        /// El ráster (GeoTIFF)
        raster: PathBuf,
        /// Los polígonos (.shp, .geojson, .gpkg), en el sistema del ráster
        #[arg(long)]
        regions: PathBuf,
        /// Prefijo de los campos nuevos (un campo de shapefile tiene como mucho 10 caracteres:
        /// `chm_` deja 6 para el estadístico, así que caben `stddev` y `median`)
        #[arg(long, default_value = "")]
        prefix: String,
        /// Divide los elementos de varias partes en un polígono por parte
        #[arg(long)]
        split_parts: bool,
        /// Shapefile; por defecto: <resultados>/<ráster>_zonal.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Comprueba que los polígonos de una capa son válidos (anillos cerrados, sin
    /// autointersecciones, huecos dentro, sin solapes entre partes) y lista los que no lo son
    IsValid {
        /// Capa de polígonos (.shp, .geojson, .gpkg)
        input: PathBuf,
    },
    /// Repara polígonos inválidos (regla par-impar: un lazo pasa a ser dos partes y los bordes
    /// duplicados se cancelan). Los válidos no se tocan; los atributos se conservan
    MakeValid {
        /// Capa de polígonos (.shp, .geojson, .gpkg) que reparar
        input: PathBuf,
        /// Shapefile; por defecto: <resultados>/<entrada>_valido.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Amplía (o, si es negativa, reduce) cada polígono una distancia en unidades del mapa, con
    /// esquinas redondeadas
    Buffer {
        /// Capa que ampliar o reducir (.shp, .geojson, .gpkg)
        input: PathBuf,
        /// Distancia en las unidades de la capa (metros en un sistema proyectado; grados si es
        /// geográfico)
        #[arg(long, allow_hyphen_values = true)]
        distance: f64,
        /// Repara antes los polígonos inválidos
        #[arg(long)]
        make_valid: bool,
        /// Shapefile; por defecto: <resultados>/<entrada>_buffer.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Une polígonos: los que comparten el valor de un campo pasan a ser un solo elemento (con el
    /// número de unidos en `n`); sin campo, toda la capa pasa a ser uno
    Dissolve {
        /// Capa de polígonos que disolver (.shp, .geojson, .gpkg)
        input: PathBuf,
        /// Campo por el que disolver
        #[arg(long)]
        field: Option<String>,
        /// Repara antes los polígonos inválidos
        #[arg(long)]
        make_valid: bool,
        /// Shapefile; por defecto: <resultados>/<entrada>_disuelto.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Unión de dos capas de polígonos como superposición: las piezas que están en las dos
    /// (atributos de ambas, los de la segunda con el prefijo `b_`), solo en la primera y solo en la
    /// segunda
    Union {
        /// La primera capa de polígonos
        first: PathBuf,
        /// La segunda capa de polígonos
        second: PathBuf,
        /// Repara antes los polígonos inválidos
        #[arg(long)]
        make_valid: bool,
        /// Shapefile; por defecto: <resultados>/<primera>_union.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Intersección de dos capas de polígonos: solo las piezas que están en las dos (atributos de
    /// ambas, los de la segunda con el prefijo `b_`)
    Intersect {
        /// La primera capa de polígonos
        first: PathBuf,
        /// La segunda capa de polígonos
        second: PathBuf,
        /// Repara antes los polígonos inválidos
        #[arg(long)]
        make_valid: bool,
        /// Shapefile; por defecto: <resultados>/<primera>_interseccion.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Reclasifica un ráster: cada valor o rango de valores pasa a ser otro. Las reglas son texto,
    /// una por línea o separadas por `;`: `10 = 1`, `0..5 = 2`, `< 0 = nodata`, `>= 100 = 4`,
    /// `nodata = 0`; decide la primera regla que coincide con una celda
    Reclass {
        /// Ráster que reclasificar (GeoTIFF y otros)
        input: PathBuf,
        /// Las reglas, como texto (p. ej. "0..10 = 1; 10..50 = 2; else = nodata")
        #[arg(long, conflicts_with = "rules_file", required_unless_present = "rules_file")]
        rules: Option<String>,
        /// Un archivo de texto con las reglas, una por línea
        #[arg(long)]
        rules_file: Option<PathBuf>,
        /// En qué se convierten las celdas a las que no se aplica ninguna regla: keep (su valor),
        /// nodata o un valor
        #[arg(long, default_value = "keep")]
        others: String,
        /// Banda que reclasificar
        #[arg(long, default_value_t = 1)]
        band: usize,
        /// Valor de la entrada que significa «sin datos», además de lo que declare su archivo
        #[arg(long, allow_hyphen_values = true)]
        src_nodata: Option<f32>,
        /// Lo que contienen las celdas vacías del resultado (por defecto: 255, 65535 o -9999 según
        /// el tipo elegido)
        #[arg(long, allow_hyphen_values = true)]
        nodata: Option<f32>,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_reclass.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Recorta un ráster con una capa de polígonos u otro ráster: lo que queda fuera de la máscara
    /// se vacía y, salvo con --no-crop, el resultado se corta a la caja de la máscara
    ClipRaster {
        /// Ráster que recortar
        input: PathBuf,
        /// Máscara: una capa de polígonos (shapefile, GeoPackage, GeoJSON, KML) o un ráster (se
        /// conservan las celdas con datos distintos de cero)
        #[arg(long)]
        mask: PathBuf,
        /// No corta a la caja de la máscara: conserva el tamaño de la entrada
        #[arg(long)]
        no_crop: bool,
        /// Conserva lo que queda fuera de la máscara en lugar de lo que queda dentro
        #[arg(long)]
        invert: bool,
        /// Lo que contienen las celdas descartadas (por defecto: lo del archivo; si no, 0 o -9999
        /// según el tipo)
        #[arg(long, allow_hyphen_values = true)]
        nodata: Option<f32>,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_recorte.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        tiles: TileArgs,
    },
    /// Quita las manchas pequeñas de un ráster de clases: las celdas conectadas de un valor con
    /// menos celdas que --threshold toman el valor de su mancha vecina mayor
    Sieve {
        /// Ráster de clases (GeoTIFF)
        input: PathBuf,
        /// Las manchas con menos celdas que esto se fusionan
        #[arg(long, default_value_t = 2)]
        threshold: usize,
        /// Las celdas se tocan por aristas y esquinas (8) en lugar de solo por aristas (4)
        #[arg(long)]
        eight: bool,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_sieve.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Distancia de cada celda a la celda más cercana de algunas clases (euclídea, exacta)
    Proximity {
        /// Ráster (GeoTIFF) del que se calcula la distancia
        input: PathBuf,
        /// Valores que cuentan como objetivos (por defecto: toda celda distinta de cero)
        #[arg(long, num_args = 1..)]
        values: Vec<f32>,
        /// Las distancias mayores que esto se dejan vacías
        #[arg(long)]
        max_distance: Option<f64>,
        /// Distancias en celdas en lugar de unidades del mapa
        #[arg(long)]
        cells: bool,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_proximity.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Rellena las celdas vacías a partir de las celdas con valor que las rodean (pesos por el
    /// inverso de la distancia en ocho direcciones)
    FillNodata {
        /// Ráster (GeoTIFF) cuyos huecos se rellenan
        input: PathBuf,
        /// Hasta dónde se buscan celdas con valor, en celdas
        #[arg(long, default_value_t = 100)]
        max_distance: usize,
        /// Pasadas de una media 3x3 sobre las celdas rellenadas
        #[arg(long, default_value_t = 0)]
        smoothing: usize,
        /// GeoTIFF; por defecto: <resultados>/<entrada>_relleno.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Quita los puntos duplicados (mismas x, y, z y los atributos pedidos)
    Dedup {
        /// Nubes LAS o LAZ de las que se quitan los puntos repetidos
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Archivo de salida (solo una entrada); por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Borra todas las copias, no solo las sobrantes
        #[arg(long)]
        all_copies: bool,
        /// Atributos que también deben coincidir: class, gps, intensity, return, count, source
        #[arg(long, value_delimiter = ',')]
        also: Vec<String>,
    },
    /// Une dos o más nubes en una y decide qué hacer con los puntos duplicados
    MergeClouds {
        /// Nubes LAS o LAZ que se unen (dos o más)
        #[arg(required = true, num_args = 2..)]
        inputs: Vec<PathBuf>,
        /// Nube unida; por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Qué hacer con los duplicados: keep (no buscarlos), mark (marcarlos como retenidos) o
        /// delete (quitarlos)
        #[arg(long, value_enum, default_value = "keep")]
        dups: DupsArg,
        /// Atributos que también deben coincidir: class, gps, intensity, return, count, source
        #[arg(long, value_delimiter = ',')]
        also: Vec<String>,
    },
    /// Puntos de una nube que han cambiado frente a una nube de referencia (detección de cambios)
    Compare {
        /// Nube en la que se buscan cambios
        input: PathBuf,
        /// Nube(s) de referencia con las que se compara
        #[arg(long, required = true)]
        reference: Vec<PathBuf>,
        /// Nube de salida con los puntos que han cambiado; por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Distancia (m) a partir de la cual un punto ha cambiado
        #[arg(long, default_value_t = 0.5)]
        distance: f64,
        /// Otros puntos cambiados que necesita un punto cambiado dentro del doble de la distancia
        #[arg(long, default_value_t = 3)]
        neighbors: usize,
    },
    /// Lleva una nube sobre una nube de referencia (punto más cercano iterativo) y escribe el
    /// resultado
    Fit {
        /// Nube que se mueve para que encaje con la de referencia
        input: PathBuf,
        /// Nube(s) de referencia sobre las que se ajusta
        #[arg(long, required = true)]
        reference: Vec<PathBuf>,
        /// Nube ajustada; por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Adelgazamiento (m) de los puntos usados para hallar el movimiento
        #[arg(long, default_value_t = 1.0)]
        thin: f64,
        /// Máximo desplazamiento horizontal (m) que se admite entre un punto y su pareja
        #[arg(long, default_value_t = 5.0)]
        max_xy: f64,
        /// Máxima diferencia de altura (m) que se admite entre un punto y su pareja
        #[arg(long, default_value_t = 2.0)]
        max_z: f64,
        /// Máximo de iteraciones del ajuste
        #[arg(long, default_value_t = 30)]
        iterations: usize,
        /// Se detiene cuando el movimiento de una iteración baja de este valor (m)
        #[arg(long, default_value_t = 0.01)]
        stop: f64,
        /// Solo traslada, no gira
        #[arg(long)]
        shift_only: bool,
        /// Punto a plano: distancia al plano tangente del punto de referencia
        #[arg(long)]
        plane: bool,
        /// Proporción de los pares más cercanos que se usan para hallar el movimiento (1 los
        /// conserva todos)
        #[arg(long, default_value_t = 1.0)]
        trim: f64,
    },
    /// Diferencia de altura entre pasadas como GeoTIFF, con las cifras del solape
    Swath {
        /// Nubes LAS o LAZ aéreas con ID de fuente o tiempo GPS
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Ráster de salida; por defecto, en la carpeta de resultados
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Tamaño de celda del ráster de diferencias (m)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Retornos que se usan: all, last, single
        #[arg(long, default_value = "last")]
        returns: String,
        /// Toma cada nube como una pasada en lugar del identificador de fuente del punto
        #[arg(long, conflicts_with = "gps_gap")]
        by_cloud: bool,
        /// Halla las pasadas a partir de los saltos del tiempo GPS mayores que esto (s), cuando el
        /// identificador de fuente del punto no las guarda
        #[arg(long)]
        gps_gap: Option<f64>,
    },
    /// Pasadas de una nube aérea halladas a partir de los saltos del tiempo GPS: cuántas hay,
    /// cuándo, cuántos puntos tienen y dónde. Con --assign, copias de las nubes que llevan el
    /// número de pasada
    #[command(name = "flightlines")]
    FlightLines {
        /// Nubes LAS o LAZ aéreas con tiempo GPS
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Un silencio del tiempo GPS mayor que esto (s) separa dos pasadas
        #[arg(long, default_value_t = 30.0)]
        gap: f64,
        /// Los tramos con menos puntos no son pasadas
        #[arg(long, default_value_t = 1000)]
        min_points: u64,
        /// Los tramos más cortos que esto (s) no son pasadas
        #[arg(long, default_value_t = 0.0)]
        min_duration: f64,
        /// Escribe el número de pasada en copias de las nubes: `source-id` o `user-data`
        #[arg(long)]
        assign: Option<String>,
        /// No escribe la tabla de resultados en CSV
        #[arg(long)]
        no_csv: bool,
        /// Sin capa de polígonos con el contorno de cada pasada
        #[arg(long)]
        no_vector: bool,
        /// Carpeta de la tabla y de las copias; por defecto: la carpeta de resultados de la primera
        /// nube
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Polígonos de Voronoi (y, con --delaunay, las aristas de Delaunay) de una capa de puntos, con
    /// los atributos de cada punto
    Voronoi {
        /// Una capa de puntos
        input: PathBuf,
        /// Las celdas se cortan por `bbox` (la caja de los puntos, algo mayor) o por una capa de
        /// polígonos; sin ello, se dejan fuera las celdas de los puntos del borde
        #[arg(long)]
        clip: Option<String>,
        /// Escribe también las aristas de la triangulación de Delaunay
        #[arg(long)]
        delaunay: bool,
        /// Los polígonos: `.shp`, `.gpkg` o `.geojson`; por defecto:
        /// <resultados>/<entrada>_voronoi.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Densidad de puntos por celdas de Voronoi, para comprobar una nube: n / suma de las áreas de
    /// las celdas de los puntos de cada celda ráster, con la proporción de celdas que alcanzan un
    /// umbral
    #[command(name = "voronoi-density")]
    VoronoiDensity {
        /// Nubes LAS o LAZ de las que se mide la densidad
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Celda del ráster de densidad (m)
        #[arg(long, default_value_t = 5.0)]
        cell: f64,
        /// Puntos por m2 que debe alcanzar la densidad
        #[arg(long, default_value_t = 8.0)]
        threshold: f64,
        /// Retornos que se usan: A, F, L o números; por defecto F (primeros retornos: pulsos)
        #[arg(long, default_value = "F")]
        returns: String,
        /// Clases que se usan: 2,5 o ~7,18
        #[arg(long)]
        class: Option<String>,
        /// Una celda de Voronoi mayor que esto (m2) es un hueco en los datos
        #[arg(long, default_value_t = 100.0)]
        max_cell_area: f64,
        /// Sin ráster de 1 donde se alcanza el umbral y 0 donde no
        #[arg(long)]
        no_pass_fail: bool,
        /// No escribe la tabla de resultados en CSV
        #[arg(long)]
        no_csv: bool,
        /// Lado de las teselas y su margen (m)
        #[arg(long, default_value_t = 250.0)]
        tile: f64,
        /// Margen (m) que cada tesela toma de sus vecinas
        #[arg(long, default_value_t = 20.0)]
        buffer: f64,
        /// Ruta base de los resultados; por defecto: <resultados>/<primera nube>_vdensidad
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Dibuja una nube (planta, 3D o perfil) en un PNG, como hace el visor
    Render {
        /// Nubes LAS o LAZ que dibujar
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// PNG que escribir; por defecto: junto a la primera entrada
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Vista: planta, 3D o perfil
        #[arg(long, value_enum, default_value_t = ViewArg::Plan)]
        view: ViewArg,
        /// Atributo con el que se colorean los puntos
        #[arg(long, value_enum, default_value_t = ColorArg::Elevation)]
        color: ColorArg,
        /// Tamaño de la imagen: ANCHOxALTO
        #[arg(long, default_value = "1600x1000")]
        size: String,
        /// Puntos que se guardan en memoria (se carga uno de cada N)
        #[arg(long, default_value_t = 8_000_000)]
        max_points: usize,
        /// Tamaño del punto, en píxeles
        #[arg(long, default_value_t = 1)]
        point_size: usize,
        /// 3D: rotación y elevación en grados
        #[arg(long, default_value_t = 30.0, allow_negative_numbers = true)]
        yaw: f64,
        /// 3D: elevación de la cámara, en grados
        #[arg(long, default_value_t = 35.0)]
        pitch: f64,
        /// Perfil: x1,y1,x2,y2,ancho de la caja del perfil
        #[arg(long)]
        profile: Option<String>,
        /// Planta: TIN sombreado bajo los puntos, a partir de las clases dadas (p. ej. 2; vacío:
        /// todas)
        #[arg(long)]
        shade: Option<String>,
        /// Planta: curvas de nivel cada tanto
        #[arg(long)]
        contours: Option<f64>,
        /// Clases que se dibujan: 2,5 o ~7,18
        #[arg(long)]
        class: Option<String>,
    },
    /// Extrae puntos de LAS/LAZ a LAS/LAZ, a columnas ASCII o a un shapefile de puntos 3D
    Export {
        /// Nubes LAS o LAZ que exportar
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Archivo (o, con varias entradas, una carpeta). .laz y .las escriben una nube, .shp un shapefile PointZ y cualquier otro, texto
        #[arg(short, long)]
        output: PathBuf,
        /// Exporta alturas sobre el suelo: cada nube menos su MDT
        #[arg(long)]
        normalize: bool,
        /// MDT con el que normalizar (una entrada); por defecto: el MDT más reciente hecho con cada
        /// nube
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Columnas en orden: x,y,z,intensity,return,returns,class,source,red,green,blue
        #[arg(long, value_delimiter = ',', default_values_t = ["x".to_string(), "y".to_string(), "z".to_string()])]
        items: Vec<String>,
        /// Separador de columnas del texto
        #[arg(long, default_value = ",")]
        delimiter: String,
        /// Escribe una primera línea con los nombres de las columnas
        #[arg(long)]
        header: bool,
        /// Decimales de las coordenadas
        #[arg(long, default_value_t = 2)]
        decimals: usize,
        /// Clases que se exportan: 2,5 o ~7,18
        #[arg(long)]
        class: Option<String>,
        /// Retornos que se exportan: A, F, L o números
        #[arg(long)]
        returns: Option<String>,
        /// Elevación mínima (m) de los puntos que se exportan
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        /// Elevación máxima (m) de los puntos que se exportan
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Conserva un punto de cada tantos
        #[arg(long, default_value_t = 1)]
        decimate: u64,
    },
    /// Lo que admite cada orden, como JSON: las opciones que dibuja un constructor de flujos para
    /// cada herramienta
    Schema,
    /// Ejecuta un flujo guardado por el constructor de flujos (un `.model.json`): cada nodo es una
    /// orden de este programa, ejecutada en orden; los productos conservados van a la carpeta de
    /// resultados
    Run {
        /// El flujo
        model: PathBuf,
        /// Sustituye un nodo de entrada (por su nombre o identificador): `nombre=ruta`
        #[arg(long = "input")]
        inputs: Vec<String>,
    },
    /// Comprueba un flujo: lista lo que impide ejecutarlo (un bucle, un archivo u opción que falta)
    ModelValidate {
        /// El flujo
        model: PathBuf,
    },
    /// Extrae los valores de uno o varios rásteres a una capa: un campo nuevo por banda con el valor bajo cada elemento (en el punto o en un punto interior de la línea o del polígono)
    Extract {
        /// La capa (puntos, líneas o polígonos), en el mismo sistema que los rásteres
        layer: PathBuf,
        /// Un ráster (se repite para varios)
        #[arg(long, required = true)]
        raster: Vec<PathBuf>,
        /// Bandas que extraer, contando desde 1 (por defecto, todas)
        #[arg(long, value_delimiter = ',')]
        bands: Vec<usize>,
        /// Interpola entre los cuatro centros de celda más cercanos en lugar de tomar la celda que contiene el punto
        #[arg(long)]
        bilinear: bool,
        /// Prefijo de los campos nuevos
        #[arg(long, default_value = "")]
        prefix: String,
        /// Capa de salida; por defecto: <resultados>/<capa>_valores.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Une a una capa las columnas de una tabla (csv, texto o xlsx) por un campo clave
    JoinTable {
        /// La capa a la que se añaden las columnas (.shp, .gpkg o .geojson)
        layer: PathBuf,
        /// La tabla (.csv, texto o .xlsx) de la que salen las columnas
        table: PathBuf,
        /// Campo clave de la capa
        #[arg(long)]
        layer_key: String,
        /// Columna clave de la tabla (por defecto, el mismo nombre)
        #[arg(long)]
        table_key: Option<String>,
        /// Prefijo de los campos nuevos
        #[arg(long, default_value = "")]
        prefix: String,
        /// Capa de salida; por defecto: <resultados>/<capa>_union.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Combina capas del mismo tipo de geometría en una (los campos son la unión de los de todas)
    Merge {
        /// Las capas que combinar
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Añade un campo con el nombre del archivo de origen de cada elemento
        #[arg(long)]
        origin: Option<String>,
        /// Capa de salida; por defecto: <resultados>/<primera>_combinada.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Divide una capa en una por cada valor distinto de un campo (un archivo por rodal, por especie...)
    Split {
        /// La capa que se divide (.shp, .gpkg o .geojson)
        input: PathBuf,
        /// El campo por el que dividir
        #[arg(long)]
        field: String,
        /// Carpeta de salida; por defecto: <resultados>/<capa>_partes
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Recorta una capa de puntos, líneas o polígonos con los polígonos de otra (con --erase, borra lo que cubren)
    ClipVector {
        /// La capa que recortar
        input: PathBuf,
        /// Los polígonos con los que recortar
        #[arg(long)]
        mask: PathBuf,
        /// Borra lo que cubren los polígonos en lugar de conservarlo
        #[arg(long)]
        erase: bool,
        /// Capa de salida; por defecto: <resultados>/<capa>_recortada.<ext> (o _borrada)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Distancia de cada elemento al elemento más cercano de otra capa, con su número de orden y, si se pide, uno de sus campos
    Near {
        /// La capa
        input: PathBuf,
        /// La capa de la que se busca el elemento más cercano
        #[arg(long)]
        to: PathBuf,
        /// Campo de la otra capa que se copia del elemento más cercano
        #[arg(long)]
        carry: Option<String>,
        /// Distancia máxima: los más lejanos quedan sin valor
        #[arg(long)]
        max: Option<f64>,
        /// Capa de salida; por defecto: <resultados>/<capa>_cercania.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Estadísticas de resumen de campos numéricos de una capa o tabla, por grupos de un campo (recuento, suma, media, mínimo, máximo, desviación)
    Summary {
        /// La capa o la tabla (csv, texto, xlsx)
        input: PathBuf,
        /// Campo por el que agrupar (sin él, un solo grupo)
        #[arg(long)]
        by: Option<String>,
        /// Campos numéricos que resumir, separados por comas (por defecto, todos los numéricos)
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
        /// Tabla CSV de salida; por defecto: <resultados>/<entrada>_resumen.csv
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Comprueba la topología de una capa: polígonos que se solapan, huecos entre polígonos y contornos que se cruzan consigo mismos; en líneas, cabos sueltos
    Topology {
        /// La capa de polígonos o de líneas
        input: PathBuf,
        /// Tolerancia: superficie mínima de un solape o un hueco, o distancia a la que dos extremos de línea se tienen por unidos
        #[arg(long, default_value_t = 1e-6)]
        tolerance: f64,
        /// Tabla CSV del informe; por defecto: <resultados>/<capa>_topologia.csv
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Rasteriza una capa: cada celda recibe el valor de un campo del último elemento que la cubre (o 1)
    Rasterize {
        /// La capa
        layer: PathBuf,
        /// Ráster de referencia: toma sus celdas
        #[arg(long)]
        reference: Option<PathBuf>,
        /// Tamaño de celda cuando no hay referencia
        #[arg(long)]
        cell: Option<f64>,
        /// Campo con el valor que se pinta (sin él, 1)
        #[arg(long)]
        field: Option<String>,
        /// GeoTIFF de salida; por defecto: <resultados>/<capa>_raster.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Filtro focal de un ráster: media, mediana, mínimo, máximo, suma, rango, desviación o mayoría sobre una ventana
    Focal {
        /// El ráster (GeoTIFF)
        input: PathBuf,
        /// mean, median, min, max, sum, range, std, majority o gaussian
        #[arg(long, default_value = "mean")]
        stat: String,
        /// Radio de la ventana en celdas
        #[arg(long, default_value_t = 1)]
        radius: usize,
        /// Ventana circular en lugar de cuadrada
        #[arg(long)]
        circle: bool,
        /// Desviación típica del filtro gaussiano, en celdas (por defecto, la mitad del radio)
        #[arg(long)]
        sigma: Option<f32>,
        /// Menos celdas con valor que esto dejan la celda vacía
        #[arg(long, default_value_t = 1)]
        min_valid: usize,
        /// GeoTIFF de salida; por defecto: <resultados>/<entrada>_focal.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Estadísticas de un ráster (recuento, mínimo, máximo, media, desviación, mediana, percentiles) y su histograma
    Rstats {
        /// El ráster (GeoTIFF)
        input: PathBuf,
        /// Intervalos del histograma
        #[arg(long, default_value_t = 20)]
        bins: usize,
        /// CSV del histograma; por defecto: <resultados>/<entrada>_histograma.csv
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Área de cada clase de un ráster dentro de cada zona de otro (tabular área), como CSV
    Tabulate {
        /// El ráster de zonas (clases enteras)
        zones: PathBuf,
        /// El ráster de clases que se mide dentro de cada zona
        #[arg(long)]
        values: PathBuf,
        /// CSV de salida; por defecto: <resultados>/<zonas>_tabulado.csv
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Métricas por celda de la rejilla: elevación y, opcionalmente, intensidad y estratos
    Metrics {
        /// Nubes LAS o LAZ de las que se calculan las métricas
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con
        /// esta misma nube
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Shapefile de celdas de salida (o .csv si se da uno); por defecto:
        /// <resultados>/<entrada>_metricas_<celda>m.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Lado de las celdas en las que se calculan las métricas (m)
        #[arg(long, default_value_t = 15.0)]
        cell: f64,
        /// Altura de corte para las métricas de cobertura
        #[arg(long, default_value_t = 2.0)]
        heightbreak: f64,
        /// Con varias hojas, da a las celdas del borde de cada hoja los puntos de las vecinas a
        /// menos de tantos metros (al menos una celda) y escribe cada celda una sola vez
        #[arg(long, default_value_t = 0.0)]
        neighbors: f64,
        /// Menor número de puntos con que una celda tiene métricas; con menos queda vacía
        #[arg(long, default_value_t = 4)]
        min_points: u32,
        /// Máximo de memoria, en MiB, para las alturas de los puntos (las nubes mayores requieren
        /// más pasadas)
        #[arg(long, default_value_t = 512)]
        memory_mb: u64,
        /// Escribe también métricas de intensidad en <nombre de salida>_intensity (mismo formato
        /// que la salida)
        #[arg(long)]
        intensity: bool,
        /// Escribe también métricas de estratos en <nombre de salida>_strata (mismo formato que la
        /// salida)
        #[arg(long)]
        strata: bool,
        /// Límites superiores de los estratos de altura
        #[arg(long, value_delimiter = ',', default_values_t = [0.15, 1.37, 5.0, 10.0, 20.0, 30.0])]
        strata_limits: Vec<f64>,
        /// Límites superiores de los estratos de intensidad
        #[arg(long, value_delimiter = ',', default_values_t = [0.15, 1.37])]
        int_strata_limits: Vec<f64>,
        /// Escribe también métricas de estructura en <nombre de salida>_estructura: fracción de
        /// huecos, índice de área foliar, diversidad de la altura del follaje, índice de
        /// complejidad vertical y altura de la base de la copa
        #[arg(long)]
        structure: bool,
        /// Estructura: grosor de las capas del perfil vertical
        #[arg(long, default_value_t = 1.0)]
        structure_layer: f64,
        /// Estructura: altura del perfil (los retornos por encima caen en su última capa)
        #[arg(long, default_value_t = 30.0)]
        structure_top: f64,
        /// Estructura: los retornos por debajo de esta altura cuentan como luz que llegó al suelo
        #[arg(long, default_value_t = 1.0)]
        gap_height: f64,
        /// Estructura: coeficiente de extinción del índice de área foliar
        #[arg(long, default_value_t = 0.5)]
        extinction: f64,
        /// Estructura: la base de la copa se busca por encima de esta altura
        #[arg(long, default_value_t = 2.0)]
        cbh_floor: f64,
        /// Estructura: una capa es copa cuando guarda esta proporción de los retornos de la capa
        /// más llena
        #[arg(long, default_value_t = 0.1)]
        cbh_share: f64,
        /// Formato de salida: shapefile de celdas, tabla de texto o ambos (por defecto: según el
        /// nombre de la salida)
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
    },
}

/// `a,b,c` como números.
fn numbers(text: &str, n: usize, what: &str) -> Result<Vec<f64>> {
    let v: Vec<f64> = text
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    if v.len() == n {
        Ok(v)
    } else {
        Err(hipso_core::Error::Param(format!(
            "{what} necesita {n} números separados por comas, no '{text}'"
        )))
    }
}

/// La ruta de salida: la dada o, si no, `<resultados>/<entrada>_<etiqueta>.<ext>` (un nombre libre:
/// un producto anterior nunca se sobrescribe salvo que se dé su ruta). Un punto `x,y` de una
/// opción.
fn xy(text: &str, option: &str) -> std::result::Result<[f64; 2], hipso_core::Error> {
    let bad = || hipso_core::Error::Param(format!("{option}: «{text}» no es x,y"));
    let (x, y) = text.split_once(',').ok_or_else(bad)?;
    let n = |t: &str| t.trim().parse::<f64>().map_err(|_| bad());
    Ok([n(x)?, n(y)?])
}

/// La extensión de capa de salida: la de la entrada si es un formato que se escribe, si no shapefile.
fn layer_ext(p: &Path) -> String {
    match p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()) {
        Some(e) if matches!(e.as_str(), "shp" | "gpkg" | "geojson") => e,
        _ => "shp".into(),
    }
}

fn out_path(given: Option<PathBuf>, input: &Path, tag: &str, ext: &str) -> PathBuf {
    given.unwrap_or_else(|| paths::unique(&paths::product(input, tag, ext)))
}

/// El MDT dado o, si no, el más reciente hecho con `input`.
fn dtm_for(given: Option<PathBuf>, input: &Path) -> Result<PathBuf> {
    given.or_else(|| paths::find_dtm(input)).ok_or_else(|| {
        hipso_core::Error::Param(format!(
            "no se ha indicado MDT y no hay ninguno hecho a partir de {} en {}: ejecuta antes `dtm` o indica --dtm",
            input.display(),
            paths::results_dir(input).display()
        ))
    })
}

fn smoothing_of(a: SmoothArg) -> Smoothing {
    match a {
        SmoothArg::Mean => Smoothing::Mean,
        SmoothArg::Median => Smoothing::Median,
        SmoothArg::Gaussian => Smoothing::Gaussian,
    }
}

/// `-o` da nombre a un resultado; con varias hojas cada una tiene el suyo, así que no se admite
/// `-o`.
fn single_output(output: Option<PathBuf>, sheets: usize) -> Result<Option<PathBuf>> {
    if sheets > 1 && output.is_some() {
        return Err(hipso_core::Error::Param(
            "con varias hojas cada una tiene su propio resultado: quita -o".into(),
        ));
    }
    Ok(output)
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .map_or("las".into(), |e| e.to_string_lossy().into_owned())
}

fn run(cmd: Cmd) -> Result<()> {
    let t = Instant::now();
    let summary = match cmd {
        Cmd::Info { inputs } => {
            println!("{}", jobs::info(&inputs)?);
            return Ok(());
        }
        Cmd::Extract { layer, raster, bands, bilinear, prefix, output } => {
            use hipso_core::geoproc;
            let how = if bilinear { geoproc::Sampling::Bilinear } else { geoproc::Sampling::Nearest };
            let b = (!bands.is_empty()).then_some(bands.as_slice());
            let e = geoproc::extract(&layer, &raster, b, how, &prefix)?;
            let out = out_path(output, &layer, "valores", &layer_ext(&layer));
            geoproc::write_extracted(&e, &out)?;
            format!(
                "{} campos ({}) en {} elementos{} -> {}",
                e.fields.len(),
                e.fields.join(", "),
                e.layer.features.len(),
                if e.without > 0 { format!("; {} sin valor (fuera del ráster o en celdas vacías)", e.without) } else { String::new() },
                out.display()
            )
        }
        Cmd::JoinTable { layer, table, layer_key, table_key, prefix, output } => {
            use hipso_core::geoproc;
            let key = table_key.unwrap_or_else(|| layer_key.clone());
            let j = geoproc::join_table(&layer, &table, &layer_key, &key, &prefix)?;
            let out = out_path(output, &layer, "union", &layer_ext(&layer));
            hipso_core::vector::write_layer(&out, &j.layer, j.kind, j.prj.as_deref())?;
            format!(
                "{} de {} elementos con fila en la tabla; {} filas de la tabla sin elemento; campos nuevos: {} -> {}",
                j.matched,
                j.layer.features.len(),
                j.unused,
                j.fields.join(", "),
                out.display()
            )
        }
        Cmd::Merge { inputs, origin, output } => {
            let (d, kind, prj) = hipso_core::geoproc::merge(&inputs, origin.as_deref())?;
            let first = inputs.first().cloned().unwrap_or_default();
            let out = out_path(output, &first, "combinada", &layer_ext(&first));
            hipso_core::vector::write_layer(&out, &d, kind, prj.as_deref())?;
            format!("{} capas combinadas en {} elementos -> {}", inputs.len(), d.features.len(), out.display())
        }
        Cmd::Split { input, field, dir } => {
            let dir = dir.unwrap_or_else(|| paths::results_dir(&input).join(format!("{}_partes", paths::stem(&input))));
            let files = hipso_core::geoproc::split_by(&input, &field, &dir, &layer_ext(&input))?;
            format!("{} capas escritas en {}", files.len(), dir.display())
        }
        Cmd::ClipVector { input, mask, erase, output } => {
            let (d, kind, prj, dropped) = hipso_core::geoproc::clip_layer(&input, &mask, erase)?;
            let out = out_path(output, &input, if erase { "borrada" } else { "recortada" }, &layer_ext(&input));
            hipso_core::vector::write_layer(&out, &d, kind, prj.as_deref())?;
            format!("{} elementos ({} se descartan por no dejar nada) -> {}", d.features.len(), dropped, out.display())
        }
        Cmd::Near { input, to, carry, max, output } => {
            let (d, kind, prj, without) = hipso_core::geoproc::near(&input, &to, carry.as_deref(), max)?;
            let out = out_path(output, &input, "cercania", &layer_ext(&input));
            hipso_core::vector::write_layer(&out, &d, kind, prj.as_deref())?;
            format!("{} elementos, {} sin elemento cercano -> {}", d.features.len(), without, out.display())
        }
        Cmd::Summary { input, by, fields, output } => {
            use hipso_core::geoproc;
            let (rows, names) = geoproc::summary(&input, by.as_deref(), &fields)?;
            let out = out_path(output, &input, "resumen", "csv");
            std::fs::write(&out, geoproc::summary_csv(&rows, &names, by.as_deref()))?;
            format!("{} grupos, {} campos -> {}", rows.len(), names.len(), out.display())
        }
        Cmd::Topology { input, tolerance, output } => {
            use hipso_core::topology;
            let (d, kind, _) = hipso_core::vecops::read_layer(&input)?;
            let issues = topology::check(&d, kind, tolerance);
            let out = out_path(output, &input, "topologia", "csv");
            std::fs::write(&out, topology::report_csv(&issues))?;
            format!("{} incumplimientos -> {}", issues.len(), out.display())
        }
        Cmd::Rasterize { layer, reference, cell, field, output } => {
            use hipso_core::rasterproc::{self, Frame};
            let (d, kind, _) = hipso_core::vecops::read_layer(&layer)?;
            let frame = match (&reference, cell) {
                (Some(r), _) => Frame::Reference(r),
                (None, Some(c)) => Frame::Cell(c),
                _ => return Err(hipso_core::Error::Param("indica --reference o --cell".into())),
            };
            let g = rasterproc::rasterize(&d, kind, frame, field.as_deref())?;
            let out = out_path(output, &layer, "raster", "tif");
            rasterproc::write_like(&g, &out, reference.as_deref().unwrap_or(&layer))?;
            format!("ráster de {} x {} celdas -> {}", g.cols, g.rows, out.display())
        }
        Cmd::Focal { input, stat, radius, circle, sigma, min_valid, output } => {
            use hipso_core::rasterproc::{self, Focal};
            let kind = if stat.eq_ignore_ascii_case("gaussian") {
                Focal::Gaussian(sigma.unwrap_or(radius as f32 / 2.0))
            } else {
                Focal::parse(&stat)?
            };
            let g = hipso_core::grid::Grid::read(&input)?;
            let r = rasterproc::focal(&g, kind, radius, circle, min_valid)?;
            let out = out_path(output, &input, "focal", "tif");
            rasterproc::write_like(&r, &out, &input)?;
            format!("filtro {stat} de radio {radius} -> {}", out.display())
        }
        Cmd::Rstats { input, bins, output } => {
            use hipso_core::rasterproc;
            let g = hipso_core::grid::Grid::read(&input)?;
            let st = rasterproc::stats(&g, bins)?;
            let out = out_path(output, &input, "histograma", "csv");
            std::fs::write(&out, rasterproc::histogram_csv(&st))?;
            format!("{}\nhistograma -> {}", rasterproc::stats_report(&st), out.display())
        }
        Cmd::Tabulate { zones, values, output } => {
            let z = hipso_core::grid::Grid::read(&zones)?;
            let v = hipso_core::grid::Grid::read(&values)?;
            let (csv, skipped) = hipso_core::rasterproc::tabulate(&z, &v)?;
            let out = out_path(output, &zones, "tabulado", "csv");
            std::fs::write(&out, csv)?;
            format!("tabla de áreas ({skipped} celdas sin valor en alguno) -> {}", out.display())
        }
        Cmd::Schema => {
            println!("{}", flow::schema().to_json());
            return Ok(());
        }
        Cmd::Run { model, inputs } => return flow::run(&model, &inputs, paths::output_dir()),
        Cmd::ModelValidate { model } => return flow::validate(&model),
        Cmd::Catalog { inputs, tolerance } => {
            let cat = hipso_core::catalog::catalog(&inputs)?;
            println!("{}", cat.report(tolerance));
            let bad = cat.failed.len()
                + cat
                    .issues(tolerance)
                    .iter()
                    .filter(|i| {
                        i.severity == hipso_core::catalog::Severity::Error
                            && i.kind != hipso_core::catalog::Kind::Unreadable
                    })
                    .count();
            if bad > 0 {
                std::process::exit(1);
            }
            return Ok(());
        }
        Cmd::Classify {
            inputs,
            output,
            neighbors,
            steps,
        } => {
            // cada hoja se clasifica por sí sola: nunca se unen en un solo archivo
            let output = single_output(output, inputs.len())?;
            let tag = format!("clasif{}", steps.tag());
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, &ext_of(p));
            hipso_core::sheets::classify_each(
                &inputs,
                &out_for,
                &steps.steps(),
                &steps.tiling(),
                neighbors,
            )?
        }
        Cmd::Dtm {
            inputs,
            output,
            cell,
            surface,
            smooth,
            smoothing,
            mosaic,
            breaklines,
            neighbors,
            steps,
        } => {
            let output = single_output(output, inputs.len())?;
            let tin = match surface {
                SurfaceMethod::Mean => "_media",
                SurfaceMethod::Tin => "_tin",
                SurfaceMethod::Idw => "_idw",
            };
            let sm = if smooth > 0.0 {
                format!("_s{}", paths::num(smooth))
            } else {
                String::new()
            };
            let tag = format!(
                "mdt_{}m{tin}{}{sm}",
                paths::num(cell),
                steps.tag_with(GroundMethod::Classes)
            );
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, "tif");
            let first = inputs.first().cloned().unwrap_or_default();
            let mosaic_to =
                mosaic.then(|| out_path(None, &first, &format!("mosaico_{tag}"), "tif"));
            let mut ground = steps.steps_with(GroundMethod::Classes);
            // un MDT se hace solo con puntos de suelo: sin pasos de ruido ni de edificios
            ground.noise = None;
            ground.buildings = None;
            ground.extras = Default::default();
            let base = jobs::DtmJob {
                inputs: Vec::new(),
                output: PathBuf::new(),
                cell,
                surface: match surface {
                    SurfaceMethod::Mean => Surface::Mean,
                    SurfaceMethod::Tin => Surface::Tin,
                    SurfaceMethod::Idw => Surface::Idw,
                },
                steps: ground,
                tiling: steps.tiling(),
                smooth,
                smoothing: smoothing_of(smoothing),
                breaklines,
            };
            hipso_core::sheets::dtm_each(&inputs, &base, &out_for, mosaic_to.as_deref(), neighbors)?.0
        }
        Cmd::Dsm {
            inputs,
            output,
            cell,
            method,
            no_fill,
            tile,
            threads,
            smooth,
            smoothing,
            neighbors,
        } => {
            let output = single_output(output, inputs.len())?;
            let (m, name) = match method {
                DsmArg::Max => (jobs::DsmMethod::Max, "max"),
                DsmArg::Tin => (jobs::DsmMethod::Tin, "tin"),
                DsmArg::Idw => (jobs::DsmMethod::Idw, "idw"),
            };
            let sm = if smooth > 0.0 {
                format!("_s{}", paths::num(smooth))
            } else {
                String::new()
            };
            let tag = format!("mds_{}m_{name}{sm}", paths::num(cell));
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, "tif");
            let base = jobs::DsmJob {
                input: PathBuf::new(),
                output: PathBuf::new(),
                cell,
                method: m,
                fill: !no_fill,
                tiling: Tiling {
                    size: tile,
                    buffer: 5.0,
                    threads,
                },
                smooth,
                smoothing: smoothing_of(smoothing),
            };
            hipso_core::sheets::dsm_each(&inputs, &base, &out_for, None, neighbors)?.0
        }
        Cmd::Chm {
            inputs,
            dtm,
            output,
            cell,
            no_fill,
            method,
            max_edge,
            tile,
            threads,
            smooth,
            smoothing,
            mosaic,
            neighbors,
        } => {
            let output = single_output(output, inputs.len())?;
            if inputs.len() > 1 && dtm.is_some() {
                return Err(hipso_core::Error::Param(
                    "--dtm indica un solo MDT: con varias hojas, cada una usa el MDT hecho a partir de ella"
                        .into(),
                ));
            }
            let pf = match method {
                ChmMethod::Pitfree => "_pitfree",
                ChmMethod::Tin => "_tin",
                ChmMethod::Idw => "_idw",
                ChmMethod::Max => "",
            };
            let sm = if smooth > 0.0 {
                format!("_s{}", paths::num(smooth))
            } else {
                String::new()
            };
            let tag = format!("chm_{}m{pf}{sm}", paths::num(cell));
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, "tif");
            let dtm_of = |p: &Path| dtm_for(dtm.clone(), p);
            let first = inputs.first().cloned().unwrap_or_default();
            let mosaic_to =
                mosaic.then(|| out_path(None, &first, &format!("mosaico_{tag}"), "tif"));
            let base = jobs::ChmJob {
                input: PathBuf::new(),
                dtm: PathBuf::new(),
                output: PathBuf::new(),
                cell,
                method: match method {
                    ChmMethod::Max => jobs::ChmMethod::Max { fill: !no_fill },
                    ChmMethod::Tin => jobs::ChmMethod::Tin,
                    ChmMethod::Idw => jobs::ChmMethod::Idw,
                    ChmMethod::Pitfree => jobs::ChmMethod::PitFree(pipeline::PitFree {
                        max_edge,
                        ..Default::default()
                    }),
                },
                tiling: Tiling {
                    size: tile,
                    buffer: 5.0,
                    threads,
                },
                smooth,
                smoothing: smoothing_of(smoothing),
            };
            hipso_core::sheets::chm_each(
                &inputs,
                &base,
                &out_for,
                &dtm_of,
                mosaic_to.as_deref(),
                neighbors,
            )?
            .0
        }
        Cmd::Trees {
            chm,
            around,
            margin,
            output,
            crowns,
            crowns_shp,
            tops_shp,
            cloud,
            tops_file,
            hmin,
            ws_a,
            ws_b,
            ws_c,
            shape,
            method,
            th_seed,
            th_cr,
            max_cr,
            max_cr_factor,
            exclusion,
            tolerance,
            ext,
            smooth,
            smoothing,
            field,
            field_x,
            field_y,
            field_h,
            field_plot,
            plot_x,
            plot_y,
            plot_radius,
            match_dist,
            format,
        } => {
            let seg = match method {
                SegArg::Dalponte => Segmenter::Dalponte,
                SegArg::Silva => Segmenter::Silva,
                SegArg::Watershed => Segmenter::Watershed,
            };
            let tag = match seg {
                Segmenter::Dalponte => "dalponte",
                Segmenter::Silva => "silva",
                Segmenter::Watershed => "watershed",
            };
            let output = output.unwrap_or_else(|| {
                paths::unique(&paths::product(&chm, &format!("arboles_{tag}"), "shp"))
            });
            let extra = |suffix: &str, ext: &str| {
                paths::unique(
                    &output.with_file_name(format!("{}_{suffix}.{ext}", paths::stem(&output))),
                )
            };
            let formats = formats_of(format, &output);
            jobs::trees(&jobs::TreesJob {
                gpkg: false,
                crown_metrics: Vec::new(),
                formats,
                crowns: crowns.then(|| extra("individuos", "tif")),
                crowns_shp: crowns_shp.then(|| extra("individuos", "shp")),
                tops_shp: tops_shp.then(|| extra("apices", "shp")),
                cloud: cloud.map(|c| {
                    let ext = c
                        .extension()
                        .map_or("las".into(), |e| e.to_string_lossy().into_owned());
                    let out = paths::unique(&paths::product(&c, &format!("treeid_{tag}"), &ext));
                    (c, out)
                }),
                chm,
                output,
                tops_file,
                tops: TopParams {
                    hmin,
                    a: ws_a,
                    b: ws_b,
                    c: ws_c,
                    shape: match shape {
                        ShapeArg::Circle => Shape::Circle,
                        ShapeArg::Square => Shape::Square,
                    },
                },
                crown: CrownParams {
                    method: seg,
                    th_tree: hmin,
                    th_seed,
                    th_cr,
                    max_cr,
                    max_cr_factor,
                    exclusion,
                    tolerance,
                    ext,
                },
                smooth,
                smoothing: smoothing_of(smoothing),
                field: field.map(|file| jobs::FieldJob {
                    file,
                    columns: hipso_core::forest::FieldColumns {
                        x: field_x,
                        y: field_y,
                        height: field_h,
                        plot: field_plot,
                    },
                    plot_x,
                    plot_y,
                    plot_radius,
                    max_dist: match_dist,
                }),
                around,
                margin,
            })?
        }
        Cmd::Obia {
            rasters,
            cloud,
            cloud_layers,
            cloud_fill,
            cloud_smooth,
            roughness,
            method,
            scale,
            shape,
            compact,
            weights,
            init,
            mask_layer,
            min,
            max,
            diagonal,
            min_cells,
            rules,
            samples,
            kmeans,
            features,
            strata,
            strata_min_cells,
            plots,
            cover_height,
            class_field,
            trees,
            seed,
            output,
            format,
            simplify,
        } => {
            let mut layers = Vec::new();
            for name in &cloud_layers {
                layers.push(
                    hipso_core::obia::layers::CloudLayer::parse(name)
                        .ok_or_else(|| Error::Param(format!("capa desconocida «{name}»")))?,
                );
            }
            let mut weighted = Vec::new();
            for w in &weights {
                let (n, v) = w
                    .split_once('=')
                    .ok_or_else(|| Error::Param(format!("«{w}»: se espera capa=peso")))?;
                weighted.push((
                    n.trim().to_string(),
                    v.trim()
                        .parse::<f32>()
                        .map_err(|_| Error::Param(format!("«{w}»: el peso no es un número")))?,
                ));
            }
            let seg_method = match method {
                ObiaMethod::Merge => hipso_core::obia::segment::Method::Merge(
                    hipso_core::obia::segment::MergeParams {
                        scale,
                        shape,
                        compact,
                        weights: weighted,
                        init,
                    },
                ),
                ObiaMethod::Mask => hipso_core::obia::segment::Method::Mask {
                    layer: mask_layer.ok_or_else(|| {
                        Error::Param("--method mask necesita --mask-layer".into())
                    })?,
                    min,
                    max,
                    diagonal,
                },
            };
            let first = rasters
                .first()
                .cloned()
                .ok_or_else(|| Error::Param("hace falta al menos un ráster".into()))?;
            let output =
                output.unwrap_or_else(|| paths::unique(&paths::product(&first, "objetos", "shp")));
            let formats = formats_of(format, &output);
            jobs::obia(&jobs::ObiaJob {
                gpkg: false,
                rasters,
                clouds: cloud,
                cloud_layers: layers,
                filter: hipso_core::filter::Filter::default(),
                cloud_fill,
                cloud_smooth,
                roughness,
                segment: hipso_core::obia::segment::SegmentParams {
                    method: seg_method,
                    min_cells,
                },
                classify: match (rules, samples, kmeans) {
                    (Some(f), _, _) => jobs::ClassifyBy::Rules(f),
                    (None, None, Some(k)) => jobs::ClassifyBy::Kmeans { k, features, seed },
                    (None, Some(file), _) => jobs::ClassifyBy::Samples {
                        file,
                        field: class_field,
                        forest: hipso_core::obia::classify::ForestParams {
                            trees,
                            seed,
                            ..Default::default()
                        },
                    },
                    (None, None, None) => jobs::ClassifyBy::Nothing,
                },
                output,
                formats,
                simplify,
                cover_height,
                strata: (strata || plots > 0).then_some(jobs::StrataOpts {
                    min_cells: strata_min_cells,
                    plots,
                }),
            })?
        }
        Cmd::Geojson { input, output } => {
            use hipso_core::view::{self, GeomKind};
            let to_shp = input.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case("geojson") || e.eq_ignore_ascii_case("json")
            });
            let output = output.unwrap_or_else(|| {
                paths::unique(&paths::product(
                    &input,
                    "convertido",
                    if to_shp { "shp" } else { "geojson" },
                ))
            });
            paths::ensure_parent(&output)?;
            let (data, kind, epsg) = if to_shp {
                let (d, k, e, skipped) = hipso_core::geojson::read_vector(&input)?;
                if skipped > 0 {
                    eprintln!(
                        "aviso: {skipped} elementos con otra geometría o sin ella no se han copiado"
                    );
                }
                (d, k, e)
            } else {
                let d = view::read_vector(&input)?;
                let k = if d.points {
                    GeomKind::Point
                } else if d.polygons {
                    GeomKind::Polygon
                } else {
                    GeomKind::Line
                };
                (d, k, hipso_core::crs::epsg_of(&input))
            };
            let prj = epsg.and_then(hipso_core::vector::prj_for_epsg);
            hipso_core::vector::write_layer(&output, &data, kind, prj.as_deref())?;
            format!(
                "{} elementos: {}{}",
                data.features.len(),
                output.display(),
                if epsg.is_none() {
                    "\nAviso: el origen no declara su sistema de coordenadas; la copia tampoco"
                } else {
                    ""
                }
            )
        }
        Cmd::Change {
            before,
            after,
            output,
            min_change,
            cover_height,
            min_patch,
            years,
            stands,
            stand_field,
            trees,
            hmin,
            ws_a,
            ws_b,
            smooth,
            match_dist,
            format,
        } => {
            let output =
                output.unwrap_or_else(|| paths::unique(&paths::product(&after, "cambio", "shp")));
            let formats = formats_of(format, &output);
            let job = hipso_core::change::ChangeJob {
                before,
                after,
                output,
                min_change,
                cover_height,
                min_patch,
                years,
                regions: stands.map(|f| (f, stand_field)),
                trees: trees.then_some(hipso_core::change::TreeChange {
                    tops: TopParams {
                        hmin,
                        a: ws_a,
                        b: ws_b,
                        c: 0.0,
                        shape: Shape::Circle,
                    },
                    smooth,
                    max_dist: match_dist,
                }),
                formats,
            };
            hipso_core::change::change(&job)?
        }
        Cmd::Import {
            input,
            output,
            epsg,
        } => {
            let ext = input
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "e57" {
                let output =
                    output.unwrap_or_else(|| paths::unique(&paths::product(&input, "e57", "laz")));
                let r = hipso_core::e57::convert(&input, &output, epsg)?;
                hipso_core::e57::describe(&r, &output)
                    + if epsg.is_none() {
                        "\nAviso: el E57 no dice su sistema de coordenadas; usa --epsg o `crs` para asignarlo"
                    } else {
                        ""
                    }
            } else {
                if ext != "ply" {
                    return Err(Error::Unsupported(format!(
                        "formato «{ext}» no soportado: se importan PLY y E57"
                    )));
                }
                let output =
                    output.unwrap_or_else(|| paths::unique(&paths::product(&input, "ply", "laz")));
                let r = hipso_core::ply::convert(&input, &output, epsg)?;
                hipso_core::ply::describe(&r, &output)
                    + if epsg.is_none() {
                        "\nAviso: el PLY no dice su sistema de coordenadas; usa --epsg o `crs` para asignarlo"
                    } else {
                        ""
                    }
            }
        }
        Cmd::Copc { inputs, output } => {
            if output.is_some() && inputs.len() > 1 {
                return Err(Error::Param("-o sirve para una sola nube".into()));
            }
            let mut lines = Vec::new();
            for p in &inputs {
                let out = output.clone().unwrap_or_else(|| {
                    let stem = paths::stem(p);
                    let stem = stem.strip_suffix(".copc").unwrap_or(&stem).to_string();
                    paths::unique(&paths::results_dir(p).join(format!("{stem}.copc.laz")))
                });
                let format = hipso_core::las::Reader::open(p)?.header.format;
                let r = hipso_core::copc_write::write(p, &out)?;
                lines.push(hipso_core::copc_write::describe(&r, format, &out));
            }
            lines.join("\n")
        }
        Cmd::Colorize {
            inputs,
            image,
            bands,
            nir,
            keep_outside,
            output,
        } => {
            if output.is_some() && inputs.len() > 1 {
                return Err(Error::Param("-o sirve para una sola nube".into()));
            }
            let pairs = inputs
                .iter()
                .map(|p| {
                    let out = output
                        .clone()
                        .unwrap_or_else(|| paths::unique(&paths::product(p, "rgb", &ext_of(p))));
                    (p.clone(), out)
                })
                .collect();
            hipso_core::colorize::run(&hipso_core::colorize::ColorizeJob {
                image,
                rgb: [bands[0], bands[1], bands[2]],
                nir,
                inputs: pairs,
                keep_outside,
            })?
        }
        Cmd::Intensity {
            inputs,
            trajectory,
            altitude,
            exponent,
            reference,
            output,
        } => {
            use hipso_core::intensity::{IntensityJob, Sensor, run};
            if output.is_some() && inputs.len() > 1 {
                return Err(Error::Param("-o sirve para una sola nube".into()));
            }
            let sensor = match (trajectory, altitude) {
                (Some(t), _) => Sensor::Trajectory(t),
                (None, Some(a)) => Sensor::Altitude(a),
                (None, None) => {
                    return Err(Error::Param(
                        "indica la trayectoria (--trajectory) o la altura de vuelo (--altitude)"
                            .into(),
                    ));
                }
            };
            let pairs = inputs
                .iter()
                .map(|p| {
                    let out = output.clone().unwrap_or_else(|| {
                        paths::unique(&paths::product(p, "intensidad", &ext_of(p)))
                    });
                    (p.clone(), out)
                })
                .collect();
            run(&IntensityJob {
                sensor,
                exponent,
                reference,
                inputs: pairs,
            })?
        }
        Cmd::Cog {
            input,
            output,
            nearest,
            tiles,
        } => {
            let output =
                output.unwrap_or_else(|| paths::unique(&paths::product(&input, "cog", "tif")));
            paths::ensure_parent(&output)?;
            // las imágenes de varias bandas o de números enteros (fotografías, escenas de satélite,
            // clases) se reescriben en flujo, sea cual sea su tamaño
            let pyr = hipso_core::pyramid::Pyramid::open(&input).ok();
            // (también un ráster grande de decimales: nunca tiene que caber en memoria;
            // HIPSO_STREAM=0 lo prohíbe)
            let big = |p: &hipso_core::pyramid::Pyramid| {
                let e = p.extent();
                let cells = ((e[2] - e[0]) / p.cell) * ((e[3] - e[1]) / p.cell);
                match std::env::var("HIPSO_STREAM").ok().as_deref() {
                    Some("0") => false,
                    Some(_) => true,
                    None => cells > 32e6,
                }
            };
            if let Some(p) = pyr.filter(|p| p.bands > 1 || p.bits <= 16 || big(p)) {
                use hipso_core::rastertools::{RasterJob, Resample, Source, stack};
                let mut job = RasterJob::new(
                    vec![Source {
                        path: input.clone(),
                        bands: Vec::new(),
                    }],
                    output,
                );
                job.nodata = p.nodata().unwrap_or(0.0);
                if p.bits > 16 {
                    // decimales: el valor de las celdas vacías no debe promediarse en las vistas
                    // reducidas
                    job.src_nodata = p.nodata();
                }
                job.compression = tiles.compression();
                if nearest {
                    job.resample = Resample::Nearest;
                }
                stack(&job)?
            } else {
                let g = hipso_core::grid::Grid::read(&input)?;
                hipso_core::cog::write(
                    &g,
                    &output,
                    hipso_core::grid::Grid::read_epsg(&input),
                    nearest,
                )?;
                format!(
                    "{} x {} celdas de {}: {}",
                    g.cols,
                    g.rows,
                    paths::num(g.cell),
                    output.display()
                )
            }
        }
        Cmd::Corridor {
            inputs,
            dtm,
            output,
            wire_class,
            pole_class,
            min_distance,
            fall_margin,
            half_width,
            link,
            gap_max,
            min_points,
            min_length,
            min_pole_height,
            cell,
            min_height,
            vegetation,
            pole_reach,
            format,
        } => {
            let first = inputs
                .first()
                .cloned()
                .ok_or_else(|| Error::Param("hace falta al menos una nube".into()))?;
            let dtm = dtm_for(dtm, &first)?;
            let output =
                output.unwrap_or_else(|| paths::unique(&paths::product(&first, "corredor", "shp")));
            let formats = formats_of(format, &output);
            let params = hipso_core::corridor::CorridorParams {
                wire_class,
                pole_class,
                link,
                min_points,
                min_length,
                min_pole_height,
                gap_max,
                half_width,
                min_distance,
                fall_margin,
                cell,
                min_height,
                pole_reach,
                vegetation: hipso_core::filter::ClassSel::parse(&vegetation)?,
                ..Default::default()
            };
            hipso_core::corridor::run(&hipso_core::corridor::CorridorJob {
                inputs,
                dtm,
                output,
                params,
                formats,
            })?
        }
        Cmd::Clip {
            inputs,
            regions,
            rect,
            circle,
            name_field,
            buffer,
            split_parts,
            radius,
            radius_field,
            square,
            side,
            side_x_field,
            side_y_field,
            sample,
            count,
            spacing,
            min_dist,
            seed,
            limit,
            layout,
            where_,
            mode,
            output,
            class,
            returns,
            zmin,
            zmax,
            dtm,
            z_offset,
            decimate,
            exclusive,
            keep_empty,
            max_open,
            metrics,
            above,
            minht,
            metrics_format,
            metrics_columns,
        } => {
            use hipso_core::clip::{ClipJob, ClipMode, SampleMetrics};
            use hipso_core::filter::{ClassSel, Filter, ReturnSel};
            use hipso_core::vector::{self, Region, Shape2};
            let mut list: Vec<Region> = Vec::new();
            if let Some(path) = &regions {
                let opts = vector::ReadOptions {
                    name_field,
                    radius,
                    radius_field,
                    square,
                    side,
                    side_x_field,
                    side_y_field,
                };
                let mut read = vector::read_regions(path, &opts)?;
                if split_parts {
                    read = read.iter().flat_map(|r| r.split_parts()).collect();
                }
                if let Some(d) = buffer.filter(|d| *d != 0.0) {
                    let geographic = hipso_core::crs::epsg_of(path)
                        .or_else(|| inputs.first().and_then(|p| hipso_core::crs::epsg_of(p)))
                        .is_some_and(|e| (4000..5000).contains(&e));
                    if geographic {
                        eprintln!("aviso: el sistema es geográfico: el buffer de {d} está en grados, no en metros");
                    }
                    read = read.iter().map(|r| r.buffered(d)).collect();
                }
                list.extend(read);
            }
            let first = inputs.first().cloned().unwrap_or_default();
            let mut layout_note = None;
            if let Some(kind) = sample {
                use hipso_core::plots::{PlotShape, Sampling, generate, write_layout};
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for p in &inputs {
                    let h = hipso_core::las::Reader::open(p)?.header.bounds;
                    b = [
                        b[0].min(h[0]),
                        b[1].min(h[1]),
                        b[2].max(h[3]),
                        b[3].max(h[4]),
                    ];
                }
                let shape = if square {
                    let w = side.ok_or_else(|| {
                        hipso_core::Error::Param(
                            "--side es el lado de las muestras en metros".into(),
                        )
                    })?;
                    PlotShape::Rect { w, h: w }
                } else {
                    PlotShape::Circle {
                        radius: radius.ok_or_else(|| {
                            hipso_core::Error::Param(
                                "--radius es el radio de las muestras en metros".into(),
                            )
                        })?,
                    }
                };
                let size = match shape {
                    PlotShape::Circle { radius } => 2.0 * radius,
                    PlotShape::Rect { w, h } => w.max(h),
                };
                let sampling = match kind {
                    SampleArg::Random => Sampling::Random {
                        count,
                        min_dist: min_dist.unwrap_or(2.0 * size),
                    },
                    SampleArg::Systematic => {
                        let sp = spacing.as_deref().ok_or_else(|| {
                            hipso_core::Error::Param(
                                "--spacing es la distancia entre muestras".into(),
                            )
                        })?;
                        let v: Vec<f64> = sp
                            .split(',')
                            .filter_map(|t| t.trim().parse().ok())
                            .collect();
                        let (dx, dy) = match v.as_slice() {
                            [a] => (*a, *a),
                            [a, c] => (*a, *c),
                            _ => {
                                return Err(hipso_core::Error::Param(
                                    "--spacing es X o X,Y".into(),
                                ));
                            }
                        };
                        Sampling::Systematic {
                            dx,
                            dy,
                            random_start: false,
                        }
                    }
                };
                let area: Option<Vec<vector::Polygon>> = match &limit {
                    Some(l) => Some(
                        vector::read_regions(l, &vector::ReadOptions::default())?
                            .into_iter()
                            .filter_map(|r| match r.shape {
                                Shape2::Polygon(p) => Some(p),
                                _ => None,
                            })
                            .collect(),
                    ),
                    None => None,
                };
                let plots = generate(b, area.as_deref(), shape, sampling, seed)?;
                let path = layout
                    .clone()
                    .unwrap_or_else(|| paths::unique(&paths::product(&first, "muestras", "shp")));
                let prj = hipso_core::las::Reader::open(&first)
                    .ok()
                    .and_then(|r| r.header.epsg)
                    .and_then(vector::prj_for_epsg);
                write_layout(&path, &plots, prj.as_deref())?;
                layout_note = Some(format!(
                    "{} muestras generadas: {}",
                    plots.len(),
                    path.display()
                ));
                list.extend(plots);
            }
            for (k, r) in rect.iter().enumerate() {
                let v = numbers(r, 4, "--rect")?;
                list.push(Region {
                    name: format!("rect{:02}", k + 1),
                    shape: Shape2::Rect([v[0], v[1], v[2], v[3]]),
                    attrs: vec![],
                });
            }
            for (k, c) in circle.iter().enumerate() {
                let v = numbers(c, 3, "--circle")?;
                list.push(Region {
                    name: format!("circ{:02}", k + 1),
                    shape: Shape2::Circle {
                        cx: v[0],
                        cy: v[1],
                        r: v[2],
                    },
                    attrs: vec![],
                });
            }
            if let Some(w) = &where_ {
                let (field, value) = w.split_once('=').ok_or_else(|| {
                    hipso_core::Error::Param("--where necesita CAMPO=VALOR".into())
                })?;
                list.retain(|r| {
                    r.attrs.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case(field.trim()) && v.eq_ignore_ascii_case(value.trim())
                    })
                });
            }
            let ext = ext_of(&first);
            let tag = match mode {
                ClipModeArg::Each => "muestra",
                ClipModeArg::Metrics => "muestras",
                ClipModeArg::Merge => "recorte",
            };
            let metrics = match (metrics, mode) {
                (None, ClipModeArg::Metrics) => Some(paths::unique(&paths::product(
                    &first,
                    "muestras_metricas",
                    "shp",
                ))),
                (m, _) => m,
            };
            let output = output.unwrap_or_else(|| paths::product(&first, tag, &ext));
            let mut job = ClipJob::new(inputs, list, output);
            job.regions_source = regions.clone();
            job.mode = match mode {
                ClipModeArg::Merge => ClipMode::Merge,
                ClipModeArg::Metrics => ClipMode::MetricsOnly,
                ClipModeArg::Each => ClipMode::PerRegion,
            };
            job.filter = Filter {
                classes: class
                    .as_deref()
                    .map_or(Ok(ClassSel::all()), ClassSel::parse)?,
                returns: returns
                    .as_deref()
                    .map_or(Ok(ReturnSel::all()), ReturnSel::parse)?,
                zmin,
                zmax,
                decimate,
                skip_noise: false,
            };
            job.dtm = dtm;
            job.z_offset = z_offset;
            job.exclusive = exclusive;
            job.keep_empty = keep_empty;
            job.max_open = max_open;
            job.metrics = metrics.map(|csv| SampleMetrics {
                formats: formats_of(metrics_format, &csv),
                csv,
                heightbreak: above,
                min_height: minht,
                only: metrics_columns,
            });
            let text = jobs::clip(&job)?;
            match layout_note {
                Some(n) => format!("{n}\n{text}"),
                None => text,
            }
        }
        Cmd::Filter {
            inputs,
            output,
            sel,
            dtm,
            z_offset,
        } => {
            let first = inputs.first().cloned().unwrap_or_default();
            let ext = first
                .extension()
                .map_or("las".into(), |e| e.to_string_lossy().into_owned());
            let tag = if inputs.len() > 1 {
                "unida"
            } else {
                "filtrada"
            };
            let mut job =
                hipso_core::ops::whole_cloud_job(inputs, out_path(output, &first, tag, &ext))?;
            job.filter = sel.filter()?;
            job.dtm = dtm;
            job.z_offset = z_offset;
            jobs::clip(&job)?
        }
        Cmd::Thin {
            inputs,
            output,
            method,
            cell,
            vcell,
            percentile,
            n,
            fraction,
            density,
            seed,
            sel,
        } => {
            use hipso_core::thin::{ThinMethod, ThinParams, report, thin_cloud};
            let params = ThinParams {
                method: match method {
                    ThinArg::Lowest => ThinMethod::Lowest,
                    ThinArg::Highest => ThinMethod::Highest,
                    ThinArg::Nearest => ThinMethod::Nearest,
                    ThinArg::Percentile => ThinMethod::Percentile(percentile),
                    ThinArg::Voxel => ThinMethod::Voxel { vertical: vcell },
                    ThinArg::Every => ThinMethod::EveryNth(n),
                    ThinArg::Remove => ThinMethod::RemoveNth(n),
                    ThinArg::Random => ThinMethod::Random { fraction, seed },
                    ThinArg::Density => ThinMethod::Density {
                        per_m2: density,
                        seed,
                    },
                },
                cell,
            };
            let filter = sel.filter()?;
            let ext_of = |p: &Path| {
                p.extension()
                    .map_or("las".into(), |e| e.to_string_lossy().into_owned())
            };
            let mut lines = Vec::new();
            if let Some(out) = output {
                let (r, k) = thin_cloud(&inputs, &out, &params, &filter)?;
                lines.push(report(r, k, &out));
            } else {
                for input in &inputs {
                    let out = out_path(
                        None,
                        input,
                        &format!("delgada_{}", params.tag()),
                        &ext_of(input),
                    );
                    let (r, k) = thin_cloud(std::slice::from_ref(input), &out, &params, &filter)?;
                    lines.push(report(r, k, &out));
                }
            }
            lines.join(
                "
",
            )
        }
        Cmd::Raster {
            inputs,
            output,
            cell,
            stat,
            sel,
        } => {
            use hipso_core::ops::{Stat, raster_stat};
            let first = inputs.first().cloned().unwrap_or_default();
            let (st, tag) = match stat {
                StatArg::Density => (Stat::Density, "densidad"),
                StatArg::Count => (Stat::Count, "cuenta"),
                StatArg::MeanZ => (Stat::MeanZ, "zmedia"),
                StatArg::MaxZ => (Stat::MaxZ, "zmax"),
                StatArg::MinZ => (Stat::MinZ, "zmin"),
                StatArg::RangeZ => (Stat::RangeZ, "zrango"),
                StatArg::MeanIntensity => (Stat::MeanIntensity, "imedia"),
                StatArg::MaxIntensity => (Stat::MaxIntensity, "imax"),
            };
            let out = out_path(
                output,
                &first,
                &format!("{tag}_{}m", paths::num(cell)),
                "tif",
            );
            let (g, epsg) = raster_stat(&inputs, cell, st, &sel.filter()?)?;
            paths::ensure_parent(&out)?;
            g.write_tiff(&out, epsg)?;
            format!("{} x {} celdas: {}", g.cols, g.rows, out.display())
        }
        Cmd::Hillshade {
            input,
            output,
            azimuth,
            altitude,
        } => {
            let g = hipso_core::grid::Grid::read(&input)?;
            let out = output
                .unwrap_or_else(|| paths::unique(&paths::product(&input, "sombreado", "png")));
            hipso_core::ops::hillshade(&g, azimuth, altitude).write_png(&out)?;
            format!("{} x {} px: {}", g.cols, g.rows, out.display())
        }
        Cmd::Contour {
            input,
            output,
            interval,
        } => {
            let g = hipso_core::grid::Grid::read(&input)?;
            let out = output.unwrap_or_else(|| {
                paths::unique(&paths::product(
                    &input,
                    &format!("curvas_{}m", paths::num(interval)),
                    "shp",
                ))
            });
            let n = hipso_core::ops::write_contours(&g, interval, &out, None)?;
            format!("{n} curvas de nivel: {}", out.display())
        }
        Cmd::Tile {
            inputs,
            output,
            size,
            buffer,
        } => {
            let first = inputs.first().cloned().unwrap_or_default();
            let ext = first
                .extension()
                .map_or("las".into(), |e| e.to_string_lossy().into_owned());
            let out = out_path(output, &first, "tile", &ext);
            let mut b = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for p in &inputs {
                let h = hipso_core::las::Reader::open(p)?.header.bounds;
                b = [
                    b[0].min(h[0]),
                    b[1].min(h[1]),
                    b[2].max(h[3]),
                    b[3].max(h[4]),
                ];
            }
            let regions = hipso_core::ops::tile_regions(b, size, buffer, "t")?;
            let mut job = hipso_core::clip::ClipJob::new(inputs, regions, out);
            job.mode = hipso_core::clip::ClipMode::PerRegion;
            jobs::clip(&job)?
        }
        Cmd::Edit {
            input,
            output,
            regions,
            radius,
            outside,
            class,
            zmin,
            zmax,
            delete,
            set_class,
        } => {
            use hipso_core::edit::{self, Action, Attr, Edit, Mode, Region, SelOp};
            use hipso_core::filter::ClassSel;
            use hipso_core::vector;
            let action = match (delete, set_class) {
                (true, _) => Action::Delete,
                (false, Some(c)) => Action::Reclassify(c),
                _ => {
                    return Err(hipso_core::Error::Param(
                        "indica qué hacer: --delete o --set-class N".into(),
                    ));
                }
            };
            let mut attr = Attr {
                z: match (zmin, zmax) {
                    (None, None) => None,
                    (a, b) => Some((a.unwrap_or(f64::NEG_INFINITY), b.unwrap_or(f64::INFINITY))),
                },
                ..Attr::default()
            };
            if let Some(c) = &class {
                let sel = ClassSel::parse(c)?;
                attr.classes = sel.set;
            }
            let mut sel = Vec::new();
            match &regions {
                Some(path) => {
                    let opts = vector::ReadOptions {
                        name_field: None,
                        radius,
                        radius_field: None,
                        ..vector::ReadOptions::default()
                    };
                    let list = vector::read_regions(path, &opts)?;
                    for (k, r) in hipso_core::ops::polygons_of(&list).into_iter().enumerate() {
                        sel.push(SelOp {
                            mode: if k == 0 { Mode::New } else { Mode::Add },
                            region: r,
                            attr: Attr {
                                classes: hipso_core::classes::ClassSet::all(),
                                z: None,
                                ..Attr::default()
                            },
                        });
                    }
                    if outside {
                        sel.push(SelOp {
                            mode: Mode::Invert,
                            region: Region::All,
                            attr: Attr::default(),
                        });
                    }
                    sel.push(SelOp {
                        mode: Mode::Intersect,
                        region: Region::All,
                        attr,
                    });
                }
                None => sel.push(SelOp {
                    mode: Mode::New,
                    region: Region::All,
                    attr,
                }),
            }
            let ext = input
                .extension()
                .map_or("las".into(), |e| e.to_string_lossy().into_owned());
            let out = out_path(output, &input, "editada", &ext);
            let label = match action {
                Action::Delete => "eliminar".to_string(),
                Action::Reclassify(c) => format!("clase {c}"),
            };
            let r = edit::save(&input, &out, &[Edit::new(sel, action, label)])?;
            edit::report_text(&r, &out)
        }
        Cmd::Crs {
            inputs,
            from,
            epsg,
            replace,
            output,
        } => {
            use hipso_core::ops::{CrsSource, assign_projection};
            let source = match (from, epsg) {
                (Some(f), _) => CrsSource::Cloud(f),
                (None, Some(e)) => CrsSource::Epsg(e),
                _ => {
                    return Err(hipso_core::Error::Param(
                        "give --from <cloud> or --epsg <code>".into(),
                    ));
                }
            };
            let output = single_output(output, inputs.len())?;
            let mut lines = Vec::new();
            for input in &inputs {
                let out = out_path(output.clone(), input, "crs", &ext_of(input));
                lines.push(assign_projection(input, &source, &out, replace)?);
            }
            lines.join("\n")
        }
        Cmd::Model {
            plots,
            field,
            response,
            plot_key,
            field_key,
            predictors,
            transform,
            criterion,
            max_vars,
            max_vif,
            candidates,
            max_corr,
            analyze,
            with_intensity,
            cells,
            output,
            interval,
            extrapolate,
            epsg,
        } => {
            use hipso_core::aba::{self, AbaJob, Criterion, Spec, Transform};
            let output = output.unwrap_or_else(|| {
                paths::unique_with(
                    &paths::product(&plots, "mapa", "tif"),
                    hipso_core::aba::COMPANIONS,
                )
            });
            let job = AbaJob {
                plots,
                field,
                response,
                field_key: field_key.unwrap_or_else(|| plot_key.clone()),
                plot_key,
                spec: Spec {
                    transform: match transform {
                        TransformArg::None => Transform::None,
                        TransformArg::LogResponse => Transform::LogResponse,
                        TransformArg::LogBoth => Transform::LogBoth,
                    },
                    criterion: match criterion {
                        CriterionArg::Bic => Criterion::Bic,
                        CriterionArg::Aicc => Criterion::Aicc,
                    },
                    predictors,
                    max_vars,
                    max_vif,
                    candidates,
                    max_corr,
                    intensity: with_intensity,
                },
                cells,
                output,
                interval_map: interval,
                mask_extrapolation: !extrapolate,
                epsg,
            };
            if analyze {
                let (sc, notes) = aba::analyze(&job)?;
                let mut text = aba::screening_text(
                    &sc,
                    &job.response,
                    job.spec.predictors.is_empty(),
                    job.spec.predictors.len(),
                );
                for n in notes {
                    text.push_str(&format!("  - {n}\n"));
                }
                text
            } else {
                aba::run(&job)?
            }
        }
        Cmd::Stack {
            inputs,
            output,
            cell,
            src_nodata,
            nodata,
            tiles,
        } => {
            use hipso_core::rastertools::{RasterJob, Source, stack};
            let mut job = RasterJob::new(
                inputs
                    .into_iter()
                    .map(|path| Source {
                        path,
                        bands: Vec::new(),
                    })
                    .collect(),
                output,
            );
            (job.cell, job.src_nodata, job.nodata) = (cell, src_nodata, nodata);
            job.compression = tiles.compression();
            stack(&job)?
        }
        Cmd::Align {
            reference,
            inputs,
            resample,
            nodata,
            tiles,
        } => {
            use hipso_core::rastertools::{AlignJob, Resample, align};
            let pairs = inputs
                .into_iter()
                .map(|p| {
                    let out = paths::unique(&paths::product(&p, "alineado", "tif"));
                    (p, out)
                })
                .collect();
            align(&AlignJob {
                reference,
                inputs: pairs,
                resample: match resample {
                    AlignResample::Auto => None,
                    AlignResample::Nearest => Some(Resample::Nearest),
                    AlignResample::Bilinear => Some(Resample::Bilinear),
                    AlignResample::Average => Some(Resample::Average),
                    AlignResample::Cubic => Some(Resample::Cubic),
                    AlignResample::Mode => Some(Resample::Mode),
                },
                dtype: None,
                nodata,
                src_nodata: None,
                compression: tiles.compression(),
            })?
        }
        Cmd::Hag {
            inputs,
            output,
            cell,
            ground_class,
            replace_z,
        } => {
            use hipso_core::hag::{HagParams, run};
            if output.is_some() && inputs.len() > 1 {
                return Err(hipso_core::Error::Param(
                    "-o solo vale con una nube; sin -o cada una da su archivo".into(),
                ));
            }
            let params = HagParams {
                cell,
                ground_class,
                replace_z,
            };
            let mut lines = Vec::new();
            for input in &inputs {
                let ext = input
                    .extension()
                    .map_or("las".into(), |e| e.to_string_lossy().into_owned());
                let out = out_path(
                    output.clone(),
                    input,
                    if replace_z { "normalizada" } else { "alturas" },
                    &ext,
                );
                lines.push(run(input, &out, &params)?);
            }
            lines.join("\n")
        }
        Cmd::Reproject {
            input,
            output,
            to,
            from,
            reference,
            resample,
            cell,
            nodata,
            tiles,
            vertical_in,
            vertical_out,
            allow_ballpark,
            allow_fallback,
        } => {
            use hipso_core::rastertools::{Resample, Target, WarpJob, warp};
            use hipso_core::reproject::{Policy, Vertical, VerticalRef};
            let how = match resample {
                AlignResample::Auto => None,
                AlignResample::Nearest => Some(Resample::Nearest),
                AlignResample::Bilinear => Some(Resample::Bilinear),
                AlignResample::Average => Some(Resample::Average),
                AlignResample::Cubic => Some(Resample::Cubic),
                AlignResample::Mode => Some(Resample::Mode),
            };
            if let Some(reference) = reference {
                let out = output.with_extension("tif");
                let mut job = WarpJob::new(input, out, Target::Reference(reference));
                job.from = from;
                job.resample = how;
                job.nodata = nodata;
                job.compression = tiles.compression();
                warp(&job)?
            } else {
                let to = to.ok_or_else(|| {
                    hipso_core::Error::Param("falta --to (o --reference)".into())
                })?;
                let mut policy = Policy::from_env();
                policy.allow_ballpark |= allow_ballpark;
                policy.only_best &= !allow_fallback;
                let parse = |v: Option<String>| v.map(|v| VerticalRef::parse(&v)).transpose();
                let vertical = Vertical {
                    input: parse(vertical_in)?,
                    output: parse(vertical_out)?,
                };
                let ext = input
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                if hipso_core::raster::is_raster_ext(&ext) && vertical.output.is_none() {
                    hipso_core::reproject::raster_file(
                        &input,
                        &output.with_extension("tif"),
                        to,
                        from,
                        how,
                        cell,
                        tiles.compression(),
                        policy,
                    )?
                } else {
                    hipso_core::reproject::file_with(&input, &output, to, from, vertical, policy)?
                }
            }
        }
        Cmd::Mosaic {
            inputs,
            output,
            overlap,
            cell,
            src_nodata,
            nodata,
            intersection,
            average,
            tiles,
        } => {
            use hipso_core::rastertools::{Extent, Overlap, RasterJob, Resample, Source, mosaic};
            let mut job = RasterJob::new(
                inputs
                    .into_iter()
                    .map(|path| Source {
                        path,
                        bands: Vec::new(),
                    })
                    .collect(),
                output,
            );
            job.overlap = match overlap {
                OverlapArg::First => Overlap::First,
                OverlapArg::Last => Overlap::Last,
                OverlapArg::Mean => Overlap::Mean,
                OverlapArg::Min => Overlap::Min,
                OverlapArg::Max => Overlap::Max,
                OverlapArg::Median => Overlap::Median,
                OverlapArg::Sum => Overlap::Sum,
                OverlapArg::Count => Overlap::Count,
            };
            (job.cell, job.src_nodata, job.nodata) = (cell, src_nodata, nodata);
            if intersection {
                job.extent = Extent::Intersection;
            }
            if average {
                job.resample = Resample::Average;
            }
            job.compression = tiles.compression();
            mosaic(&job)?
        }
        Cmd::FlightLines {
            inputs,
            gap,
            min_points,
            min_duration,
            assign,
            no_csv,
            no_vector,
            output,
        } => {
            use hipso_core::flightlines::{FlightJob, FlightParams, Write};
            let into = match assign.as_deref() {
                None => Write::None,
                Some("source-id") => Write::SourceId,
                Some("user-data") => Write::UserData,
                Some(o) => {
                    return Err(hipso_core::Error::Param(format!("--assign: «{o}» no válido (source-id, user-data)")));
                }
            };
            let first = inputs.first().cloned().unwrap_or_default();
            let dir = output.unwrap_or_else(|| paths::results_dir(&first));
            hipso_core::flightlines::run(&FlightJob {
                inputs,
                params: FlightParams { gap, min_points, min_duration, ..Default::default() },
                into,
                output_dir: dir,
                csv: !no_csv,
                vector: !no_vector,
            })?
        }
        Cmd::Voronoi {
            input,
            clip,
            delaunay,
            output,
        } => {
            use hipso_core::voronoi::Clip;
            let (d, _, _) = hipso_core::vecops::read_layer(&input)?;
            let clip = match clip.as_deref() {
                None => Clip::None,
                Some("bbox") => hipso_core::voronoi::clip_box_of(&d),
                Some(p) => hipso_core::voronoi::clip_from_layer(std::path::Path::new(p))?,
            };
            let out = out_path(output, &input, "voronoi", "shp");
            hipso_core::voronoi::run(&input, &out, &clip, delaunay)?
        }
        Cmd::VoronoiDensity {
            inputs,
            cell,
            threshold,
            returns,
            class,
            max_cell_area,
            no_pass_fail,
            no_csv,
            tile,
            buffer,
            output,
        } => {
            use hipso_core::filter::{ClassSel, Filter, ReturnSel};
            let first = inputs.first().cloned().unwrap_or_default();
            let out = output.unwrap_or_else(|| paths::product(&first, "vdensidad", "tif"));
            let mut job = hipso_core::vdensity::VDensityJob::new(inputs, out);
            job.cell = cell;
            job.threshold = threshold;
            job.max_cell_area = max_cell_area;
            job.filter = Filter {
                classes: class.as_deref().map_or(Ok(ClassSel::all()), ClassSel::parse)?,
                returns: ReturnSel::parse(&returns)?,
                skip_noise: true,
                ..Filter::default()
            };
            job.pass_fail = !no_pass_fail;
            job.csv = !no_csv;
            job.tiling.size = tile;
            job.tiling.buffer = buffer;
            hipso_core::vdensity::run(&job)?
        }
        Cmd::Flood {
            input,
            seed,
            border,
            bathtub,
            level,
            levels,
            relative,
            connectivity,
            extent,
            no_depth,
            no_csv,
            output,
        } => {
            use hipso_core::flood::{FloodJob, Relative, Seeds};
            let seeds = if bathtub {
                Seeds::All
            } else if !seed.is_empty() {
                Seeds::Points(seed.iter().map(|s| xy(s, "--seed")).collect::<std::result::Result<_, _>>()?)
            } else if border {
                Seeds::Border
            } else {
                return Err(hipso_core::Error::Param("indica de dónde viene el agua: --seed x,y, --border o --bathtub".into()).into());
            };
            let mut all = level;
            if let Some(r) = levels {
                let p: Vec<f64> = r.split(':').filter_map(|v| v.trim().replace(',', ".").parse().ok()).collect();
                let [a, b, st] = p[..] else {
                    return Err(hipso_core::Error::Param(format!("--levels: «{r}» no válido (desde:hasta:paso)")).into());
                };
                if !(st > 0.0) || b < a || (b - a) / st > 1000.0 {
                    return Err(hipso_core::Error::Param("--levels: el paso debe ser positivo y salir 1000 niveles como máximo".into()).into());
                }
                let n = ((b - a) / st + 1e-9).floor() as usize;
                all.extend((0..=n).map(|k| a + k as f64 * st));
            }
            let out = output.unwrap_or_else(|| paths::product(&input, "inundacion", "shp"));
            let mut job = FloodJob::new(input, seeds, all, out);
            job.relative = match relative {
                None => Relative::Absolute,
                Some(FloodRelative::Min) => Relative::MinZ,
                Some(FloodRelative::Seed) => Relative::Seed,
            };
            job.connectivity = connectivity;
            job.extent = extent;
            job.depth = !no_depth;
            job.csv = !no_csv;
            hipso_core::flood::run(&job)?
        }
        Cmd::Ridges {
            input,
            drainage,
            min_area,
            min_length,
            smooth,
            output,
        } => {
            use hipso_core::terrainlines::{Kind, LinesJob};
            let kind = if drainage { Kind::Drainage } else { Kind::Ridge };
            let out = output.unwrap_or_else(|| paths::product(&input, if drainage { "drenaje" } else { "crestas" }, "shp"));
            let mut job = LinesJob::new(input, kind, out);
            job.min_area = min_area;
            job.min_length = min_length;
            job.smooth = smooth;
            hipso_core::terrainlines::run(&job)?
        }
        Cmd::LeastCost {
            input,
            from,
            to,
            via,
            model,
            power,
            reference_slope,
            max_slope,
            barriers,
            prefer,
            prefer_factor,
            nearest,
            cost_distance,
            output,
        } => {
            use hipso_core::pathfind::{CostModel, PathJob};
            let mut pts = vec![xy(&from, "--from")?];
            for v in &via {
                pts.push(xy(v, "--via")?);
            }
            for t in &to {
                pts.push(xy(t, "--to")?);
            }
            if to.len() > 1 && !nearest {
                return Err(hipso_core::Error::Param("varios --to piden --nearest".into()).into());
            }
            let model = match model.as_str() {
                "slope" => CostModel::Slope { power, reference_pct: reference_slope },
                "tobler" => CostModel::Tobler,
                "uniform" => CostModel::Uniform,
                m => match m.split_once(':') {
                    Some(("raster", f)) => CostModel::Raster(PathBuf::from(f)),
                    _ => return Err(hipso_core::Error::Param(format!("--model: «{m}» no válido (slope, tobler, uniform, raster:<tif>)")).into()),
                },
            };
            let out = output.unwrap_or_else(|| paths::product(&input, "ruta", "shp"));
            let mut job = PathJob::new(input, pts, out);
            job.model = model;
            job.constraints.max_slope_pct = max_slope;
            job.constraints.barriers = barriers;
            job.constraints.prefer = prefer.map(|p| (p, prefer_factor));
            job.nearest = nearest;
            job.cost_distance = cost_distance;
            hipso_core::pathfind::run(&job)?
        }
        Cmd::Piles {
            surface,
            piles,
            base,
            min_height,
            density,
            output,
            no_csv,
            base_raster,
            height_raster,
        } => {
            use hipso_core::piles::{Base, PileJob};
            let bad = |t: &str| {
                hipso_core::Error::Param(format!(
                    "--base: «{t}» no válido (plane, lowest, median, interpolated, level:<z>, dtm:<archivo>)"
                ))
            };
            let base = match base.as_str() {
                "plane" => Base::Plane,
                "lowest" => Base::LowestPerimeter,
                "median" => Base::MedianPerimeter,
                "interpolated" => Base::Interpolated,
                t => match t.split_once(':') {
                    Some(("level", z)) => Base::Level(z.trim().parse().map_err(|_| bad(t))?),
                    Some(("dtm", f)) => Base::Dtm(PathBuf::from(f)),
                    _ => return Err(bad(t).into()),
                },
            };
            let out = output.unwrap_or_else(|| paths::product(&surface, "pilas", "shp"));
            let out = if out.extension().is_some() { out } else { out.with_extension("shp") };
            let mut job = PileJob::new(surface, piles, out);
            job.base = base;
            job.min_height = min_height;
            job.density = density;
            job.csv = !no_csv;
            job.base_raster = base_raster;
            job.height_raster = height_raster;
            hipso_core::piles::run(&job)?
        }
        Cmd::CutFill {
            existing,
            design,
            regions,
            tolerance,
            output,
            no_csv,
            classified,
        } => {
            use hipso_core::cutfill::{CutFillJob, Design};
            let bad = |t: &str| {
                hipso_core::Error::Param(format!(
                    "--design: «{t}» no válido (un archivo, level:<z> o plane:<z0>,<pendiente %>,<orientación>,<x>,<y>)"
                ))
            };
            let number = |t: &str, whole: &str| t.trim().replace(',', ".").parse::<f64>().map_err(|_| bad(whole));
            let design = match design.split_once(':') {
                Some(("level", z)) => Design::Level(number(z, &design)?),
                Some(("plane", v)) => {
                    let p: Vec<&str> = v.split(',').collect();
                    if p.len() != 5 {
                        return Err(bad(&design).into());
                    }
                    Design::Plane {
                        z0: number(p[0], &design)?,
                        slope_pct: number(p[1], &design)?,
                        aspect_deg: number(p[2], &design)?,
                        origin: [number(p[3], &design)?, number(p[4], &design)?],
                    }
                }
                _ => Design::Surface(PathBuf::from(&design)),
            };
            let out = output.unwrap_or_else(|| paths::product(&existing, "cutfill", "tif"));
            let mut job = CutFillJob::new(existing, design, out);
            job.regions = regions;
            job.tolerance = tolerance;
            job.csv = !no_csv;
            job.classified = classified;
            hipso_core::cutfill::run(&job)?
        }
        Cmd::Calc {
            expr,
            inputs,
            output,
            dtype,
            nearest,
            align,
        } => {
            use hipso_core::rastertools::{Dtype, Resample};
            let mut named: Vec<(String, PathBuf)> = Vec::new();
            for (i, a) in inputs.iter().enumerate() {
                match a.split_once('=') {
                    Some((n, p)) => named.push((n.trim().to_string(), PathBuf::from(p))),
                    None if inputs.len() == 1 => named.push(("A".to_string(), PathBuf::from(a))),
                    None => {
                        return Err(hipso_core::Error::Param(format!(
                            "«{a}» (ráster {}): con varios rásteres cada uno va como NOMBRE=archivo",
                            i + 1
                        ))
                        .into());
                    }
                }
            }
            let out = out_path(output, &named[0].1, "calc", "tif");
            let mut job = hipso_core::rastercalc::CalcJob::new(expr, named, out);
            job.dtype = match dtype {
                CalcDtype::F32 => Dtype::F32,
                CalcDtype::U8 => Dtype::U8,
                CalcDtype::U16 => Dtype::U16,
            };
            job.nearest = nearest;
            job.align = align.and_then(|a| match a {
                AlignResample::Auto => Some(Resample::Bilinear),
                AlignResample::Nearest => Some(Resample::Nearest),
                AlignResample::Bilinear => Some(Resample::Bilinear),
                AlignResample::Average => Some(Resample::Average),
                AlignResample::Cubic => Some(Resample::Cubic),
                AlignResample::Mode => Some(Resample::Mode),
            });
            hipso_core::rastercalc::run(&job)?
        }
        Cmd::Vectorize {
            input,
            output,
            range,
            breaks,
            classes: _,
            min_cells,
            simplify,
            diagonal,
        } => {
            let out = out_path(output, &input, "poligonos", "shp");
            let mut job = hipso_core::rastervec::VecJob::new(input, out);
            job.mode = if let [lo, hi] = range[..] {
                hipso_core::rastervec::Mode::Range { lo, hi }
            } else if !breaks.is_empty() {
                hipso_core::rastervec::Mode::Breaks(breaks)
            } else {
                hipso_core::rastervec::Mode::Classes
            };
            job.min_cells = min_cells;
            job.simplify = simplify;
            job.diagonal = diagonal;
            hipso_core::rastervec::run(&job)?
        }
        Cmd::Ponds {
            input,
            output,
            method,
            smooth,
            nodata,
            min_depth,
            min_area,
            max_area,
            max_depth,
            min_volume,
            max_slope,
            min_catchment,
            depth_raster,
            gpkg,
        } => {
            let out = out_path(output, &input, "charcos", "shp");
            let mut job = hipso_core::ponds::PondJob::new(input, out);
            job.method = match method.as_str() {
                "depression" => hipso_core::ponds::Method::Depression,
                "local" => hipso_core::ponds::Method::Local,
                o => return Err(hipso_core::Error::Param(format!("--method: «{o}» no válido (depression, local)")).into()),
            };
            job.smooth = smooth;
            job.nodata = nodata;
            job.min_depth = min_depth;
            job.min_area = min_area;
            job.max_area = max_area;
            job.max_depth = max_depth;
            job.min_volume = min_volume;
            job.max_slope = max_slope;
            job.min_catchment = min_catchment;
            job.depth_raster = depth_raster;
            job.gpkg = gpkg;
            hipso_core::ponds::run(&job)?
        }
        Cmd::Terrain {
            input,
            product,
            output,
            percent,
            edges,
            z_factor,
            azimuth,
            altitude,
            radius,
            threshold,
            observer,
            eye,
            target,
            max_distance,
        } => {
            use hipso_core::terrain::{Product, TerrainJob, run as terrain};
            let product = match product {
                TerrainArg::Slope => Product::Slope,
                TerrainArg::Aspect => Product::Aspect,
                TerrainArg::Hillshade => Product::Hillshade,
                TerrainArg::PlanCurvature => Product::PlanCurvature,
                TerrainArg::ProfileCurvature => Product::ProfileCurvature,
                TerrainArg::Curvature => Product::Curvature,
                TerrainArg::Roughness => Product::Roughness,
                TerrainArg::Tri => Product::Ruggedness,
                TerrainArg::Tpi => Product::Position,
                TerrainArg::FillDepth => Product::FillDepth,
                TerrainArg::FlowAccumulation => Product::FlowAccumulation,
                TerrainArg::Twi => Product::Wetness,
                TerrainArg::Streams => Product::Streams,
                TerrainArg::Subbasins => Product::Subbasins,
                TerrainArg::Viewshed => Product::Viewshed,
            };
            let out = out_path(output, &input, product.tag(), "tif");
            let mut job = TerrainJob::new(input, out, product);
            job.percent = percent;
            job.edges = match edges.as_str() {
                "nodata" => hipso_core::terrain::Edges::NoData,
                "repeat" => hipso_core::terrain::Edges::RepeatEdge,
                "center" => hipso_core::terrain::Edges::Center,
                o => return Err(hipso_core::Error::Param(format!("--edges: «{o}» no válido (nodata, repeat, center)")).into()),
            };
            job.z_factor = z_factor;
            (job.azimuth, job.altitude) = (azimuth, altitude);
            (job.radius, job.threshold) = (radius, threshold);
            job.observer = observer.and_then(|o| Some([*o.first()?, *o.get(1)?]));
            (job.observer_height, job.target_height) = (eye, target);
            job.max_distance = max_distance;
            terrain(&job)?
        }
        Cmd::Zonal {
            raster,
            regions,
            prefix,
            split_parts,
            output,
        } => {
            use hipso_core::{geomops, grid::Grid, vector, zonal};
            let g = Grid::read(&raster)?;
            let mut list = vector::read_regions(&regions, &vector::ReadOptions::default())?;
            if split_parts {
                list = list.iter().flat_map(|r| r.split_parts()).collect();
            }
            let (a, b) = (hipso_core::crs::epsg_of(&raster), hipso_core::crs::epsg_of(&regions));
            let mut warn = String::new();
            if let (Some(a), Some(b)) = (a, b)
                && a != b
            {
                warn = format!("\naviso: el ráster es EPSG:{a} y los polígonos EPSG:{b}: los resultados no tendrán sentido; reproyecta antes");
            }
            let mut layer = zonal::zonal_layer(&g, &list, &prefix)?;
            layer.prj = std::fs::read_to_string(regions.with_extension("prj"))
                .ok()
                .or_else(|| b.or(a).and_then(vector::prj_for_epsg));
            let out = out_path(output, &raster, "zonal", "shp");
            geomops::write_polygons(&layer, &out)?;
            let with = layer
                .features
                .iter()
                .filter(|f| f.attrs.iter().any(|(k, v)| k.ends_with("count") && v != "0"))
                .count();
            format!(
                "{} polígonos, {with} con celdas; campos nuevos: {} -> {}{warn}",
                layer.features.len(),
                zonal::FIELDS.iter().map(|f| format!("{prefix}{f}")).collect::<Vec<_>>().join(", "),
                out.display()
            )
        }
        Cmd::IsValid { input } => {
            let layer = hipso_core::geomops::read_polygons(&input)?;
            let bad = hipso_core::geomops::invalid(&layer);
            if bad.is_empty() {
                format!("{} polígonos, todos válidos", layer.features.len())
            } else {
                let mut lines: Vec<String> = bad
                    .iter()
                    .take(20)
                    .map(|(i, why)| format!("  polígono {}: {why}", i + 1))
                    .collect();
                if bad.len() > 20 {
                    lines.push(format!("  … y {} más", bad.len() - 20));
                }
                format!(
                    "{} de {} polígonos no son válidos (se pueden reparar con make-valid):\n{}",
                    bad.len(),
                    layer.features.len(),
                    lines.join("\n")
                )
            }
        }
        Cmd::MakeValid { input, output } => {
            let layer = hipso_core::geomops::read_polygons(&input)?;
            let (fixed, n) = hipso_core::geomops::make_valid(&layer)?;
            let out = out_path(output, &input, "valido", "shp");
            hipso_core::geomops::write_polygons(&fixed, &out)?;
            format!("{n} de {} polígonos reparados -> {}", fixed.features.len(), out.display())
        }
        Cmd::Buffer {
            input,
            distance,
            make_valid,
            output,
        } => {
            use hipso_core::geomops;
            let mut layer = geomops::read_polygons(&input)?;
            if make_valid {
                layer = geomops::make_valid(&layer)?.0;
            }
            if hipso_core::crs::epsg_of(&input).is_some_and(|e| (4000..5000).contains(&e)) {
                eprintln!("aviso: el sistema es geográfico: la distancia de {distance} está en grados, no en metros");
            }
            let r = geomops::buffer(&layer, distance)?;
            let out = out_path(output, &input, "buffer", "shp");
            geomops::write_polygons(&r, &out)?;
            let empty = r.features.iter().filter(|f| f.geom.0.is_empty()).count();
            format!(
                "{} polígonos con buffer de {distance}{} -> {}",
                r.features.len() - empty,
                if empty > 0 { format!("; {empty} desaparecen y no se escriben") } else { String::new() },
                out.display()
            )
        }
        Cmd::Dissolve {
            input,
            field,
            make_valid,
            output,
        } => {
            use hipso_core::geomops;
            let mut layer = geomops::read_polygons(&input)?;
            if make_valid {
                layer = geomops::make_valid(&layer)?.0;
            }
            let n = layer.features.len();
            let r = geomops::dissolve(&layer, field.as_deref())?;
            let out = out_path(output, &input, "disuelto", "shp");
            geomops::write_polygons(&r, &out)?;
            format!("{n} polígonos disueltos en {} -> {}", r.features.len(), out.display())
        }
        Cmd::Union {
            first,
            second,
            make_valid,
            output,
        } => {
            use hipso_core::geomops;
            let (mut a, mut b) = (geomops::read_polygons(&first)?, geomops::read_polygons(&second)?);
            if make_valid {
                a = geomops::make_valid(&a)?.0;
                b = geomops::make_valid(&b)?.0;
            }
            let (ea, eb) = (hipso_core::crs::epsg_of(&first), hipso_core::crs::epsg_of(&second));
            let warn = match (ea, eb) {
                (Some(x), Some(y)) if x != y => format!("\naviso: las capas están en EPSG:{x} y EPSG:{y}: reproyecta antes"),
                _ => String::new(),
            };
            let r = geomops::union(&a, &b)?;
            let out = out_path(output, &first, "union", "shp");
            geomops::write_polygons(&r, &out)?;
            format!("{} piezas -> {}{warn}", r.features.len(), out.display())
        }
        Cmd::Intersect {
            first,
            second,
            make_valid,
            output,
        } => {
            use hipso_core::geomops;
            let (mut a, mut b) = (geomops::read_polygons(&first)?, geomops::read_polygons(&second)?);
            if make_valid {
                a = geomops::make_valid(&a)?.0;
                b = geomops::make_valid(&b)?.0;
            }
            let (ea, eb) = (hipso_core::crs::epsg_of(&first), hipso_core::crs::epsg_of(&second));
            let warn = match (ea, eb) {
                (Some(x), Some(y)) if x != y => format!("\naviso: las capas están en EPSG:{x} y EPSG:{y}: reproyecta antes"),
                _ => String::new(),
            };
            let r = geomops::intersect(&a, &b)?;
            let out = out_path(output, &first, "interseccion", "shp");
            geomops::write_polygons(&r, &out)?;
            format!("{} piezas -> {}{warn}", r.features.len(), out.display())
        }
        Cmd::Reclass {
            input,
            rules,
            rules_file,
            others,
            band,
            src_nodata,
            nodata,
            output,
            tiles,
        } => {
            use hipso_core::reclass::{Out, ReclassJob, Table};
            let text = match (rules, rules_file) {
                (Some(t), _) => t,
                (None, Some(f)) => std::fs::read_to_string(f)?,
                _ => String::new(),
            };
            let mut table = Table::parse(&text)?;
            if !table.explicit_else {
                table.otherwise = match others.trim().to_lowercase().as_str() {
                    "keep" | "igual" => Out::Keep,
                    "nodata" => Out::NoData,
                    v => Out::Value(v.replace(',', ".").parse::<f32>().map_err(|_| {
                        hipso_core::Error::Param(format!(
                            "--others: «{v}» no es keep, nodata ni un número"
                        ))
                    })?),
                };
            }
            let out = out_path(output, &input, "reclass", "tif");
            let mut job = ReclassJob::new(input, out, table);
            job.band = band;
            job.src_nodata = src_nodata;
            job.nodata = nodata;
            job.compression = tiles.compression();
            hipso_core::reclass::run(&job)?
        }
        Cmd::ClipRaster {
            input,
            mask,
            no_crop,
            invert,
            nodata,
            output,
            tiles,
        } => {
            use hipso_core::rastermask::{Mask, MaskJob, clip, regions_of};
            let ext = mask
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let m = if hipso_core::raster::is_raster_ext(&ext) {
                Mask::Raster(mask)
            } else {
                Mask::Regions(regions_of(&mask, hipso_core::crs::epsg_of(&input))?)
            };
            let out = out_path(output, &input, "recorte", "tif");
            let mut job = MaskJob::new(input, out, m);
            job.crop = !no_crop;
            job.invert = invert;
            job.nodata = nodata;
            job.compression = tiles.compression();
            clip(&job)?
        }
        Cmd::Sieve {
            input,
            threshold,
            eight,
            output,
        } => {
            use hipso_core::{grid::Grid, rasterops};
            let g = Grid::read(&input)?;
            let conn = if eight {
                rasterops::Connectivity::Eight
            } else {
                rasterops::Connectivity::Four
            };
            let r = rasterops::sieve(&g, threshold, conn);
            let changed = g.data.iter().zip(&r.data).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
            let out = out_path(output, &input, "sieve", "tif");
            paths::ensure_parent(&out)?;
            r.write_tiff(&out, Grid::read_epsg(&input))?;
            format!("{changed} celdas cambiadas -> {}", out.display())
        }
        Cmd::Proximity {
            input,
            values,
            max_distance,
            cells,
            output,
        } => {
            use hipso_core::{grid::Grid, rasterops};
            let g = Grid::read(&input)?;
            let targets = if values.is_empty() {
                rasterops::Targets::NonZero
            } else {
                rasterops::Targets::Values(values)
            };
            let r = rasterops::proximity(&g, &targets, max_distance, cells)?;
            let out = out_path(output, &input, "proximity", "tif");
            paths::ensure_parent(&out)?;
            r.write_tiff(&out, Grid::read_epsg(&input))?;
            format!("distancias -> {}", out.display())
        }
        Cmd::FillNodata {
            input,
            max_distance,
            smoothing,
            output,
        } => {
            use hipso_core::{grid::Grid, rasterops};
            let g = Grid::read(&input)?;
            let r = rasterops::fill_nodata(&g, max_distance, smoothing);
            let empty = |g: &Grid| g.data.iter().filter(|v| v.is_nan()).count();
            let (before, after) = (empty(&g), empty(&r));
            let out = out_path(output, &input, "relleno", "tif");
            paths::ensure_parent(&out)?;
            r.write_tiff(&out, Grid::read_epsg(&input))?;
            format!(
                "{} celdas rellenadas, {after} siguen vacías -> {}",
                before - after,
                out.display()
            )
        }
        Cmd::Dedup {
            inputs,
            output,
            all_copies,
            also,
        } => {
            use hipso_core::cloudtools::remove_duplicates;
            let f = dup_fields(&also)?;
            let output = single_output(output, inputs.len())?;
            let mut lines = Vec::new();
            for input in &inputs {
                let out = out_path(output.clone(), input, "sin_duplicados", &ext_of(input));
                let (read, removed) = remove_duplicates(input, &out, f, all_copies)?;
                lines.push(format!(
                    "{read} puntos leídos, {removed} duplicados quitados: {}",
                    out.display()
                ));
            }
            lines.join("\n")
        }
        Cmd::ApplyEdits {
            inputs,
            edits,
            output,
        } => {
            let list = hipso_core::edit::file::read(&edits)?;
            if !list.iter().any(|e| e.enabled) {
                return Err(hipso_core::Error::Param(format!(
                    "{} no tiene ninguna edición activa",
                    edits.display()
                )));
            }
            let output = single_output(output, inputs.len())?;
            let mut lines = Vec::new();
            for input in &inputs {
                let out = out_path(output.clone(), input, "editada", &ext_of(input));
                let r = hipso_core::edit::save(input, &out, &list)?;
                lines.push(hipso_core::edit::report_text(&r, &out));
            }
            lines.join("\n")
        }
        Cmd::MergeClouds {
            inputs,
            output,
            dups,
            also,
        } => {
            use hipso_core::merge::{DupAction, merge};
            let first = inputs.first().cloned().unwrap_or_default();
            let out = out_path(output, &first, "unida", &ext_of(&first));
            let action = match dups {
                DupsArg::Keep => DupAction::Keep,
                DupsArg::Mark => DupAction::Mark,
                DupsArg::Delete => DupAction::Delete,
            };
            let r = merge(&inputs, &out, action, dup_fields(&also)?)?;
            let mut text = format!("{} nubes unidas, {} puntos", inputs.len(), r.points);
            match action {
                DupAction::Keep => {}
                DupAction::Mark => text += &format!(", {} duplicados marcados", r.duplicates),
                DupAction::Delete => text += &format!(", {} duplicados quitados", r.duplicates),
            }
            for n in &r.notes {
                text += &format!("\n  aviso: {n}");
            }
            format!("{text}: {}", out.display())
        }
        Cmd::Compare {
            input,
            reference,
            output,
            distance,
            neighbors,
        } => {
            use hipso_core::cloudtools::{CompareParams, compare_clouds, compare_text};
            let out = out_path(output, &input, "cambios", &ext_of(&input));
            let rep = compare_clouds(
                &reference,
                &input,
                &out,
                &CompareParams {
                    min_dist: distance,
                    min_neighbors: neighbors,
                },
            )?;
            compare_text(&rep, &out)
        }
        Cmd::Fit {
            input,
            reference,
            output,
            thin,
            max_xy,
            max_z,
            iterations,
            stop,
            shift_only,
            plane,
            trim,
        } => {
            use hipso_core::cloudtools::{FitParams, fit_cloud, fit_text};
            let out = out_path(output, &input, "ajustada", &ext_of(&input));
            let rep = fit_cloud(
                &reference,
                &input,
                &out,
                &FitParams {
                    thin,
                    max_xy,
                    max_z,
                    max_iter: iterations,
                    stop_dist: stop,
                    shift_only,
                    plane,
                    trim,
                },
            )?;
            fit_text(&rep, &out)
        }
        Cmd::Swath {
            inputs,
            output,
            cell,
            returns,
            by_cloud,
            gps_gap,
        } => {
            use hipso_core::cloudtools::{SwathBy, SwathReturns, swath_separation_by, swath_text};
            let returns = match returns.as_str() {
                "all" => SwathReturns::All,
                "last" => SwathReturns::Last,
                "single" => SwathReturns::Single,
                o => {
                    return Err(hipso_core::Error::Param(format!(
                        "'{o}': usa all, last o single"
                    )));
                }
            };
            let first = inputs.first().cloned().unwrap_or_default();
            let out = out_path(output, &first, "pasadas", "tif");
            let by = match (gps_gap, by_cloud) {
                (Some(g), _) => SwathBy::GpsGap(g),
                (None, true) => SwathBy::Cloud,
                (None, false) => SwathBy::SourceId,
            };
            let rep = swath_separation_by(&inputs, &out, cell, returns, by)?;
            swath_text(&rep, &out)
        }
        Cmd::StemVolume {
            inputs,
            output,
            csv,
            normalized,
            base_height,
            top_diameter,
            log_length,
            voxel,
            stripe_low,
            stripe_high,
            max_diameter,
            tree_clouds,
            stem_classes,
            assign_dist,
            stool_dist,
            fork_height,
            max_gap,
            lost_top_share,
        } => {
            use hipso_core::tls::{TlsJob, TlsParams, run};
            let output = single_output(output, inputs.len())?;
            let mut lines = Vec::new();
            for input in &inputs {
                let job = TlsJob {
                    input: input.clone(),
                    output: out_path(output.clone(), input, "volumen", "shp"),
                    params: TlsParams {
                        voxel,
                        normalized,
                        base_height,
                        top_diam: top_diameter,
                        log_len: log_length,
                        stripe: (stripe_low, stripe_high),
                        max_diam: max_diameter,
                        assign_dist,
                        stool_dist,
                        fork_height,
                        max_gap,
                        lost_top_share,
                        ..TlsParams::default()
                    },
                    formats: paths::Formats { vector: true, csv },
                    tree_clouds,
                    stem_classes,
                };
                lines.push(run(&job)?);
            }
            lines.join(
                "
",
            )
        }
        Cmd::Render {
            inputs,
            output,
            view,
            color,
            size,
            max_points,
            point_size,
            yaw,
            pitch,
            profile,
            shade,
            contours,
            class,
        } => {
            use hipso_core::filter::ClassSel;
            use hipso_core::view::{self, ColorBy, Orbit, Plan, ProfileBox, Style};
            let (w, h) = size
                .split_once('x')
                .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
                .filter(|(w, h)| *w > 0 && *h > 0 && w * h <= 1 << 28)
                .ok_or_else(|| {
                    hipso_core::Error::Param(format!("--size necesita ANCHOxALTO, no '{size}'"))
                })?;
            let mut store = view::load(&inputs, max_points)?;
            let mut st = Style::fit(&store);
            st.color_by = match color {
                ColorArg::Elevation => ColorBy::Elevation,
                ColorArg::Earth => ColorBy::ElevationEarth,
                ColorArg::Class => ColorBy::Class,
                ColorArg::Intensity => ColorBy::Intensity,
                ColorArg::Return => ColorBy::Return,
                ColorArg::Source => ColorBy::Source,
                ColorArg::File => ColorBy::File,
                ColorArg::Rgb => ColorBy::Rgb,
                ColorArg::Tree => ColorBy::Tree,
                ColorArg::Hag => ColorBy::HeightAboveGround,
                ColorArg::ScanAngle => ColorBy::ScanAngle,
                ColorArg::ClassIntensity => ColorBy::ClassIntensity,
                ColorArg::ElevationIntensity => ColorBy::ElevationIntensity,
            };
            if st.color_by == ColorBy::HeightAboveGround {
                if !store.compute_hag(1.0) {
                    return Err(hipso_core::Error::Param(
                        "--color hag necesita puntos de suelo clasificados (clase 2) en la nube"
                            .into(),
                    ));
                }
                st.hag_max = store.hag_range();
            }
            st.point_size = point_size;
            let mask_of =
                |c: &str| -> Result<hipso_core::classes::ClassSet> { Ok(ClassSel::parse(c)?.set) };
            if let Some(c) = &class {
                st.classes = mask_of(c)?;
            }
            let img = match view {
                ViewArg::Plan => {
                    let v = Plan::fit(store.bounds, w, h);
                    let mut img = view::render_plan(&store, &v, w, h, &st);
                    if shade.is_some() || contours.is_some() {
                        let mask = match &shade {
                            Some(c) if !c.is_empty() => mask_of(c)?,
                            _ => hipso_core::classes::ClassSet::all(),
                        };
                        let g = view::surface(&store, mask, 1024)?;
                        if shade.is_some() {
                            let sh = view::shade(&g, &st, 315.0, 45.0, true);
                            let mut base = view::Image::filled(w, h, st.background);
                            view::paint_surface(&mut base, &g, &sh, &v);
                            img = base;
                        }
                        if let Some(iv) = contours {
                            view::paint_contours(
                                &mut img,
                                &view::contours(&g, iv),
                                &v,
                                [250, 250, 250],
                            );
                        }
                    }
                    img
                }
                ViewArg::Three => {
                    let mut o = Orbit::fit(store.bounds);
                    o.yaw = yaw.to_radians();
                    o.pitch = pitch.to_radians();
                    view::render_3d(&store, &o, w, h, &st)
                }
                ViewArg::Profile => {
                    let b = &store.bounds;
                    let bx = match &profile {
                        Some(p) => {
                            let v = numbers(p, 5, "--profile")?;
                            ProfileBox {
                                a: [v[0], v[1]],
                                b: [v[2], v[3]],
                                width: v[4],
                            }
                        }
                        None => ProfileBox {
                            a: [b[0], (b[1] + b[4]) / 2.0],
                            b: [b[3], (b[1] + b[4]) / 2.0],
                            width: ((b[4] - b[1]) / 20.0).max(1.0),
                        },
                    };
                    view::render_profile(&store, &bx, w, h, &st, None).0
                }
            };
            let first = inputs.first().cloned().unwrap_or_default();
            let out = output.unwrap_or_else(|| paths::product(&first, "vista", "png"));
            img.write_png(&out)?;
            format!(
                "{} de {} puntos (1 de cada {}), {}x{} px: {}",
                store.len(),
                store.total,
                store.stride,
                w,
                h,
                out.display()
            )
        }
        Cmd::Export {
            inputs,
            output,
            normalize,
            dtm,
            items,
            delimiter,
            header,
            decimals,
            class,
            returns,
            zmin,
            zmax,
            decimate,
        } => {
            use hipso_core::export::{self, AsciiJob, Item};
            use hipso_core::filter::{ClassSel, Filter, ReturnSel};
            let filter = Filter {
                classes: class
                    .as_deref()
                    .map_or(Ok(ClassSel::all()), ClassSel::parse)?,
                returns: returns
                    .as_deref()
                    .map_or(Ok(ReturnSel::all()), ReturnSel::parse)?,
                zmin,
                zmax,
                decimate,
                skip_noise: false,
            };
            if inputs.len() > 1 && dtm.is_some() {
                return Err(hipso_core::Error::Param(
                    "--dtm indica un solo MDT: con varias nubes, cada una usa el MDT hecho a partir de ella"
                        .into(),
                ));
            }
            let dtms = if normalize {
                inputs
                    .iter()
                    .map(|p| dtm_for(dtm.clone(), p))
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let ext = output
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "shp" || ext == "las" || ext == "laz" {
                let mut total = export::Exported::default();
                export::begin_for(&inputs);
                for (k, i) in inputs.iter().enumerate() {
                    let out = if inputs.len() > 1 {
                        output.with_file_name(format!("{}_{}.{ext}", paths::stem(&output), k + 1))
                    } else {
                        output.clone()
                    };
                    let d = dtms.get(k).map(PathBuf::as_path);
                    total.add(&if ext == "shp" {
                        export::shapefile_points(i, &out, &filter, d)?
                    } else {
                        export::cloud(i, &out, &filter, d)?
                    });
                }
                total.text(&output.display().to_string())
            } else {
                let items = items
                    .iter()
                    .map(|t| match t.to_ascii_lowercase().as_str() {
                        "x" => Ok(Item::X),
                        "y" => Ok(Item::Y),
                        "z" => Ok(Item::Z),
                        "intensity" | "i" => Ok(Item::Intensity),
                        "return" | "r" => Ok(Item::ReturnNumber),
                        "returns" | "n" => Ok(Item::Returns),
                        "class" | "c" => Ok(Item::Class),
                        "source" | "s" => Ok(Item::SourceId),
                        "red" => Ok(Item::Red),
                        "green" | "g" => Ok(Item::Green),
                        "blue" | "b" => Ok(Item::Blue),
                        o => Err(hipso_core::Error::Param(format!("dato desconocido '{o}'"))),
                    })
                    .collect::<Result<Vec<_>>>()?;
                let delimiter = match delimiter.as_str() {
                    "tab" => '\t',
                    "space" => ' ',
                    d => d.chars().next().unwrap_or(','),
                };
                let (files, n) = export::ascii(&AsciiJob {
                    inputs,
                    output,
                    items,
                    delimiter,
                    header,
                    filter,
                    decimals,
                    dtms,
                })?;
                n.text(&format!(
                    "{} archivo(s): {}",
                    files.len(),
                    files
                        .first()
                        .map_or(String::new(), |f| f.display().to_string())
                ))
            }
        }
        Cmd::Metrics {
            inputs,
            dtm,
            output,
            cell,
            heightbreak,
            neighbors,
            min_points,
            memory_mb,
            intensity,
            strata,
            strata_limits,
            int_strata_limits,
            structure,
            structure_layer,
            structure_top,
            gap_height,
            extinction,
            cbh_floor,
            cbh_share,
            format,
        } => {
            let output = single_output(output, inputs.len())?;
            if inputs.len() > 1 && dtm.is_some() {
                return Err(hipso_core::Error::Param(
                    "--dtm indica un solo MDT: con varias hojas, cada una usa el MDT hecho a partir de ella"
                        .into(),
                ));
            }
            let formats = formats_of(format, output.as_deref().unwrap_or(Path::new("x.shp")));
            let tag = format!("metricas_{}m", paths::num(cell));
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, "shp");
            let dtm_of = |p: &Path| dtm_for(dtm.clone(), p);
            let base = jobs::MetricsJob {
                input: PathBuf::new(),
                dtm: PathBuf::new(),
                output: PathBuf::new(),
                formats,
                params: MetricParams {
                    cell,
                    heightbreak,
                    min_points,
                    memory: memory_mb.saturating_mul(1 << 20),
                    strata: strata_limits,
                    int_strata: int_strata_limits,
                    structure: structure.then_some(hipso_core::metrics::StructureParams {
                        layer: structure_layer,
                        top: structure_top,
                        gap_height,
                        k: extinction,
                        cbh_floor,
                        cbh_share,
                    }),
                },
                intensity,
                strata,
            };
            hipso_core::sheets::metrics_each(&inputs, &base, &out_for, &dtm_of, neighbors)?
        }
    };
    println!("{summary}");
    eprintln!("terminado en {:.2} s", t.elapsed().as_secs_f64());
    Ok(())
}

/// Sustituye cada `--recipe ARCHIVO` por los argumentos que guarda el archivo (véase
/// `rules::recipe_args`).
fn expand_recipes(args: Vec<String>) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let path = if a == "--recipe" {
            Some(
                it.next()
                    .ok_or_else(|| Error::Param("--recipe necesita un archivo".into()))?,
            )
        } else {
            a.strip_prefix("--recipe=").map(str::to_string)
        };
        match path {
            Some(p) => {
                let text = std::fs::read_to_string(&p)
                    .map_err(|e| Error::Param(format!("no se lee la receta '{p}': {e}")))?;
                out.extend(hipso_core::rules::recipe_args(&text)?);
            }
            None => out.push(a),
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    // El análisis y `run` retienen los argumentos de todas las órdenes; las compilaciones de
    // depuración necesitan más pila que el hilo principal.
    let result = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let cli = match expand_recipes(std::env::args().collect()) {
                Ok(args) => Cli::parse_from(args),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            paths::set_output_dir(cli.results_dir);
            run(cli.cmd)
        })
        .map_err(hipso_core::Error::Io)
        .and_then(|h| {
            h.join()
                .unwrap_or_else(|_| Err(hipso_core::Error::Format("error interno".into())))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod recipe_tests {
    use super::*;
    use hipso_core::recipe;

    #[derive(Parser)]
    struct Probe {
        #[command(flatten)]
        steps: StepArgs,
    }

    /// Lo que escribe la ventana de clasificación como receta lo lee la línea de comandos como la
    /// misma clasificación, para todos los métodos de suelo y de ruido.
    #[test]
    fn a_written_recipe_gives_the_command_line_the_same_steps() -> Result<()> {
        let texts = [
            "--overlap --overlap-cell 4 --ground kraus --iterations 7 --g -3 --noise sor --sor-k 8 --sor-m 2.5 --tile 300 --buffer 20 --threads 2",
            "--ground mcc --mcc-spacing 2 --mcc-threshold 0.4 --noise isolated --noise-step 3 --isolated 4",
            "--ground csf --csf-resolution 1 --csf-rigidness 2 --csf-threshold 0.3",
            "--ground pmf --pmf-window 30 --pmf-slope 0.5",
            "--ground smrf --smrf-cell 0.5 --smrf-slope 0.3 --smrf-threshold 0.4 --smrf-scalar 1.5",
            "--ground axelsson --axelsson-seed-cell 30 --axelsson-angle 10 --axelsson-dist 1 --axelsson-flat 0.2 --buildings --building-offset 2.5 --building-window 40 --building-area 30",
            "--ground classes --noise elm --elm-cell 12 --elm-threshold 2.5 --vegetation --wires --poles --rule from=1;height=..0.5;to=3 --rule angle=25;to=12",
        ];
        for text in texts {
            let loaded = recipe::parse(text)?;
            let written = recipe::write(&loaded.recipe);
            let mut args = vec!["hipso".to_string()];
            args.extend(hipso_core::rules::recipe_args(&written)?);
            let probe = Probe::try_parse_from(args).map_err(|e| Error::Param(e.to_string()))?;
            assert_eq!(probe.steps.steps(), loaded.recipe.steps, "{text}");
            assert_eq!(probe.steps.tile, loaded.recipe.tiling.size, "{text}");
            assert_eq!(probe.steps.threads, loaded.recipe.tiling.threads, "{text}");
        }
        Ok(())
    }
}
