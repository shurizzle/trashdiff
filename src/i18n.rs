use std::fmt::{self, Write};

use chrono::{DateTime, Datelike, NaiveDate, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    It,
    En,
}

impl Lang {
    pub fn from_req(cookie: Option<&str>, accept_language: Option<&str>, default: Lang) -> Lang {
        if let Some(c) = cookie.and_then(parse_cookie_lang) {
            return c;
        }
        if accept_language
            .map(|a| a.to_ascii_lowercase().starts_with("it"))
            .unwrap_or(false)
        {
            Lang::It
        } else {
            default
        }
    }

    pub fn from_env() -> Lang {
        match std::env::var("LANG") {
            Ok(l) if l.to_ascii_lowercase().starts_with("it") => Lang::It,
            _ => Lang::En,
        }
    }
}

impl fmt::Display for Lang {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Lang::It => "it",
            Lang::En => "en",
        })
    }
}

impl fmt::Debug for Lang {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Lang::It => "IT",
            Lang::En => "EN",
        })
    }
}

fn parse_cookie_lang(cookie: &str) -> Option<Lang> {
    cookie.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == "lang").then_some(match v {
            "it" => Lang::It,
            _ => Lang::En,
        })
    })
}

fn week_of_month(date: chrono::NaiveDate) -> u32 {
    (date.day() - 1) / 7 + 1
}

#[derive(Clone, Copy)]
pub enum T<'a> {
    TitleHome,
    TitleAdmin,
    NavHome,
    NavAdmin,
    NowOpen(&'a str),
    WindowUntil(DateTime<Tz>),
    Pause(DateTime<Tz>),
    Week(NaiveDate),
    ColDay,
    ColType,
    ColWindow,
    PickupTimeLabel,
    TzLabel,
    LangLabel,
    EmptyHint,
    Save,
    ErrTz,
    ErrTime,
    ErrIo,
    ErrLang,
    ErrType,
    ErrOverlap(&'a str, u32),
}

enum LocalizedT<'a> {
    It(T<'a>),
    En(T<'a>),
}

impl<'a> From<(Lang, T<'a>)> for LocalizedT<'a> {
    fn from((l, t): (Lang, T<'a>)) -> Self {
        match l {
            Lang::It => LocalizedT::It(t),
            Lang::En => LocalizedT::En(t),
        }
    }
}

impl LocalizedDisplay for DateTime<Tz> {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match lng {
            Lang::It => {
                let day = days(Lang::It)[day_index(self.weekday())];
                f.write_str(day)?;
                f.write_char(' ')?;
                fmt::Display::fmt(&self.format("%d/%m"), f)?;
                f.write_str(" alle ")?;
                fmt::Display::fmt(&self.format("%H:%M"), f)
            }
            Lang::En => {
                let day = days(Lang::En)[day_index(self.weekday())];
                f.write_str(day)?;
                f.write_char(' ')?;
                fmt::Display::fmt(&self.format("%d/%m"), f)?;
                f.write_str(" at ")?;
                fmt::Display::fmt(&self.format("%H:%M"), f)
            }
        }
    }
}

