//! Reading a KMZ (or a bare KML) into groups of placemarks.
//!
//! Deliberately tolerant: the namespace is ignored, unknown elements are walked
//! through, and a placemark contributes whatever geometry it happens to carry.
//! A KMZ written by platescan, by Google Earth, or by hand all land in the same
//! shape — a list of folders, each with points and route lines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// One placemark: a position, and whatever the KML hung off it.
pub struct Mark {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Instant from `<TimeStamp>` or the start of a `<TimeSpan>`.
    pub when: Option<String>,
    /// Balloon HTML, with any picture reference rewritten to the site's copy.
    pub desc: String,
    /// Site-relative path to the extracted picture, e.g. `images/plate.jpg`.
    pub image: Option<String>,
    /// Anchor id, assigned once every source has been read.
    pub uid: String,
}

/// A KML folder. For a platescan export, one clip.
pub struct Group {
    pub name: String,
    pub marks: Vec<Mark>,
    /// Each route is an ordered run of (lat, lon).
    pub routes: Vec<Vec<(f64, f64)>>,
    /// Assigned once every source has been read.
    pub color: String,
}

impl Group {
    fn new(name: String) -> Self {
        Group { name, marks: Vec::new(), routes: Vec::new(), color: String::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.routes.is_empty()
    }

    /// Every position the group puts on the map, for bounds and centring.
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.marks
            .iter()
            .map(|m| (m.lat, m.lon))
            .chain(self.routes.iter().flatten().copied())
    }
}

/// Where a source's pictures live: inside the archive, or beside the .kml.
enum Store {
    Zip(zip::ZipArchive<std::fs::File>),
    Dir(PathBuf),
}

impl Store {
    fn read(&mut self, member: &str) -> Option<Vec<u8>> {
        match self {
            Store::Zip(z) => {
                // Balloons written elsewhere may reference a picture by a path
                // that does not match the entry exactly; fall back to the leaf.
                let name = if z.index_for_name(member).is_some() {
                    member.to_string()
                } else {
                    let leaf = member.rsplit('/').next()?;
                    let hit = (0..z.len()).find_map(|i| {
                        let n = z.by_index(i).ok()?.name().to_string();
                        (n.rsplit('/').next() == Some(leaf)).then_some(n)
                    })?;
                    hit
                };
                let mut f = z.by_name(&name).ok()?;
                let mut buf = Vec::new();
                std::io::copy(&mut f, &mut buf).ok()?;
                Some(buf)
            }
            Store::Dir(base) => std::fs::read(base.join(member)).ok(),
        }
    }
}

/// Read one `.kmz` or `.kml`, extracting referenced pictures into `images_dir`.
///
/// `taken` carries picture names already written by earlier sources so two
/// inputs that both hold `plate.jpg` do not overwrite each other.
pub fn read(
    path: &Path,
    images_dir: &Path,
    taken: &mut HashMap<String, u64>,
) -> Result<(Vec<Group>, String)> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let (text, mut store) = open(path)?;
    let (mut groups, doc_name) = parse(&text, &stem)?;

    for g in &mut groups {
        for m in &mut g.marks {
            let Some(span) = img_src_span(&m.desc) else { continue };
            let member = unescape_entities(&m.desc[span.clone()]);
            let Some(bytes) = store.read(&member) else {
                m.desc.replace_range(span, "");
                continue;
            };
            let leaf = member.rsplit('/').next().unwrap_or("image");
            let mut local = safe_name(leaf);
            let digest = hash(&bytes);
            match taken.get(&local) {
                Some(seen) if *seen != digest => local = format!("{}-{local}", safe_name(&stem)),
                _ => {}
            }
            taken.insert(local.clone(), digest);
            let dest = images_dir.join(&local);
            std::fs::write(&dest, &bytes)
                .with_context(|| format!("failed to write {}", dest.display()))?;
            let rel = format!("images/{local}");
            m.desc.replace_range(span, &rel);
            m.image = Some(rel);
        }
    }
    Ok((groups, doc_name))
}

