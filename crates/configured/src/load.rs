use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
};

use config::{Map, Source, Value, ValueKind};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;

/// Extensions config-rs reads with this crate's formats, in lookup order.
const EXTENSIONS: [&str; 4] = ["toml", "json", "yaml", "yml"];
const MASK: &str = "<redacted>";

/// A loaded config: the typed value, plus what was loaded, for `config:show`.
pub struct Loaded<T> {
    pub value: T,
    pub listing: Listing,
}

/// Every key set in a file or by an env var, as loaded. Secret values are
/// held as [`SecretString`] and printed only by [`Listing::show_secret_values`].
pub struct Listing(Vec<Entry>);

/// One key of the listing. `secret` is the real value when `shown` is redacted.
struct Entry {
    path: String,
    shown: String,
    secret: Option<SecretString>,
}

impl Listing {
    /// Every key set in a file or by an env var, one per line, sorted (keys
    /// left out keep their defaults and are not listed). Values from the
    /// `secret_` forms and keys named like a secret (`*_key`, `*_secret`,
    /// `*_password`, `*_token`) print as `<redacted>`, as do passwords in URLs.
    pub fn show(&self) -> String {
        self.render(false)
    }

    /// [`Listing::show`] with the real secret values. Only for an explicit
    /// request, such as `config:show --print-secret-values`.
    pub fn show_secret_values(&self) -> String {
        self.render(true)
    }

