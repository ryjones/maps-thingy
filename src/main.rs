//! kmz2map — turn KMZ placemarks into an SVG map site you can host on Pages.
//!
//! Reads one or more KMZ (or bare KML) files and writes a directory holding
//! `index.html`, a standalone `map.svg`, and the pictures lifted out of the
//! archives. `index.html` is the artefact to publish: the map is inlined there,
//! so the linked pictures load and the placemarks stay interactive.

mod kml;
mod proj;
mod render;
mod sanitize;
mod tiles;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;

use proj::{pick_zoom, project, TILE};
use render::{Tile, View};
use tiles::Fetcher;

/// Categorical slots in fixed order. The marks sit on photographic tiles rather
/// than on the page surface, so they keep light-mode steps in either page theme
/// and earn their contrast from a white casing instead. Any two groups can end
/// up adjacent on a map, and past the third slot such a pairing stops being
/// reliably separable under colour-vision deficiency — so hue never carries
/// identity alone here: the legend, the hover label and each card name the
/// group. Past the eighth, groups share one neutral rather than cycling.
const SERIES: [&str; 8] = [
    "#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#4a3aa7", "#e34948",
];
const OVERFLOW: &str = "#6b6a63";

/// The smallest map worth drawing: a single placemark, or a track that doubles
/// back on itself, would otherwise project to a sliver.
const MIN_W: f64 = 480.0;
const MIN_H: f64 = 320.0;

#[derive(Parser, Debug)]
#[command(name = "kmz2map", version, about, long_about = None)]
struct Cli {
    /// KMZ or KML files to map. Several are merged into one map.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,

    /// Output directory
    #[arg(short, long, default_value = "site")]
    out: PathBuf,

    /// OSM zoom level (default: the closest that fits --max-px)
    #[arg(short, long)]
    zoom: Option<u8>,

    /// Largest map edge, in pixels
    #[arg(long, default_value_t = 2048.0)]
    max_px: f64,

    /// Margin around the placemarks, in pixels
    #[arg(long, default_value_t = 48.0)]
    padding: f64,

    /// Draw the routes and placemarks without a basemap, and without asking
    /// the tile server for anything
    #[arg(long)]
    no_tiles: bool,

    /// Refuse to fetch more than this many tiles, rather than hammering the
    /// tile server for a map that is too large to be useful
    #[arg(long, default_value_t = 256)]
    max_tiles: usize,

    /// Tile cache directory (default: $XDG_CACHE_HOME/maps-thingy/tiles)
    #[arg(long)]
    tile_cache: Option<PathBuf>,

    /// Seconds to wait between tile fetches
    #[arg(long, default_value_t = 0.1)]
    pause: f64,

    /// Tile fetch timeout, in seconds
    #[arg(long, default_value_t = 20.0)]
    timeout: f64,

    /// Page title (default: the KML document name)
    #[arg(long)]
    title: Option<String>,

    /// Suppress progress output
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.max_px < MIN_W {
        bail!("--max-px must be at least {MIN_W:.0}");
    }
    if cli.padding < 0.0 {
        bail!("--padding cannot be negative");
    }

    let images = cli.out.join("images");
    std::fs::create_dir_all(&images)
        .with_context(|| format!("failed to create {}", images.display()))?;

    let mut groups = Vec::new();
    let mut taken: HashMap<String, u64> = HashMap::new();
    let mut doc_names = Vec::new();
    for input in &cli.inputs {
        let (got, name) = kml::read(input, &images, &mut taken)
            .with_context(|| format!("failed to read {}", input.display()))?;
        groups.extend(got);
        doc_names.push(name);
    }
    groups.retain(|g| !g.is_empty());
    if groups.is_empty() {
        bail!("no placemarks with coordinates found");
    }

    for (gi, g) in groups.iter_mut().enumerate() {
        g.color = SERIES.get(gi).copied().unwrap_or(OVERFLOW).to_string();
    }
    for (i, m) in groups.iter_mut().flat_map(|g| g.marks.iter_mut()).enumerate() {
        m.uid = format!("s{i:04}");
    }

