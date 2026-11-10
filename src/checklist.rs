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

pub fn parse(text: &str) -> Vec<Step> {
    if !looks_structured(text) {
        return text.lines().map(plain_item).filter(|s| !s.is_empty()).map(Step::task).collect();
    }
    let mut out: Vec<Step> = Vec::new();
    let mut section = String::new();
    // The last ordinary heading: ALL-CAPS headings under it are shown as "Parent · Sub".
    let mut parent = String::new();
    // Whether the last item can still take detail lines.
    let mut open = false;
    for raw in text.lines() {
        let line = raw.trim();
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

        if let Some(rest) = bare.strip_prefix("How:") {
            if open {
                push_how(&mut out, rest.trim());
            }
        } else if let Some(rest) = bare.strip_prefix("Note:") {
            if open {
                push_how(&mut out, &format!("Note: {}", rest.trim()));
            }
        } else if let Some(rest) = bare.strip_prefix("Reward:") {
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
            // a stray paragraph (links, extra notes) belongs to the item above
            push_how(&mut out, line);
        } else {
            let t = plain_item(line);
            if !t.is_empty() {
                out.push(Step { section: section.clone(), ..Step::task(t) });
                open = true;
            }
        }
    }
    out
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
    fn plain() {
        let s = parse("1. Chapter One\n- [ ] Chapter Two\n\u{2022} Chapter Three\n\nChapter Four");
        let t: Vec<_> = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(t, ["Chapter One", "Chapter Two", "Chapter Three", "Chapter Four"]);
        assert!(s.iter().all(|x| x.kind == StepKind::Task));
    }
}