fn open(path: &Path) -> Result<(String, Store)> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    match zip::ZipArchive::new(file) {
        Ok(mut zip) => {
            let entry = (0..zip.len())
                .filter_map(|i| Some(zip.by_index(i).ok()?.name().to_string()))
                .find(|n| n == "doc.kml")
                .or_else(|| {
                    (0..zip.len())
                        .filter_map(|i| Some(zip.by_index(i).ok()?.name().to_string()))
                        .find(|n| n.to_ascii_lowercase().ends_with(".kml"))
                });
            let Some(entry) = entry else {
                bail!("{}: a zip, but with no .kml inside", path.display());
            };
            let text = {
                let mut f = zip.by_name(&entry)?;
                let mut buf = Vec::new();
                std::io::copy(&mut f, &mut buf)?;
                String::from_utf8_lossy(&buf).into_owned()
            };
            Ok((text, Store::Zip(zip)))
        }
        Err(_) => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            Ok((String::from_utf8_lossy(&bytes).into_owned(), Store::Dir(base)))
        }
    }
}

/// Placemark fields collected while its subtree is being read.
#[derive(Default)]
struct Pending {
    name: String,
    desc: String,
    when: Option<String>,
    points: Vec<(f64, f64)>,
    routes: Vec<Vec<(f64, f64)>>,
}

fn parse(text: &str, stem: &str) -> Result<(Vec<Group>, String)> {
    let mut reader = Reader::from_str(text);
    reader.trim_text(true);
    reader.check_end_names(false);

    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    // Folders nest; the innermost named one owns the placemarks below it.
    let mut folders: Vec<Option<String>> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut text_buf = String::new();
    let mut doc_name: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.local_name().as_ref());
                if name == "Placemark" {
                    pending = Some(Pending::default());
                } else if name == "Folder" || name == "Document" {
                    folders.push(None);
                }
                path.push(name);
                text_buf.clear();
            }
            Ok(Event::Text(e)) => {
                text_buf.push_str(&e.unescape().unwrap_or_default());
            }
            Ok(Event::CData(e)) => {
                text_buf.push_str(&String::from_utf8_lossy(e.as_ref()));
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.local_name().as_ref());
                let parent = path.iter().rev().nth(1).cloned().unwrap_or_default();
                let value = std::mem::take(&mut text_buf);

                if let Some(pm) = pending.as_mut() {
                    match name.as_str() {
                        "name" if parent == "Placemark" => pm.name = value,
                        "description" if parent == "Placemark" => pm.desc = value,
                        "when" | "begin" if pm.when.is_none() && !value.is_empty() => {
                            pm.when = Some(value)
                        }
                        "coordinates" => {
                            let coords = parse_coords(&value);
                            if parent == "Point" {
                                pm.points.extend(coords);
                            } else if coords.len() > 1 {
                                pm.routes.push(coords);
                            }
                        }
                        _ => {}
                    }
                } else if name == "name" && (parent == "Folder" || parent == "Document") {
                    if parent == "Document" && doc_name.is_none() {
                        doc_name = Some(value.clone());
                    }
                    if let Some(slot) = folders.last_mut() {
                        *slot = Some(value);
                    }
                }


                if name == "Placemark" {
                    if let Some(pm) = pending.take() {
                        let key = folders
                            .iter()
                            .rev()
                            .find_map(|f| f.clone())
                            .or_else(|| doc_name.clone())
                            .unwrap_or_else(|| stem.to_string());
                        let gi = *index.entry(key.clone()).or_insert_with(|| {
                            groups.push(Group::new(key.clone()));
                            groups.len() - 1
                        });
                        let g = &mut groups[gi];
                        for (lat, lon) in pm.points {
                            g.marks.push(Mark {
                                name: pm.name.clone(),
                                lat,
                                lon,
                                when: pm.when.clone(),
                                desc: pm.desc.clone(),
                                image: None,
                                uid: String::new(),
                            });
                        }
                        g.routes.extend(pm.routes);
                    }
                } else if name == "Folder" || name == "Document" {
                    folders.pop();
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => bail!("not parseable as KML: {e}"),
        }
    }

    groups.retain(|g| !g.is_empty());
    Ok((groups, doc_name.unwrap_or_else(|| stem.to_string())))
}

