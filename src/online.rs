//! Background lookups: a guide's page title, and each site's favicon (cached on disk), so the
//! list shows the same little icons as your browser's bookmarks bar.

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use crate::links;

pub enum Job {
    Title { game: u64, guide: u64, url: String },
    Favicon { host: String },
    /// The readable text of a guide page, for turning into a checklist.
    PageText { guide: u64, url: String },
}

pub enum Done {
    Title { game: u64, guide: u64, title: Option<String> },
    Favicon { host: String, image: Option<egui::ColorImage> },
    PageText { guide: u64, text: Result<String, String> },
}

pub struct Online {
    tx: Sender<Job>,
    pub rx: Receiver<Done>,
}

impl Online {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel();
        std::thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) JAGT/0.1")
                .build();
            for job in jobs {
                let result = match job {
                    Job::Title { game, guide, url } => Done::Title { game, guide, title: fetch_title(&agent, &url) },
                    Job::Favicon { host } => {
                        let image = favicon(&agent, &host);
                        Done::Favicon { host, image }
                    }
                    Job::PageText { guide, url } => Done::PageText { guide, text: page_text(&agent, &url) },
                };
                if done.send(result).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Online { tx, rx }
    }

    pub fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }
}

pub fn cache_dir() -> PathBuf {
    crate::model::data_dir().join("cache").join("favicons")
}

pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(cache_dir());
}

fn favicon(agent: &ureq::Agent, host: &str) -> Option<egui::ColorImage> {
    let safe: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || ".-".contains(*c)).collect();
    if safe.is_empty() {
        return None;
    }
    let path = cache_dir().join(format!("{safe}.png"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let url = format!("https://www.google.com/s2/favicons?domain={safe}&sz=64");
            let resp = agent.get(&url).call().ok()?;
            let mut b = Vec::new();
            resp.into_reader().take(1 << 20).read_to_end(&mut b).ok()?;
            let _ = std::fs::create_dir_all(cache_dir());
            let _ = std::fs::write(&path, &b);
            b
        }
    };
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

fn fetch_title(agent: &ureq::Agent, url: &str) -> Option<String> {
    if links::is_youtube(url) {
        // oEmbed gives the real title without scraping YouTube's page.
        let api = format!("https://www.youtube.com/oembed?format=json&url={}", links::percent_encode(url));
        let v: serde_json::Value = agent.get(&api).call().ok()?.into_json().ok()?;
        let title = v.get("title")?.as_str()?.trim().to_string();
        let author = v.get("author_name").and_then(|a| a.as_str()).unwrap_or("").trim();
        return Some(if author.is_empty() { title } else { format!("{title} \u{2014} {author}") });
    }
    let resp = agent.get(url).call().ok()?;
    let mut buf = Vec::new();
    resp.into_reader().take(768 * 1024).read_to_end(&mut buf).ok()?;
    let html = String::from_utf8_lossy(&buf);
    let title = meta_content(&html, "og:title").or_else(|| title_tag(&html))?;
    let title = clean(&decode_entities(&title));
    // Pages that need a login come back as e.g. "Google Sheets: Sign-in"; not worth keeping.
    let lower = title.to_lowercase();
    if title.is_empty() || lower.contains("sign-in") || lower.contains("sign in") || lower == "google sheets" || lower == "just a moment..." {
        None
    } else {
        Some(title)
    }
}

fn page_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(401 | 403 | 429, _) => {
            "This site blocks automatic reading. Select the guide's text in your browser, copy it, and paste it here instead.".to_string()
        }
        ureq::Error::Status(code, _) => format!("The site answered with error {code}."),
        other => format!("Couldn't reach the page: {other}"),
    })?;
    let mut buf = Vec::new();
    resp.into_reader().take(8 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = html_to_text(&String::from_utf8_lossy(&buf));
    if text.trim().is_empty() {
        Err("The page had no readable text (it may need a login or JavaScript). Copy and paste the text instead.".into())
    } else {
        Ok(text)
    }
}

/// Removes every `<tag ...>...</tag>` block (scripts, styles...).
fn strip_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&open) {
        let start = at + i;
        // `<nav` must not match `<navigation-thing`: the next char has to end the tag name
        let after = lower.as_bytes().get(start + open.len()).copied().unwrap_or(b'>');
        if after.is_ascii_alphanumeric() || after == b'-' {
            out.push_str(&html[at..start + open.len()]);
            at = start + open.len();
            continue;
        }
        out.push_str(&html[at..start]);
        match lower[start..].find(&close) {
            Some(j) => at = start + j + close.len(),
            None => {
                at = html.len();
                break;
            }
        }
    }
    out.push_str(&html[at.min(html.len())..]);
    out
}

