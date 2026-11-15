//! Turns pasted guide text into checklist items.
//!
//! Two kinds of text are understood:
//! - **A plain list**: one item per line (bullets, numbers and `[ ]` boxes are stripped).
//! - **A structured guide** like the common Steam/GameFAQs 100% guides:
//!   ```text
//!   Castle Town (1st Playthrough)              <- area heading
//!   Chest 1 - 🍸Potion; on the road to ...     <- chest: name; location
//!   🏆Wandering the Dark* - Begin a journey... <- achievement (* = hidden)
//!   ✅Objective 14: Frozen Rail Ride           <- objective
//!   "Use ice-based magic to ..."               <- requirement
//!   How: after getting the map chest, ...      <- how-to
//!   Reward: Flawless Arm Guards.               <- reward
//!   ```

use crate::model::{Step, StepKind};

fn is_marker(c: char) -> bool {
    let u = c as u32;
    (0x2600..0x27C0).contains(&u) || u >= 0x1F000 || u == 0xFE0F || u == 0x200D || (0x2190..0x2200).contains(&u)
}

/// Removes emoji and pictographs (the pixel font can't draw them) and tidies spacing.
pub fn strip_symbols(s: &str) -> String {
    let t: String = s.chars().filter(|c| !is_marker(*c)).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Curly quotes are drawn full-width by the Japanese pixel font; straight ones look right.
pub fn plain_quotes(s: &str) -> String {
    s.replace(['\u{2019}', '\u{2018}'], "'").replace(['\u{201c}', '\u{201d}'], "\"")
}

fn display(s: &str) -> String {
    plain_quotes(&strip_symbols(s))
}

fn trim_end_dot(s: &str) -> &str {
    s.trim().trim_end_matches('.').trim()
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// "Chest 12 - ..." → Some("12")
fn chest_number(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("Chest ")?;
    let n = rest.split(|c: char| !c.is_ascii_digit()).next()?;
    (!n.is_empty()).then_some(n)
}

fn looks_structured(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        t.starts_with('\u{2705}')
            || t.starts_with('\u{1F3C6}')
            || t.starts_with("How:")
            || t.starts_with("Reward:")
            || t.starts_with("End:")
            || t.starts_with("Objective:")
            || t.starts_with("Task:")
            || t.starts_with("Requirements:")
            || t.starts_with("Rewards:")
            || numbered(t).is_some()
            || counted(t).is_some()
            || chest_number(t).is_some()
            || t.starts_with("Objective ") && t[10..].starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Title-cases an ALL-CAPS heading: "CASTLE TOWN OBJECTIVES" → "Castle Town Objectives".
fn heading(s: &str) -> String {
    let s = display(s);
    if s.chars().any(|c| c.is_lowercase()) {
        return s;
    }
    s.split(' ')
        .map(|w| {
            let lower = w.to_lowercase();
            if ["of", "the", "in", "and", "a"].contains(&lower.as_str()) {
                lower
            } else {
                cap_first(&lower)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_heading(line: &str) -> bool {
    let s = strip_symbols(line);
    if s.is_empty() || s.len() > 60 || s.contains(':') || s.contains(';') || s.contains("http") {
        return false;
    }
    let all_caps = s.chars().any(|c| c.is_alphabetic()) && !s.chars().any(|c| c.is_lowercase());
    let in_parens = s.ends_with(')') && s.contains('(');
    let no_sentence = !s.ends_with(['.', '!', '?', ',']) && s.split(' ').count() <= 6;
    all_caps || in_parens || no_sentence
}

/// Lines that describe the item above them (quest-style guides).
const FIELDS: [&str; 12] = [
    "From:", "Available:", "Objective:", "Location:", "End:", "Deadline:", "Ends:", "Start:", "Task:", "Requirements:", "Client:", "Walkthrough:",
];

/// "Quest #12 More's Task", "Subquest #2: No Stone Unturned", "Gold Beetle #3: Grand Trad"
/// → ("Quest", "12", "More's Task"). The label is a short capitalised name.
fn numbered(line: &str) -> Option<(String, String, String)> {
    let at = line.find(" #")?;
    let label = line[..at].trim();
    let ok_label = (1..=30).contains(&label.len())
        && label.starts_with(|c: char| c.is_uppercase())
        && label.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-');
    if !ok_label {
        return None;
    }
    let after = &line[at + 2..];
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = after[digits.len()..].trim_start_matches([':', ' ', '-', '\u{2013}']).trim();
    Some((label.to_string(), digits, rest.to_string()))
}

/// "Miman - 5", "Demon Statue Anzu \u{2013} 1" → ("Miman", "5")
fn counted(line: &str) -> Option<(String, String)> {
    let digits: String = line.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    if digits.is_empty() || digits.len() > 4 {
        return None;
    }
    let head = line[..line.len() - digits.len()].trim_end();
    let label = head.strip_suffix(" -").or_else(|| head.strip_suffix(" \u{2013}"))?.trim();
    let ok = (3..=40).contains(&label.len())
        && label.starts_with(|c: char| c.is_uppercase())
        && label.split(' ').count() <= 5
        && label.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '\'' || c == '-');
    ok.then(|| (label.to_string(), digits))
}

/// Page furniture that isn't guide content: bylines ("October 10, 2024 by LuckySlime 1 Comment")
/// and comment timestamps ("June 15, 2024 at 4:57 pm").
fn is_page_noise(line: &str) -> bool {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let Some(rest) = MONTHS.iter().find_map(|m| line.strip_prefix(m).and_then(|r| r.strip_prefix(' '))) else { return false };
    // "10, 2024 by ..." / "15, 2024 at 4:57 pm"
    let mut parts = rest.splitn(3, ' ');
    let day = parts.next().unwrap_or("").trim_end_matches(',');
    let year = parts.next().unwrap_or("");
    let tail = parts.next().unwrap_or("");
    day.chars().all(|c| c.is_ascii_digit()) && year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) && (tail.starts_with("by ") || tail.starts_with("at "))
}

/// "Brute Talisman - Complete Subquest Talisman Hunt" → ("Brute Talisman", "Complete Subquest Talisman Hunt"):
/// a collectible's name, then how to get it.
fn named_collectible(line: &str) -> Option<(String, String)> {
    let (name, how) = line.split_once(" \u{2013} ").or_else(|| line.split_once(" - "))?;
    let name = name.trim();
    let ok = (3..=40).contains(&name.len())
        && name.starts_with(|c: char| c.is_uppercase())
        && name.split(' ').count() <= 5
        && !how.trim().is_empty()
        // "Chest 1 - Potion; ..." is the numbered-chest format, handled separately
        && !name.ends_with(|c: char| c.is_ascii_digit())
        // "Return to Miman - ..." is an instruction, not an item
        && !["Return", "Go", "Head", "Talk", "Find", "Collect", "Get", "Bring", "Take", "Use", "Visit", "Speak", "Enter", "Open"]
            .iter()
            .any(|v| name.split(' ').next() == Some(*v))
        && kind_for_label(name) == StepKind::Chest;
    ok.then(|| (name.to_string(), how.trim().to_string()))
}

/// What a numbered or counted label is: collectibles show as chests, trophies and endings as achievements.
fn kind_for_label(label: &str) -> StepKind {
    let l = label.to_lowercase();
    if ["trophy", "achievement", "ending"].iter().any(|k| l.contains(k)) {
        StepKind::Achievement
    } else if ["chest", "beetle", "collectible", "miman", "statue", "husk", "amalgam", "abscess", "bead", "feather", "relic", "treasure", "talisman", "periapt", "memento", "record", "card"]
        .iter()
        .any(|k| l.contains(k))
    {
        StepKind::Chest
    } else {
        StepKind::Task
    }
}

/// PowerPyx-style trophy lists: blocks separated by "|" lines, alternating
/// [name, description] and [how to get it]. `None` unless the text is mostly that.
fn trophy_blocks(text: &str) -> Option<Vec<Step>> {
    if text.lines().filter(|l| l.trim() == "|").count() < 6 {
        return None;
    }
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for l in text.lines().map(str::trim) {
        if l == "|" {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else if !l.is_empty() && !l.starts_with("## ") {
            cur.push(display(l));
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let b = &blocks[i];
        if b.len() >= 2 && b[0].len() < 70 && !b[0].ends_with('.') {
            let how = blocks.get(i + 1).map(|h| h.join(" ")).unwrap_or_default();
            let missable = how.to_lowercase().contains("missable");
            out.push(Step {
                anchor: b[0].clone(),
                text: b[0].clone(),
                kind: StepKind::Achievement,
                section: "Trophies".into(),
                detail: if missable { format!("Missable. {}", b[1]) } else { b[1].clone() },
                how: how.replace("\u{bb} ", "See: "),
                ..Default::default()
            });
            i += 2;
        } else {
            i += 1;
        }
    }
    (out.len() >= 3).then_some(out)
}

fn is_field(line: &str) -> bool {
    FIELDS.iter().any(|f| line.starts_with(f))
}

/// The first "M/D" date in `s`, e.g. "9/25" from "End: 9/25 (Sunday)".
fn first_date(s: &str) -> Option<String> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '/'))
        .find(|w| crate::model::parse_md(w).is_some())
        .map(str::to_string)
}

/// A checklist made only of the page's headings (`## Heading` lines), for long prose guides
/// where every paragraph would otherwise become an item.
pub fn parse_headings(text: &str) -> Vec<Step> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("## "))
        .map(display)
        .filter(|t| !t.is_empty())
        .map(|t| Step { anchor: t.clone(), ..Step::task(t) })
        .collect()
}

pub fn parse(text: &str) -> Vec<Step> {
    if let Some(trophies) = trophy_blocks(text) {
        return trophies;
    }
    if !looks_structured(text) {
        // plain list; "## " lines (from a fetched page) become areas
        let mut section = String::new();
        let mut out = Vec::new();
        for l in text.lines() {
            if let Some(h) = l.trim().strip_prefix("## ") {
                section = heading(h);
                continue;
            }
            let t = plain_item(l);
            if !t.is_empty() {
                out.push(Step { section: section.clone(), ..Step::task(t) });
            }
        }
        return out;
    }
    let mut out: Vec<Step> = Vec::new();
    let mut section = String::new();
    // The last ordinary heading: ALL-CAPS headings under it are shown as "Parent · Sub".
    let mut parent = String::new();
    // Whether the last item can still take detail lines.
    let mut open = false;
    // stop at the page's comments, and skip bylines and comment dates
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .take_while(|l| !matches!(*l, "## Comments" | "## Leave a Reply" | "## Leave a Comment"))
        .filter(|l| !l.is_empty() && !is_page_noise(l))
        .collect();
    // the item just added from a numbered/counted line still wants its description
    let mut wants_description = false;
    for (li, raw) in lines.iter().enumerate() {
        let explicit_heading = raw.strip_prefix("## ");
        let line = explicit_heading.unwrap_or(raw).trim();
        let next_is_field = lines.get(li + 1).is_some_and(|n| is_field(n));
        if line.is_empty() {
            continue;
        }
        let objective = line.starts_with('\u{2705}');
        let trophy = line.starts_with('\u{1F3C6}');
        let bare = strip_symbols(line);
        let push_how = |out: &mut Vec<Step>, text: &str| {
            if let Some(cur) = out.last_mut() {
                if !cur.how.is_empty() {
                    cur.how.push(' ');
                }
                cur.how.push_str(&display(text));
            }
        };

        if explicit_heading.is_some() || is_field(line) {
            wants_description = false;
        }
        if let Some(h) = explicit_heading {
            section = heading(h);
            parent = section.clone();
            open = false;
        } else if let Some((field, rest)) = FIELDS.iter().find_map(|f| bare.strip_prefix(f).map(|r| (*f, r.trim()))) {
            if let Some(cur) = out.last_mut().filter(|_| open) {
                match field {
                    "Objective:" => cur.detail = format!("Objective: {}", display(rest)),
                    "Task:" | "Requirements:" => {
                        if !cur.detail.is_empty() {
                            cur.detail.push(' ');
                        }
                        cur.detail.push_str(&display(rest));
                    }
                    "Client:" => cur.how = format!("Client: {}. {}", display(rest), cur.how).trim().to_string(),
                    "Walkthrough:" => {
                        if !cur.how.is_empty() {
                            cur.how.push(' ');
                        }
                        cur.how.push_str(&display(rest));
                    }
                    "End:" | "Deadline:" | "Ends:" => {
                        if let Some(d) = first_date(rest) {
                            cur.deadline = d;
                            cur.optional = true;
                        }
                    }
                    _ => {
                        if !cur.how.is_empty() {
                            cur.how.push(' ');
                        }
                        cur.how.push_str(&format!("{field} {}", display(rest)));
                    }
                }
            }
        } else if wants_description && !is_field(line) && !line.starts_with("## ") && numbered(&bare).is_none() && counted(&bare).is_none() {
            // the line right under "Quest #20 ..." describes it ("Automatically accepted on 06/26")
            wants_description = false;
            if let Some(cur) = out.last_mut() {
                if cur.detail.is_empty() {
                    cur.detail = display(line);
                } else {
                    cur.detail = format!("{} {}", cur.detail, display(line));
                }
            }
        } else if let Some((label, n, rest)) = numbered(&bare) {
            // "Quest #12 More's Task", "Subquest #24: Rite of Resurrection (Missable)"
            let missable = rest.contains("(Missable)");
            let main = rest.contains("(MAIN QUEST)");
            let name = display(&rest.replace("(Missable)", "").replace("(MAIN QUEST)", ""));
            let text = if name.is_empty() { format!("{label} #{n}") } else { format!("{label} #{n}: {name}") };
            let detail = if missable { "Missable." } else if main { "Main quest." } else { "" };
            out.push(Step { anchor: format!("{label} #{n}"), section: section.clone(), kind: kind_for_label(&label), detail: detail.into(), ..Step::task(text) });
            open = true;
            wants_description = true;
        } else if let Some((label, n)) = counted(&bare) {
            // "Miman - 5": its location follows on the next line
            out.push(Step { anchor: format!("{label} - {n}"), section: section.clone(), kind: kind_for_label(&label), ..Step::task(format!("{label} {n}")) });
            open = true;
            wants_description = true;
        } else if let Some((name, how)) = named_collectible(&bare).filter(|_| !trophy && !objective) {
            // "Brute Talisman - Complete Subquest Talisman Hunt"
            out.push(Step { anchor: name.clone(), section: section.clone(), kind: StepKind::Chest, detail: display(&how), ..Step::task(display(&name)) });
            open = true;
        } else if next_is_field && !is_field(line) && !line.starts_with("- ") && !line.ends_with('.') && !["Reward:", "Rewards:", "How:", "Note:"].iter().any(|p| bare.starts_with(p)) {
            // "Hidden Memories" followed by "From: ..." is a quest, not a heading
            let name = display(line);
            out.push(Step { anchor: name.clone(), section: section.clone(), ..Step::task(name) });
            open = true;
        } else if let Some((tag, name)) = bracket_marker(&bare) {
            // "[Combat] - Battle vs. Bandits"
            out.push(Step { anchor: name.clone(), section: section.clone(), ..Step::task(format!("{tag}: {name}")) });
            open = true;
        } else if let Some(rest) = bare.strip_prefix("How:") {
            if open {
                push_how(&mut out, rest.trim());
            }
        } else if let Some(rest) = bare.strip_prefix("Note:") {
            if open {
                push_how(&mut out, &format!("Note: {}", rest.trim()));
            }
        } else if let Some(rest) = bare.strip_prefix("Reward:").or_else(|| bare.strip_prefix("Rewards:")) {
            if let Some(cur) = out.last_mut().filter(|_| open) {
                cur.reward = trim_end_dot(&display(rest)).to_string();
            }
        } else if open && (line.starts_with(['\u{201c}', '"']) || (line.starts_with("- ") && out.last().is_some_and(|s| s.detail.is_empty()))) {
            let cur = out.last_mut().unwrap();
            let d = display(line.trim_start_matches("- ").trim_matches(['\u{201c}', '\u{201d}', '"']));
            cur.detail = d.trim_matches('"').to_string();
        } else if objective || (bare.starts_with("Objective ") && bare[10..].starts_with(|c: char| c.is_ascii_digit())) {
            let mut name = bare.trim().to_string();
            let hidden = name.ends_with('*');
            name = trim_end_dot(name.trim_end_matches('*')).to_string();
            // "Objective 37:Treasure Hunt" → "Objective 37: Treasure Hunt"
            if let Some(i) = name.find(':') {
                if !name[i + 1..].starts_with(' ') {
                    name.insert(i + 1, ' ');
                }
            }
            out.push(Step {
                anchor: name.clone(),
                text: plain_quotes(&name),
                kind: StepKind::Objective,
                section: section.clone(),
                hidden,
                ..Default::default()
            });
            open = true;
        } else if trophy {
            let (name, desc) = match bare.split_once(" - ") {
                Some((n, d)) => (n.trim().to_string(), d.trim().to_string()),
                None => (bare.trim().to_string(), String::new()),
            };
            let hidden = name.ends_with('*');
            let name = name.trim_end_matches('*').trim().to_string();
            out.push(Step {
                anchor: name.split('/').next().unwrap_or(&name).trim().to_string(),
                text: plain_quotes(&name.replace('/', " / ")),
                kind: StepKind::Achievement,
                section: section.clone(),
                detail: plain_quotes(&desc),
                hidden,
                ..Default::default()
            });
            open = true;
        } else if let Some(n) = chest_number(&bare) {
            // "Chest 1 - Potion; on the road to ..." (the item's emoji is already stripped)
            let rest = bare["Chest ".len() + n.len()..].trim_start_matches([' ', '-', ':']).trim();
            let (item, place) = rest.split_once(';').unwrap_or((rest, ""));
            let place = trim_end_dot(place);
            out.push(Step {
                // The location is unique on the page; "Chest 1" repeats in every area.
                anchor: place.split(' ').take_while(|w| !w.starts_with('(')).take(8).collect::<Vec<_>>().join(" "),
                text: plain_quotes(&format!("Chest {n}: {}", item.trim())),
                kind: StepKind::Chest,
                section: section.clone(),
                detail: plain_quotes(&cap_first(place)),
                ..Default::default()
            });
            open = true;
        } else if is_heading(line) {
            let shouting = !strip_symbols(line).chars().any(|c| c.is_lowercase());
            if shouting && !parent.is_empty() {
                section = format!("{parent} \u{b7} {}", heading(line));
            } else {
                section = heading(line);
                parent = section.clone();
            }
            open = false;
        } else if open {
            // text under an item: the first line describes it, the rest is how-to
            match out.last_mut() {
                Some(cur) if cur.detail.is_empty() && cur.how.is_empty() => cur.detail = display(line),
                _ => push_how(&mut out, line),
            }
        } else if looks_like_list_item(line) {
            // short list-style lines between items ("Find the key"), but not prose
            out.push(Step { section: section.clone(), ..Step::task(plain_item(line)) });
            open = true;
        }
    }
    out
}

/// "[Combat] - Battle vs. Bandits" → ("Combat", "Battle vs. Bandits")
fn bracket_marker(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('[')?;
    let (tag, name) = rest.split_once(']')?;
    let name = name.trim().trim_start_matches(['-', ':']).trim();
    let ok = !tag.is_empty() && tag.len() <= 24 && tag.chars().all(|c| c.is_alphanumeric() || c == ' ') && !name.is_empty();
    ok.then(|| (display(tag), display(name)))
}

/// Short, not a sentence, not a "Label: value" line.
fn looks_like_list_item(line: &str) -> bool {
    let t = plain_item(line);
    !t.is_empty() && t.len() <= 60 && !t.ends_with(['.', '!', '?', ':', ',']) && !t.contains(": ")
}

/// "1. Chapter One" / "- [ ] Chapter One" / "• Chapter One" → "Chapter One"
pub fn plain_item(s: &str) -> String {
    let t = s.trim();
    let t = t.trim_start_matches(['-', '*', '\u{2022}', '\u{00b7}']).trim_start();
    let t = t.strip_prefix("[ ]").or_else(|| t.strip_prefix("[x]")).unwrap_or(t).trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let t = if digits > 0 && t[digits..].starts_with(['.', ')']) { t[digits + 1..].trim_start() } else { t };
    display(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Castle Town (1st Playthrough)
Chest 1 - \u{1F378}Potion; on the road to Castle of Dreams (before entering Castle Town).

\u{1F3C6}Wandering the Dark* - Begin a journey to escape the Dark World.
How: enter Castle Town proper.

\u{2705}Objective 14: Frozen Rail Ride
\u{201c}Use ice-based magic to create a rail, then ride the rail a long distance.\u{201d}
How: after getting the map chest, stand between the two lanterns.
Reward: Flawless Arm Guards.

\u{1F3C6}Proud Player/Critical Competitor - Complete the game in Proud or Critical Mode.
Castle Town (cleared data)
Chest 7 - \u{2652}Aquarius Relic; we\u{2019}ll go to where the west cog was.

\u{2705}Objective 37:Treasure Hunt in the Castle Town
\u{201c}Open all of the treasure chests.\u{201d}
Reward: Coil (Yellow).

Enemy EXP Info: https://docs.google.com/spreadsheets/d/x/edit
Additional Playthroughs
Note: be sure to use the NG+ feature when doing a new playthrough.

CASTLE TOWN OBJECTIVES

\u{2705}Objective 36: Turn the Tide
\u{201c}Defeat the Demon Tower with Spellweaver\u{2019}s Finish command.\u{201d}
Reward: Cheshire Cat.
";

    #[test]
    fn structured() {
        let s = parse(SAMPLE);
        assert_eq!(s.len(), 7, "{:#?}", s.iter().map(|x| &x.text).collect::<Vec<_>>());
        assert_eq!(s[0].text, "Chest 1: Potion");
        assert_eq!(s[0].kind, StepKind::Chest);
        assert_eq!(s[0].section, "Castle Town (1st Playthrough)");
        assert_eq!(s[0].detail, "On the road to Castle of Dreams (before entering Castle Town)");
        assert_eq!(s[0].anchor, "on the road to Castle of Dreams");

        assert_eq!(s[1].text, "Wandering the Dark");
        assert!(s[1].hidden);
        assert_eq!(s[1].kind, StepKind::Achievement);
        assert_eq!(s[1].detail, "Begin a journey to escape the Dark World.");
        assert_eq!(s[1].how, "enter Castle Town proper.");

        assert_eq!(s[2].text, "Objective 14: Frozen Rail Ride");
        assert_eq!(s[2].detail, "Use ice-based magic to create a rail, then ride the rail a long distance.");
        assert_eq!(s[2].reward, "Flawless Arm Guards");
        assert!(s[2].how.starts_with("after getting"));

        assert_eq!(s[3].text, "Proud Player / Critical Competitor");
        assert_eq!(s[3].anchor, "Proud Player");

        assert_eq!(s[4].text, "Chest 7: Aquarius Relic");
        assert_eq!(s[4].section, "Castle Town (cleared data)");
        assert_eq!(s[4].detail, "We'll go to where the west cog was");

        assert_eq!(s[5].text, "Objective 37: Treasure Hunt in the Castle Town");
        assert!(s[5].how.contains("https://docs.google.com"), "{}", s[5].how);

        assert_eq!(s[6].section, "Additional Playthroughs \u{b7} Castle Town Objectives");
        assert_eq!(s[6].detail, "Defeat the Demon Tower with Spellweaver's Finish command.");
    }

    #[test]
    fn quests_and_fetched_headings() {
        let text = "## Free Time 3\n## Kindness Quests\nHidden Memories\nFrom: Old Man in Lowtown\nObjective: Deliver Jurah Carving\nLocation: Collapsed Fort\nReward: 2000G, 100R\nEnd: 7/9\nA Peculiar Message\nFrom: Citizen in City Streets\nReward: 2000G, 180R\nEnd: 5/20";
        let s = parse(text);
        assert_eq!(s.len(), 2, "{:#?}", s);
        assert_eq!(s[0].text, "Hidden Memories");
        assert_eq!(s[0].section, "Kindness Quests");
        assert_eq!(s[0].detail, "Objective: Deliver Jurah Carving");
        assert_eq!(s[0].how, "From: Old Man in Lowtown Location: Collapsed Fort");
        assert_eq!(s[0].reward, "2000G, 100R");
        assert_eq!(s[0].deadline, "7/9");
        assert!(s[0].optional);
        assert_eq!(s[1].deadline, "5/20");

        let h = parse_headings("## Chapter 1 - Start\nsome prose\n## Chapter 2 - Road");
        assert_eq!(h.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["Chapter 1 - Start", "Chapter 2 - Road"]);
        let fe = parse("## Free Time 1\nStart: 2/20\nEnd: 2/28\n## Kindness Quests\nLost Hair Clip\nFrom: Girl\nEnd: 2/28\nAfter the cutscenes, you'll explore the town and talk to everyone you can find there.\n[Combat] - Battle vs. Bandits\n- Victory: Seize the red jewel.");
        assert_eq!(fe.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["Lost Hair Clip", "Combat: Battle vs. Bandits"]);
        let plain = parse("## Act 1\n- Find the key\n- Open the door");
        assert_eq!(plain[1].section, "Act 1");
    }

    #[test]
    fn numbered_counted_and_trophy_lists() {
        // PowerPyx quest list (Metaphor)
        let q = parse("## Side Quests\nQuest #11 Save the Country (MAIN QUEST)\nQuest #14 Pagan\u{2019}s Dilemma\nAccepted from the Mustari Man outside the Inn in Sunshade Row.\nTask: Purchase the Breath of Fresh Air.\nReward: Rusted Accessory, Increases Tolerance (+4)\nSubquest #24: Rite of Resurrection (Missable)\nClient: Neko Shogun\nRequirements: Find 5 Life Stones.\nWalkthrough: Farm them from battles.\nRewards: Wargod Talisman x1");
        let t: Vec<_> = q.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(t, ["Quest #11: Save the Country", "Quest #14: Pagan's Dilemma", "Subquest #24: Rite of Resurrection"]);
        assert_eq!(q[0].detail, "Main quest.");
        assert_eq!(q[1].detail, "Accepted from the Mustari Man outside the Inn in Sunshade Row. Purchase the Breath of Fresh Air.");
        assert_eq!(q[1].reward, "Rusted Accessory, Increases Tolerance (+4)");
        assert_eq!(q[2].detail, "Missable. Find 5 Life Stones.");
        assert_eq!(q[2].how, "Client: Neko Shogun. Farm them from battles.");
        assert_eq!(q[2].reward, "Wargod Talisman x1");

        // collectibles: numbered (Gold Beetle) and counted (Miman, Demon Statue)
        let c = parse("## Tamachi\nGold Beetle #1: Northern Border Fort\nOnce you're at the camp, pick it up under a tree.\nMiman \u{2013} 1\nIn front of the very first leyline fount.\nDemon Statue Anzu \u{2013} 1\nAt the very end of this path.");
        let t: Vec<_> = c.iter().map(|s| (s.text.as_str(), s.kind, s.detail.as_str())).collect();
        assert_eq!(t, [
            ("Gold Beetle #1: Northern Border Fort", StepKind::Chest, "Once you're at the camp, pick it up under a tree."),
            ("Miman 1", StepKind::Chest, "In front of the very first leyline fount."),
            ("Demon Statue Anzu 1", StepKind::Chest, "At the very end of this path."),
        ]);
        assert_eq!(c[0].section, "Tamachi");

        // PowerPyx trophy list
        let tr = parse("## Trophy Guide\n|\nThe Traveller\nAcquired all trophies.\n|\nEarn all other trophies.\n|\nSummon Mask Time\nCreated all vessels and masks.\n|\nMissable! Collect various items.\n|\nBookworm\nRead every book.\n|\nBuy books in town.\n|");
        let t: Vec<_> = tr.iter().map(|s| (s.text.as_str(), s.kind)).collect();
        assert_eq!(t, [("The Traveller", StepKind::Achievement), ("Summon Mask Time", StepKind::Achievement), ("Bookworm", StepKind::Achievement)]);
        assert_eq!(tr[1].detail, "Missable. Created all vessels and masks.");
        assert_eq!(tr[2].how, "Buy books in town.");
    }

    #[test]
    fn page_furniture_and_reward_lines() {
        let t = "## Quests\nOctober 10, 2024 by LuckySlime 1 Comment\nQuest #20 Providing a Spark\nAutomatically accepted on 06/26\nTask: Travel to Komero\nReward: Hero's Incense x5\nBrute Talisman \u{2013} Complete Subquest Talisman Hunt\nMiman \u{2013} 1\nIn front of the very first leyline fount.\n## Comments\nGreat guide!\nJune 15, 2024 at 4:57 pm\nQuest #99 Not A Real Quest";
        let s = parse(t);
        let names: Vec<_> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(names, ["Quest #20: Providing a Spark", "Brute Talisman", "Miman 1"]);
        assert_eq!(s[0].detail, "Automatically accepted on 06/26 Travel to Komero");
        assert_eq!(s[1].detail, "Complete Subquest Talisman Hunt");
        assert_eq!(s[1].kind, StepKind::Chest);
        assert!(is_page_noise("June 15, 2024 at 4:57 pm") && !is_page_noise("June 15 is the deadline"));
        assert!(named_collectible("Return to Miman \u{2013} 3 and talk to him").is_none());
    }

    #[test]
    fn plain() {
        let s = parse("1. Chapter One\n- [ ] Chapter Two\n\u{2022} Chapter Three\n\nChapter Four");
        let t: Vec<_> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(t, ["Chapter One", "Chapter Two", "Chapter Three", "Chapter Four"]);
        assert!(s.iter().all(|x| x.kind == StepKind::Task));
    }
}
