use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use bitflags::bitflags;
use chrono::{DateTime, Datelike, Duration, NaiveTime, Weekday};
use chrono_tz::Tz;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Week: u8 {
        const FIRST = 0b00000001;
        const SECOND = 0b00000010;
        const THIRD = 0b00000100;
        const FOURTH = 0b00001000;
        const FIFTH = 0b00010000;
    }
}

impl Serialize for Week {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(None)?;
        if self.contains(Self::FIRST) {
            seq.serialize_element(&1u8)?;
        }
        if self.contains(Self::SECOND) {
            seq.serialize_element(&2u8)?;
        }
        if self.contains(Self::THIRD) {
            seq.serialize_element(&3u8)?;
        }
        if self.contains(Self::FOURTH) {
            seq.serialize_element(&4u8)?;
        }
        if self.contains(Self::FIFTH) {
            seq.serialize_element(&5u8)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Week {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Week;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a sequence of week numbers 1..=5")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Week, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut w = Week::empty();
                while let Some(n) = seq.next_element::<u8>()? {
                    w |= match n {
                        1 => Week::FIRST,
                        2 => Week::SECOND,
                        3 => Week::THIRD,
                        4 => Week::FOURTH,
                        5 => Week::FIFTH,
                        _ => return Err(serde::de::Error::custom("week must be in 1..=5")),
                    };
                }
                Ok(w)
            }
        }

        deserializer.deserialize_seq(Visitor)
    }
}

impl Default for Week {
    fn default() -> Self {
        Self::empty()
    }
}

pub fn week_of(n: u32) -> Week {
    match n {
        1 => Week::FIRST,
        2 => Week::SECOND,
        3 => Week::THIRD,
        4 => Week::FOURTH,
        5 => Week::FIFTH,
        _ => Week::empty(),
    }
}

pub const DAY_KEYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

pub fn day_index(wd: Weekday) -> usize {
    wd.num_days_from_monday() as usize
}

pub fn day_index_of(day: &str) -> usize {
    DAY_KEYS
        .iter()
        .position(|d| *d == day)
        .unwrap_or(DAY_KEYS.len())
}

pub fn sort_key(e: &Entry) -> (bool, u32) {
    (e.weeks.is_empty(), e.weeks.bits().trailing_zeros())
}

#[derive(Serialize, Deserialize)]
pub struct Db {
    pub timezone: String,
    pub pickup_time: String,
    #[serde(default)]
    pub schedule: Vec<Entry>,
    #[serde(default)]
    pub default_lang: Option<Lang>,
}

#[derive(Deserialize)]
pub struct DbOld {
    timezone: String,
    pickup_time: String,
    #[serde(default)]
    schedule: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub day: String,
    pub weeks: Week,
    #[serde(rename = "type")]
    pub kind: String,
}

fn default_db() -> Db {
    Db {
        timezone: "Europe/Rome".to_string(),
        pickup_time: "05:00".to_string(),
        schedule: Vec::new(),
        default_lang: None,
    }
}

fn migrate_old(old: DbOld) -> Db {
    let schedule = old
        .schedule
        .into_iter()
        .map(|(day, kind)| Entry {
            day,
            weeks: Week::all(),
            kind,
        })
        .collect();
    Db {
        timezone: old.timezone,
        pickup_time: old.pickup_time,
        schedule,
        default_lang: None,
    }
}

fn week_of_month(date: chrono::NaiveDate) -> u32 {
    (date.day() - 1) / 7 + 1
}

pub struct State {
    pub db_path: PathBuf,
    pub timezone: Tz,
    pub pickup_time: NaiveTime,
    pub schedule: Vec<Entry>,
    pub default_lang: Option<Lang>,
}

impl State {
    pub fn load(db_path: PathBuf) -> Result<State, String> {
        let db = if db_path.exists() {
            let mut f = File::open(&db_path)
                .map_err(|e| format!("cannot open {:?}: {e}", db_path))?;
            f.lock_shared()
                .map_err(|e| format!("lock {:?}: {e}", db_path))?;
            let mut raw = String::new();
            f.read_to_string(&mut raw)
                .map_err(|e| format!("cannot read {:?}: {e}", db_path))?;
            drop(f);
            match toml::from_str::<Db>(&raw) {
                Ok(db) => db,
                Err(_) => {
                    let old: DbOld = toml::from_str(&raw)
                        .map_err(|e| format!("invalid database {:?}: {e}", db_path))?;
                    let db = migrate_old(old);
                    Self::save_file(&db_path, &db)?;
                    db
                }
            }
        } else {
            let db = default_db();
            Self::save_file(&db_path, &db)?;
            db
        };
        let timezone: Tz = db
            .timezone
            .parse()
            .map_err(|e| format!("invalid timezone '{}': {e}", db.timezone))?;
        let pickup_time = NaiveTime::parse_from_str(&db.pickup_time, "%H:%M").map_err(|e| {
            format!(
                "invalid pickup_time '{}' (expected HH:MM): {e}",
                db.pickup_time
            )
        })?;
        Ok(State {
            db_path,
            timezone,
            pickup_time,
            schedule: db.schedule,
            default_lang: db.default_lang,
        })
    }

    pub fn save_file(db_path: &PathBuf, db: &Db) -> Result<(), String> {
        let raw = toml::to_string_pretty(db).map_err(|e| format!("serialization: {e}"))?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(db_path)
            .map_err(|e| format!("open {:?}: {e}", db_path))?;
        f.lock().map_err(|e| format!("lock {:?}: {e}", db_path))?;
        f.set_len(0)
            .map_err(|e| format!("write {:?}: {e}", db_path))?;
        io::Write::write_all(&mut f, raw.as_bytes())
            .map_err(|e| format!("write {:?}: {e}", db_path))?;
        f.sync_all()
            .map_err(|e| format!("sync {:?}: {e}", db_path))?;
        Ok(())
    }

    pub fn type_for(&self, date: chrono::NaiveDate) -> &str {
        let day = DAY_KEYS[day_index(date.weekday())];
        let week = week_of_month(date);
        self.schedule
            .iter()
            .find(|e| e.day == day && e.weeks.contains(week_of(week)))
            .map(|e| e.kind.as_str())
            .unwrap_or("")
    }

    pub fn boundary(&self, date: chrono::NaiveDate) -> DateTime<Tz> {
        date.and_time(self.pickup_time)
            .and_local_timezone(self.timezone)
            .earliest()
            .expect("local boundary not resolvable")
    }

    pub fn next_boundary(&self, now: DateTime<Tz>) -> (chrono::NaiveDate, Weekday, &str) {
        for i in 0..=7 {
            let date = now.date_naive() + Duration::days(i);
            let b = self.boundary(date);
            if b > now {
                return (date, date.weekday(), self.type_for(date));
            }
        }
        unreachable!()
    }
}
