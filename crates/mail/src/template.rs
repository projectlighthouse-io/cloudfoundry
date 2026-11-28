//! Markdown-ish mail templates, rendered to a text and an HTML body.

use crate::{Error, Message};

/// A mail template: a `Subject:` line, a blank line, then the body.
///
/// ```text
/// Subject: Verify your email address
///
/// # Confirm your address
///
/// Hi, confirm that **{{email}}** is yours.
///
/// [Verify email address]({{link}})
///
/// - The link works for an hour.
/// - Ignore this mail if you did not sign up.
/// ```
///
/// Blocks are separated by blank lines: `# ` / `## ` headings, `- ` lists,
/// paragraphs. A paragraph that is only a link becomes a button. Inline:
/// `**bold**`, `` `code` `` and `[text](url)`. `{{name}}` is replaced by a
/// value after rendering (HTML-escaped in the HTML body), so values never
/// turn into markup.
#[derive(Clone, Debug)]
pub struct Template {
    subject: String,
    text: String,
    html: String,
}

impl Template {
    pub fn parse(source: &str) -> Result<Self, Error> {
        let source = source.replace("\r\n", "\n");
        let (head, body) = source.split_once("\n\n").unwrap_or((&source, ""));
        let subject = head
            .lines()
            .find_map(|l| l.strip_prefix("Subject:"))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Template("the first line must be `Subject: ...`".into()))?;
        let blocks: Vec<Vec<&str>> = body.split("\n\n").map(|b| b.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect()).filter(|b: &Vec<&str>| !b.is_empty()).collect();
        let text = blocks.iter().map(|b| text_block(b)).collect::<Vec<_>>().join("\n\n");
        let html = blocks.iter().map(|b| html_block(b)).collect::<String>();
        Ok(Self { subject, text, html: format!("{HEAD}{html}{TAIL}") })
    }

    /// The mail for `to`, with each `{{name}}` replaced. A placeholder without
    /// a value is an error, so a typo never reaches a user.
    pub fn message(&self, to: &str, vars: &[(&str, &str)]) -> Result<Message, Error> {
        let raw = |s: &str| fill(s, vars, |v| v.to_owned());
        Ok(Message::new(to, raw(&self.subject)?, raw(&self.text)?).html(fill(&self.html, vars, escape)?))
    }
}

const HEAD: &str = r#"<!doctype html><html><body style="margin:0;padding:24px;background:#f5f5f4;font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;color:#1c1917"><div style="max-width:560px;margin:0 auto;background:#ffffff;border-radius:8px;padding:32px;line-height:1.55;font-size:15px">"#;
const TAIL: &str = "</div></body></html>";

fn fill(s: &str, vars: &[(&str, &str)], value: impl Fn(&str) -> String) -> Result<String, Error> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else { break };
        let name = rest[start + 2..start + 2 + len].trim();
        let v = vars.iter().find(|(k, _)| *k == name).ok_or_else(|| Error::Template(format!("no value for {{{{{name}}}}}")))?;
        out.push_str(&rest[..start]);
        out.push_str(&value(v.1));
        rest = &rest[start + 2 + len + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `[label](url)` alone on its block.
fn lone_link<'a>(block: &[&'a str]) -> Option<(&'a str, &'a str)> {
    let [line] = block else { return None };
    let (label, rest) = line.trim().strip_prefix('[')?.split_once("](")?;
    let url = rest.strip_suffix(')')?;
    (!label.contains(']') && !url.contains(' ')).then_some((label, url))
}

fn heading<'a>(block: &[&'a str]) -> Option<(u8, &'a str)> {
    let [line] = block else { return None };
    line.strip_prefix("## ").map(|t| (2, t)).or_else(|| line.strip_prefix("# ").map(|t| (1, t)))
}

fn list<'a>(block: &[&'a str]) -> Option<Vec<&'a str>> {
    block.iter().map(|l| l.trim_start().strip_prefix("- ")).collect()
}

fn text_block(block: &[&str]) -> String {
    if let Some((label, url)) = lone_link(block) {
        return format!("{label}: {url}");
    }
    if let Some((_, t)) = heading(block) {
        return inline_text(t);
    }
    if let Some(items) = list(block) {
        return items.iter().map(|i| format!("- {}", inline_text(i))).collect::<Vec<_>>().join("\n");
    }
    block.iter().map(|l| inline_text(l)).collect::<Vec<_>>().join("\n")
}

