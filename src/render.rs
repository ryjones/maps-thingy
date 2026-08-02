//! Drawing the map, as one SVG and as the page that hosts it.
//!
//! Both outputs share a single SVG string. Tiles are inlined as data URIs
//! because a basemap is useless if it goes missing, but the placemark pictures
//! stay linked: a scan can carry hundreds of them, and inlining would turn a
//! 200 kB map into a 30 MB one. The cost of linking is that `map.svg` has to be
//! opened as a document — an `<img src="map.svg">` in a README renders the map
//! and silently drops the pictures — which is why `index.html` is the artefact
//! meant for publishing.

use std::fmt::Write as _;

use base64::Engine as _;

use crate::kml::Group;
use crate::proj::{meters_per_pixel, project, TILE};
use crate::sanitize::{esc, scrub};
use crate::tiles::ATTRIBUTION;

/// The window onto the tile grid, in that grid's pixels.
#[derive(Clone, Copy)]
pub struct View {
    pub x0: f64,
    pub y0: f64,
    pub w: f64,
    pub h: f64,
}

pub struct Tile {
    pub tx: i64,
    pub ty: i64,
    pub bytes: Vec<u8>,
}

const SVG_CSS: &str = concat!(
    ".route{fill:none;stroke-linecap:round;stroke-linejoin:round}",
    ".mark circle{stroke:#fff;stroke-width:1.5;opacity:.92}",
    ".mark:hover circle,.mark:focus circle{stroke-width:2.5;opacity:1}",
    ".chrome{font-family:ui-sans-serif,system-ui,-apple-system,'Helvetica Neue',",
    "Arial,sans-serif;paint-order:stroke;stroke:#fff;stroke-width:3;",
    "stroke-linejoin:round}",
    ".hidden{display:none}",
);

pub fn svg(
    groups: &[Group],
    view: View,
    zoom: u8,
    tiles: &[Tile],
    title: &str,
    subtitle: &str,
) -> String {
    let mut s = String::new();
    let w = &mut s;
    let _ = write!(
        w,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"map\" \
         viewBox=\"0 0 {:.0} {:.0}\" width=\"{:.0}\" height=\"{:.0}\" \
         role=\"img\" aria-label=\"{}\">",
        view.w,
        view.h,
        view.w,
        view.h,
        esc(title)
    );
    let _ = write!(w, "<title>{}</title>", esc(title));
    let _ = write!(w, "<style>{SVG_CSS}</style>");
    // A flat fill behind the tiles, so a gap from an unreachable tile reads as
    // missing basemap rather than as a hole in the page.
    let _ = write!(
        w,
        "<rect width=\"{:.0}\" height=\"{:.0}\" fill=\"#e8e4dd\"/>",
        view.w, view.h
    );

    let _ = write!(w, "<g class=\"tiles\">");
    for t in tiles {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&t.bytes);
        let _ = write!(
            w,
            "<image x=\"{:.0}\" y=\"{:.0}\" width=\"{TILE:.0}\" height=\"{TILE:.0}\" \
             href=\"data:image/png;base64,{b64}\"/>",
            t.tx as f64 * TILE - view.x0,
            t.ty as f64 * TILE - view.y0
        );
    }
    let _ = write!(w, "</g>");

    for (gi, g) in groups.iter().enumerate() {
        let _ = write!(w, "<g class=\"group\" data-group=\"{gi}\">");
        for route in &g.routes {
            let mut pts = String::new();
            for (lat, lon) in route {
                let (px, py) = project(*lat, *lon, zoom);
                let _ = write!(pts, "{:.1},{:.1} ", px - view.x0, py - view.y0);
            }
            let pts = pts.trim_end();
            // A white casing under the line keeps it readable over dark tiles.
            let _ = write!(
                w,
                "<polyline class=\"route\" points=\"{pts}\" stroke=\"#fff\" \
                 stroke-width=\"6\" opacity=\".7\"/>\
                 <polyline class=\"route\" points=\"{pts}\" stroke=\"{}\" \
                 stroke-width=\"3\"/>",
                g.color
            );
        }
        for m in &g.marks {
            let (px, py) = project(m.lat, m.lon, zoom);
            let mut label = if m.name.is_empty() { "placemark".to_string() } else { m.name.clone() };
            if let Some(when) = &m.when {
                label.push_str(" — ");
                label.push_str(when);
            }
            // Falling back to the anchor keeps the link meaningful for a
            // placemark that carries no picture.
            let href = m.image.clone().unwrap_or_else(|| format!("#{}", m.uid));
            let _ = write!(
                w,
                "<a class=\"mark\" id=\"pin-{uid}\" data-uid=\"{uid}\" href=\"{href}\" \
                 target=\"_blank\"><title>{label}</title>\
                 <circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"5\" fill=\"{fill}\"/></a>",
                uid = m.uid,
                href = esc(&href),
                label = esc(&label),
                cx = px - view.x0,
                cy = py - view.y0,
                fill = g.color,
            );
        }
        let _ = write!(w, "</g>");
    }

    let _ = write!(w, "{}", chrome(groups, view, zoom, title, subtitle));
    let _ = write!(w, "</svg>");
    s
}

