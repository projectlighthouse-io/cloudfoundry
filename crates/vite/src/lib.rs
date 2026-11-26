//! Resolve asset tags from Vite.
//!
//! ```ignore
//! let vite = Vite::load("public", &["resources/js/app.ts"])?;
//! let head: String = vite.tags();        // dev server tags or manifest tags
//! let version: &str = vite.manifest_hash();
//! ```
//!
//! Dev: if `public/hot` exists (written by the Vite config `art new`
//! generates), tags point at the dev server it names. It is checked on every
//! call, so starting or stopping Vite needs no app restart.
//!
//! Production: `public/build/.vite/manifest.json`, read once. A missing entry
//! point is a startup error.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Deserialize)]
struct Chunk {
    file: String,
    #[serde(default)]
    css: Vec<String>,
    #[serde(default)]
    imports: Vec<String>,
}

pub struct Vite {
    hot: PathBuf,
    entries: Vec<String>,
    /// Tags from the manifest, built once. `None` when there is no build yet.
    built: Option<String>,
    hash: String,
}

#[derive(Debug)]
pub enum Error {
    /// The manifest exists but cannot be read or parsed.
    Manifest(String),
    /// An entry point is not in the manifest.
    MissingEntry(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Manifest(e) => write!(f, "vite manifest: {e}"),
            Error::MissingEntry(e) => write!(f, "vite manifest has no entry `{e}`; run the build"),
        }
    }
}

impl std::error::Error for Error {}

impl Vite {
    /// `public` is the public directory: `hot` and `build/` live in it.
    pub fn load(public: impl AsRef<Path>, entries: &[&str]) -> Result<Self, Error> {
        let public = public.as_ref();
        let entries: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
        let manifest_path = public.join("build/.vite/manifest.json");

        let (built, hash) = match fs::read(&manifest_path) {
            Ok(bytes) => {
                let manifest: BTreeMap<String, Chunk> =
                    serde_json::from_slice(&bytes).map_err(|e| Error::Manifest(e.to_string()))?;
                (Some(manifest_tags(&manifest, &entries)?), format!("{:016x}", fnv1a(&bytes)))
            }
            // No build yet: fine in dev, where the hot file takes over.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, String::new()),
            Err(e) => return Err(Error::Manifest(e.to_string())),
        };
        Ok(Vite { hot: public.join("hot"), entries, built, hash })
    }

    /// `<script>` and `<link>` tags for the entry points.
    pub fn tags(&self) -> String {
        if let Ok(url) = fs::read_to_string(&self.hot) {
            let url = url.trim().trim_end_matches('/');
            let mut out = format!(r#"<script type="module" src="{url}/@vite/client"></script>"#);
            for e in &self.entries {
                out.push_str(&format!(r#"<script type="module" src="{url}/{e}"></script>"#));
            }
            return out;
        }
        self.built.clone().unwrap_or_default()
    }

    /// Changes whenever the build changes. Empty without a build. Used as the
    /// Inertia asset version.
    pub fn manifest_hash(&self) -> &str {
        &self.hash
    }
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`, so every app
/// instance computes the same version for the same build.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

fn manifest_tags(manifest: &BTreeMap<String, Chunk>, entries: &[String]) -> Result<String, Error> {
    let (mut css, mut preload, mut scripts) = (BTreeSet::new(), BTreeSet::new(), vec![]);
    for e in entries {
        let chunk = manifest.get(e).ok_or_else(|| Error::MissingEntry(e.clone()))?;
        scripts.push(chunk.file.clone());
        collect(manifest, chunk, &mut css, &mut preload);
    }
    let mut out = String::new();
    for c in css {
        out.push_str(&format!(r#"<link rel="stylesheet" href="/build/{c}">"#));
    }
    for p in preload.iter().filter(|p| !scripts.contains(p)) {
        out.push_str(&format!(r#"<link rel="modulepreload" href="/build/{p}">"#));
    }
    for s in scripts {
        out.push_str(&format!(r#"<script type="module" src="/build/{s}"></script>"#));
    }
    Ok(out)
}

/// CSS of a chunk and everything it imports; files of its imports.
fn collect(manifest: &BTreeMap<String, Chunk>, chunk: &Chunk, css: &mut BTreeSet<String>, preload: &mut BTreeSet<String>) {
    css.extend(chunk.css.iter().cloned());
    for i in &chunk.imports {
        if let Some(dep) = manifest.get(i) {
            if preload.insert(dep.file.clone()) {
                collect(manifest, dep, css, preload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vite-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("build/.vite")).unwrap();
        d
    }

    #[test]
    fn manifest_tags_and_hot_file() {
        let d = tmp("tags");
        fs::write(
            d.join("build/.vite/manifest.json"),
            r#"{"app.ts":{"file":"assets/app.js","css":["assets/app.css"],"imports":["_v.js"],"isEntry":true},
                "_v.js":{"file":"assets/v.js","css":["assets/v.css"]}}"#,
        )
        .unwrap();
        let v = Vite::load(&d, &["app.ts"]).unwrap();
        assert_eq!(
            v.tags(),
            r#"<link rel="stylesheet" href="/build/assets/app.css"><link rel="stylesheet" href="/build/assets/v.css"><link rel="modulepreload" href="/build/assets/v.js"><script type="module" src="/build/assets/app.js"></script>"#
        );
        assert_eq!(v.manifest_hash().len(), 16);
        assert!(matches!(Vite::load(&d, &["nope.ts"]), Err(Error::MissingEntry(_))));

        fs::write(d.join("hot"), "http://localhost:5173\n").unwrap();
        assert!(v.tags().starts_with(r#"<script type="module" src="http://localhost:5173/@vite/client">"#));
        assert!(v.tags().ends_with(r#"src="http://localhost:5173/app.ts"></script>"#));
    }

    #[test]
    fn no_build_is_fine() {
        let d = tmp("none");
        let v = Vite::load(&d, &["app.ts"]).unwrap();
        assert_eq!((v.tags().as_str(), v.manifest_hash()), ("", ""));
    }
}
