//! URL helpers: what kind of guide a link is, and how to reopen it at the saved spot.
//!
//! - YouTube: `t=` is set to the saved time.
//! - Google Sheets: `#gid=…&range=A<row>` scrolls to the saved row.
//! - Everything else: a `#:~:text=` fragment scrolls the page to the saved phrase
//!   (supported by Chrome, Edge, Opera, Safari and Firefox 131+).

use crate::model::{Guide, Kind};

/// Tabs put to sleep by a tab-suspender extension wrap the real page:
/// `chrome-extension://…/suspended.html#ttl=<title>&pos=<scroll>&uri=<real url>`.
/// Returns the real URL and the page title.
pub fn unwrap_suspended(input: &str) -> Option<(String, Option<String>)> {
    let s = input.trim();
    if !["chrome-extension://", "moz-extension://", "extension://"].iter().any(|p| s.starts_with(p)) {
        return None;
    }
    let frag = s.split_once('#')?.1;
    let at = frag.find("uri=")?;
    let uri = &frag[at + 4..];
    let uri = if uri.starts_with("http") { uri.to_string() } else { percent_decode(uri) };
    let title = frag[..at]
        .split('&')
        .find_map(|p| p.strip_prefix("ttl="))
        .map(|t| percent_decode(t).trim().to_string())
        .filter(|t| !t.is_empty());
    Some((uri, title))
}

/// Every link in a blob of pasted text (separated by spaces, new lines or commas).
pub fn extract_all(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let token = token.trim_matches([',', ';']);
        if normalize(token).is_some() && !out.iter().any(|t| normalize(t) == normalize(token)) {
            out.push(token.to_string());
        }
    }
    out
}

/// Turns what was typed or pasted into a URL, or `None` if it doesn't look like one.
pub fn normalize(input: &str) -> Option<String> {
    if let Some((uri, _)) = unwrap_suspended(input) {
        return normalize(&uri);
    }
    let s = input.trim().trim_matches(|c| c == '<' || c == '>' || c == '"');
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(s.to_string());
    }
    // "www.example.com/x" or "example.com/x"
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains('.') && !host.starts_with('.') && !host.ends_with('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) {
        return Some(format!("https://{s}"));
    }
    None
}

/// "www.gamefaqs.gamespot.com" → "gamefaqs.gamespot.com"
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let h = rest.split(['/', '?', '#']).next().unwrap_or("");
    let h = h.rsplit('@').next().unwrap_or(h);
    let h = h.split(':').next().unwrap_or(h);
    let h = h.to_ascii_lowercase();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

fn path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    rest.find('/').map_or(String::new(), |i| rest[i..].to_ascii_lowercase())
}

fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

pub fn is_youtube(url: &str) -> bool {
    let h = host(url);
    host_is(&h, "youtube.com") || h == "youtu.be" || host_is(&h, "youtube-nocookie.com")
}

pub fn is_google_sheet(url: &str) -> bool {
    host(url) == "docs.google.com" && path(url).starts_with("/spreadsheets")
}

pub fn detect_kind(url: &str) -> Kind {
    let h = host(url);
    let p = path(url);
    let any = |list: &[&str]| list.iter().any(|d| host_is(&h, d));
    if is_youtube(url) || any(&["twitch.tv", "vimeo.com", "dailymotion.com", "bilibili.com", "nicovideo.jp"]) {
        Kind::Video
    } else if is_google_sheet(url)
        || any(&["airtable.com", "sheets.zoho.com"])
        || [".xlsx", ".xls", ".csv", ".ods"].iter().any(|e| p.ends_with(e))
    {
        Kind::Sheet
    } else if any(&["mapgenie.io", "game-maps.com", "interactivemap.app"]) || p.contains("/map") && any(&["ign.com", "game8.co"]) {
        Kind::Map
    } else if any(&[
        "psnprofiles.com",
        "playstationtrophies.org",
        "trueachievements.com",
        "truetrophies.com",
        "truesteamachievements.com",
        "exophase.com",
        "powerpyx.com",
        "xboxachievements.com",
        "achievementhunter.com",
        "knoef.info",
    ]) || (host_is(&h, "steamcommunity.com") && p.starts_with("/stats"))
    {
        Kind::Trophy
    } else if any(&["fandom.com", "fextralife.com", "wikia.org", "wiki.gg", "wikipedia.org", "bulbapedia.bulbagarden.net", "serebii.net", "gamerguides.com", "strategywiki.org"])
        || h.contains("wiki")
        || p.starts_with("/wiki/")
    {
        Kind::Wiki
    } else if (h == "docs.google.com" && p.starts_with("/document")) || p.ends_with(".pdf") || p.ends_with(".txt") {
        Kind::Doc
    } else {
        Kind::Web
    }
}

/// A title to show until the real one has been fetched: "gamefaqs.gamespot.com — walkthrough".
pub fn fallback_title(url: &str) -> String {
    let h = host(url);
    let last = path(url)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".html")
        .trim_end_matches(".htm")
        .replace(['-', '_'], " ");
    let last = percent_decode(&last);
    if last.is_empty() || last.len() > 60 || last.chars().all(|c| c.is_ascii_digit()) {
        h
    } else {
        format!("{h} \u{2014} {last}")
    }
}