/// Title, scale bar and the attribution the tile licence requires.
fn chrome(groups: &[Group], view: View, zoom: u8, title: &str, subtitle: &str) -> String {
    let mut s = String::new();
    let w = &mut s;
    let _ = write!(w, "<g class=\"chrome\" pointer-events=\"none\">");
    // The page prints its own heading above the map, and hides this pair; they
    // are here so the standalone map.svg still says what it is.
    let _ = write!(
        w,
        "<text class=\"caption\" x=\"14\" y=\"26\" font-size=\"16\" font-weight=\"600\" \
         fill=\"#0b0b0b\">{}</text>",
        esc(title)
    );
    if !subtitle.is_empty() {
        let _ = write!(
            w,
            "<text class=\"caption\" x=\"14\" y=\"46\" font-size=\"12\" fill=\"#52514e\">{}</text>",
            esc(subtitle)
        );
    }

    let lats: Vec<f64> = groups.iter().flat_map(|g| g.points()).map(|(lat, _)| lat).collect();
    let mid = if lats.is_empty() { 0.0 } else { lats.iter().sum::<f64>() / lats.len() as f64 };
    let mpp = meters_per_pixel(mid, zoom);
    // A round distance that lands somewhere readable on the page.
    let (metres, px) = [10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000, 50_000]
        .into_iter()
        .map(|m| (m, m as f64 / mpp))
        .find(|(_, px)| (60.0..=200.0).contains(px))
        .unwrap_or_else(|| ((100.0 * mpp) as i64, 100.0));
    let shown = if metres < 1000 {
        format!("{metres} m")
    } else {
        format!("{} km", metres as f64 / 1000.0)
    };
    let by = view.h - 16.0;
    let _ = write!(
        w,
        "<path d=\"M14,{a:.0} L14,{by:.0} L{end:.0},{by:.0} L{end:.0},{a:.0}\" fill=\"none\" \
         stroke=\"#0b0b0b\" stroke-width=\"2\"/>\
         <text x=\"{mid_x:.0}\" y=\"{ty:.0}\" font-size=\"11\" text-anchor=\"middle\" \
         fill=\"#0b0b0b\">{shown}</text>",
        a = by - 6.0,
        by = by,
        end = 14.0 + px,
        mid_x = 14.0 + px / 2.0,
        ty = by - 10.0,
    );
    let _ = write!(
        w,
        "<text x=\"{:.0}\" y=\"{:.0}\" font-size=\"11\" text-anchor=\"end\" \
         fill=\"#52514e\">{ATTRIBUTION}</text>",
        view.w - 12.0,
        view.h - 12.0
    );
    let _ = write!(w, "</g>");
    s
}

