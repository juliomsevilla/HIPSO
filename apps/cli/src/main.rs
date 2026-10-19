//! `easylidar`: forestry products from LiDAR point clouds, from the command line.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::{Args, Parser, Subcommand, ValueEnum};
use rustlidar_core::axelsson::AxelssonParams;
use rustlidar_core::buildings::BuildingParams;
use rustlidar_core::csf::CsfParams;
use rustlidar_core::ground::GroundParams;
use rustlidar_core::mcc::MccParams;
use rustlidar_core::metrics::MetricParams;
use rustlidar_core::noise::NoiseParams;
use rustlidar_core::pipeline::{self, Ground, Noise, Steps, Surface, Tiling};
use rustlidar_core::pmf::PmfParams;
use rustlidar_core::rules::ClassRule;
use rustlidar_core::trees::{CrownParams, Segmenter, Shape, Smoothing, TopParams};
use rustlidar_core::{Result, jobs, paths};

#[derive(Parser)]
#[command(
    name = "easylidar",
    version,
    about = "Streaming LiDAR forestry tools: classification, DTM, canopy height model and grid metrics"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum GroundMethod {
    /// Iterative filter of Kraus & Pfeifer (default)
    Kraus,
    /// Multiscale Curvature Classification (Evans & Hudak 2007)
    Mcc,
    /// Cloth Simulation Filter (Zhang et al. 2016)
    Csf,
    /// Progressive Morphological Filter (Zhang et al. 2003)
    Pmf,
    /// Progressive TIN densification (Axelsson 2000)
    Axelsson,
    /// Points already classified as ground (class 2)
    Classes,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum NoiseMethod {
    /// Isolated points in 3 x 3 x 3 cells
    Isolated,
    /// Statistical outlier removal on k nearest neighbours
    Sor,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ChmMethod {
    /// Highest point per cell
    Max,
    /// Pit-free stack of TINs (Khosravipour et al. 2014)
    Pitfree,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ClipModeArg {
    /// One cloud with the points of all the plots
    Merge,
    /// One cloud per plot
    Each,
    /// No clouds: only the plots layer with its metrics
    Metrics,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SampleArg {
    Random,
    Systematic,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ViewArg {
    Plan,
    #[value(name = "3d")]
    Three,
    Profile,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ColorArg {
    Elevation,
    Earth,
    Class,
    Intensity,
    Return,
    Source,
    File,
    Rgb,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ThinArg {
    /// Lowest point of each cell
    Lowest,
    /// Highest point of each cell
    Highest,
    /// Point nearest to each cell's centre
    Nearest,
    /// Point at --percentile of each cell's elevations
    Percentile,
    /// Nearest to the centre of each 3D cell (--vcell high)
    Voxel,
    /// Every n-th point (--n)
    Every,
    /// Remove every n-th point (--n)
    Remove,
    /// Random sample (--fraction)
    Random,
    /// Random sample down to --density points per m²
    Density,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum OverlapArg {
    First,
    Last,
    Mean,
    Min,
    Max,
    Median,
    Sum,
    Count,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum StatArg {
    Density,
    Count,
    MeanZ,
    MaxZ,
    MinZ,
    RangeZ,
    MeanIntensity,
    MaxIntensity,
}

/// Point selection options shared by the tools that read the whole cloud.
#[derive(Args)]
struct SelArgs {
    /// Classes to keep: 2,5 or ~7,18 for all but those
    #[arg(long)]
    class: Option<String>,
    /// Returns to keep: A, F, L or numbers (e.g. 1,2)
    #[arg(long)]
    returns: Option<String>,
    #[arg(long, allow_negative_numbers = true)]
    zmin: Option<f64>,
    #[arg(long, allow_negative_numbers = true)]
    zmax: Option<f64>,
    /// Keep one point in this many
    #[arg(long, default_value_t = 1)]
    decimate: u64,
}

impl SelArgs {
    fn filter(&self) -> Result<rustlidar_core::filter::Filter> {
        use rustlidar_core::filter::{ClassSel, Filter, ReturnSel};
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
    Dalponte,
    Silva,
    Watershed,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum FormatArg {
    /// Shapefile only
    Vector,
    /// Text table only (best for very large runs)
    Csv,
    /// Both
    Both,
}

/// The formats asked for, or those the output name implies (`.csv`: text, else vector).
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
enum SmoothArg {
    Mean,
    Median,
    Gaussian,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ShapeArg {
    Circle,
    Square,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum SurfaceMethod {
    /// Mean elevation of the ground points per cell (streamed, one pass)
    Mean,
    /// Delaunay triangulation (TIN) of the ground points (default)
    Tin,
    /// Inverse-distance weighting of the nearest ground points
    Idw,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum DsmArg {
    /// Highest point of each cell (streamed, one pass)
    Max,
    /// TIN through the highest point of each cell (default)
    Tin,
    /// Inverse-distance weighting of the highest points of the cells
    Idw,
}

#[derive(Args)]
struct GroundArgs {
    /// How ground points are found (classify: kraus; dtm: classes, the class 2 already in the cloud)
    #[arg(long, value_enum)]
    ground: Option<GroundMethod>,
    /// Kraus-Pfeifer: cell size of the filter's intermediate surfaces
    #[arg(long, default_value_t = 5.0)]
    filter_cell: f64,
    /// Kraus-Pfeifer: filter iterations
    #[arg(long, default_value_t = 5)]
    iterations: u32,
    /// Kraus-Pfeifer: weight function g
    #[arg(long, default_value_t = -2.0, allow_negative_numbers = true)]
    g: f64,
    /// Kraus-Pfeifer: weight function w
    #[arg(long, default_value_t = 2.5)]
    w: f64,
    /// Kraus-Pfeifer: weight function a
    #[arg(long, default_value_t = 1.0)]
    a: f64,
    /// Kraus-Pfeifer: weight function b
    #[arg(long, default_value_t = 4.0)]
    b: f64,
    /// Kraus-Pfeifer: ground = within this distance of the final surface (default: residual <= g + w)
    #[arg(long)]
    tolerance: Option<f64>,
    /// MCC: cell size of scale domain 2 (about the point spacing)
    #[arg(long, default_value_t = 1.5)]
    mcc_spacing: f64,
    /// MCC: curvature threshold
    #[arg(long, default_value_t = 0.3)]
    mcc_threshold: f64,
    /// CSF: distance between cloth particles
    #[arg(long, default_value_t = 0.5)]
    csf_resolution: f64,
    /// CSF: rigidness, 1 (steep terrain) to 3 (flat)
    #[arg(long, default_value_t = 1)]
    csf_rigidness: u8,
    /// CSF: height above the cloth up to which points are ground
    #[arg(long, default_value_t = 0.5)]
    csf_threshold: f64,
    /// PMF: largest window
    #[arg(long, default_value_t = 20.0)]
    pmf_window: f64,
    /// PMF: terrain slope (rise over run)
    #[arg(long, default_value_t = 1.0)]
    pmf_slope: f64,
    /// Axelsson: side of the cells whose lowest points seed the TIN (larger than any building)
    #[arg(long, default_value_t = 20.0)]
    axelsson_seed_cell: f64,
    /// Axelsson: iteration angle (degrees)
    #[arg(long, default_value_t = 8.0)]
    axelsson_angle: f64,
    /// Axelsson: iteration distance (m)
    #[arg(long, default_value_t = 1.4)]
    axelsson_dist: f64,
    /// Axelsson: points this close to a triangle (m) are ground whatever the angle
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
    /// Classify noise (class 7) first, with this method
    #[arg(long, value_enum)]
    noise: Option<NoiseMethod>,
    /// Noise (sor): neighbours per point
    #[arg(long, default_value_t = 10)]
    sor_k: usize,
    /// Noise (sor): standard deviations above the mean distance
    #[arg(long, default_value_t = 3.0)]
    sor_m: f64,
    /// Noise (isolated): cell size in x and y
    #[arg(long, default_value_t = 4.0)]
    noise_step: f64,
    /// Noise: cell size in z
    #[arg(long, default_value_t = 4.0)]
    noise_step_z: f64,
    /// Noise: most other points around a point still called isolated
    #[arg(long, default_value_t = 5)]
    isolated: u32,
    /// Classify vegetation by height above ground: low (3), medium (4) and high (5)
    #[arg(long)]
    vegetation: bool,
    /// Classify wires (14): nearly horizontal lines high above the ground
    #[arg(long)]
    wires: bool,
    /// Classify poles and towers (15): thin, nearly vertical lines
    #[arg(long)]
    poles: bool,
    /// Classify by echo and scan angle, last and in order. Repeat for several rules. Parts,
    /// separated by `;`: to=CLASS (needed), from=1,2 (or ~7), returns=F|L|A|1..9, angle=25..90
    /// (absolute degrees; `angle=25` is 25 and over). For example: `angle=25;to=12`
    #[arg(long = "rule", value_parser = parse_rule)]
    rules: Vec<ClassRule>,
    /// Classify buildings (6) and high vegetation (5), and drop roofs from the ground
    #[arg(long)]
    buildings: bool,
    /// Buildings: height above the terrain from which points are buildings or trees
    #[arg(long, default_value_t = 2.0)]
    building_offset: f64,
    /// Buildings: widest building expected
    #[arg(long, default_value_t = 60.0)]
    building_window: f64,
    /// Buildings: smallest footprint (square units)
    #[arg(long, default_value_t = 40.0)]
    building_area: f64,
    /// Tile size
    #[arg(long, default_value_t = 250.0)]
    tile: f64,
    /// Buffer read around each tile (raised to half the building window when classifying buildings)
    #[arg(long, default_value_t = 30.0)]
    buffer: f64,
    /// Threads (0: one per core)
    #[arg(long, default_value_t = 0)]
    threads: usize,
}

impl StepArgs {
    /// What distinguishes the products of these settings in a file name.
    fn tag(&self) -> String {
        self.tag_with(GroundMethod::Kraus)
    }

    fn tag_with(&self, default: GroundMethod) -> String {
        let g = match self.ground.ground.unwrap_or(default) {
            GroundMethod::Kraus => "",
            GroundMethod::Mcc => "_mcc",
            GroundMethod::Csf => "_csf",
            GroundMethod::Pmf => "_pmf",
            GroundMethod::Axelsson => "_axelsson",
            GroundMethod::Classes => "_clase2",
        };
        let n = match self.noise {
            None => "",
            Some(NoiseMethod::Isolated) => "_aislados",
            Some(NoiseMethod::Sor) => "_sor",
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

    fn extras(&self) -> rustlidar_core::extras::Extras {
        use rustlidar_core::extras::{Extras, PoleParams, VegParams, WireParams};
        Extras {
            vegetation: self.vegetation.then(VegParams::default),
            wires: self.wires.then(WireParams::default),
            poles: self.poles.then(PoleParams::default),
        }
    }

    fn steps_with(&self, default: GroundMethod) -> Steps {
        let g = &self.ground;
        Steps {
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
    /// Full report on LAS/LAZ files: header, classes, returns, density, warnings
    Info {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Classify noise, ground and buildings, tile by tile in parallel, and write the points
    Classify {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output LAS, or LAZ when it ends in .laz; default: <results>/<input>_clasif<settings>
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        steps: StepArgs,
    },
    /// DTM as GeoTIFF, one per sheet. Default: the ground class (2) already in the cloud;
    /// --ground tries another method
    Dtm {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output GeoTIFF; default: <results>/<input>_mdt_<cell>m<settings>.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// DTM cell size
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// How cells get their elevation
        #[arg(long, value_enum, default_value_t = SurfaceMethod::Tin)]
        surface: SurfaceMethod,
        /// Smooth the DTM with a window of this size in map units (0: none)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Also join the sheets' DTMs into one mosaic (by default each sheet keeps its own)
        #[arg(long)]
        mosaic: bool,
        #[command(flatten)]
        steps: StepArgs,
    },
    /// Canopy height model as GeoTIFF, one per sheet
    Chm {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Ground model (GeoTIFF or ASCII grid); default: the newest DTM made from this same cloud
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Output GeoTIFF; default: <results>/<input>_chm_<cell>m.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Leave cells without points empty (max method)
        #[arg(long)]
        no_fill: bool,
        /// Highest point per cell, or pit-free TIN stack (Khosravipour et al. 2014)
        #[arg(long, value_enum, default_value_t = ChmMethod::Max)]
        method: ChmMethod,
        /// Pit-free: longest triangle edge in the layers above 0
        #[arg(long, default_value_t = 1.0)]
        max_edge: f64,
        /// Pit-free: tile size
        #[arg(long, default_value_t = 250.0)]
        tile: f64,
        /// Pit-free: threads (0: one per core)
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Smooth the CHM with a window of this size in map units (0: none)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Also join the sheets' CHMs into one mosaic (by default each sheet keeps its own)
        #[arg(long)]
        mosaic: bool,
    },
    /// Digital surface model (highest returns) as a GeoTIFF, one per sheet
    Dsm {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output GeoTIFF (one input only); default: <results>/<input>_mds_<cell>m_<method>.tif
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        #[arg(long, value_enum, default_value_t = DsmArg::Tin)]
        method: DsmArg,
        /// Leave cells without points empty (max method)
        #[arg(long)]
        no_fill: bool,
        /// Tile size (tin and idw methods)
        #[arg(long, default_value_t = 250.0)]
        tile: f64,
        /// Threads (0: one per core)
        #[arg(long, default_value_t = 0)]
        threads: usize,
        /// Smooth with a window of this size in map units (0: none)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
    },
    /// Individual trees from a canopy height model: tops (local maxima), crowns (Dalponte,
    /// Silva or watershed), polygons, per-point treeID and a check against field data
    Trees {
        /// Canopy height model (GeoTIFF or ASCII grid)
        chm: PathBuf,
        /// Tree tops (.shp, or .csv if you name one); default: <results>/<chm>_arboles_<method>.shp (never an existing file)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Also write the crown raster (tree ids)
        #[arg(long)]
        crowns: bool,
        /// Also write crown polygons (shapefile)
        #[arg(long)]
        crowns_shp: bool,
        /// Also write tree tops (PointZ shapefile)
        #[arg(long)]
        tops_shp: bool,
        /// Copy this cloud adding a treeID attribute (LAS/LAZ), to <results>/<cloud>_treeid
        #[arg(long)]
        cloud: Option<PathBuf>,
        /// Tops given by hand: CSV of x,y lines (instead of searching)
        #[arg(long)]
        tops_file: Option<PathBuf>,
        /// Lowest tree top
        #[arg(long, default_value_t = 2.0)]
        hmin: f64,
        /// Window diameter = a + b h + c h² (map units)
        #[arg(long, default_value_t = 3.0)]
        ws_a: f64,
        #[arg(long, default_value_t = 0.1)]
        ws_b: f64,
        #[arg(long, default_value_t = 0.0)]
        ws_c: f64,
        /// Window shape of the tree-top search
        #[arg(long, value_enum, default_value_t = ShapeArg::Circle)]
        shape: ShapeArg,
        /// Crown method
        #[arg(long, value_enum, default_value_t = SegArg::Dalponte)]
        method: SegArg,
        /// Dalponte: a pixel joins above this share of its top's height...
        #[arg(long, default_value_t = 0.45)]
        th_seed: f64,
        /// ...and of the crown's mean height
        #[arg(long, default_value_t = 0.55)]
        th_cr: f64,
        /// Dalponte: crown reach in pixels
        #[arg(long, default_value_t = 10.0)]
        max_cr: f64,
        /// Silva: crown radius as a share of tree height
        #[arg(long, default_value_t = 0.6)]
        max_cr_factor: f64,
        /// Silva: drop pixels lower than this share of tree height
        #[arg(long, default_value_t = 0.3)]
        exclusion: f64,
        /// Watershed: minimum height step between crowns
        #[arg(long, default_value_t = 1.0)]
        tolerance: f64,
        /// Watershed: peak search radius in pixels
        #[arg(long, default_value_t = 1)]
        ext: usize,
        /// Smooth the CHM first with a window of this size (map units; 0: none)
        #[arg(long, default_value_t = 0.0)]
        smooth: f64,
        #[arg(long, value_enum, default_value_t = SmoothArg::Mean)]
        smoothing: SmoothArg,
        /// Field inventory (tab/;/, delimited, decimal commas allowed) to check against
        #[arg(long)]
        field: Option<PathBuf>,
        #[arg(long, default_value = "X_arbol")]
        field_x: String,
        #[arg(long, default_value = "Y_arbol")]
        field_y: String,
        #[arg(long, default_value = "Ht")]
        field_h: String,
        #[arg(long, default_value = "Parcela")]
        field_plot: String,
        #[arg(long, default_value = "X_Parcela")]
        plot_x: String,
        #[arg(long, default_value = "Y_Parcela")]
        plot_y: String,
        /// Plot radius for the check
        #[arg(long, default_value_t = 10.0)]
        plot_radius: f64,
        /// Output format: shapefile of tree tops, text table, or both
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
        /// A detection matches a field tree within this distance
        #[arg(long, default_value_t = 3.0)]
        match_dist: f64,
    },
    /// Clip clouds with polygons, plots or rectangles
    Clip {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Polygons (.shp), plot centres (.shp points, with --radius), or a list (.csv/.txt)
        #[arg(long)]
        regions: Option<PathBuf>,
        /// A rectangle: minx,miny,maxx,maxy (repeat for several)
        #[arg(long)]
        rect: Vec<String>,
        /// A circular plot: x,y,radius (repeat for several)
        #[arg(long)]
        circle: Vec<String>,
        /// The attribute that names each region: a field name or its 1-based position
        #[arg(long)]
        name_field: Option<String>,
        /// Radius of plots given as points
        #[arg(long)]
        radius: Option<f64>,
        /// Attribute holding each point's own radius (metres)
        #[arg(long)]
        radius_field: Option<String>,
        /// Plots given as points are squares instead of circles
        #[arg(long)]
        square: bool,
        /// Side of square plots, in metres (or give the fields that hold the sides)
        #[arg(long)]
        side: Option<f64>,
        #[arg(long)]
        side_x_field: Option<String>,
        #[arg(long)]
        side_y_field: Option<String>,
        /// Generate the plots instead of reading them: at random or on a regular grid
        #[arg(long, value_enum)]
        sample: Option<SampleArg>,
        /// Random sampling: number of plots
        #[arg(long, default_value_t = 10)]
        count: usize,
        /// Systematic sampling: spacing between centres in metres, X or X,Y
        #[arg(long)]
        spacing: Option<String>,
        /// Random sampling: least distance between centres (default: twice the plot size)
        #[arg(long)]
        min_dist: Option<f64>,
        /// Seed of the random sampling: the same seed gives the same plots
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Generated plots stay inside these polygons (a stand)
        #[arg(long)]
        limit: Option<PathBuf>,
        /// Where the layer with the plots is saved; default: next to the results
        #[arg(long)]
        layout: Option<PathBuf>,
        /// Keep only the regions whose attribute has a value: FIELD=VALUE
        #[arg(long = "where")]
        where_: Option<String>,
        #[arg(long, value_enum, default_value_t = ClipModeArg::Merge)]
        mode: ClipModeArg,
        /// Output file (a base name with --mode each); default: next to the results
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Classes to keep: 2,5 or ~7,18 for all but those
        #[arg(long)]
        class: Option<String>,
        /// Returns to keep: A, F, L or numbers (e.g. 1,2)
        #[arg(long)]
        returns: Option<String>,
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Ground model: write heights above ground instead of elevations
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Added to every elevation written
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        z_offset: f64,
        /// Keep one point in this many
        #[arg(long, default_value_t = 1)]
        decimate: u64,
        /// Overlapping regions: a point goes only to the first
        #[arg(long)]
        exclusive: bool,
        /// Write files for regions without points
        #[arg(long)]
        keep_empty: bool,
        /// Files open at once with --mode each
        #[arg(long, default_value_t = 64)]
        max_open: usize,
        /// Also write CloudMetrics of every region to this CSV
        #[arg(long)]
        metrics: Option<PathBuf>,
        /// Metrics: height break for cover
        #[arg(long, default_value_t = 2.0)]
        above: f64,
        /// Metrics table format: shapefile of the regions, text table, or both
        #[arg(long, value_enum)]
        metrics_format: Option<FormatArg>,
        /// Metrics: only returns above this height
        #[arg(long, default_value_t = 0.0)]
        minht: f64,
    },
    /// Join, filter, normalise or convert clouds in one pass (LAS or LAZ by the output extension)
    Filter {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output; default: <results>/<input>_filtrada.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        sel: SelArgs,
        /// Write heights above this ground model instead of elevations
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Added to every elevation written
        #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
        z_offset: f64,
    },
    /// Thin (decimate) a cloud: by cell, 3D cell, every n-th point, at random or to a density.
    /// Each input gets its own thinned file unless -o is given (then they are joined).
    Thin {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = ThinArg::Lowest)]
        method: ThinArg,
        /// Cell size in metres (cell methods)
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Height of the 3D cell in metres (voxel)
        #[arg(long, default_value_t = 1.0)]
        vcell: f64,
        /// Percentile 0-100 (percentile; 50 = median)
        #[arg(long, default_value_t = 50.0)]
        percentile: f64,
        /// n of every / remove
        #[arg(long, default_value_t = 2)]
        n: u64,
        /// Share of points to keep, 0-1 (random)
        #[arg(long, default_value_t = 0.5)]
        fraction: f64,
        /// Points per m² to leave (density)
        #[arg(long, default_value_t = 1.0)]
        density: f64,
        /// Seed of the random methods
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[command(flatten)]
        sel: SelArgs,
    },
    /// Grid of a point statistic (density, count, elevation, intensity) as GeoTIFF
    Raster {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        #[arg(long, value_enum, default_value_t = StatArg::Density)]
        stat: StatArg,
        #[command(flatten)]
        sel: SelArgs,
    },
    /// Hillshade PNG of a DTM or CHM
    Hillshade {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 315.0)]
        azimuth: f64,
        #[arg(long, default_value_t = 45.0)]
        altitude: f64,
    },
    /// Contour lines of a DTM or CHM as a 3D polyline shapefile
    Contour {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 5.0)]
        interval: f64,
    },
    /// Split clouds into square tiles (with an optional buffer), one file per tile
    Tile {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Base name of the tiles; default: <results>/<input>_tile.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 1000.0)]
        size: f64,
        #[arg(long, default_value_t = 0.0)]
        buffer: f64,
    },
    /// Edit points like features: delete or reclassify the points inside polygons that pass
    /// the attribute tests; writes a new file, never the original
    Edit {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Polygons, plot centres (with --radius) or a list; default: the whole cloud
        #[arg(long)]
        regions: Option<PathBuf>,
        #[arg(long)]
        radius: Option<f64>,
        /// Select the points outside the regions instead
        #[arg(long)]
        outside: bool,
        /// Only points of these classes are selected (2,5 or ~7,18)
        #[arg(long)]
        class: Option<String>,
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Delete the selected points
        #[arg(long, conflicts_with = "set_class")]
        delete: bool,
        /// Change the selected points to this class
        #[arg(long)]
        set_class: Option<u8>,
    },
    /// Give clouds without a coordinate system the one of another cloud, or an EPSG code
    Crs {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// A cloud that has the coordinate system to copy
        #[arg(long, conflicts_with = "epsg")]
        from: Option<PathBuf>,
        /// EPSG code, e.g. 25830
        #[arg(long)]
        epsg: Option<u16>,
        /// Replace the coordinate system of clouds that already have one
        #[arg(long)]
        replace: bool,
        /// Output (one input only); default: <results>/<input>_crs.<ext>
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Measure the stems of a terrestrial or mobile cloud along the trunk and cube each tree
    /// with Smalian's formula (a shapefile/CSV of trees, a CSV of logs, optional clouds by tree)
    Cubicar {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Trees table (.shp or .csv); default: <results>/<input>_cubicado.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Write a text table besides the shapefile
        #[arg(long)]
        csv: bool,
        /// The z of the cloud are heights above ground already
        #[arg(long)]
        normalized: bool,
        /// Height above ground (m) where the first log starts
        #[arg(long, default_value_t = 0.10)]
        base_height: f64,
        /// Diameter (m) at which the merchantable stem ends
        #[arg(long, default_value_t = 0.07)]
        top_diameter: f64,
        /// Distance (m) between the measured diameters: the length of a log
        #[arg(long, default_value_t = 0.5)]
        log_length: f64,
        /// Voxel (m) the cloud is reduced to
        #[arg(long, default_value_t = 0.03)]
        voxel: f64,
        /// Stripe (m above ground) where stems are looked for
        #[arg(long, default_value_t = 0.7)]
        stripe_low: f64,
        #[arg(long, default_value_t = 3.5)]
        stripe_high: f64,
        /// Largest stem diameter expected (m)
        #[arg(long, default_value_t = 1.0)]
        max_diameter: f64,
        /// Also write one cloud per tree
        #[arg(long)]
        tree_clouds: bool,
        /// Distance (m) up to which a point goes to the nearest tree in the clouds by tree
        #[arg(long, default_value_t = 8.0)]
        assign_dist: f64,
        /// Distance (m) between the bases of the stems of one stool (coppice sprouts)
        #[arg(long, default_value_t = 0.6)]
        stool_dist: f64,
        /// Height (m) of the diameter: stems apart at it are sprouts, each a tree; stems that
        /// are one at it and fork above it are one forked tree
        #[arg(long, default_value_t = 1.3)]
        fork_height: f64,
        /// Longest stretch (m) of a stem that may go unmeasured before it is taken as lost
        #[arg(long, default_value_t = 2.0)]
        max_gap: f64,
    },
    /// Multiband image from rasters: every band of every input becomes a band of the output
    Stack {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        /// Pixel size of the output; default: the finest input's
        #[arg(long)]
        cell: Option<f64>,
        /// Value of the inputs that means no data
        #[arg(long)]
        src_nodata: Option<f32>,
        /// Value written where there is no data
        #[arg(long, default_value_t = 0.0)]
        nodata: f32,
    },
    /// Mosaic of rasters with the same bands; --overlap says what to do where they overlap
    Mosaic {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value_t = OverlapArg::First)]
        overlap: OverlapArg,
        #[arg(long)]
        cell: Option<f64>,
        #[arg(long)]
        src_nodata: Option<f32>,
        #[arg(long, default_value_t = 0.0)]
        nodata: f32,
        /// Output covers only where all inputs coincide
        #[arg(long)]
        intersection: bool,
        /// Average the source pixels when reducing (default: nearest)
        #[arg(long)]
        average: bool,
    },
    /// Remove duplicate points (same x, y, z and the attributes asked for)
    Dedup {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Delete every copy, not only the extra ones
        #[arg(long)]
        all_copies: bool,
        /// Attributes that must agree too: class, gps, intensity, return, count, source
        #[arg(long, value_delimiter = ',')]
        also: Vec<String>,
    },
    /// Points of a cloud that changed against a reference cloud (change detection)
    Compare {
        /// Cloud to look for changes in
        input: PathBuf,
        /// Reference cloud(s)
        #[arg(long, required = true)]
        reference: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Distance (m) from which a point has changed
        #[arg(long, default_value_t = 0.5)]
        distance: f64,
        /// Other changed points a changed point needs within twice the distance
        #[arg(long, default_value_t = 3)]
        neighbors: usize,
    },
    /// Move a cloud onto a reference cloud (iterative closest point) and write the result
    Fit {
        input: PathBuf,
        #[arg(long, required = true)]
        reference: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Thinning (m) of the points used to find the motion
        #[arg(long, default_value_t = 1.0)]
        thin: f64,
        #[arg(long, default_value_t = 5.0)]
        max_xy: f64,
        #[arg(long, default_value_t = 2.0)]
        max_z: f64,
        #[arg(long, default_value_t = 30)]
        iterations: usize,
        #[arg(long, default_value_t = 0.01)]
        stop: f64,
        /// Only translate, do not rotate
        #[arg(long)]
        shift_only: bool,
    },
    /// Height difference between flight lines as a GeoTIFF, with the figures of the overlap
    Swath {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 1.0)]
        cell: f64,
        /// Returns used: all, last, single
        #[arg(long, default_value = "last")]
        returns: String,
        /// Take each cloud as one flight line instead of the point source id
        #[arg(long)]
        by_cloud: bool,
    },
    /// Draws a cloud (plan, 3D or profile) into a PNG, as the viewer does
    Render {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// PNG to write; default: next to the first input
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = ViewArg::Plan)]
        view: ViewArg,
        #[arg(long, value_enum, default_value_t = ColorArg::Elevation)]
        color: ColorArg,
        /// Image size: WIDTHxHEIGHT
        #[arg(long, default_value = "1600x1000")]
        size: String,
        /// Points kept in memory (one in every N is loaded)
        #[arg(long, default_value_t = 8_000_000)]
        max_points: usize,
        #[arg(long, default_value_t = 1)]
        point_size: usize,
        /// 3D: rotation and elevation in degrees
        #[arg(long, default_value_t = 30.0, allow_negative_numbers = true)]
        yaw: f64,
        #[arg(long, default_value_t = 35.0)]
        pitch: f64,
        /// Profile: x1,y1,x2,y2,width
        #[arg(long)]
        profile: Option<String>,
        /// Plan: shaded TIN under the points, from the classes given (e.g. 2; empty: all)
        #[arg(long)]
        shade: Option<String>,
        /// Plan: contour lines every this much
        #[arg(long)]
        contours: Option<f64>,
        /// Classes drawn: 2,5 or ~7,18
        #[arg(long)]
        class: Option<String>,
    },
    /// Extracts points from LAS/LAZ to ASCII columns or a 3D point shapefile
    Export {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// File (or, with several inputs, a folder). .shp writes a PointZ shapefile
        #[arg(short, long)]
        output: PathBuf,
        /// Columns in order: x,y,z,intensity,return,returns,class,source,red,green,blue
        #[arg(long, value_delimiter = ',', default_values_t = ["x".to_string(), "y".to_string(), "z".to_string()])]
        items: Vec<String>,
        #[arg(long, default_value = ",")]
        delimiter: String,
        #[arg(long)]
        header: bool,
        #[arg(long, default_value_t = 2)]
        decimals: usize,
        #[arg(long)]
        class: Option<String>,
        #[arg(long)]
        returns: Option<String>,
        #[arg(long, allow_negative_numbers = true)]
        zmin: Option<f64>,
        #[arg(long, allow_negative_numbers = true)]
        zmax: Option<f64>,
        /// Keep one point in this many
        #[arg(long, default_value_t = 1)]
        decimate: u64,
    },
    /// Metrics by grid cell: elevation, and optionally intensity and strata
    Metrics {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Ground model (GeoTIFF or ASCII grid); default: the newest DTM made from this same cloud
        #[arg(long)]
        dtm: Option<PathBuf>,
        /// Output shapefile of cells (or .csv if you name one); default: <results>/<input>_metricas_<cell>m.shp
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 15.0)]
        cell: f64,
        /// Height break for the cover metrics
        #[arg(long, default_value_t = 2.0)]
        heightbreak: f64,
        #[arg(long, default_value_t = 4)]
        min_points: u32,
        /// Most memory, in MiB, for point heights (larger clouds take more passes)
        #[arg(long, default_value_t = 512)]
        memory_mb: u64,
        /// Also write intensity metrics to <output stem>_intensity (same format as the output)
        #[arg(long)]
        intensity: bool,
        /// Also write strata metrics to <output stem>_strata (same format as the output)
        #[arg(long)]
        strata: bool,
        /// Upper limits of the height strata
        #[arg(long, value_delimiter = ',', default_values_t = [0.15, 1.37, 5.0, 10.0, 20.0, 30.0])]
        strata_limits: Vec<f64>,
        /// Upper limits of the intensity strata
        #[arg(long, value_delimiter = ',', default_values_t = [0.15, 1.37])]
        int_strata_limits: Vec<f64>,
        /// Output format: shapefile of cells, text table, or both (default: by the output name)
        #[arg(long, value_enum)]
        format: Option<FormatArg>,
    },
}

/// `a,b,c` as numbers.
fn numbers(text: &str, n: usize, what: &str) -> Result<Vec<f64>> {
    let v: Vec<f64> = text
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    if v.len() == n {
        Ok(v)
    } else {
        Err(rustlidar_core::Error::Param(format!(
            "{what} needs {n} numbers separated by commas, not '{text}'"
        )))
    }
}

/// The output path: the one given, else `<results>/<input>_<tag>.<ext>` (a free name: an earlier
/// product is never overwritten unless its path is given).
fn out_path(given: Option<PathBuf>, input: &Path, tag: &str, ext: &str) -> PathBuf {
    given.unwrap_or_else(|| paths::unique(&paths::product(input, tag, ext)))
}

/// The DTM given, else the newest one made from `input`.
fn dtm_for(given: Option<PathBuf>, input: &Path) -> Result<PathBuf> {
    given.or_else(|| paths::find_dtm(input)).ok_or_else(|| {
        rustlidar_core::Error::Param(format!(
            "no DTM given and none made from {} found in {}: run `dtm` first or pass --dtm",
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

/// `-o` names one result; with several sheets each has its own, so `-o` is not accepted.
fn single_output(output: Option<PathBuf>, sheets: usize) -> Result<Option<PathBuf>> {
    if sheets > 1 && output.is_some() {
        return Err(rustlidar_core::Error::Param(
            "with several sheets each one gets its own result: leave out -o".into(),
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
        Cmd::Classify {
            inputs,
            output,
            steps,
        } => {
            // every sheet is classified on its own: they are never joined into one file
            let output = single_output(output, inputs.len())?;
            let tag = format!("clasif{}", steps.tag());
            let out_for = |p: &Path| out_path(output.clone(), p, &tag, &ext_of(p));
            rustlidar_core::sheets::classify_each(
                &inputs,
                &out_for,
                &steps.steps(),
                &steps.tiling(),
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
            // a DTM is made from ground points only: no noise or building steps
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
            };
            rustlidar_core::sheets::dtm_each(&inputs, &base, &out_for, mosaic_to.as_deref())?.0
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
            let mut lines = Vec::new();
            for input in &inputs {
                let out = out_path(
                    output.clone(),
                    input,
                    &format!("mds_{}m_{name}{sm}", paths::num(cell)),
                    "tif",
                );
                lines.push(jobs::dsm(&jobs::DsmJob {
                    input: input.clone(),
                    output: out,
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
                })?);
            }
            lines.join("\n")
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
        } => {
            let output = single_output(output, inputs.len())?;
            if inputs.len() > 1 && dtm.is_some() {
                return Err(rustlidar_core::Error::Param(
                    "--dtm names one DTM: with several sheets each uses the DTM made from it"
                        .into(),
                ));
            }
            let pf = if method == ChmMethod::Pitfree {
                "_pitfree"
            } else {
                ""
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
            rustlidar_core::sheets::chm_each(
                &inputs,
                &base,
                &out_for,
                &dtm_of,
                mosaic_to.as_deref(),
            )?
            .0
        }
        Cmd::Trees {
            chm,
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
            let base = paths::stem(&chm);
            let dir = chm
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            let output = output
                .unwrap_or_else(|| paths::unique(&dir.join(format!("{base}_arboles_{tag}.shp"))));
            let extra = |suffix: &str, ext: &str| {
                paths::unique(
                    &output.with_file_name(format!("{}_{suffix}.{ext}", paths::stem(&output))),
                )
            };
            let formats = formats_of(format, &output);
            jobs::trees(&jobs::TreesJob {
                formats,
                crowns: crowns.then(|| extra("copas", "tif")),
                crowns_shp: crowns_shp.then(|| extra("copas", "shp")),
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
                    columns: rustlidar_core::forest::FieldColumns {
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
            })?
        }
        Cmd::Clip {
            inputs,
            regions,
            rect,
            circle,
            name_field,
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
        } => {
            use rustlidar_core::clip::{ClipJob, ClipMode, SampleMetrics};
            use rustlidar_core::filter::{ClassSel, Filter, ReturnSel};
            use rustlidar_core::vector::{self, Region, Shape2};
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
                list.extend(vector::read_regions(path, &opts)?);
            }
            let first = inputs.first().cloned().unwrap_or_default();
            let mut layout_note = None;
            if let Some(kind) = sample {
                use rustlidar_core::plots::{PlotShape, Sampling, generate, write_layout};
                let mut b = [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ];
                for p in &inputs {
                    let h = rustlidar_core::las::Reader::open(p)?.header.bounds;
                    b = [
                        b[0].min(h[0]),
                        b[1].min(h[1]),
                        b[2].max(h[3]),
                        b[3].max(h[4]),
                    ];
                }
                let shape = if square {
                    let w = side.ok_or_else(|| {
                        rustlidar_core::Error::Param(
                            "--side gives the side of the plots in metres".into(),
                        )
                    })?;
                    PlotShape::Rect { w, h: w }
                } else {
                    PlotShape::Circle {
                        radius: radius.ok_or_else(|| {
                            rustlidar_core::Error::Param(
                                "--radius gives the radius of the plots in metres".into(),
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
                            rustlidar_core::Error::Param(
                                "--spacing gives the distance between plots".into(),
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
                                return Err(rustlidar_core::Error::Param(
                                    "--spacing is X or X,Y".into(),
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
                    .unwrap_or_else(|| paths::unique(&paths::product(&first, "parcelas", "shp")));
                let prj = rustlidar_core::las::Reader::open(&first)
                    .ok()
                    .and_then(|r| r.header.epsg)
                    .and_then(vector::prj_for_epsg);
                write_layout(&path, &plots, prj.as_deref())?;
                layout_note = Some(format!(
                    "{} parcelas generadas: {}",
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
                    rustlidar_core::Error::Param("--where needs FIELD=VALUE".into())
                })?;
                list.retain(|r| {
                    r.attrs.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case(field.trim()) && v.eq_ignore_ascii_case(value.trim())
                    })
                });
            }
            let ext = ext_of(&first);
            let tag = match mode {
                ClipModeArg::Each => "parcela",
                ClipModeArg::Metrics => "parcelas",
                ClipModeArg::Merge => "recorte",
            };
            let metrics = match (metrics, mode) {
                (None, ClipModeArg::Metrics) => Some(paths::unique(&paths::product(
                    &first,
                    "parcelas_metricas",
                    "shp",
                ))),
                (m, _) => m,
            };
            let output = output.unwrap_or_else(|| paths::product(&first, tag, &ext));
            let mut job = ClipJob::new(inputs, list, output);
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
                rustlidar_core::ops::whole_cloud_job(inputs, out_path(output, &first, tag, &ext))?;
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
            use rustlidar_core::thin::{ThinMethod, ThinParams, report, thin_cloud};
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
            use rustlidar_core::ops::{Stat, raster_stat};
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
            let g = rustlidar_core::grid::Grid::read(&input)?;
            let out = output.unwrap_or_else(|| {
                paths::unique(
                    &input.with_file_name(format!("{}_sombreado.png", paths::stem(&input))),
                )
            });
            rustlidar_core::ops::hillshade(&g, azimuth, altitude).write_png(&out)?;
            format!("{} x {} px: {}", g.cols, g.rows, out.display())
        }
        Cmd::Contour {
            input,
            output,
            interval,
        } => {
            let g = rustlidar_core::grid::Grid::read(&input)?;
            let out = output.unwrap_or_else(|| {
                paths::unique(&input.with_file_name(format!(
                    "{}_curvas_{}m.shp",
                    paths::stem(&input),
                    paths::num(interval)
                )))
            });
            let n = rustlidar_core::ops::write_contours(&g, interval, &out, None)?;
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
                let h = rustlidar_core::las::Reader::open(p)?.header.bounds;
                b = [
                    b[0].min(h[0]),
                    b[1].min(h[1]),
                    b[2].max(h[3]),
                    b[3].max(h[4]),
                ];
            }
            let regions = rustlidar_core::ops::tile_regions(b, size, buffer, "t")?;
            let mut job = rustlidar_core::clip::ClipJob::new(inputs, regions, out);
            job.mode = rustlidar_core::clip::ClipMode::PerRegion;
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
            use rustlidar_core::edit::{self, Action, Attr, Edit, Mode, Region, SelOp};
            use rustlidar_core::filter::ClassSel;
            use rustlidar_core::vector;
            let action = match (delete, set_class) {
                (true, _) => Action::Delete,
                (false, Some(c)) => Action::Reclassify(c),
                _ => {
                    return Err(rustlidar_core::Error::Param(
                        "say what to do: --delete or --set-class N".into(),
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
                    for (k, r) in rustlidar_core::ops::polygons_of(&list)
                        .into_iter()
                        .enumerate()
                    {
                        sel.push(SelOp {
                            mode: if k == 0 { Mode::New } else { Mode::Add },
                            region: r,
                            attr: Attr {
                                classes: rustlidar_core::classes::ClassSet::all(),
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
            let r = edit::save(&input, &out, &[Edit { sel, action, label }])?;
            edit::report_text(&r, &out)
        }
        Cmd::Crs {
            inputs,
            from,
            epsg,
            replace,
            output,
        } => {
            use rustlidar_core::ops::{CrsSource, assign_projection};
            let source = match (from, epsg) {
                (Some(f), _) => CrsSource::Cloud(f),
                (None, Some(e)) => CrsSource::Epsg(e),
                _ => {
                    return Err(rustlidar_core::Error::Param(
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
        Cmd::Stack {
            inputs,
            output,
            cell,
            src_nodata,
            nodata,
        } => {
            use rustlidar_core::rastertools::{RasterJob, Source, stack};
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
            stack(&job)?
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
        } => {
            use rustlidar_core::rastertools::{
                Extent, Overlap, RasterJob, Resample, Source, mosaic,
            };
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
            mosaic(&job)?
        }
        Cmd::Dedup {
            inputs,
            output,
            all_copies,
            also,
        } => {
            use rustlidar_core::cloudtools::{DupFields, remove_duplicates};
            let mut f = DupFields::default();
            for a in &also {
                match a.as_str() {
                    "class" => f.class = true,
                    "gps" => f.gps_time = true,
                    "intensity" => f.intensity = true,
                    "return" => f.return_number = true,
                    "count" => f.return_count = true,
                    "source" => f.source_id = true,
                    o => {
                        return Err(rustlidar_core::Error::Param(format!(
                            "'{o}' is not an attribute: class, gps, intensity, return, count, source"
                        )));
                    }
                }
            }
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
        Cmd::Compare {
            input,
            reference,
            output,
            distance,
            neighbors,
        } => {
            use rustlidar_core::cloudtools::{CompareParams, compare_clouds, compare_text};
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
        } => {
            use rustlidar_core::cloudtools::{FitParams, fit_cloud, fit_text};
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
        } => {
            use rustlidar_core::cloudtools::{SwathReturns, swath_separation, swath_text};
            let returns = match returns.as_str() {
                "all" => SwathReturns::All,
                "last" => SwathReturns::Last,
                "single" => SwathReturns::Single,
                o => {
                    return Err(rustlidar_core::Error::Param(format!(
                        "'{o}': use all, last or single"
                    )));
                }
            };
            let first = inputs.first().cloned().unwrap_or_default();
            let out = out_path(output, &first, "pasadas", "tif");
            let rep = swath_separation(&inputs, &out, cell, returns, by_cloud)?;
            swath_text(&rep, &out)
        }
        Cmd::Cubicar {
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
            assign_dist,
            stool_dist,
            fork_height,
            max_gap,
        } => {
            use rustlidar_core::tls::{TlsJob, TlsParams, run};
            let output = single_output(output, inputs.len())?;
            let mut lines = Vec::new();
            for input in &inputs {
                let job = TlsJob {
                    input: input.clone(),
                    output: out_path(output.clone(), input, "cubicado", "shp"),
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
                        ..TlsParams::default()
                    },
                    formats: paths::Formats { vector: true, csv },
                    tree_clouds,
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
            use rustlidar_core::filter::ClassSel;
            use rustlidar_core::view::{self, ColorBy, Orbit, Plan, ProfileBox, Style};
            let (w, h) = size
                .split_once('x')
                .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
                .filter(|(w, h)| *w > 0 && *h > 0 && w * h <= 1 << 28)
                .ok_or_else(|| {
                    rustlidar_core::Error::Param(format!("--size needs WIDTHxHEIGHT, not '{size}'"))
                })?;
            let store = view::load(&inputs, max_points)?;
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
            };
            st.point_size = point_size;
            let mask_of = |c: &str| -> Result<rustlidar_core::classes::ClassSet> {
                Ok(ClassSel::parse(c)?.set)
            };
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
                            _ => rustlidar_core::classes::ClassSet::all(),
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
                    view::render_profile(&store, &bx, w, h, &st).0
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
            use rustlidar_core::export::{self, AsciiJob, Item};
            use rustlidar_core::filter::{ClassSel, Filter, ReturnSel};
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
            if output
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("shp"))
            {
                let mut total = 0;
                for (k, i) in inputs.iter().enumerate() {
                    let out = if inputs.len() > 1 {
                        output.with_file_name(format!("{}_{}.shp", paths::stem(&output), k + 1))
                    } else {
                        output.clone()
                    };
                    total += export::shapefile_points(i, &out, &filter)?;
                }
                format!("{total} puntos exportados a {}", output.display())
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
                        o => Err(rustlidar_core::Error::Param(format!("unknown item '{o}'"))),
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
                })?;
                format!(
                    "{n} puntos exportados a {} archivo(s): {}",
                    files.len(),
                    files
                        .first()
                        .map_or(String::new(), |f| f.display().to_string())
                )
            }
        }
        Cmd::Metrics {
            inputs,
            dtm,
            output,
            cell,
            heightbreak,
            min_points,
            memory_mb,
            intensity,
            strata,
            strata_limits,
            int_strata_limits,
            format,
        } => {
            let output = single_output(output, inputs.len())?;
            if inputs.len() > 1 && dtm.is_some() {
                return Err(rustlidar_core::Error::Param(
                    "--dtm names one DTM: with several sheets each uses the DTM made from it"
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
                },
                intensity,
                strata,
            };
            rustlidar_core::sheets::metrics_each(&inputs, &base, &out_for, &dtm_of)?
        }
    };
    println!("{summary}");
    eprintln!("done in {:.2} s", t.elapsed().as_secs_f64());
    Ok(())
}

fn main() -> ExitCode {
    // Parsing and `run` hold every command's arguments; debug builds need more than the main thread's stack.
    let result = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| run(Cli::parse().cmd))
        .map_err(rustlidar_core::Error::Io)
        .and_then(|h| {
            h.join()
                .unwrap_or_else(|_| Err(rustlidar_core::Error::Format("internal error".into())))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