fn html_block(block: &[&str]) -> String {
    if let Some((label, url)) = lone_link(block) {
        return format!(
            r#"<p style="margin:24px 0"><a href="{}" style="display:inline-block;background:#1c1917;color:#ffffff;text-decoration:none;padding:10px 18px;border-radius:6px;font-weight:600">{}</a></p>"#,
            escape(url),
            inline_html(label)
        );
    }
    if let Some((level, t)) = heading(block) {
        let size = if level == 1 { 22 } else { 17 };
        return format!(r#"<h{level} style="margin:0 0 16px;font-size:{size}px">{}</h{level}>"#, inline_html(t));
    }
    if let Some(items) = list(block) {
        let li: String = items.iter().map(|i| format!("<li>{}</li>", inline_html(i))).collect();
        return format!(r#"<ul style="margin:0 0 16px;padding-left:20px">{li}</ul>"#);
    }
    let lines: Vec<String> = block.iter().map(|l| inline_html(l)).collect();
    format!(r#"<p style="margin:0 0 16px">{}</p>"#, lines.join("<br>"))
}

/// Inline spans: (kind, inner, url) found left to right.
enum Span<'a> {
    Plain(&'a str),
    Bold(&'a str),
    Code(&'a str),
    Link(&'a str, &'a str),
}

fn spans(mut s: &str) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    loop {
        let next = [s.find("**"), s.find('`'), s.find('[')].into_iter().flatten().min();
        let Some(at) = next else { break };
        let (before, rest) = s.split_at(at);
        let parsed = if let Some(r) = rest.strip_prefix("**") {
            r.split_once("**").map(|(inner, after)| (Span::Bold(inner), after))
        } else if let Some(r) = rest.strip_prefix('`') {
            r.split_once('`').map(|(inner, after)| (Span::Code(inner), after))
        } else {
            rest[1..].split_once("](").and_then(|(label, r)| r.split_once(')').map(|(url, after)| (Span::Link(label, url), after)))
        };
        if let Some((span, after)) = parsed {
            if !before.is_empty() {
                out.push(Span::Plain(before));
            }
            out.push(span);
            s = after;
        } else {
            // Not markup after all: keep the marker as text.
            let cut = at + rest.chars().next().map_or(1, char::len_utf8);
            out.push(Span::Plain(&s[..cut]));
            s = &s[cut..];
        }
    }
    if !s.is_empty() {
        out.push(Span::Plain(s));
    }
    out
}

fn inline_text(s: &str) -> String {
    spans(s)
        .into_iter()
        .map(|sp| match sp {
            Span::Plain(t) | Span::Bold(t) | Span::Code(t) => t.to_owned(),
            Span::Link(label, url) => format!("{label} ({url})"),
        })
        .collect()
}

fn inline_html(s: &str) -> String {
    spans(s)
        .into_iter()
        .map(|sp| match sp {
            Span::Plain(t) => escape(t),
            Span::Bold(t) => format!("<strong>{}</strong>", escape(t)),
            Span::Code(t) => format!(r#"<code style="font-family:ui-monospace,Menlo,monospace">{}</code>"#, escape(t)),
            Span::Link(label, url) => format!(r#"<a href="{}" style="color:#1c1917">{}</a>"#, escape(url), escape(label)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const SRC: &str = "Subject: Verify {{what}}\n\n# Hi {{name}}\n\nConfirm **{{email}}** is yours, see [the docs](https://acme.test/docs).\nSecond line with `code`.\n\n[Verify email address]({{link}})\n\n- one hour\n- or ignore it\n";

    #[test]
    fn renders_text_and_html() {
        let t = Template::parse(SRC).unwrap();
        let vars = [("what", "your email"), ("name", "<Ada>"), ("email", "ada@acme.test"), ("link", "https://acme.test/v?a=1&b=2")];
        let m = t.message("ada@acme.test", &vars).unwrap();
        assert_eq!(m.subject, "Verify your email");
        assert_eq!(
            m.text,
            "Hi <Ada>\n\nConfirm ada@acme.test is yours, see the docs (https://acme.test/docs).\nSecond line with code.\n\nVerify email address: https://acme.test/v?a=1&b=2\n\n- one hour\n- or ignore it"
        );
        let html = m.html.unwrap();
        assert!(html.contains("<h1 style=\"margin:0 0 16px;font-size:22px\">Hi &lt;Ada&gt;</h1>"), "values are escaped: {html}");
        assert!(html.contains("<strong>ada@acme.test</strong>"));
        assert!(html.contains(r#"href="https://acme.test/v?a=1&amp;b=2""#), "button link");
        assert!(html.contains("<br>Second line with <code"));
        assert!(html.contains("<li>one hour</li><li>or ignore it</li>"));
    }

    #[test]
    fn values_never_become_markup() {
        let t = Template::parse("Subject: x\n\nHello {{name}}").unwrap();
        let m = t.message("a@b.c", &[("name", "**[x](javascript:1)**")]).unwrap();
        assert_eq!(m.text, "Hello **[x](javascript:1)**");
        assert!(!m.html.unwrap().contains("<a"), "no link from a value");
    }

    #[test]
    fn needs_a_subject_and_every_value() {
        assert!(matches!(Template::parse("Hello"), Err(Error::Template(_))));
        let t = Template::parse("Subject: x\n\n{{link}}").unwrap();
        assert!(matches!(t.message("a@b.c", &[]), Err(Error::Template(_))));
    }

    #[test]
    fn stray_markers_stay_text() {
        assert_eq!(inline_text("2 ** 3 and [not a link] and `tick"), "2 ** 3 and [not a link] and `tick");
    }
}