const PAGE_CSS: &str = r#"
:root{color-scheme:light dark;--bg:#fcfcfb;--panel:#fff;--ink:#0b0b0b;--ink2:#52514e;--line:#e2e0da;--link:#2a78d6}
@media (prefers-color-scheme:dark){
  :root{--bg:#1a1a19;--panel:#232321;--ink:#fff;--ink2:#c3c2b7;--line:#3a3a37;--link:#3987e5}
}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);
  font:15px/1.55 ui-sans-serif,system-ui,-apple-system,'Helvetica Neue',Arial,sans-serif}
a{color:var(--link)}
.wrap{max-width:1200px;margin:0 auto;padding:24px 20px 64px}
h1{font-size:24px;margin:0 0 4px}
.sub{color:var(--ink2);margin:0 0 20px;font-size:14px}
.mapbox{background:var(--panel);border:1px solid var(--line);border-radius:10px;overflow:auto}
.mapbox svg.map{display:block;width:100%;height:auto}
.mapbox .caption{display:none}
.legend{display:flex;flex-wrap:wrap;gap:8px;margin:14px 0 0;padding:0;list-style:none}
.legend button{display:flex;align-items:center;gap:7px;cursor:pointer;background:var(--panel);
  color:var(--ink);border:1px solid var(--line);border-radius:999px;padding:5px 12px;
  font:inherit;font-size:13px}
.legend button[aria-pressed=false]{opacity:.45}
.dot{width:10px;height:10px;border-radius:50%;flex:none;display:inline-block}
.count{color:var(--ink2);font-variant-numeric:tabular-nums}
h2{font-size:16px;margin:34px 0 12px;font-weight:600}
.grid{display:grid;gap:14px;grid-template-columns:repeat(auto-fill,minmax(240px,1fr));
  padding:0;margin:0;list-style:none}
.card{background:var(--panel);border:1px solid var(--line);border-radius:10px;overflow:hidden;
  scroll-margin-top:20px}
.card.flash{outline:2px solid var(--link);outline-offset:2px}
.card>img{display:block;width:100%;height:auto;background:#111}
.card .body{padding:10px 12px 12px;font-size:13px;color:var(--ink2);overflow-wrap:anywhere}
.card .body b{color:var(--ink)}
.card .body img{display:none}
.card h3{margin:0 0 4px;font-size:14px;color:var(--ink);display:flex;align-items:center;gap:7px}
.card .locate{margin-left:auto;font-size:12px}
#peek{position:fixed;pointer-events:none;z-index:9;display:none;background:var(--panel);
  border:1px solid var(--line);border-radius:8px;padding:6px;
  box-shadow:0 6px 24px rgba(0,0,0,.28);max-width:280px}
#peek img{display:block;width:100%;height:auto;border-radius:4px}
#peek span{display:block;font-size:12px;color:var(--ink2);padding:5px 2px 1px}
footer{margin-top:40px;color:var(--ink2);font-size:12px}
"#;

