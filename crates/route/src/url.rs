use std::{collections::HashMap, fmt};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Route name to path template, e.g. `runs.show` to `/runs/{id}`.
pub(crate) struct Urls(pub(crate) HashMap<String, String>);

impl Urls {
    pub(crate) fn url(&self, name: &str, params: &[(&str, &str)]) -> Result<String, UrlError> {
        let template = self.0.get(name).ok_or_else(|| UrlError::UnknownName(name.to_owned()))?;
        let mut out = String::with_capacity(template.len());
        let mut keys = vec![];
        let mut rest = template.as_str();

        // ponytail: ignores matchit's `{{`/`}}` escapes; add if a route needs a literal brace.
        while let Some(open) = rest.find('{') {
            // matchit validated the template, so every `{` has its `}`.
            let Some(len) = rest[open..].find('}') else { break };
            let close = open + len;
            out.push_str(&rest[..open]);
            let raw = &rest[open + 1..close];
            let (key, catch_all) = raw.strip_prefix('*').map_or((raw, false), |k| (k, true));
            let value = params
                .iter()
                .find(|(k, _)| *k == key)
                .ok_or_else(|| UrlError::MissingParam(key.to_owned()))?
                .1;
            if catch_all {
                let parts: Vec<String> = value.split('/').map(|s| utf8_percent_encode(s, SEGMENT).to_string()).collect();
                out.push_str(&parts.join("/"));
            } else {
                out.extend(utf8_percent_encode(value, SEGMENT));
            }
            keys.push(key);
            rest = &rest[close + 1..];
        }
        out.push_str(rest);

        match params.iter().find(|(k, _)| !keys.contains(k)) {
            Some((k, _)) => Err(UrlError::ExtraParam(k.to_string())),
            None => Ok(out),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UrlError {
    UnknownName(String),
    MissingParam(String),
    ExtraParam(String),
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownName(n) => write!(f, "no route is named `{n}`"),
            Self::MissingParam(p) => write!(f, "missing route parameter `{p}`"),
            Self::ExtraParam(p) => write!(f, "route has no parameter `{p}`"),
        }
    }
}

impl std::error::Error for UrlError {}