    fn render(&self, reveal: bool) -> String {
        let width = self.0.iter().map(|e| e.path.len()).max().unwrap_or(0);
        self.0
            .iter()
            .map(|e| {
                let v = match &e.secret {
                    Some(real) if reveal => real.expose_secret(),
                    _ => &e.shown,
                };
                format!("{:width$}  {v}\n", e.path)
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum Error {
    /// `.env` exists but does not parse.
    DotEnv(String),
    /// `APP_ENV` is not a plain name, a config file is missing, or one name
    /// has files in several formats.
    Files(String),
    /// A file does not parse (config-rs).
    Source(config::ConfigError),
    /// Every `env(...)` problem at once: unset required vars and bad syntax,
    /// each with the config path it belongs to.
    Env(Vec<String>),
    /// The resolved config does not fit the struct. Secret values are
    /// scrubbed from the message.
    Shape(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DotEnv(e) => write!(f, "config: .env: {e}"),
            Self::Files(e) => write!(f, "config: {e}"),
            Self::Source(e) => write!(f, "config: {e}"),
            Self::Env(problems) => {
                write!(f, "config: {} env problem(s)", problems.len())?;
                problems.iter().try_for_each(|p| write!(f, "\n  {p}"))
            }
            Self::Shape(e) => write!(f, "config: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(e) => Some(e),
            _ => None,
        }
    }
}

/// Load `.env` (if present; never overriding real env vars), then
/// `<dir>/default.<ext>` (required) and `<dir>/<APP_ENV>.<ext>` (optional,
/// only when `APP_ENV` is set). Later files override earlier ones key by key.
/// `env(...)` values are resolved after merging, so an env file can replace
/// one with a literal. Fails on the first stage that has problems, reporting
/// all problems of that stage.
pub fn load<T: DeserializeOwned>(dir: impl AsRef<Path>) -> Result<Loaded<T>, Error> {
    match dotenvy::dotenv() {
        Err(e) if !e.not_found() => return Err(Error::DotEnv(e.to_string())),
        _ => {}
    }
    let app_env = lookup("APP_ENV").map_err(Error::Files)?;
    load_from(dir.as_ref(), app_env.as_deref(), &lookup)
}

/// An env var; set but empty counts as unset.
fn lookup(var: &str) -> Result<Option<String>, String> {
    match std::env::var(var) {
        Ok(v) if v.is_empty() => Ok(None),
        Ok(v) => Ok(Some(v)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{var} is not valid UTF-8")),
    }
}

type Lookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

fn load_from<T: DeserializeOwned>(dir: &Path, app_env: Option<&str>, lookup: Lookup<'_>) -> Result<Loaded<T>, Error> {
    let mut builder = config::Config::builder();
    let default = find(dir, "default")?.ok_or_else(|| Error::Files(format!("no default.{{toml,json,yaml}} in {}", dir.display())))?;
    builder = builder.add_source(config::File::from(default));
    if let Some(env) = app_env {
        if env.is_empty() || !env.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') || env == "default" {
            return Err(Error::Files(format!("APP_ENV `{env}` must be a plain name (letters, digits, `_`, `-`), not `default`")));
        }
        if let Some(path) = find(dir, env)? {
            builder = builder.add_source(config::File::from(path));
        }
    }
    let tree = builder.build().and_then(|c| c.collect()).map_err(Error::Source)?;

    let mut r = Resolver { lookup, secrets: BTreeSet::new(), secret_values: Vec::new(), problems: Vec::new() };
    let tree = r.table(tree, "");
    if !r.problems.is_empty() {
        r.problems.sort();
        return Err(Error::Env(r.problems));
    }

    let mut shown = Vec::new();
    flatten(&tree, "", &r.secrets, &mut shown);
    shown.sort_by(|a: &Entry, b: &Entry| a.path.cmp(&b.path));
    let value = Value::new(None, ValueKind::Table(tree)).try_deserialize::<T>().map_err(|e| {
        let mut msg = e.to_string();
        for s in &r.secret_values {
            msg = msg.replace(s.expose_secret(), MASK);
        }
        Error::Shape(msg)
    })?;
    Ok(Loaded { value, listing: Listing(shown) })
}

/// The one file named `<name>.<ext>`; an error when several formats exist,
/// since config-rs would silently pick one.
fn find(dir: &Path, name: &str) -> Result<Option<PathBuf>, Error> {
    let found: Vec<PathBuf> = EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).filter(|p| p.is_file()).collect();
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        many => Err(Error::Files(format!("{name} has several files: {}", many.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")))),
    }
}

/// A parsed `env(...)` form.
#[derive(Debug, PartialEq)]
struct Call<'a> {
    required: bool,
    secret: bool,
    var: &'a str,
    default: Option<&'a str>,
}

/// `None`: a plain string. `Some(Err)`: starts like a form but is malformed,
/// so a typo fails instead of becoming a literal.
fn parse(s: &str) -> Option<Result<Call<'_>, String>> {
    const FORMS: [(&str, bool, bool); 4] =
        [("required_secret_env(", true, true), ("required_env(", true, false), ("secret_env(", false, true), ("env(", false, false)];
    let (prefix, required, secret) = FORMS.into_iter().find(|(p, ..)| s.starts_with(p))?;
    let name = &prefix[..prefix.len() - 1];
    let Some(inner) = s[prefix.len()..].strip_suffix(')') else {
        return Some(Err(format!("`{s}`: missing closing `)`")));
    };
    let (var, default) = match inner.split_once(',') {
        Some((v, d)) => (v.trim(), Some(d.trim())),
        None => (inner.trim(), None),
    };
    let valid = !var.is_empty() && !var.starts_with(|c: char| c.is_ascii_digit()) && var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !valid {
        return Some(Err(format!("`{s}`: `{var}` is not an env var name")));
    }
    if required && default.is_some() {
        return Some(Err(format!("`{s}`: {name} takes no default; use {} for one", &name["required_".len()..])));
    }
    Some(Ok(Call { required, secret, var, default }))
}

struct Resolver<'a> {
    lookup: Lookup<'a>,
    secrets: BTreeSet<String>,
    secret_values: Vec<SecretString>,
    problems: Vec<String>,
}

impl Resolver<'_> {
    fn table(&mut self, t: Map<String, Value>, path: &str) -> Map<String, Value> {
        t.into_iter()
            .filter_map(|(k, v)| {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                self.value(v, &p).map(|v| (k, v))
            })
            .collect()
    }

    /// `None`: drop the key (an optional env var that is unset).
    fn value(&mut self, v: Value, path: &str) -> Option<Value> {
        let origin = v.origin().map(str::to_owned);
        let kind = match v.kind {
            ValueKind::Table(t) => ValueKind::Table(self.table(t, path)),
            ValueKind::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for item in a {
                    if let Some(item) = self.value(item, &format!("{path}[{}]", out.len())) {
                        out.push(item);
                    }
                }
                ValueKind::Array(out)
            }
            ValueKind::String(s) => match parse(&s) {
                None => ValueKind::String(s),
                Some(Err(e)) => {
                    self.problems.push(format!("{path}: {e}"));
                    return None;
                }
                Some(Ok(call)) => {
                    let found = match (self.lookup)(call.var) {
                        Ok(v) => v.or_else(|| call.default.map(str::to_owned)),
                        Err(e) => {
                            self.problems.push(format!("{path}: {e}"));
                            return None;
                        }
                    };
                    let Some(found) = found else {
                        if call.required {
                            self.problems.push(format!("{path}: {} is not set (`{s}`)", call.var));
                        }
                        return None;
                    };
                    if call.secret {
                        self.secrets.insert(path.to_owned());
                        self.secret_values.push(SecretString::from(found.clone()));
                    }
                    ValueKind::String(found)
                }
            },
            other => other,
        };
        Some(Value::new(origin.as_ref(), kind))
    }
}

