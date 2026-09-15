use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::NaiveTime;
use chrono_tz::Tz;
use serde::Deserialize;

use crate::i18n::{Lang, Localized, T, days_full};
use crate::schedule::{DAY_KEYS, Db, Entry, State, Week, day_index_of, sort_key, week_of};
use crate::views::{AdminForm, FormErrors};

#[derive(Deserialize)]
struct WriteRow {
    weeks: Week,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct AdminWrite {
    timezone: String,
    pickup_time: String,
    default_lang: Option<Lang>,
    schedule: [Vec<WriteRow>; 7],
}

fn admin_write_form(w: AdminWrite) -> AdminForm {
    let entries = w
        .schedule
        .iter()
        .enumerate()
        .flat_map(|(i, rows)| {
            rows.iter().map(move |r| Entry {
                day: DAY_KEYS[i].to_string(),
                weeks: r.weeks,
                kind: r.kind.clone(),
            })
        })
        .collect();
    AdminForm {
        timezone: w.timezone,
        pickup_time: w.pickup_time,
        entries,
        action: String::new(),
        default_lang: w.default_lang.map(|l| l.to_string()).unwrap_or_default(),
    }
}

#[derive(serde::Serialize)]
pub struct WriteErrors {
    pub fields: HashMap<String, String>,
    pub overlaps: HashMap<String, Vec<u8>>,
}

impl From<FormErrors> for WriteErrors {
    fn from(errs: FormErrors) -> Self {
        let overlaps = errs
            .bad_weeks
            .iter()
            .map(|((day, idx), w)| {
                let weeks = (1..=5)
                    .filter(|n| w.contains(week_of(*n)))
                    .map(|n| n as u8)
                    .collect();
                (format!("{day}:{idx}"), weeks)
            })
            .collect();
        WriteErrors {
            fields: errs.fields,
            overlaps,
        }
    }
}

#[derive(Debug)]
pub enum JsonWrite {
    ParseErr(String),
    Saved,
    Invalid(FormErrors),
}

pub fn admin_json_write(db_path: &PathBuf, body: &[u8], lng: Lang) -> JsonWrite {
    let w: AdminWrite = match serde_json::from_slice(body) {
        Ok(w) => w,
        Err(e) => return JsonWrite::ParseErr(format!("invalid JSON: {e}")),
    };
    match process_admin(admin_write_form(w), db_path, lng) {
        Ok(None) => JsonWrite::Saved,
        Ok(Some(_)) => unreachable!("empty action produces no rows"),
        Err((_f, errs)) => JsonWrite::Invalid(errs),
    }
}

pub fn body_is_json(ct: Option<&str>, body: &[u8]) -> bool {
    if ct
        .map(|c| c.to_ascii_lowercase().contains("json"))
        .unwrap_or(false)
    {
        return true;
    }
    body.iter()
        .find(|b| !b.is_ascii_whitespace())
        .map(|b| *b == b'{')
        .unwrap_or(false)
}

pub fn validate_and_save(db_path: &PathBuf, f: &AdminForm, lng: Lang) -> Result<(), FormErrors> {
    let mut errs = FormErrors::default();
    if let Err(e) = f.timezone.parse::<Tz>() {
        errs.fields.insert(
            "timezone".to_string(),
            format!("{}: {e}", Localized::from((lng, T::ErrTz))),
        );
    }
    if let Err(e) = NaiveTime::parse_from_str(&f.pickup_time, "%H:%M") {
        errs.fields.insert(
            "pickup_time".to_string(),
            format!("{}: {e}", Localized::from((lng, T::ErrTime))),
        );
    }
    let default_lang = match f.default_lang.as_str() {
        "" => None,
        "it" => Some(Lang::It),
        "en" => Some(Lang::En),
        other => {
            errs.fields.insert(
                "default_lang".to_string(),
                format!("{}: {other}", Localized::from((lng, T::ErrLang))),
            );
            None
        }
    };
    let mut schedule: Vec<Entry> = Vec::new();
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    for day in DAY_KEYS {
        let mut day_entries: Vec<&Entry> = f.entries.iter().filter(|e| e.day == *day).collect();
        day_entries.sort_by_key(|e| sort_key(e));
        for (idx, e) in day_entries.iter().enumerate() {
            let di = day_index_of(day);
            let weeks = e.weeks;
            if !weeks.is_empty() {
                if e.kind.trim().is_empty() {
                    let key = format!("{day}:{idx}");
                    errs.fields
                        .insert(key, Localized::from((lng, T::ErrType)).to_string());
                    continue;
                }
                for w in 1..=5 {
                    if !weeks.contains(week_of(w)) {
                        continue;
                    }
                    if !seen.insert((e.day.clone(), w)) {
                        let key = format!("{day}:{idx}");
                        errs.fields.entry(key).or_insert_with(|| {
                            Localized::from((lng, T::ErrOverlap(days_full(lng)[di], w))).to_string()
                        });
                        errs.bad_weeks
                            .entry((day, idx))
                            .or_default()
                            .insert(week_of(w));
                    }
                }
                schedule.push(Entry {
                    day: e.day.clone(),
                    weeks,
                    kind: e.kind.trim().to_string(),
                });
            }
        }
    }
    if !errs.fields.is_empty() || !errs.bad_weeks.is_empty() {
        return Err(errs);
    }
    let db = Db {
        timezone: f.timezone.to_string(),
        pickup_time: f.pickup_time.to_string(),
        schedule,
        default_lang,
    };
    if let Err(e) = State::save_file(db_path, &db) {
        let mut errs = FormErrors::default();
        errs.fields.insert(
            "form".to_string(),
            format!("{}: {e}", Localized::from((lng, T::ErrIo))),
        );
        return Err(errs);
    }
    Ok(())
}

#[allow(clippy::result_large_err)]
pub fn process_admin(
    mut f: AdminForm,
    db_path: &PathBuf,
    lng: Lang,
) -> Result<Option<AdminForm>, (AdminForm, FormErrors)> {
    if let Some(day) = f.action.strip_prefix("add:") {
        let mut covered = Week::empty();
        for e in f.entries.iter().filter(|e| e.day == day) {
            covered |= e.weeks;
        }
        f.entries.push(Entry {
            day: day.to_string(),
            weeks: Week::all().difference(covered),
            kind: String::new(),
        });
        return Ok(Some(f));
    }
    if let Some(spec) = f.action.strip_prefix("del:") {
        let (day, idx) = spec.split_once(':').unwrap_or(("", "0"));
        let idx = idx.parse::<usize>().unwrap_or(usize::MAX);
        let mut i = 0;
        f.entries.retain(|e| {
            if e.day != day {
                return true;
            }
            let keep = i != idx;
            i += 1;
            keep
        });
        return Ok(Some(f));
    }
    match validate_and_save(db_path, &f, lng) {
        Ok(()) => Ok(None),
        Err(errs) => Err((f, errs)),
    }
}

pub fn form_from_body(body: &[u8]) -> AdminForm {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in form_urlencoded::parse(body) {
        groups
            .entry(k.into_owned())
            .or_default()
            .push(v.into_owned());
    }
    let get = |key: &str| {
        groups
            .get(key)
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default()
    };
    let mut entries = Vec::new();
    for day in DAY_KEYS {
        for i in 0.. {
            if !groups.contains_key(&format!("{day}_type_{i}")) {
                break;
            }
            let weeks = groups
                .get(&format!("{day}_weeks_{i}"))
                .map(|vs| {
                    vs.iter().fold(Week::empty(), |acc, v| {
                        acc | v.parse::<u32>().map(week_of).unwrap_or_default()
                    })
                })
                .unwrap_or_default();
            entries.push(Entry {
                day: day.to_string(),
                weeks,
                kind: get(&format!("{day}_type_{i}")),
            });
        }
    }
    let action = if groups.contains_key("save") {
        "save".to_string()
    } else if let Some(v) = groups.get("add") {
        format!("add:{}", v[0])
    } else if let Some(v) = groups.get("del") {
        format!("del:{}", v[0])
    } else {
        String::new()
    };

    AdminForm {
        timezone: get("timezone"),
        pickup_time: get("pickup_time"),
        entries,
        action,
        default_lang: get("default_lang"),
    }
}
