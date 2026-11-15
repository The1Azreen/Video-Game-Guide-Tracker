//! Achievement sync: Steam (official Web API) and Xbox (OpenXBL, xbl.io).
//!
//! Steam needs a Web API key (steamcommunity.com/dev/apikey) and a profile whose "Game details"
//! are public. OpenXBL needs the key from xbl.io after signing in there with your Xbox account.
//! Keys live only in the local settings file.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Steam,
    Xbox,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::Xbox => "Xbox",
        }
    }
}

/// One achievement as a platform reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Achievement {
    pub name: String,
    pub description: String,
    pub achieved: bool,
    /// Share of players who have it, in percent (when the platform says).
    pub rarity: Option<f32>,
    /// The platform's internal name (Steam's "apiname"), to match global percentages.
    pub id: String,
}

/// For matching names across sites: lower case, letters and digits only.
pub fn key(s: &str) -> String {
    let lowered: String = s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The candidate whose name best matches `title`: an exact match, else one that contains the
/// title (or is contained by it), preferring the closest length.
pub fn best_match(candidates: &[(String, String)], title: &str) -> Option<usize> {
    let want = key(title);
    if let Some(i) = candidates.iter().position(|(_, n)| key(n) == want) {
        return Some(i);
    }
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (_, n))| {
            let k = key(n);
            !k.is_empty() && (k.contains(&want) || want.contains(&k))
        })
        .min_by_key(|(_, (_, n))| key(n).len().abs_diff(want.len()))
        .map(|(i, _)| i)
}

fn get_json(agent: &ureq::Agent, url: &str, header: Option<(&str, &str)>) -> Result<Value, ureq::Error> {
    let mut req = agent.get(url).set("Accept", "application/json");
    if let Some((k, v)) = header {
        req = req.set(k, v);
    }
    // a reply that isn't JSON comes back as an io::Error, which ureq turns into a transport error
    Ok(req.call()?.into_json::<Value>()?)
}

fn enc(s: &str) -> String {
    crate::links::percent_encode(s)
}

// ---------------------------------------------------------------------------------------------
// Steam

/// "https://steamcommunity.com/id/name/", ".../profiles/7656…", "7656…" or a vanity name → SteamID64
/// and the profile name.
pub fn steam_resolve(agent: &ureq::Agent, api_key: &str, input: &str) -> Result<(String, String), String> {
    let input = input.trim().trim_end_matches('/');
    let id = if let Some(p) = input.split("/profiles/").nth(1) {
        p.split('/').next().unwrap_or("").to_string()
    } else if input.len() == 17 && input.chars().all(|c| c.is_ascii_digit()) {
        input.to_string()
    } else {
        let vanity = input.split("/id/").nth(1).unwrap_or(input).split('/').next().unwrap_or("");
        let url = format!("https://api.steampowered.com/ISteamUser/ResolveVanityURL/v1/?key={}&vanityurl={}", enc(api_key), enc(vanity));
        let v = get_json(agent, &url, None).map_err(steam_error)?;
        match v["response"]["success"].as_i64() {
            Some(1) => v["response"]["steamid"].as_str().unwrap_or("").to_string(),
            _ => return Err(format!("Steam doesn't know the profile \"{vanity}\". Paste your profile link or 17-digit SteamID.")),
        }
    };
    let url = format!("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v2/?key={}&steamids={}", enc(api_key), enc(&id));
    let v = get_json(agent, &url, None).map_err(steam_error)?;
    let name = v["response"]["players"][0]["personaname"].as_str().ok_or("Steam found no profile with that ID.")?;
    Ok((id, name.to_string()))
}