// ---------------------------------------------------------------------------------------------
// Time stamps

/// "1:02:03" / "62:03" / "125" / "1h2m3s" / "2m" → seconds.
pub fn parse_hms(s: &str) -> Option<u32> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Some(0);
    }
    if s.contains(':') {
        let mut total = 0u32;
        for part in s.split(':') {
            total = total.checked_mul(60)?.checked_add(part.trim().parse::<u32>().ok()?)?;
        }
        return Some(total);
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok();
    }
    let s = s.strip_suffix('s').map(|x| if x.ends_with(|c: char| c.is_ascii_digit()) { format!("{x}s") } else { x.to_string() }).unwrap_or(s);
    let mut total = 0u32;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u32 = num.parse().ok()?;
        num.clear();
        total += match c {
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            _ => return None,
        };
    }
    if !num.is_empty() {
        total += num.parse::<u32>().ok()?;
    }
    Some(total)
}

pub fn fmt_hms(secs: u32) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// The `t=` / `start=` already in a YouTube link.
pub fn youtube_time(url: &str) -> Option<u32> {
    if !is_youtube(url) {
        return None;
    }
    let (_, query, frag) = split(url);
    params(query)
        .chain(params(frag))
        .find(|(k, _)| *k == "t" || *k == "start" || *k == "time_continue")
        .and_then(|(_, v)| parse_hms(v))
        .filter(|t| *t > 0)
}

/// The row in a Google Sheets `range=A132` / `range=B12:C20` fragment.
pub fn sheet_row(url: &str) -> Option<u32> {
    if !is_google_sheet(url) {
        return None;
    }
    let (_, query, frag) = split(url);
    let range = params(frag).chain(params(query)).find(|(k, _)| *k == "range")?.1;
    let first = percent_decode(range);
    let first = first.split(':').next()?;
    first.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '$').parse().ok().filter(|r| *r > 0)
}

// ---------------------------------------------------------------------------------------------
// Resume links

pub fn resume_url(g: &Guide) -> String {
    let url = g.url.trim();
    if is_youtube(url) {
        let (base, query, frag) = split(url);
        let mut q: Vec<String> = params(query).filter(|(k, _)| !["t", "start", "time_continue"].contains(k)).map(|(k, v)| join_kv(k, v)).collect();
        if g.video_secs > 0 {
            q.push(format!("t={}s", g.video_secs));
        }
        let frag: Vec<String> = params(frag).filter(|(k, _)| *k != "t").map(|(k, v)| join_kv(k, v)).collect();
        return assemble(base, &q.join("&"), &frag.join("&"));
    }
    if is_google_sheet(url) && g.sheet_row > 0 {
        let (base, query, frag) = split(url);
        let gid = params(frag).chain(params(query)).find(|(k, _)| *k == "gid").map(|(_, v)| v.to_string());
        let frag = match gid {
            Some(gid) => format!("gid={gid}&range=A{}", g.sheet_row),
            None => format!("range=A{}", g.sheet_row),
        };
        return assemble(base, query, &frag);
    }
    if !is_google_sheet(url) && g.kind != Kind::Video {
        // multi-page guides reopen the page your next item is on
        let page = if g.resume_page.trim().is_empty() { url } else { g.resume_page.trim() };
        return with_text(page, &g.jump_text);
    }
    url.to_string()
}

/// The page, scrolled to `text` (a text fragment). Unchanged if `text` is empty.
pub fn with_text(url: &str, text: &str) -> String {
    let url = url.trim();
    let text = text.trim();
    if text.is_empty() || url.contains(":~:") {
        return url.to_string();
    }
    let directive = format!(":~:text={}", text_directive(text));
    if url.contains('#') {
        format!("{url}{directive}")
    } else {
        format!("{url}#{directive}")
    }
}

fn split(url: &str) -> (&str, &str, &str) {
    let (rest, frag) = url.split_once('#').unwrap_or((url, ""));
    let (base, query) = rest.split_once('?').unwrap_or((rest, ""));
    (base, query, frag)
}

fn params(s: &str) -> impl Iterator<Item = (&str, &str)> {
    s.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, "")))
}

fn join_kv(k: &str, v: &str) -> String {
    if v.is_empty() {
        k.to_string()
    } else {
        format!("{k}={v}")
    }
}

fn assemble(base: &str, query: &str, frag: &str) -> String {
    let mut s = base.to_string();
    if !query.is_empty() {
        s.push('?');
        s.push_str(query);
    }
    if !frag.is_empty() {
        s.push('#');
        s.push_str(frag);
    }
    s
}