    let positions: Vec<(f64, f64)> = groups.iter().flat_map(|g| g.points()).collect();
    let (south, north) = min_max(positions.iter().map(|p| p.0));
    let (west, east) = min_max(positions.iter().map(|p| p.1));
    let zoom = cli
        .zoom
        .unwrap_or_else(|| pick_zoom(((south, west), (north, east)), cli.max_px, cli.padding))
        .clamp(1, proj::MAX_ZOOM);

    let pixels: Vec<(f64, f64)> =
        positions.iter().map(|(lat, lon)| project(*lat, *lon, zoom)).collect();
    let (min_x, max_x) = min_max(pixels.iter().map(|p| p.0));
    let (min_y, max_y) = min_max(pixels.iter().map(|p| p.1));
    let mut view = View {
        x0: min_x - cli.padding,
        y0: min_y - cli.padding,
        w: max_x - min_x + 2.0 * cli.padding,
        h: max_y - min_y + 2.0 * cli.padding,
    };
    if view.w < MIN_W {
        view.x0 -= (MIN_W - view.w) / 2.0;
        view.w = MIN_W;
    }
    if view.h < MIN_H {
        view.y0 -= (MIN_H - view.h) / 2.0;
        view.h = MIN_H;
    }

    let mut fetched = Vec::new();
    if !cli.no_tiles {
        let fetcher = Fetcher {
            cache: cli.tile_cache.clone().unwrap_or_else(Fetcher::default_cache),
            pause: cli.pause.max(0.0),
            timeout: cli.timeout,
            quiet: cli.quiet,
        };
        let tx0 = (view.x0 / TILE).floor() as i64;
        let tx1 = ((view.x0 + view.w) / TILE).floor() as i64;
        let ty0 = (view.y0 / TILE).floor() as i64;
        let ty1 = ((view.y0 + view.h) / TILE).floor() as i64;
        let wanted = ((tx1 - tx0 + 1) * (ty1 - ty0 + 1)) as usize;
        if wanted > cli.max_tiles {
            bail!(
                "{wanted} tiles needed at zoom {zoom}, over --max-tiles {}; \
                 lower --zoom or --max-px, or pass --no-tiles",
                cli.max_tiles
            );
        }
        if !cli.quiet {
            println!("zoom {zoom}, {wanted} tile(s), cache {}", fetcher.cache.display());
        }
        for tx in tx0..=tx1 {
            for ty in ty0..=ty1 {
                if let Some(bytes) = fetcher.tile(zoom, tx, ty) {
                    fetched.push(Tile { tx, ty, bytes });
                }
            }
        }
    }

    let title = cli.title.clone().unwrap_or_else(|| doc_names[0].clone());
    let marks: usize = groups.iter().map(|g| g.marks.len()).sum();
    let routes: usize = groups.iter().map(|g| g.routes.len()).sum();
    let subtitle = format!(
        "{marks} placemark(s) in {} group(s){} · zoom {zoom}",
        groups.len(),
        if routes > 0 { format!(", {routes} route(s)") } else { String::new() },
    );

    let map = render::svg(&groups, view, zoom, &fetched, &title, &subtitle);
    let sources: Vec<String> = cli
        .inputs
        .iter()
        .map(|p| p.file_name().unwrap_or(p.as_os_str()).to_string_lossy().into_owned())
        .collect();

    write(&cli.out.join("map.svg"), &format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{map}"))?;
    write(&cli.out.join("index.html"), &render::page(&groups, &map, &title, &subtitle, &sources))?;
    // Pages runs Jekyll by default, which would skip files it considers its own.
    write(&cli.out.join(".nojekyll"), "")?;

    if !cli.quiet {
        println!("{marks} placemark(s) -> {}", cli.out.join("index.html").display());
    }
    Ok(())
}

fn write(path: &std::path::Path, body: &str) -> Result<()> {
    std::fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))
}

fn min_max(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}
