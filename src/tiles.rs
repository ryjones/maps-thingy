//! Basemap tiles, fetched once and kept.
//!
//! Fetching shells out to curl rather than linking an HTTP and TLS stack for
//! the handful of requests a map needs. Tiles land in a cache keyed by
//! z/x/y, so re-rendering the same area — the common case while adjusting a
//! map — never touches the network, which is also what the OpenStreetMap tile
//! policy asks for. That policy is why the requests are sequential, spaced,
//! and sent with an agent string that identifies this tool.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const URL: &str = "https://tile.openstreetmap.org";
const USER_AGENT: &str = "maps-thingy/1.0 (+https://github.com/ryjones/maps-thingy)";
pub const ATTRIBUTION: &str = "© OpenStreetMap contributors";

pub struct Fetcher {
    pub cache: PathBuf,
    pub pause: f64,
    pub timeout: f64,
    pub quiet: bool,
}

impl Fetcher {
    /// Default cache directory: `$XDG_CACHE_HOME/maps-thingy/tiles`, else
    /// `~/.cache/maps-thingy/tiles`.
    pub fn default_cache() -> PathBuf {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|| PathBuf::from(".cache"));
        base.join("maps-thingy").join("tiles")
    }

    /// PNG bytes for one tile, or `None` if it is off the world or unreachable.
    /// A missing tile leaves a gap in the basemap rather than failing the run.
    pub fn tile(&self, z: u8, x: i64, y: i64) -> Option<Vec<u8>> {
        let n = 1i64 << z;
        if !(0..n).contains(&y) {
            return None;
        }
        let x = x.rem_euclid(n);
        let path = self.cache.join(z.to_string()).join(x.to_string()).join(format!("{y}.png"));
        if let Ok(bytes) = std::fs::read(&path) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
        match self.download(z, x, y, &path) {
            Ok(bytes) => {
                std::thread::sleep(std::time::Duration::from_secs_f64(self.pause));
                Some(bytes)
            }
            Err(e) => {
                if !self.quiet {
                    eprintln!("  tile {z}/{x}/{y}: {e}");
                }
                None
            }
        }
    }

    fn download(&self, z: u8, x: i64, y: i64, dest: &Path) -> Result<Vec<u8>> {
        let parent = dest.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        // Downloading beside the target and renaming keeps a failed or partial
        // transfer from poisoning the cache.
        let tmp = dest.with_extension("part");
        let out = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--location",
                "--user-agent",
                USER_AGENT,
                "--max-time",
                &format!("{:.0}", self.timeout.max(1.0)),
                "--output",
            ])
            .arg(&tmp)
            .arg(format!("{URL}/{z}/{x}/{y}.png"))
            .output()
            .context("failed to run curl (is it installed?)")?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let bytes = std::fs::read(&tmp).context("curl reported success but wrote nothing")?;
        if bytes.is_empty() {
            let _ = std::fs::remove_file(&tmp);
            bail!("empty response");
        }
        std::fs::rename(&tmp, dest)
            .with_context(|| format!("failed to store {}", dest.display()))?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_off_the_top_and_bottom_of_the_world_are_skipped() {
        let f = Fetcher {
            cache: std::env::temp_dir().join("maps-thingy-test-cache"),
            pause: 0.0,
            timeout: 1.0,
            quiet: true,
        };
        assert!(f.tile(2, 0, -1).is_none());
        assert!(f.tile(2, 0, 4).is_none());
    }

    #[test]
    fn a_cached_tile_is_read_instead_of_fetched() {
        let cache = std::env::temp_dir().join("maps-thingy-test-cache-hit");
        let dir = cache.join("3").join("1");
        std::fs::create_dir_all(&dir).expect("cache dir");
        std::fs::write(dir.join("2.png"), b"not really a png").expect("seed");
        let f = Fetcher { cache, pause: 0.0, timeout: 1.0, quiet: true };
        assert_eq!(f.tile(3, 1, 2).as_deref(), Some(&b"not really a png"[..]));
    }

    #[test]
    fn longitude_wraps_around_the_date_line() {
        // x = 8 at zoom 3 is one past the last column; it is column 0.
        let cache = std::env::temp_dir().join("maps-thingy-test-cache-wrap");
        let dir = cache.join("3").join("0");
        std::fs::create_dir_all(&dir).expect("cache dir");
        std::fs::write(dir.join("1.png"), b"wrapped").expect("seed");
        let f = Fetcher { cache, pause: 0.0, timeout: 1.0, quiet: true };
        assert_eq!(f.tile(3, 8, 1).as_deref(), Some(&b"wrapped"[..]));
    }
}