/// Percent-encodes for a text fragment directive, where `-`, `,` and `&` are syntax.
fn text_directive(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn percent_encode(s: &str) -> String {
    text_directive(s)
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guide(url: &str) -> Guide {
        Guide::new(1, url.to_string())
    }

    #[test]
    fn kinds() {
        assert_eq!(detect_kind("https://www.youtube.com/watch?v=abc"), Kind::Video);
        assert_eq!(detect_kind("https://youtu.be/abc?t=30"), Kind::Video);
        assert_eq!(detect_kind("https://docs.google.com/spreadsheets/d/xyz/edit#gid=0"), Kind::Sheet);
        assert_eq!(detect_kind("https://www.powerpyx.com/elden-ring-trophy-guide/"), Kind::Trophy);
        assert_eq!(detect_kind("https://psnprofiles.com/guide/123"), Kind::Trophy);
        assert_eq!(detect_kind("https://eldenring.wiki.fextralife.com/Bosses"), Kind::Wiki);
        assert_eq!(detect_kind("https://mapgenie.io/elden-ring/maps/the-lands-between"), Kind::Map);
        assert_eq!(detect_kind("https://gamefaqs.gamespot.com/ps5/123-game/faqs/456"), Kind::Web);
    }

    #[test]
    fn hms() {
        assert_eq!(parse_hms("1:02:03"), Some(3723));
        assert_eq!(parse_hms("62:03"), Some(3723));
        assert_eq!(parse_hms("125"), Some(125));
        assert_eq!(parse_hms("1h2m3s"), Some(3723));
        assert_eq!(parse_hms("2m"), Some(120));
        assert_eq!(parse_hms("90s"), Some(90));
        assert_eq!(parse_hms("abc"), None);
        assert_eq!(fmt_hms(3723), "1:02:03");
        assert_eq!(fmt_hms(65), "1:05");
    }

    #[test]
    fn youtube_resume() {
        let mut g = guide("https://www.youtube.com/watch?v=abc&t=95s");
        assert_eq!(g.video_secs, 95);
        g.video_secs = 3723;
        assert_eq!(resume_url(&g), "https://www.youtube.com/watch?v=abc&t=3723s");
        let mut g = guide("https://youtu.be/abc");
        g.video_secs = 10;
        assert_eq!(resume_url(&g), "https://youtu.be/abc?t=10s");
    }

    #[test]
    fn sheet_resume() {
        let mut g = guide("https://docs.google.com/spreadsheets/d/xyz/edit#gid=123&range=A40");
        assert_eq!(g.sheet_row, 40);
        g.sheet_row = 132;
        assert_eq!(resume_url(&g), "https://docs.google.com/spreadsheets/d/xyz/edit#gid=123&range=A132");
        let mut g = guide("https://docs.google.com/spreadsheets/d/xyz/edit?usp=sharing");
        g.sheet_row = 5;
        assert_eq!(resume_url(&g), "https://docs.google.com/spreadsheets/d/xyz/edit?usp=sharing#range=A5");
    }

    #[test]
    fn text_jump() {
        let mut g = guide("https://www.powerpyx.com/guide/");
        g.jump_text = "Chapter 5 - The Swamp".into();
        assert_eq!(resume_url(&g), "https://www.powerpyx.com/guide/#:~:text=Chapter%205%20%2D%20The%20Swamp");
        let mut g = guide("https://example.com/guide#part-2");
        g.jump_text = "Boss".into();
        assert_eq!(resume_url(&g), "https://example.com/guide#part-2:~:text=Boss");
    }

    #[test]
    fn suspended_tabs() {
        let s = "chrome-extension://noogafoofpebimajpfpamcfhoaifemoa/suspended.html#ttl=Chapter%205%3A%20Cai's%20Battle%20Walkthrough%20Guide%20%7C%20Fire%20Emblem&pos=8900&uri=https://game8.co/games/Fire-Emblem-Fortunes-Weave/archives/623328";
        let (url, title) = unwrap_suspended(s).unwrap();
        assert_eq!(url, "https://game8.co/games/Fire-Emblem-Fortunes-Weave/archives/623328");
        assert_eq!(title.as_deref(), Some("Chapter 5: Cai's Battle Walkthrough Guide | Fire Emblem"));
        assert_eq!(normalize(s).as_deref(), Some(url.as_str()));
        let all = extract_all(&format!("Btw https://www.neoseeker.com/x/walkthrough , {s} , {s}"));
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn multi_page_resume() {
        let mut g = guide("https://example.com/walkthrough");
        g.steps = vec![crate::model::Step { text: "Talk to Rin".into(), anchor: "Talk to Rin".into(), url: "https://example.com/week-1".into(), ..Default::default() }];
        g.follow_checklist(None);
        assert_eq!(resume_url(&g), "https://example.com/week-1#:~:text=Talk%20to%20Rin");
    }

    #[test]
    fn urls() {
        assert_eq!(normalize("gamefaqs.gamespot.com/x").as_deref(), Some("https://gamefaqs.gamespot.com/x"));
        assert_eq!(normalize("hello world"), None);
        assert_eq!(normalize("notes"), None);
        assert_eq!(host("https://www.Example.com:8080/a?b#c"), "example.com");
        assert_eq!(fallback_title("https://www.powerpyx.com/elden-ring-walkthrough/"), "powerpyx.com \u{2014} elden ring walkthrough");
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
