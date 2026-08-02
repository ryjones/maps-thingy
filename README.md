# maps-thingy

Small tools that turn tracks and placemarks into SVG you can commit and publish.

[check out the demo map](demo)

## kmz2map — a KMZ becomes a map site

[one clip, mapped](demo-scan/) — 52 sightings, the crops linked from the pins.

Reads one or more KMZ (or bare KML) files and writes a directory ready for
GitHub Pages:

```
cargo build --release
./target/release/kmz2map scan.kmz -o site
```

```
site/
  index.html   the map inlined as SVG, plus a card per placemark
  map.svg      the same map standalone
  images/      pictures lifted out of the KMZ
  .nojekyll
```

Drop the `.nojekyll` if you are publishing into a subdirectory of a site that
still wants Jekyll, as `demo-scan/` here does — it only counts at the root, and
at the root it would turn Jekyll off for everything.

Publish `index.html`. Placemark pictures are *linked*, not base64'd, which keeps
`map.svg` in the hundreds of kilobytes instead of tens of megabytes — the catch
is that an `<img src="map.svg">` in a README renders the map and silently drops
the pictures, because that context refuses to load anything external. Open
`map.svg` as its own document, or use the page.

Every KML folder becomes a group with its own colour, a legend chip that toggles
it, and its own cards; `<LineString>`s are drawn as routes and `<Point>`s as
pins. Hovering a pin shows the picture, clicking one jumps to its card.

The basemap is OpenStreetMap tiles, cached under `$XDG_CACHE_HOME/maps-thingy/`
so a re-render never re-fetches. Useful flags:

| | |
|---|---|
| `-z`, `--zoom` | force an OSM zoom level; the default is the closest that fits |
| `--max-px` | largest map edge in pixels (default 2048) — this is what picks the zoom |
| `--no-tiles` | routes and pins only: no basemap, no network |
| `--max-tiles` | refuse to fetch more than this many (default 256) |
| `--title` | override the KML document name |

Several inputs merge into one map, so a trip scanned clip by clip can be
published as a single page:

```
./target/release/kmz2map trip-*.kmz -o site --title "17 July"
```

It pairs with platescan, whose `--kmz` export it reads directly, but nothing
about it is specific to that: any KMZ with points, lines and balloon pictures
maps.

## gpx2svg / mapoverlay — a GPX becomes a chart and a map

```
pip install gpxpy requests Pillow
./doit 2025_0520_125814_F
```

`gpx2svg.py` plots elevation and speed along the track; `mapoverlay.py` draws the
track over OSM tiles, embedding them so the SVG stands alone.