fn flatten(t: &Map<String, Value>, path: &str, secrets: &BTreeSet<String>, out: &mut Vec<Entry>) {
    for (k, v) in t {
        let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
        leaf(v, &p, looks_secret(k), secrets, out);
    }
}

fn leaf(v: &Value, path: &str, named_secret: bool, secrets: &BTreeSet<String>, out: &mut Vec<Entry>) {
    let plain = |shown: String| Entry { path: path.to_owned(), shown, secret: None };
    let redacted = |shown: String, real: String| Entry { path: path.to_owned(), shown, secret: Some(SecretString::from(real)) };
    match &v.kind {
        ValueKind::Table(t) => flatten(t, path, secrets, out),
        ValueKind::Array(a) => {
            for (i, item) in a.iter().enumerate() {
                leaf(item, &format!("{path}[{i}]"), named_secret, secrets, out);
            }
        }
        _ if named_secret || secrets.contains(path) => out.push(redacted(MASK.to_owned(), v.to_string())),
        ValueKind::String(s) => match mask_url_password(s) {
            masked if masked != *s => out.push(redacted(masked, s.clone())),
            _ => out.push(plain(s.clone())),
        },
        ValueKind::Nil => out.push(plain("null".to_owned())),
        _ => out.push(plain(v.to_string())),
    }
}

/// Backstop for secrets written as literals in a file.
fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["key", "secret", "password", "token"].iter().any(|w| k.split('_').any(|part| part == *w))
}

/// `scheme://user:password@host/db` -> `scheme://user:<redacted>@host/db`.
fn mask_url_password(url: &str) -> String {
    let Some(scheme) = url.find("://") else { return url.to_owned() };
    let rest = &url[scheme + 3..];
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let Some(at) = authority.rfind('@') else { return url.to_owned() };
    match authority[..at].split_once(':') {
        Some((user, _)) => format!("{}{user}:{MASK}{}", &url[..scheme + 3], &rest[at..]),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde::Deserialize;

    use super::*;

    fn dir(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("configured-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (name, body) in files {
            std::fs::write(d.join(name), body).unwrap();
        }
        d
    }

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Result<Option<String>, String> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| Ok(vars.get(k).filter(|v| !v.is_empty()).cloned())
    }

    #[derive(Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    struct App {
        name: String,
        #[serde(default)]
        port: Option<u16>,
        debug: bool,
        key: SecretString,
        #[serde(default)]
        hosts: Vec<String>,
        #[serde(default)]
        nested: Nested,
    }

    #[derive(Deserialize, Debug, Default)]
    #[serde(default, deny_unknown_fields)]
    struct Nested {
        token: Option<SecretString>,
        #[serde(with = "crate::duration")]
        every: std::time::Duration,
    }

    const BASE: &str = r#"