impl<'a> fmt::Display for LocalizedT<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocalizedT::It(t) => match t {
                T::TitleHome => f.write_str("Differenziata"),
                T::TitleAdmin => f.write_str("Backoffice"),
                T::NavHome => f.write_str("Home"),
                T::NavAdmin => f.write_str("Backoffice"),
                T::NowOpen(what) => {
                    f.write_str("Adesso puoi buttare: ")?;
                    f.write_str(what)
                }
                T::WindowUntil(dt) => {
                    f.write_str("Finestra aperta fino a ")?;
                    LocalizedDisplay::fmt(dt, Lang::It, f)?;
                    f.write_char('.')
                }
                T::Pause(dt) => {
                    f.write_str("Pausa: nessun ritiro in corso. Prossima raccolta da ")?;
                    LocalizedDisplay::fmt(dt, Lang::It, f)?;
                    f.write_char('.')
                }
                T::Week(date) => {
                    fmt::Display::fmt(&week_of_month(*date), f)?;
                    f.write_str("ª settimana")
                }
                T::ColDay => f.write_str("Giorno"),
                T::ColType => f.write_str("Tipo"),
                T::ColWindow => f.write_str("Finestra"),
                T::PickupTimeLabel => f.write_str("Ora ritiro (globale, HH:MM)"),
                T::TzLabel => f.write_str("Timezone"),
                T::LangLabel => f.write_str("Lingua di default"),
                T::EmptyHint => f.write_str(concat!(
                    "Una riga per ogni ritiro: spunta le settimane del mese (1-5) e ",
                    "scrivi il tipo. Il + duplica il giorno, il - elimina la riga. ",
                    "Campo tipo vuoto = riga ignorata.",
                )),
                T::Save => f.write_str("Salva"),
                T::ErrTz => f.write_str("timezone non valida"),
                T::ErrTime => f.write_str("orario ritiro non valido (atteso HH:MM)"),
                T::ErrIo => f.write_str("errore salvataggio database"),
                T::ErrLang => f.write_str("lingua di default non valida"),
                T::ErrType => f.write_str("tipo non compilato per le settimane spuntate"),
                T::ErrOverlap(day, which) => {
                    f.write_str("Sovrapposizione: ")?;
                    f.write_str(day)?;
                    f.write_str(" della ")?;
                    fmt::Display::fmt(&which, f)?;
                    f.write_str("ª settimana già assegnato.")
                }
            },
            LocalizedT::En(t) => match t {
                T::TitleHome => f.write_str("Waste collection"),
                T::TitleAdmin => f.write_str("Backoffice"),
                T::NavHome => f.write_str("Home"),
                T::NavAdmin => f.write_str("Backoffice"),
                T::NowOpen(what) => {
                    f.write_str("You can throw now: ")?;
                    f.write_str(what)
                }
                T::WindowUntil(dt) => {
                    f.write_str("Window open until ")?;
                    LocalizedDisplay::fmt(dt, Lang::En, f)?;
                    f.write_char('.')
                }
                T::Pause(dt) => {
                    f.write_str("Pause: no pickup running. Next pickup from ")?;
                    LocalizedDisplay::fmt(dt, Lang::En, f)?;
                    f.write_char('.')
                }
                T::Week(date) => {
                    f.write_str("Week ")?;
                    fmt::Display::fmt(&week_of_month(*date), f)
                }
                T::ColDay => f.write_str("Day"),
                T::ColType => f.write_str("Type"),
                T::ColWindow => f.write_str("Window"),
                T::PickupTimeLabel => f.write_str("Pickup time (global, HH:MM)"),
                T::TzLabel => f.write_str("Timezone"),
                T::LangLabel => f.write_str("Default language"),
                T::EmptyHint => f.write_str(concat!(
                    "One row per pickup: tick the weeks of the month (1-5) and write ",
                    "the type. The + duplicates the day, the - removes the row. Empty ",
                    "type field = row ignored.",
                )),
                T::Save => f.write_str("Save"),
                T::ErrTz => f.write_str("invalid timezone"),
                T::ErrTime => f.write_str("invalid pickup_time (expected HH:MM)"),
                T::ErrIo => f.write_str("database save error"),
                T::ErrLang => f.write_str("invalid default language"),
                T::ErrType => f.write_str("type required for selected weeks"),
                T::ErrOverlap(day, which) => {
                    f.write_str("Overlap: ")?;
                    f.write_str(day)?;
                    f.write_str(" of week ")?;
                    fmt::Display::fmt(&which, f)?;
                    f.write_str(" already assigned.")
                }
            },
        }
    }
}

