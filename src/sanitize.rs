//! Balloon HTML goes straight into a page that gets published, and it came out
//! of a file this tool did not write. Rather than trusting it, the markup is
//! re-emitted from an allowlist: known tags keep known attributes, anything
//! else loses its tag (text is kept), and script-like elements lose their
//! contents too.

const KEEP: &[&str] = &[
    "a", "b", "br", "code", "div", "em", "h1", "h2", "h3", "h4", "hr", "i", "img", "li", "ol",
    "p", "pre", "small", "span", "strong", "sub", "sup", "table", "tbody", "td", "th", "thead",
    "tr", "u", "ul",
];
/// Dropped along with everything they contain.
const DROP_SUBTREE: &[&str] = &["script", "style", "iframe", "object", "embed", "svg", "math"];
const VOID: &[&str] = &["br", "hr", "img"];

fn attr_ok(tag: &str, attr: &str) -> bool {
    match attr {
        "title" | "alt" => true,
        "href" => tag == "a",
        "src" | "width" | "height" => tag == "img",
        "colspan" | "rowspan" => tag == "td" || tag == "th",
        _ => false,
    }
}

/// Relative paths and plain web URLs only — no `javascript:`, no smuggled data.
fn url_ok(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    match v.split_once(':') {
        None => true,
        Some((scheme, _)) if scheme.contains('/') || scheme.contains('?') => true,
        Some((scheme, _)) => {
            matches!(scheme, "http" | "https" | "mailto") || v.starts_with("data:image/")
        }
    }
}

pub fn scrub(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = input[i..].find('<').map(|p| i + p).unwrap_or(bytes.len());
            out.push_str(&input[i..next]);
            i = next;
            continue;
        }
        if input[i..].starts_with("<!--") {
            i = input[i..].find("-->").map(|p| i + p + 3).unwrap_or(bytes.len());
            continue;
        }
        let Some(end) = input[i..].find('>').map(|p| i + p) else {
            // An unterminated '<' is text, not a tag.
            out.push_str("&lt;");
            i += 1;
            continue;
        };
        let inner = &input[i + 1..end];
        let closing = inner.starts_with('/');
        let name: String = inner
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        i = end + 1;

        if name.is_empty() {
            continue;
        }
        if DROP_SUBTREE.contains(&name.as_str()) {
            if !closing {
                let close = format!("</{name}");
                if let Some(p) = input[i..].to_ascii_lowercase().find(&close) {
                    i = input[i + p..].find('>').map(|e| i + p + e + 1).unwrap_or(bytes.len());
                } else {
                    i = bytes.len();
                }
            }
            continue;
        }
        if !KEEP.contains(&name.as_str()) {
            continue;
        }
        if closing {
            if !VOID.contains(&name.as_str()) {
                out.push_str(&format!("</{name}>"));
            }
            continue;
        }
        out.push('<');
        out.push_str(&name);
        for (attr, value) in attrs(inner) {
            if !attr_ok(&name, &attr) {
                continue;
            }
            if (attr == "href" || attr == "src") && !url_ok(&value) {
                continue;
            }
            out.push_str(&format!(" {attr}=\"{}\"", esc(&value)));
        }
        if VOID.contains(&name.as_str()) {
            out.push_str("/>");
        } else {
            out.push('>');
        }
    }
    out
}

/// Attribute pairs of a start tag. Bare attributes are reported with an empty
/// value; neither form survives the allowlist unless it is named there.
fn attrs(inner: &str) -> Vec<(String, String)> {
    let bytes = inner.as_bytes();
    let mut i = inner
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(inner.len());
    let mut out = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        if start == i {
            break;
        }
        let attr = inner[start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            out.push((attr, String::new()));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = match bytes.get(i) {
            Some(&q @ (b'"' | b'\'')) => {
                let open = i + 1;
                let close = inner[open..].find(q as char).map(|p| open + p).unwrap_or(inner.len());
                i = (close + 1).min(inner.len());
                inner[open..close].to_string()
            }
            _ => {
                let start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                inner[start..i].to_string()
            }
        };
        out.push((attr, unescape(&value)));
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Escape for both text nodes and quoted attribute values.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_platescan_balloon() {
        let balloon = r#"<img src="images/a.jpg" style="max-width:420px"/><br/><b>ABC123</b> — clip<br/><i>Model output</i>"#;
        let out = scrub(balloon);
        assert!(out.contains(r#"<img src="images/a.jpg"/>"#), "{out}");
        assert!(out.contains("<b>ABC123</b>"));
        assert!(!out.contains("style"), "unlisted attributes go: {out}");
    }

    #[test]
    fn drops_script_and_its_contents() {
        let out = scrub("before<script>alert(1)</script>after");
        assert_eq!(out, "beforeafter");
    }

    #[test]
    fn strips_handlers_and_javascript_urls() {
        let out = scrub(r#"<a href="javascript:alert(1)" onclick="alert(2)">x</a>"#);
        assert_eq!(out, "<a>x</a>");
        let ok = scrub(r#"<a href="https://example.org/p?a=1">x</a>"#);
        assert_eq!(ok, r#"<a href="https://example.org/p?a=1">x</a>"#);
    }

    #[test]
    fn unlisted_tags_lose_the_tag_but_keep_the_text() {
        assert_eq!(scrub("<blink>still here</blink>"), "still here");
    }

    #[test]
    fn a_stray_bracket_is_text() {
        assert_eq!(scrub("5 < 6"), "5 &lt; 6");
    }

    #[test]
    fn relative_urls_pass_and_odd_schemes_do_not() {
        assert!(url_ok("images/a.jpg"));
        assert!(url_ok("./a.jpg?v=2"));
        assert!(url_ok("data:image/png;base64,AAA"));
        assert!(!url_ok("javascript:alert(1)"));
        assert!(!url_ok("data:text/html,<b>"));
    }
}