fn local_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// KML coordinate lists are whitespace-separated `lon,lat[,alt]` triples.
fn parse_coords(text: &str) -> Vec<(f64, f64)> {
    text.split_whitespace()
        .filter_map(|chunk| {
            let mut parts = chunk.split(',');
            let lon: f64 = parts.next()?.parse().ok()?;
            let lat: f64 = parts.next()?.parse().ok()?;
            ((-180.0..=180.0).contains(&lon) && (-85.05..=85.05).contains(&lat))
                .then_some((lat, lon))
        })
        .collect()
}

/// Byte range of the first `<img src=…>` value in balloon HTML, so the
/// reference can be read and then rewritten in place.
fn img_src_span(desc: &str) -> Option<std::ops::Range<usize>> {
    // Lowercasing ASCII leaves every byte index where it was.
    let lower = desc.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("<img") {
        let start = from + rel;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        from = end.max(start + 4);
        let Some(at) = lower[start..end].find("src") else { continue };
        let mut i = start + at + 3;
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let (open, term): (usize, &[u8]) = match bytes.get(i) {
            Some(b'"') => (i + 1, b"\""),
            Some(b'\'') => (i + 1, b"'"),
            _ => (i, b" "),
        };
        let close = lower[open..end]
            .find(term[0] as char)
            .map(|e| open + e)
            .unwrap_or(end);
        if close > open {
            return Some(open..close);
        }
    }
    None
}

fn unescape_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "image".into() } else { cleaned }
}

/// FNV-1a, only ever compared against itself to spot a name collision.
fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
<name>License plate scan</name>
<Folder><name>clip one</name>
<Placemark><name>Route</name><LineString><coordinates>
-122.25,47.59,0 -122.26,47.60,0</coordinates></LineString></Placemark>
<Placemark><name>ABC123</name>
<TimeStamp><when>2026-08-01T20:13:03Z</when></TimeStamp>
<description><![CDATA[<img src="files/a.jpg"/><br/><b>ABC123</b>]]></description>
<Point><coordinates>-122.249349,47.590145,0</coordinates></Point>
</Placemark></Folder>
<Folder><name>clip two</name>
<Placemark><name>XYZ</name><Point><coordinates>-122.3,47.5</coordinates></Point></Placemark>
</Folder></Document></kml>"#;

    #[test]
    fn folders_become_groups() {
        let (groups, name) = parse(DOC, "stem").expect("parses");
        assert_eq!(name, "License plate scan");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "clip one");
        assert_eq!(groups[0].marks.len(), 1, "the route placemark adds no marks");
        assert_eq!(groups[0].routes.len(), 1);
        assert_eq!(groups[0].routes[0].len(), 2);
        assert_eq!(groups[1].marks[0].name, "XYZ");
    }

    #[test]
    fn placemark_keeps_its_time_and_balloon() {
        let (groups, _) = parse(DOC, "stem").expect("parses");
        let m = &groups[0].marks[0];
        assert_eq!(m.name, "ABC123");
        assert_eq!(m.when.as_deref(), Some("2026-08-01T20:13:03Z"));
        assert!(m.desc.contains("<b>ABC123</b>"));
        assert!((m.lat - 47.590145).abs() < 1e-9);
        assert!((m.lon + 122.249349).abs() < 1e-9);
    }

    #[test]
    fn coordinates_survive_missing_altitude_and_junk() {
        assert_eq!(parse_coords("-122.3,47.5"), vec![(47.5, -122.3)]);
        assert_eq!(parse_coords("nonsense 1,2,3"), vec![(2.0, 1.0)]);
        assert!(parse_coords("999,999,0").is_empty());
    }

    #[test]
    fn finds_the_picture_reference() {
        let d = r#"<img src="files/a.jpg" style="x"/>rest"#;
        let span = img_src_span(d).expect("found");
        assert_eq!(&d[span], "files/a.jpg");
        assert_eq!(img_src_span("no picture here"), None);
        let single = "<IMG SRC='b.png'>";
        assert_eq!(&single[img_src_span(single).unwrap()], "b.png");
    }

    #[test]
    fn placemarks_outside_a_folder_land_under_the_document() {
        let doc = r#"<kml><Document><name>Loose</name>
<Placemark><name>P</name><Point><coordinates>1,2</coordinates></Point></Placemark>
</Document></kml>"#;
        let (groups, _) = parse(doc, "stem").expect("parses");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "Loose");
    }
}
