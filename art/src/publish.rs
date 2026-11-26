//! `art publish <package>`: copy a dependency's stub files into the app.
//!
//! A package declares them in its own Cargo.toml:
//!
//! ```toml
//! [package.metadata.art.publish]
//! "stubs/handlers/auth.rs" = "src/handlers/auth.rs"
//! "stubs/migrations/create_users.up.sql" = "migrations/{timestamp}_create_users.up.sql"
//!
//! [package.metadata.art]
//! notes = "Printed after publishing: the lines to add to routes.rs and main.rs."
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

pub fn run(package: &str, list: bool, force: bool) -> Result<(), String> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;

    let pkg = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == package)
        .ok_or_else(|| format!("`{package}` is not a dependency of this app; add it to Cargo.toml first"))?;
    let src_dir = Path::new(pkg["manifest_path"].as_str().unwrap_or_default()).parent().map(Path::to_path_buf).unwrap_or_default();
    let art = &pkg["metadata"]["art"];
    let table = art["publish"].as_object().ok_or_else(|| format!("`{package}` has nothing to publish"))?;

    let ts = next_version(&timestamp());
    for (from, to) in table {
        let Some(to) = to.as_str() else { continue };
        let dest = PathBuf::from(to.replace("{timestamp}", &ts));
        if list {
            println!("{from} -> {to}");
            continue;
        }
        if !force && let Some(existing) = already_published(to) {
            println!("skip     {} (exists; --force to overwrite)", existing.display());
            continue;
        }
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        fs::copy(src_dir.join(from), &dest).map_err(|e| format!("{from}: {e}"))?;
        println!("publish  {}", dest.display());
    }

    if !list && let Some(notes) = art["notes"].as_str() {
        println!("\n{notes}");
    }
    Ok(())
}

/// The existing file for `template`. A `{timestamp}` part matches any
/// timestamp, so a migration is never published twice.
fn already_published(template: &str) -> Option<PathBuf> {
    let Some((before, after)) = template.split_once("{timestamp}") else {
        let p = PathBuf::from(template);
        return p.exists().then_some(p);
    };
    let path = Path::new(before);
    let (dir, prefix) = if before.ends_with('/') { (path, "") } else { (path.parent()?, path.file_name()?.to_str()?) };
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(prefix) && n.ends_with(after))
    })
}

/// sqlx uses the timestamp as the migration version, so two publishes in the
/// same second would collide. Take one past the newest existing version.
fn next_version(now: &str) -> String {
    let newest = fs::read_dir("migrations")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.split('_').next()?.parse::<u64>().ok())
        .max();
    let now: u64 = now.parse().unwrap_or(0);
    match newest {
        Some(n) if n >= now => (n + 1).to_string(),
        _ => now.to_string(),
    }
}

/// `YYYYMMDDHHMMSS` in UTC, as sqlx-cli names migrations.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let t = super::timestamp();
        assert_eq!(t.len(), 14);
        assert!(t.starts_with("20"));
    }
}