/// The part of the page that holds the guide: Steam guide sections, `<article>`, `<main>`, or
/// the whole body.
fn main_region(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    if let Some(i) = lower.find("class=\"guide subsections\"") {
        let end = lower[i..].find("commentthread").map_or(html.len(), |j| i + j);
        return &html[i..end];
    }
    for tag in ["article", "main"] {
        if let (Some(i), Some(j)) = (lower.find(&format!("<{tag}")), lower.rfind(&format!("</{tag}>"))) {
            if j > i {
                return &html[i..j];
            }
        }
    }
    match lower.find("<body") {
        Some(i) => &html[i..],
        None => html,
    }
}

/// Readable text from HTML: one block per line, headings as `## Heading`, list items as `- item`.
pub fn html_to_text(html: &str) -> String {
    let mut h = main_region(html).to_string();
    for tag in ["script", "style", "noscript", "svg", "nav", "footer", "form", "iframe", "button", "select"] {
        h = strip_blocks(&h, tag);
    }
    let mut out = String::with_capacity(h.len() / 2);
    let mut rest = h.as_str();
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else { break };
        let tag = rest[i + 1..i + j].trim().to_ascii_lowercase();
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        let heading = matches!(name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
        if heading && !closing {
            out.push_str("\n## ");
        } else if name == "li" && !closing {
            out.push_str("\n- ");
        } else if heading
            || matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "ul" | "ol" | "table" | "blockquote" | "section" | "dt" | "dd")
        {
            out.push('\n');
        } else if closing && matches!(name.as_str(), "td" | "th") {
            out.push_str(" | ");
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let text = decode_entities(&out);
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = clean(l);
        let l = l.trim_end_matches(" |").trim().to_string();
        if l.is_empty() || l == "##" || l == "-" {
            continue;
        }
        lines.push(l);
    }
    lines.join("\n")
}

fn title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = start + lower[start..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</title")?;
    Some(html[open_end..close].to_string())
}

fn meta_content(html: &str, prop: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("\"{prop}\"");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&needle) {
        let at = from + i;
        let tag_start = lower[..at].rfind('<')?;
        let tag_end = at + lower[at..].find('>')?;
        let tag = &html[tag_start..tag_end];
        let tl = &lower[tag_start..tag_end];
        if tl.starts_with("<meta") {
            if let Some(c) = tl.find("content=") {
                let rest = &tag[c + 8..];
                let q = rest.chars().next()?;
                if q == '"' || q == '\'' {
                    let body = &rest[1..];
                    return body.find(q).map(|e| body[..e].to_string());
                }
            }
        }
        from = at + needle.len();
    }
    None
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // look for the ';' within the next 12 bytes, without cutting a multi-byte character
        let mut window = rest.len().min(12);
        while !rest.is_char_boundary(window) {
            window -= 1;
        }
        let Some(end) = rest[..window].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('\u{2013}'),
            "mdash" => Some('\u{2014}'),
            "hellip" => Some('\u{2026}'),
            "rsaquo" | "raquo" => Some('>'),
            "lsaquo" | "laquo" => Some('<'),
            "copy" => Some('\u{a9}'),
            "trade" => Some('\u{2122}'),
            "reg" => Some('\u{ae}'),
            "bull" | "middot" => Some('\u{b7}'),
            "rsquo" | "lsquo" => Some('\''),
            "ldquo" | "rdquo" => Some('"'),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_next_to_non_ascii() {
        // a bare '&' followed by curly quotes used to cut a character in half and crash
        assert_eq!(decode_entities("Tom & Jerry\u{2019}s \u{2019}\u{2019}\u{2019}\u{2019}"), "Tom & Jerry\u{2019}s \u{2019}\u{2019}\u{2019}\u{2019}");
        assert_eq!(decode_entities("&\u{2019}\u{2019}\u{2019}\u{2019}\u{2019}"), "&\u{2019}\u{2019}\u{2019}\u{2019}\u{2019}");
        assert_eq!(decode_entities("Rock &amp; Roll\u{2019}s"), "Rock & Roll\u{2019}s");
    }

    #[test]
    fn page_to_text() {
        let html = r#"<html><body><nav>Menu</nav><article><h2>Castle Town</h2><script>var x=1;</script>
            <p>Chest 1 - Potion; on the road.<br>&#9989;Objective 14: Frozen Rail Ride</p>
            <ul><li>First</li><li>Second &amp; third</li></ul></article><footer>bye</footer></body></html>"#;
        let t = html_to_text(html);
        assert_eq!(t, "## Castle Town\nChest 1 - Potion; on the road.\n\u{2705}Objective 14: Frozen Rail Ride\n- First\n- Second & third");
    }

    #[test]
    fn titles() {
        let html = r#"<html><head><TITLE> Elden Ring &amp; DLC  Guide &#8211; Part 1 </TITLE><meta property="og:title" content="Elden Ring Walkthrough"></head>"#;
        assert_eq!(meta_content(html, "og:title").as_deref(), Some("Elden Ring Walkthrough"));
        assert_eq!(clean(&decode_entities(&title_tag(html).unwrap())), "Elden Ring & DLC Guide \u{2013} Part 1");
    }
}