const PAGE_JS: &str = r#"
const peek = document.getElementById('peek');
const svg = document.querySelector('svg.map');
for (const pin of svg.querySelectorAll('.mark')) {
  const card = document.getElementById(pin.dataset.uid);
  pin.addEventListener('mouseenter', () => {
    peek.innerHTML = '';
    const img = card && card.querySelector('img');
    if (img) {
      const copy = document.createElement('img');
      copy.src = img.getAttribute('src');
      peek.appendChild(copy);
    }
    const cap = document.createElement('span');
    cap.textContent = pin.querySelector('title').textContent;
    peek.appendChild(cap);
    peek.style.display = 'block';
  });
  pin.addEventListener('mousemove', (e) => {
    const pad = 16;
    peek.style.left = Math.min(e.clientX + pad, innerWidth - peek.offsetWidth - 8) + 'px';
    peek.style.top = Math.max(8,
      Math.min(e.clientY + pad, innerHeight - peek.offsetHeight - 8)) + 'px';
  });
  pin.addEventListener('mouseleave', () => { peek.style.display = 'none'; });
  pin.addEventListener('click', (e) => {
    if (!card) return;
    e.preventDefault();
    card.scrollIntoView({behavior: 'smooth', block: 'center'});
    card.classList.add('flash');
    setTimeout(() => card.classList.remove('flash'), 1600);
  });
}
for (const btn of document.querySelectorAll('.legend button')) {
  btn.addEventListener('click', () => {
    const on = btn.getAttribute('aria-pressed') === 'true';
    btn.setAttribute('aria-pressed', String(!on));
    svg.querySelector('.group[data-group="' + btn.dataset.group + '"]')
       .classList.toggle('hidden', on);
    for (const c of document.querySelectorAll('.card[data-group="' + btn.dataset.group + '"]'))
      c.hidden = on;
  });
}
for (const link of document.querySelectorAll('.locate')) {
  link.addEventListener('click', (e) => {
    const pin = document.getElementById('pin-' + link.dataset.uid);
    if (!pin) return;
    e.preventDefault();
    pin.scrollIntoView({behavior: 'smooth', block: 'center', inline: 'center'});
    const dot = pin.querySelector('circle');
    dot.setAttribute('r', '11');
    setTimeout(() => dot.setAttribute('r', '5'), 1200);
  });
}
"#;