pub fn days(lng: Lang) -> [&'static str; 7] {
    match lng {
        Lang::It => ["Lun", "Mar", "Mer", "Gio", "Ven", "Sab", "Dom"],
        Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    }
}

fn day_index(wd: Weekday) -> usize {
    wd.num_days_from_monday() as usize
}

pub fn days_full(lng: Lang) -> [&'static str; 7] {
    match lng {
        Lang::It => [
            "Lunedì",
            "Martedì",
            "Mercoledì",
            "Giovedì",
            "Venerdì",
            "Sabato",
            "Domenica",
        ],
        Lang::En => [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
    }
}

pub struct HtmlEscape<T>(pub T);

pub struct EscapeWriter<'a, 'b> {
    f: &'a mut fmt::Formatter<'b>,
}

impl<'a, 'b> Write for EscapeWriter<'a, 'b> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut last = 0;

        for (i, b) in s.bytes().enumerate() {
            let escaped = match b {
                b'<' => "&lt;",
                b'>' => "&gt;",
                b'&' => "&amp;",
                b'"' => "&quot;",
                b'\'' => "&#39;",
                _ => continue,
            };

            if last < i {
                self.f.write_str(&s[last..i])?;
            }
            self.f.write_str(escaped)?;
            last = i + 1;
        }

        if last < s.len() {
            self.f.write_str(&s[last..])?;
        }
        Ok(())
    }
}

impl<T: fmt::Display> fmt::Display for HtmlEscape<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut writer = EscapeWriter { f };
        write!(writer, "{}", self.0)
    }
}

pub fn esc<T: fmt::Display>(x: T) -> HtmlEscape<T> {
    HtmlEscape(x)
}

pub trait LocalizedDisplay {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<'a> LocalizedDisplay for T<'a> {
    #[inline(always)]
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&LocalizedT::from((lng, *self)), f)
    }
}

pub struct Localized<T: LocalizedDisplay>(Lang, T);

impl<T: LocalizedDisplay> fmt::Display for Localized<T> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LocalizedDisplay::fmt(&self.1, self.0, f)
    }
}

impl<T: LocalizedDisplay> From<(Lang, T)> for Localized<T> {
    #[inline(always)]
    fn from((lng, t): (Lang, T)) -> Self {
        Localized(lng, t)
    }
}

pub struct LocalizedRef<'a, T: LocalizedDisplay + 'a>(Lang, &'a T);

impl<'a, T: LocalizedDisplay + 'a> fmt::Display for LocalizedRef<'a, T> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        LocalizedDisplay::fmt(self.1, self.0, f)
    }
}

impl<'a, T: LocalizedDisplay + 'a> From<(Lang, &'a T)> for LocalizedRef<'a, T> {
    #[inline(always)]
    fn from((lng, t): (Lang, &'a T)) -> Self {
        LocalizedRef(lng, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_priority_cookie_over_accept_over_config_over_en() {
        assert_eq!(
            Lang::from_req(Some("lang=it"), Some("en-US"), Lang::En),
            Lang::It
        );
        assert_eq!(
            Lang::from_req(None, Some("it-IT"), Lang::En),
            Lang::It
        );
        assert_eq!(
            Lang::from_req(None, Some("en-US"), Lang::It),
            Lang::It
        );
        assert_eq!(
            Lang::from_req(None, Some("en-US"), Lang::En),
            Lang::En
        );
    }

    #[test]
    fn lang_roundtrips_toml() {
        #[derive(Serialize, Deserialize)]
        struct T {
            lang: Lang,
        }
        let it: T = toml::from_str("lang = \"it\"").unwrap();
        let en: T = toml::from_str("lang = \"en\"").unwrap();
        assert_eq!(it.lang, Lang::It);
        assert_eq!(en.lang, Lang::En);
        assert_eq!(
            toml::to_string(&T { lang: Lang::It }).unwrap().trim(),
            "lang = \"it\""
        );
    }
}
