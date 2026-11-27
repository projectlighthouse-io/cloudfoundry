//! `art publish <package>`: copy a dependency's stub files into the app.
//!
//! A package declares them in its own Cargo.toml:
//!
//! ```toml
//! [package.metadata.art.publish]
//! "stubs/handlers/auth.rs" = "src/handlers/auth.rs"
//! "stubs/migrations/create_users.up.sql" = "migrations/{timestamp}_create_users.up.sql"
//!
//! [package.metadata.art.publish-sqlite]   # `art publish <pkg> --driver sqlite`
//! "stubs/migrations/sqlite/create_users.up.sql" = "migrations/{timestamp}_create_users.up.sql"
//!
//! [package.metadata.art.with-ui]         # added with `--with-ui`; same destination replaces
//! "stubs/ui/handlers/auth.rs" = "src/handlers/auth.rs"
//! "stubs/pages/login.vue" = "resources/js/pages/auth/login.vue"
//!
//! [package.metadata.art]
//! notes = "Printed after publishing: the lines to add to routes.rs and main.rs."
//! with-ui-notes = "Printed instead with `--with-ui`."
//! agents = "Appended once to the app's AGENTS.md, for coding agents."
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

pub fn run(package: &str, driver: Option<&str>, with_ui: bool, list: bool, force: bool) -> Result<(), String> {
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
    // Driver-specific stubs live under `publish-<driver>`, e.g. `publish-sqlite`.
    let key = driver.map_or("publish".to_owned(), |d| format!("publish-{d}"));
    let table = art[&key].as_object().ok_or_else(|| match driver {
        Some(d) => format!("`{package}` has no stubs for driver `{d}`"),
        None => format!("`{package}` has nothing to publish"),
    })?;
    let mut entries = pairs(table);
    if with_ui {
        let ui = art["with-ui"].as_object().ok_or_else(|| format!("`{package}` has no UI stubs"))?;
        entries = with_ui_entries(entries, pairs(ui));
    }

    let versions = versions(&entries, &next_version(&timestamp()));
    for (from, to) in entries {
        let dest = PathBuf::from(match versions.iter().find(|(name, _)| *name == migration_name(to)) {
            Some((_, v)) => to.replace("{timestamp}", v),
            None => to.to_owned(),
        });
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

    if !list && let Some(agents) = art["agents"].as_str() {
        append_agent_notes(package, agents)?;
    }

    let notes = if with_ui { art["with-ui-notes"].as_str().or(art["notes"].as_str()) } else { art["notes"].as_str() };
    if !list && let Some(notes) = notes {
        println!("\n{notes}");
    }
    Ok(())
}

/// Add the package's notes for coding agents to the app's AGENTS.md, once.
fn append_agent_notes(package: &str, notes: &str) -> Result<(), String> {
    let path = Path::new("AGENTS.md");
    let Ok(current) = fs::read_to_string(path) else { return Ok(()) };
    if let Some(updated) = with_agent_notes(&current, package, notes) {
        fs::write(path, updated).map_err(|e| format!("AGENTS.md: {e}"))?;
        println!("update   AGENTS.md ({package} notes)");
    }
    Ok(())
}

/// `current` with a `## <package>` section of notes, or `None` if it has one already.
fn with_agent_notes(current: &str, package: &str, notes: &str) -> Option<String> {
    let marker = format!("<!-- art publish {package} -->");
    (!current.contains(&marker)).then(|| format!("{}\n\n## {package}\n\n{marker}\n{}\n", current.trim_end(), notes.trim()))
}

fn pairs(table: &serde_json::Map<String, Value>) -> Vec<(&str, &str)> {
    table.iter().filter_map(|(from, to)| Some((from.as_str(), to.as_str()?))).collect()
}

/// Add the UI entries; one with the same destination as a base entry replaces its source.
fn with_ui_entries<'a>(mut entries: Vec<(&'a str, &'a str)>, ui: Vec<(&'a str, &'a str)>) -> Vec<(&'a str, &'a str)> {
    for (from, to) in ui {
        match entries.iter_mut().find(|(_, t)| *t == to) {
            Some(entry) => entry.0 = from,
            None => entries.push((from, to)),
        }
    }
    entries
}

/// `migrations/{timestamp}_create_users.up.sql` -> `create_users`; up and down share it.
fn migration_name(template: &str) -> &str {
    let file = template.rsplit('/').next().unwrap_or(template);
    let name = file.strip_prefix("{timestamp}_").unwrap_or(file);
    name.trim_end_matches(".sql").trim_end_matches(".up").trim_end_matches(".down")
}

/// One version per migration (up and down share it), counting up from
/// `first` in publish order, so a package can ship several migrations.
fn versions<'a>(entries: &[(&str, &'a str)], first: &str) -> Vec<(&'a str, String)> {
    let first: u64 = first.parse().unwrap_or(0);
    let mut out: Vec<(&str, String)> = Vec::new();
    for (_, to) in entries.iter().filter(|(_, to)| to.contains("{timestamp}")) {
        let name = migration_name(to);
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, (first + out.len() as u64).to_string()));
        }
    }
    out
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
    fn with_ui_replaces_by_destination_and_adds_the_rest() {
        let base = vec![("stubs/handlers/login.rs", "src/handlers/login.rs"), ("stubs/users.rs", "src/users.rs")];
        let ui = vec![("stubs/ui/handlers/login.rs", "src/handlers/login.rs"), ("stubs/pages/login.vue", "resources/js/pages/auth/login.vue")];
        assert_eq!(
            super::with_ui_entries(base, ui),
            [
                ("stubs/ui/handlers/login.rs", "src/handlers/login.rs"),
                ("stubs/users.rs", "src/users.rs"),
                ("stubs/pages/login.vue", "resources/js/pages/auth/login.vue"),
            ]
        );
    }

    #[test]
    fn agent_notes_are_added_once() {
        let once = super::with_agent_notes("# app\n", "auth", "- use attempt").unwrap();
        assert_eq!(once, "# app\n\n## auth\n\n<!-- art publish auth -->\n- use attempt\n");
        assert_eq!(super::with_agent_notes(&once, "auth", "- use attempt"), None);
    }

    #[test]
    fn each_migration_gets_its_own_version() {
        let entries = [
            ("a", "migrations/{timestamp}_create_social_accounts.up.sql"),
            ("b", "migrations/{timestamp}_create_social_accounts.down.sql"),
            ("c", "src/social.rs"),
            ("d", "migrations/{timestamp}_allow_null_passwords.up.sql"),
            ("e", "migrations/{timestamp}_allow_null_passwords.down.sql"),
        ];
        assert_eq!(
            super::versions(&entries, "20261127100000"),
            [("create_social_accounts", "20261127100000".to_owned()), ("allow_null_passwords", "20261127100001".to_owned())]
        );
    }

    #[test]
    fn timestamp_shape() {
        let t = super::timestamp();
        assert_eq!(t.len(), 14);
        assert!(t.starts_with("20"));
    }
}