pub fn page(groups: &[Group], map: &str, title: &str, subtitle: &str, sources: &[String]) -> String {
    let total: usize = groups.iter().map(|g| g.marks.len()).sum();
    let mut s = String::new();
    let w = &mut s;
    let _ = write!(w, "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">");
    let _ = write!(w, "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    let _ = write!(w, "<title>{}</title>", esc(title));
    let _ = write!(w, "<style>{PAGE_CSS}</style></head><body><div class=\"wrap\">");
    let _ = write!(w, "<h1>{}</h1>", esc(title));
    let _ = write!(w, "<p class=\"sub\">{}</p>", esc(subtitle));
    let _ = write!(w, "<div class=\"mapbox\">{map}</div>");

    // Identity is never left to colour alone: the legend names every group, and
    // each placemark's own card repeats it.
    if groups.len() > 1 {
        let _ = write!(w, "<ul class=\"legend\">");
        for (gi, g) in groups.iter().enumerate() {
            let _ = write!(
                w,
                "<li><button type=\"button\" aria-pressed=\"true\" data-group=\"{gi}\">\
                 <span class=\"dot\" style=\"background:{}\"></span>{} \
                 <span class=\"count\">{}</span></button></li>",
                g.color,
                esc(&g.name),
                g.marks.len()
            );
        }
        let _ = write!(w, "</ul>");
    }

    let _ = write!(w, "<h2>Placemarks <span class=\"count\">({total})</span></h2>");
    let _ = write!(w, "<ul class=\"grid\">");
    for (gi, g) in groups.iter().enumerate() {
        for m in &g.marks {
            let _ = write!(w, "<li class=\"card\" id=\"{}\" data-group=\"{gi}\">", m.uid);
            if let Some(img) = &m.image {
                let _ = write!(
                    w,
                    "<img loading=\"lazy\" src=\"{}\" alt=\"{}\">",
                    esc(img),
                    esc(&m.name)
                );
            }
            let _ = write!(w, "<div class=\"body\">");
            let heading = if m.name.is_empty() { "Placemark" } else { &m.name };
            let _ = write!(
                w,
                "<h3><span class=\"dot\" style=\"background:{}\"></span>{}\
                 <a class=\"locate\" data-uid=\"{}\" href=\"#pin-{}\">on map</a></h3>",
                g.color,
                esc(heading),
                m.uid,
                m.uid
            );
            if groups.len() > 1 {
                let _ = write!(w, "<div>{}</div>", esc(&g.name));
            }
            let _ = write!(w, "{}", scrub(&m.desc));
            let _ = write!(w, "<div>{:.6}, {:.6}</div>", m.lat, m.lon);
            let _ = write!(w, "</div></li>");
        }
    }
    let _ = write!(w, "</ul>");

    let names: Vec<String> = sources.iter().map(|s| esc(s)).collect();
    let _ = write!(
        w,
        "<footer>Map data {ATTRIBUTION}. Built from {} by \
         <a href=\"https://github.com/ryjones/maps-thingy\">maps-thingy</a>.</footer>",
        names.join(", ")
    );
    let _ = write!(w, "</div><div id=\"peek\"></div><script>{PAGE_JS}</script></body></html>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kml::Mark;

    fn group() -> Group {
        let mut g = Group {
            name: "clip one".into(),
            marks: Vec::new(),
            routes: vec![vec![(47.59, -122.25), (47.60, -122.26)]],
            color: "#2a78d6".into(),
        };
        g.marks.push(Mark {
            name: "ABC123".into(),
            lat: 47.5901,
            lon: -122.2493,
            when: Some("2026-08-01T20:13:03Z".into()),
            desc: "<b>ABC123</b><script>alert(1)</script>".into(),
            image: Some("images/a.jpg".into()),
            uid: "s0000".into(),
        });
        g
    }

    fn view() -> View {
        View { x0: 0.0, y0: 0.0, w: 800.0, h: 600.0 }
    }

    #[test]
    fn the_map_carries_the_route_the_pin_and_the_attribution() {
        let out = svg(&[group()], view(), 14, &[], "Scan", "1 placemark");
        assert!(out.contains("<polyline"), "route drawn");
        assert!(out.contains("id=\"pin-s0000\""), "pin anchored");
        assert!(out.contains("href=\"images/a.jpg\""), "picture linked, not inlined");
        assert!(out.contains(ATTRIBUTION), "tile licence honoured");
        assert!(!out.contains("base64"), "no tiles supplied, so nothing inlined");
    }

    #[test]
    fn tiles_are_inlined_at_their_grid_position() {
        let tiles = vec![Tile { tx: 2, ty: 3, bytes: vec![0x89, b'P', b'N', b'G'] }];
        let v = View { x0: 512.0, y0: 512.0, w: 512.0, h: 512.0 };
        let out = svg(&[group()], v, 14, &tiles, "Scan", "");
        assert!(out.contains("data:image/png;base64,iVBORw=="), "{out}");
        assert!(out.contains("x=\"0\" y=\"256\""), "placed relative to the view");
    }

    #[test]
    fn the_standalone_map_captions_itself_and_the_page_hides_that() {
        let g = group();
        let map = svg(std::slice::from_ref(&g), view(), 14, &[], "Scan", "1 placemark");
        assert!(map.contains("class=\"caption\""), "map.svg says what it is");
        let html = page(&[g], &map, "Scan", "1 placemark", &[]);
        assert!(
            html.contains(".mapbox .caption{display:none}"),
            "the page prints its own heading instead"
        );
    }

    #[test]
    fn the_page_inlines_the_map_and_scrubs_the_balloon() {
        let g = group();
        let map = svg(std::slice::from_ref(&g), view(), 14, &[], "Scan", "");
        let html = page(&[g], &map, "Scan", "1 placemark", &["scan.kmz".into()]);
        assert!(html.contains("<svg"), "map inlined");
        assert!(html.contains("<b>ABC123</b>"));
        assert!(!html.contains("alert(1)"), "script dropped from the balloon");
        assert!(html.contains("47.590100, -122.249300"));
    }

    #[test]
    fn a_legend_appears_only_once_there_is_something_to_tell_apart() {
        let one = group();
        let map = svg(std::slice::from_ref(&one), view(), 14, &[], "Scan", "");
        assert!(!page(&[one], &map, "Scan", "", &[]).contains("class=\"legend\""));

        let (a, mut b) = (group(), group());
        b.name = "clip two".into();
        b.color = "#eb6834".into();
        let two = vec![a, b];
        let map = svg(&two, view(), 14, &[], "Scan", "");
        let html = page(&two, &map, "Scan", "", &[]);
        assert!(html.contains("class=\"legend\""));
        assert!(html.contains("clip two"));
    }
}
