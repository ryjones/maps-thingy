//! Web mercator, the projection the tile servers themselves use.
//!
//! Everything downstream works in the tile grid's pixel space: a point's
//! position, a tile's corner, and the SVG viewBox are all the same units, so
//! the overlay lines up with the basemap without a second transform.

pub const TILE: f64 = 256.0;
pub const MAX_ZOOM: u8 = 19;

/// Pixel coordinates of a fix at `zoom`, measured from the top-left of the world.
pub fn project(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let n = TILE * f64::powi(2.0, zoom as i32);
    let x = (lon + 180.0) / 360.0 * n;
    let s = lat.to_radians().sin().clamp(-0.9999, 0.9999);
    let y = (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * n;
    (x, y)
}

/// Bounding box as ((south, west), (north, east)).
pub type Bounds = ((f64, f64), (f64, f64));

/// The closest zoom in, so the map is as detailed as `max_px` allows.
pub fn pick_zoom(bounds: Bounds, max_px: f64, pad: f64) -> u8 {
    let ((south, west), (north, east)) = bounds;
    for z in (1..=MAX_ZOOM).rev() {
        let (x0, y0) = project(north, west, z);
        let (x1, y1) = project(south, east, z);
        if (x1 - x0) + 2.0 * pad <= max_px && (y1 - y0) + 2.0 * pad <= max_px {
            return z;
        }
    }
    1
}

/// Ground resolution, for the scale bar. Only true at this latitude — mercator
/// stretches everything away from the equator.
pub fn meters_per_pixel(lat: f64, zoom: u8) -> f64 {
    156_543.033_928 * lat.to_radians().cos() / f64::powi(2.0, zoom as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_island_sits_at_the_middle_of_the_world() {
        let (x, y) = project(0.0, 0.0, 0);
        assert!((x - 128.0).abs() < 1e-9);
        assert!((y - 128.0).abs() < 1e-9);
    }

    #[test]
    fn north_and_east_move_the_expected_way() {
        let (x0, y0) = project(47.6, -122.3, 12);
        let (x1, y1) = project(47.7, -122.2, 12);
        assert!(x1 > x0, "east is right");
        assert!(y1 < y0, "north is up");
    }

    #[test]
    fn zoom_fits_inside_the_budget() {
        let bounds = ((47.58, -122.26), (47.60, -122.23));
        let z = pick_zoom(bounds, 2048.0, 48.0);
        let (x0, y0) = project(bounds.1.0, bounds.0.1, z);
        let (x1, y1) = project(bounds.0.0, bounds.1.1, z);
        assert!(x1 - x0 + 96.0 <= 2048.0 && y1 - y0 + 96.0 <= 2048.0);
        // And one zoom further in would not fit.
        let (nx0, ny0) = project(bounds.1.0, bounds.0.1, z + 1);
        let (nx1, ny1) = project(bounds.0.0, bounds.1.1, z + 1);
        assert!(nx1 - nx0 + 96.0 > 2048.0 || ny1 - ny0 + 96.0 > 2048.0);
    }
}