/// Looks the game up on the Steam store: (app id, store name).
pub fn steam_find_app(agent: &ureq::Agent, title: &str) -> Result<(u32, String), String> {
    let url = format!("https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US", enc(title));
    let v = get_json(agent, &url, None).map_err(|e| format!("Couldn't search the Steam store: {e}"))?;
    let items: Vec<(String, String)> = v["items"]
        .as_array()
        .map(|a| a.iter().filter_map(|i| Some((i["id"].as_u64()?.to_string(), i["name"].as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let i = best_match(&items, title).or(if items.is_empty() { None } else { Some(0) }).ok_or(format!("\"{title}\" isn't on the Steam store."))?;
    Ok((items[i].0.parse().unwrap_or(0), items[i].1.clone()))
}

pub fn steam_achievements(agent: &ureq::Agent, api_key: &str, steam_id: &str, app_id: u32) -> Result<Vec<Achievement>, String> {
    let url = format!(
        "https://api.steampowered.com/ISteamUserStats/GetPlayerAchievements/v1/?appid={app_id}&key={}&steamid={}&l=english",
        enc(api_key),
        enc(steam_id)
    );
    match get_json(agent, &url, None) {
        Ok(v) => {
            let mut list = parse_steam(&v)?;
            // how rare each one is: public, no key needed; skipped quietly if it fails
            let url = format!("https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v0002/?gameid={app_id}&format=json");
            if let Ok(p) = get_json(agent, &url, None) {
                apply_percentages(&mut list, &p);
            }
            Ok(list)
        }
        Err(ureq::Error::Status(400, _)) => Err("Steam has no achievements for that game on your account (check the app ID, and that you own it).".into()),
        Err(e) => Err(steam_error(e)),
    }
}

fn parse_steam(v: &Value) -> Result<Vec<Achievement>, String> {
    let stats = &v["playerstats"];
    if stats["success"].as_bool() == Some(false) {
        return Err(stats["error"].as_str().unwrap_or("Steam couldn't read the achievements.").to_string());
    }
    let list = stats["achievements"].as_array().ok_or("This game has no achievements on Steam.")?;
    Ok(list
        .iter()
        .map(|a| Achievement {
            name: a["name"].as_str().or(a["apiname"].as_str()).unwrap_or("").to_string(),
            description: a["description"].as_str().unwrap_or("").to_string(),
            achieved: a["achieved"].as_i64() == Some(1),
            rarity: None,
            id: a["apiname"].as_str().unwrap_or("").to_string(),
        })
        .filter(|a| !a.name.is_empty())
        .collect())
}

/// Fills in `rarity` from GetGlobalAchievementPercentagesForApp (percent comes as a number or a
/// string, depending on the day).
fn apply_percentages(list: &mut [Achievement], v: &Value) {
    let Some(all) = v["achievementpercentages"]["achievements"].as_array() else { return };
    for p in all {
        let (Some(name), Some(pct)) = (p["name"].as_str(), p["percent"].as_f64().or_else(|| p["percent"].as_str()?.parse().ok())) else { continue };
        if let Some(a) = list.iter_mut().find(|a| a.id == name) {
            a.rarity = Some(pct as f32);
        }
    }
}

fn steam_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(401 | 403, _) => "Steam refused: check the API key, and that your profile's Game details are set to Public.".into(),
        ureq::Error::Status(code, _) => format!("Steam answered with error {code}."),
        other => format!("Couldn't reach Steam: {other}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Xbox (OpenXBL)

const XBL: &str = "https://xbl.io/api/v2";

fn xbl(agent: &ureq::Agent, api_key: &str, path: &str) -> Result<Value, String> {
    get_json(agent, &format!("{XBL}{path}"), Some(("X-Authorization", api_key))).map_err(|e| match e {
        ureq::Error::Status(401 | 403, _) => "OpenXBL refused the key. Copy it again from xbl.io.".to_string(),
        ureq::Error::Status(429, _) => "OpenXBL's free limit was reached; try again in a while.".to_string(),
        ureq::Error::Status(code, _) => format!("OpenXBL answered with error {code}."),
        other => format!("Couldn't reach OpenXBL: {other}"),
    })
}

/// The signed-in gamertag.
pub fn xbox_account(agent: &ureq::Agent, api_key: &str) -> Result<String, String> {
    let v = xbl(agent, api_key, "/account")?;
    parse_gamertag(&v).ok_or("OpenXBL didn't return a gamertag.".into())
}

fn parse_gamertag(v: &Value) -> Option<String> {
    v["profileUsers"][0]["settings"]
        .as_array()?
        .iter()
        .find(|s| s["id"].as_str() == Some("Gamertag"))?["value"]
        .as_str()
        .map(str::to_string)
}

/// Finds the game among the titles your account has played: (title id, name).
pub fn xbox_find_title(agent: &ureq::Agent, api_key: &str, title: &str) -> Result<(String, String), String> {
    let v = xbl(agent, api_key, "/achievements")?;
    let titles = parse_titles(&v);
    let i = best_match(&titles, title).ok_or(format!("\"{title}\" isn't among the games your Xbox account has played."))?;
    Ok(titles[i].clone())
}

fn parse_titles(v: &Value) -> Vec<(String, String)> {
    v["titles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let id = t["titleId"].as_str().map(str::to_string).or(t["titleId"].as_u64().map(|n| n.to_string()))?;
                    Some((id, t["name"].as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn xbox_achievements(agent: &ureq::Agent, api_key: &str, title_id: &str) -> Result<Vec<Achievement>, String> {
    let v = xbl(agent, api_key, &format!("/achievements/title/{}", enc(title_id.trim())))?;
    let list = parse_xbox(&v);
    if list.is_empty() {
        Err("OpenXBL returned no achievements for that title ID.".into())
    } else {
        Ok(list)
    }
}

fn parse_xbox(v: &Value) -> Vec<Achievement> {
    v["achievements"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Achievement {
                        name: x["name"].as_str()?.to_string(),
                        description: x["description"].as_str().or(x["lockedDescription"].as_str()).unwrap_or("").to_string(),
                        achieved: x["progressState"].as_str() == Some("Achieved"),
                        rarity: x["rarity"]["currentPercentage"].as_f64().map(|p| p as f32),
                        id: x["id"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steam_response() {
        let v = json!({"playerstats": {"steamID": "1", "gameName": "Slay the Princess", "success": true, "achievements": [
            {"apiname": "ACH_1", "achieved": 1, "unlocktime": 1700000000, "name": "The Body", "description": "Reach the end."},
            {"apiname": "ACH_2", "achieved": 0, "unlocktime": 0, "name": "Our Song", "description": ""}
        ]}});
        let a = parse_steam(&v).unwrap();
        let mut a = a;
        assert_eq!(a, vec![
            Achievement { name: "The Body".into(), description: "Reach the end.".into(), achieved: true, rarity: None, id: "ACH_1".into() },
            Achievement { name: "Our Song".into(), description: "".into(), achieved: false, rarity: None, id: "ACH_2".into() },
        ]);
        let pct = json!({"achievementpercentages": {"achievements": [{"name": "ACH_2", "percent": "3.4"}, {"name": "ACH_1", "percent": 61.5}]}});
        apply_percentages(&mut a, &pct);
        assert_eq!(a[0].rarity, Some(61.5));
        assert_eq!(a[1].rarity, Some(3.4));
        let err = json!({"playerstats": {"error": "Profile is not public", "success": false}});
        assert_eq!(parse_steam(&err).unwrap_err(), "Profile is not public");
        assert!(parse_steam(&json!({"playerstats": {"success": true}})).is_err(), "no achievements");
    }

    #[test]
    fn xbox_responses() {
        let account = json!({"profileUsers": [{"id": "2535", "settings": [{"id": "GameDisplayPicRaw", "value": "x"}, {"id": "Gamertag", "value": "Azreen"}]}]});
        assert_eq!(parse_gamertag(&account).as_deref(), Some("Azreen"));
        let titles = json!({"titles": [{"titleId": "1234", "name": "Metaphor: ReFantazio"}, {"titleId": 999, "name": "Forza Horizon 5"}]});
        assert_eq!(parse_titles(&titles), vec![("1234".to_string(), "Metaphor: ReFantazio".to_string()), ("999".to_string(), "Forza Horizon 5".to_string())]);
        let ach = json!({"achievements": [
            {"name": "Shake on It", "description": "Unlock a follower.", "progressState": "Achieved", "rarity": {"currentCategory": "Rare", "currentPercentage": 4.5}},
            {"name": "Bookworm", "lockedDescription": "Read every book.", "progressState": "NotStarted"}
        ]});
        let a = parse_xbox(&ach);
        assert_eq!(a.len(), 2);
        assert!(a[0].achieved && !a[1].achieved);
        assert_eq!(a[1].description, "Read every book.");
        assert_eq!((a[0].rarity, a[1].rarity), (Some(4.5), None));
    }

    #[test]
    fn title_matching() {
        let c = |v: &[&str]| v.iter().enumerate().map(|(i, n)| (i.to_string(), n.to_string())).collect::<Vec<_>>();
        let titles = c(&["Metaphor: ReFantazio - Digital Deluxe", "Metaphor: ReFantazio", "Persona 5 Royal"]);
        assert_eq!(best_match(&titles, "Metaphor ReFantazio"), Some(1), "exact (ignoring punctuation) wins");
        assert_eq!(best_match(&c(&["Slay the Princess - The Pristine Cut", "Slay the Spire"]), "Slay the Princess"), Some(0));
        assert_eq!(best_match(&c(&["Halo Infinite"]), "Okami HD"), None);
    }
}