name = "env(APP_NAME, demo)"
port = "env(PORT)"
debug = "env(DEBUG, false)"
key = "required_secret_env(APP_KEY)"
hosts = ["a", "env(EXTRA_HOST)", "b"]
"#;

    #[test]
    fn resolves_defaults_types_and_secrets() {
        let d = dir(&[("default.toml", BASE)]);
        let l = load_from::<App>(&d, None, &env(&[("APP_KEY", "s3cr3t"), ("PORT", "8080"), ("DEBUG", "true")])).unwrap();
        assert_eq!(l.value.name, "demo");
        assert_eq!(l.value.port, Some(8080));
        assert!(l.value.debug);
        assert_eq!(l.value.key.expose_secret(), "s3cr3t");
        assert_eq!(l.value.hosts, ["a", "b"], "unset optional array items are dropped");
        assert!(!l.listing.show().contains("s3cr3t"), "{}", l.listing.show());
        assert!(l.listing.show().lines().any(|line| line.starts_with("key ") && line.ends_with("<redacted>")), "{}", l.listing.show());
        assert!(l.listing.show_secret_values().lines().any(|line| line.starts_with("key ") && line.ends_with("s3cr3t")));
    }

    #[test]
    fn unset_optional_means_default_not_empty() {
        let d = dir(&[("default.toml", BASE)]);
        let l = load_from::<App>(&d, None, &env(&[("APP_KEY", "k"), ("PORT", "")])).unwrap();
        assert_eq!(l.value.port, None);
        assert!(!l.value.debug);
    }

    #[test]
    fn reports_every_missing_required_var_at_once() {
        let d = dir(&[("default.toml", "name = \"required_env(APP_NAME)\"\ndebug = false\nkey = \"required_secret_env(APP_KEY)\"\n")]);
        let err = load_from::<App>(&d, None, &env(&[("APP_NAME", "")])).err().unwrap();
        let Error::Env(problems) = &err else { panic!("{err}") };
        assert_eq!(problems.len(), 2, "{err}");
        assert!(err.to_string().contains("APP_KEY") && err.to_string().contains("APP_NAME"), "{err}");
    }

    #[test]
    fn malformed_forms_fail_instead_of_becoming_literals() {
        for bad in ["env(APP_KEY", "required_env(APP_KEY, x)", "env(1BAD)", "secret_env()", "env(A-B)"] {
            assert!(matches!(parse(bad), Some(Err(_))), "{bad}");
        }
        for plain in ["environment", "my env(X)", "", "envy"] {
            assert!(parse(plain).is_none(), "{plain}");
        }
        assert_eq!(parse("env(X, a, b)"), Some(Ok(Call { required: false, secret: false, var: "X", default: Some("a, b") })));
    }

    #[test]
    fn env_file_overrides_and_app_env_is_checked() {
        let d = dir(&[("default.toml", BASE), ("production.toml", "debug = true\n[nested]\nevery = \"2h\"\ntoken = \"secret_env(TOKEN)\"\n")]);
        let vars = env(&[("APP_KEY", "k"), ("TOKEN", "t0k")]);
        let l = load_from::<App>(&d, Some("production"), &vars).unwrap();
        assert!(l.value.debug);
        assert_eq!(l.value.nested.every, std::time::Duration::from_secs(7200));
        assert!(!l.listing.show().contains("t0k"));
        assert!(load_from::<App>(&d, Some("staging"), &vars).is_ok(), "missing env file is fine");
        for bad in ["../etc", "default", "a/b"] {
            assert!(matches!(load_from::<App>(&d, Some(bad), &vars), Err(Error::Files(_))), "{bad}");
        }
    }

    #[test]
    fn missing_default_and_duplicate_formats_fail() {
        assert!(matches!(load_from::<App>(&dir(&[]), None, &env(&[])), Err(Error::Files(_))));
        let d = dir(&[("default.toml", BASE), ("default.yaml", "name: x\n")]);
        assert!(matches!(load_from::<App>(&d, None, &env(&[("APP_KEY", "k")])), Err(Error::Files(_))));
    }

    #[test]
    fn shape_errors_never_print_secrets() {
        #[derive(Deserialize, Debug)]
        struct Port {
            #[allow(dead_code)]
            port: u16,
        }
        let d = dir(&[("default.toml", "port = \"required_secret_env(PORT)\"\n")]);
        let err = load_from::<Port>(&d, None, &env(&[("PORT", "hunter2")])).err().unwrap();
        assert!(matches!(err, Error::Shape(_)), "{err}");
        assert!(!err.to_string().contains("hunter2"), "{err}");
    }

    #[test]
    fn unknown_keys_fail_and_yaml_works() {
        let d = dir(&[("default.yaml", "name: x\ndebug: false\nkey: \"required_secret_env(APP_KEY)\"\nnmae: typo\n")]);
        assert!(matches!(load_from::<App>(&d, None, &env(&[("APP_KEY", "k")])), Err(Error::Shape(_))));
    }

    #[test]
    fn url_passwords_and_named_keys_are_redacted_until_asked() {
        let d = dir(&[("default.toml", "name = \"postgres://u:pw@h/db\"\ndebug = false\nkey = \"literal-in-file\"\n")]);
        let l = load_from::<App>(&d, None, &env(&[])).unwrap();
        let (shown, revealed) = (l.listing.show(), l.listing.show_secret_values());
        assert!(!shown.contains("pw@") && !shown.contains("literal-in-file"), "{shown}");
        assert!(shown.contains("postgres://u:<redacted>@h/db"), "{shown}");
        assert!(revealed.contains("postgres://u:pw@h/db") && revealed.contains("literal-in-file"), "{revealed}");
    }

    #[test]
    fn masks() {
        assert_eq!(mask_url_password("postgres://u:pw@h/db"), "postgres://u:<redacted>@h/db");
        assert_eq!(mask_url_password("postgres://h/db?x=a@b"), "postgres://h/db?x=a@b");
        assert_eq!(mask_url_password("postgres://u@h/db"), "postgres://u@h/db");
        assert!(looks_secret("app_key") && looks_secret("client_secret") && looks_secret("API_TOKEN"));
        assert!(!looks_secret("max_connections") && !looks_secret("monkey"));
    }
}
