//! The arithmetic of web map tiles (the XYZ scheme of OpenStreetMap, the IGN and most other
//! providers): which tiles cover an area, and where each one lies in Web Mercator metres.

/// Half the width of the world in Web Mercator (EPSG:3857) metres.
pub const HALF_WORLD: f64 = 20_037_508.342_789_244;

/// The side of a tile in pixels.
pub const TILE_PX: f64 = 256.0;

/// A tile: zoom, column from the west, row from the north.
pub type Tile = (u8, u32, u32);

/// The tile extent in Web Mercator metres: min x, min y, max x, max y.
pub fn extent(z: u8, x: u32, y: u32) -> [f64; 4] {
    let n = f64::from(1u32 << z.min(30));
    let side = 2.0 * HALF_WORLD / n;
    let (x0, y1) = (
        -HALF_WORLD + f64::from(x) * side,
        HALF_WORLD - f64::from(y) * side,
    );
    [x0, y1 - side, x0 + side, y1]
}

/// The zoom whose tiles have about one pixel per screen pixel when the view shows
/// `metres_per_pixel` Web Mercator metres in each, never beyond `max_zoom`. A little coarser is
/// preferred to a little finer: fewer tiles, and the screen scales them down well.
pub fn zoom_for(metres_per_pixel: f64, max_zoom: u8) -> u8 {
    if !(metres_per_pixel > 0.0) || !metres_per_pixel.is_finite() {
        return 0;
    }
    let world_px = 2.0 * HALF_WORLD / metres_per_pixel;
    let z = (world_px / TILE_PX).log2().round();
    z.clamp(0.0, f64::from(max_zoom)) as u8
}

/// The tiles of zoom `z` that touch `bounds` (Web Mercator metres: min x, min y, max x, max y),
/// west to east and north to south. Empty when the bounds are not on the map.
pub fn tiles_in(bounds: [f64; 4], z: u8) -> Vec<Tile> {
    let n = 1u32 << z.min(30);
    let side = 2.0 * HALF_WORLD / f64::from(n);
    let col = |x: f64| ((x + HALF_WORLD) / side).floor();
    let row = |y: f64| ((HALF_WORLD - y) / side).floor();
    let (c0, c1) = (col(bounds[0]), col(bounds[2]));
    let (r0, r1) = (row(bounds[3]), row(bounds[1]));
    if !(c0.is_finite() && c1.is_finite() && r0.is_finite() && r1.is_finite()) {
        return Vec::new();
    }
    let max = f64::from(n - 1);
    if c1 < 0.0 || r1 < 0.0 || c0 > max || r0 > max {
        return Vec::new();
    }
    let (c0, c1) = (c0.clamp(0.0, max) as u32, c1.clamp(0.0, max) as u32);
    let (r0, r1) = (r0.clamp(0.0, max) as u32, r1.clamp(0.0, max) as u32);
    let mut out = Vec::new();
    for y in r0..=r1 {
        for x in c0..=c1 {
            out.push((z, x, y));
        }
    }
    out
}

/// The URL of a tile from a template with `{z}`, `{x}` and `{y}`.
pub fn url(template: &str, z: u8, x: u32, y: u32) -> String {
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Web Mercator metres of a longitude and latitude.
    fn merc(lon: f64, lat: f64) -> (f64, f64) {
        let x = lon.to_radians() * 6_378_137.0;
        let y = (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0)
            .tan()
            .ln()
            * 6_378_137.0;
        (x, y)
    }

    #[test]
    fn madrid_is_in_the_tile_every_map_says() {
        // the Puerta del Sol on the slippy-map scheme (the formula of the OSM wiki, worked out
        // apart in Python): z12, x 2005, y 1544; the IGN serves a real tile there
        let (x, y) = merc(-3.7038, 40.4168);
        let t = tiles_in([x, y, x, y], 12);
        assert_eq!(t, vec![(12, 2005, 1544)]);
        let e = extent(12, 2005, 1544);
        assert!(e[0] <= x && x <= e[2] && e[1] <= y && y <= e[3]);
    }

    #[test]
    fn the_world_is_one_tile_at_zoom_zero() {
        let e = extent(0, 0, 0);
        assert_eq!(e, [-HALF_WORLD, -HALF_WORLD, HALF_WORLD, HALF_WORLD]);
        assert_eq!(tiles_in([-1e9, -1e9, 1e9, 1e9], 0), vec![(0, 0, 0)]);
        assert_eq!(tiles_in([-1e9, -1e9, 1e9, 1e9], 2).len(), 16);
    }

    #[test]
    fn neighbours_share_an_edge_and_zoom_follows_resolution() {
        let (a, b) = (extent(8, 100, 90), extent(8, 101, 90));
        assert!((a[2] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
        // zoom 0 has 156543 m per pixel; each zoom halves it
        assert_eq!(zoom_for(156_543.03, 19), 0);
        assert_eq!(zoom_for(1.194, 19), 17);
        assert_eq!(zoom_for(0.001, 19), 19);
        assert_eq!(zoom_for(f64::NAN, 19), 0);
    }

    #[test]
    fn areas_off_the_map_have_no_tiles_and_urls_fill_in() {
        assert!(tiles_in([1e9, 0.0, 2e9, 1.0], 5).is_empty());
        assert_eq!(
            url("https://t/{z}/{x}/{y}.png", 3, 4, 5),
            "https://t/3/4/5.png"
        );
    }
}
