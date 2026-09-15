use std::collections::HashMap;
use std::fmt::{self, Write};

use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc};
use chrono_tz::{TZ_VARIANTS, Tz};
use serde::{Serialize, ser::SerializeStruct};

use crate::i18n::{Lang, Localized, LocalizedDisplay, LocalizedRef, T, days, days_full, esc};
use crate::schedule::{DAY_KEYS, Entry, State, Week, day_index, sort_key, week_of};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub fn from_req(cookie: Option<&str>) -> Theme {
        cookie.and_then(parse_cookie_theme).unwrap_or(Theme::Auto)
    }

    pub fn next(self) -> Theme {
        match self {
            Theme::Auto => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Auto,
        }
    }

    pub fn scheme(self) -> &'static str {
        match self {
            Theme::Auto => "light dark",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        })
    }
}

fn parse_cookie_theme(cookie: &str) -> Option<Theme> {
    cookie.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == "theme").then_some(match v {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::Auto,
        })
    })
}

pub struct Page<Title: LocalizedDisplay, Body: LocalizedDisplay>(pub Title, pub Body, pub Theme);

impl<Title: LocalizedDisplay, Body: LocalizedDisplay> LocalizedDisplay for Page<Title, Body> {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let other_lang = if lng == Lang::It { Lang::En } else { Lang::It };
        let next_theme = self.2.next();
        f.write_str(concat!("<!doctype html>", "<html lang=\""))?;
        fmt::Display::fmt(&lng, f)?;
        f.write_str("\" style=\"color-scheme:")?;
        f.write_str(self.2.scheme())?;
        f.write_str(concat!(
            "\">",
            "<head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
            "<title>"
        ))?;
        fmt::Display::fmt(&esc(LocalizedRef::from((lng, &self.0))), f)?;
        f.write_str(concat!("</title>", "<style>",))?;
        f.write_str(include_str!(concat!(env!("OUT_DIR"), "/style.min.css")))?;
        let dark = include_str!(concat!(env!("OUT_DIR"), "/dark.min.css"));
        match self.2 {
            Theme::Auto => {
                f.write_str("@media(prefers-color-scheme:dark){")?;
                f.write_str(dark)?;
                f.write_str("}")?;
            }
            Theme::Dark => f.write_str(dark)?,
            Theme::Light => {}
        }
        f.write_str("</style></head><body>")?;

        f.write_str("<nav><a href=\"/\">")?;
        fmt::Display::fmt(&Localized::from((lng, T::NavHome)), f)?;
        f.write_str("</a><a href=\"/admin\">")?;
        fmt::Display::fmt(&Localized::from((lng, T::NavAdmin)), f)?;
        f.write_str("</a><span style=\"float:right\"><a href=\"/theme/")?;
        fmt::Display::fmt(&next_theme, f)?;
        f.write_str("\">")?;
        fmt::Display::fmt(&self.2, f)?;
        f.write_str("</a> <a href=\"/lang/")?;
        fmt::Display::fmt(&other_lang, f)?;
        f.write_str("\">")?;
        fmt::Debug::fmt(&lng, f)?;
        f.write_str("</a></span></nav>")?;
        fmt::Display::fmt(&LocalizedRef::from((lng, &self.1)), f)?;
        f.write_str("</body></html>")
    }
}

pub struct HomeHtml<'a>(pub &'a HomeView);

impl<'a> LocalizedDisplay for HomeHtml<'a> {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let view = self.0;
        let pickup = view.pickup_time;
        let today = Utc::now().with_timezone(&view.timezone).date_naive();

        f.write_str("<h1>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::TitleHome))), f)?;
        f.write_str("</h1>")?;

        f.write_str("<div class=\"now\"><p>")?;
        if let Some(kind) = &view.now.kind {
            f.write_str("<strong>")?;
            fmt::Display::fmt(&esc(Localized::from((lng, T::NowOpen(kind)))), f)?;
            f.write_str("</strong></p><p>")?;
            fmt::Display::fmt(
                &esc(Localized::from((lng, T::WindowUntil(view.now.until)))),
                f,
            )?;
        } else {
            fmt::Display::fmt(&esc(Localized::from((lng, T::Pause(view.now.until)))), f)?;
        }
        f.write_str(concat!("</p></div>", "<h2>"))?;

        fmt::Display::fmt(&esc(Localized::from((lng, T::Week(today)))), f)?;
        f.write_str(concat!("</h2>", "<table><tr><th>"))?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColDay))), f)?;
        f.write_str("</th><th>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColType))), f)?;
        f.write_str("</th><th>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColWindow))), f)?;
        f.write_str("</th></tr>")?;
        let dnames = days(lng);
        for i in 0..7 {
            f.write_str("<tr><td>")?;
            f.write_str(dnames[i])?;
            f.write_str("</td><td>")?;
            if let Some(kind) = &view.week[i] {
                fmt::Display::fmt(&esc(kind.as_str()), f)?;
            } else {
                f.write_char('—')?;
            }
            f.write_str("</td><td>")?;
            f.write_str(dnames[(i + 6) % 7])?;
            f.write_char(' ')?;
            fmt::Display::fmt(&pickup.format("%H:%M"), f)?;
            f.write_str(" → ")?;
            f.write_str(dnames[i])?;
            f.write_char(' ')?;
            fmt::Display::fmt(&pickup.format("%H:%M"), f)?;
            f.write_str("</td></tr>")?;
        }
        f.write_str("</table>")
    }
}

pub struct NowWindow {
    pub kind: Option<String>,
    pub until: DateTime<Tz>,
}

impl Serialize for NowWindow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("NowWindow", 2)?;
        st.serialize_field("kind", &self.kind)?;
        st.serialize_field("until", &self.until.to_rfc3339())?;
        st.end()
    }
}

pub struct HomeView {
    pub timezone: Tz,
    pub pickup_time: NaiveTime,
    pub now: NowWindow,
    pub week: [Option<String>; 7],
}

impl Serialize for HomeView {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut buf = [0u8; 5];
        let mut st = s.serialize_struct("HomeView", 4)?;
        st.serialize_field("timezone", &self.timezone)?;
        st.serialize_field("pickup_time", hhmm(&self.pickup_time, &mut buf))?;
        st.serialize_field("now", &self.now)?;
        st.serialize_field("week", &self.week)?;
        st.end()
    }
}

fn hhmm<'a>(t: &NaiveTime, buf: &'a mut [u8; 5]) -> &'a str {
    use std::io::Write;
    write!(&mut buf[..], "{}", t.format("%H:%M")).unwrap();
    std::str::from_utf8(buf).unwrap()
}

pub fn home_view(st: &State, now: DateTime<Utc>) -> HomeView {
    let now = now.with_timezone(&st.timezone);
    let pickup = st.pickup_time;
    let today = now.date_naive();
    let monday = today - Duration::days(day_index(now.weekday()) as i64);
    let kind = |d: chrono::NaiveDate| {
        let k = st.type_for(d);
        (!k.is_empty()).then(|| k.to_string())
    };
    let (open_date, _wd, _open_type) = st.next_boundary(now);
    let until = open_date
        .and_time(pickup)
        .and_local_timezone(st.timezone)
        .earliest()
        .expect("local boundary not resolvable");
    HomeView {
        timezone: st.timezone,
        pickup_time: pickup,
        now: NowWindow {
            kind: kind(open_date),
            until,
        },
        week: std::array::from_fn(|i| kind(monday + Duration::days(i as i64))),
    }
}

pub struct AdminRow {
    pub weeks: Week,
    pub kind: String,
}

pub struct AdminJson {
    pub timezone: Tz,
    pub pickup_time: NaiveTime,
    pub default_lang: Option<Lang>,
    pub schedule: [Vec<AdminRow>; 7],
}

impl Serialize for AdminRow {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("AdminRow", 2)?;
        st.serialize_field("weeks", &self.weeks)?;
        st.serialize_field("type", &self.kind)?;
        st.end()
    }
}

impl Serialize for AdminJson {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut buf = [0u8; 5];
        let mut st = s.serialize_struct("AdminJson", 4)?;
        st.serialize_field("timezone", &self.timezone)?;
        st.serialize_field("pickup_time", hhmm(&self.pickup_time, &mut buf))?;
        st.serialize_field("default_lang", &self.default_lang)?;
        st.serialize_field("schedule", &self.schedule)?;
        st.end()
    }
}

pub fn admin_json(st: &State) -> AdminJson {
    let rows_for = |day: &str| {
        let mut rows: Vec<AdminRow> = st
            .schedule
            .iter()
            .filter(|e| e.day == day)
            .map(|e| AdminRow {
                weeks: e.weeks,
                kind: e.kind.clone(),
            })
            .collect();
        rows.sort_by_key(|r| r.weeks.bits().trailing_zeros());
        rows
    };
    AdminJson {
        timezone: st.timezone,
        pickup_time: st.pickup_time,
        default_lang: st.default_lang,
        schedule: std::array::from_fn(|i| rows_for(DAY_KEYS[i])),
    }
}

pub fn admin_form_from_state(st: &State) -> AdminForm {
    AdminForm {
        timezone: st.timezone.to_string(),
        pickup_time: st.pickup_time.format("%H:%M").to_string(),
        entries: st.schedule.clone(),
        action: String::new(),
        default_lang: st.default_lang.map(|l| l.to_string()).unwrap_or_default(),
    }
}

pub struct AdminForm {
    pub timezone: String,
    pub pickup_time: String,
    pub entries: Vec<Entry>,
    pub action: String,
    pub default_lang: String,
}

#[derive(Debug, Default)]
pub struct FormErrors {
    pub fields: HashMap<String, String>,
    pub bad_weeks: HashMap<(&'static str, usize), Week>,
}

fn overlap_pattern(lng: Lang) -> &'static str {
    match lng {
        Lang::It => "Sovrapposizione: %s della %dª settimana già assegnato.",
        Lang::En => "Overlap: %s of week %d already assigned.",
    }
}

fn js_json_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '<' => f.write_str("\\u003c")?,
            '>' => f.write_str("\\u003e")?,
            '&' => f.write_str("\\u0026")?,
            _ => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

fn write_admin_i18n_json(lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str("const I18N={\"errTime\":")?;
    js_json_str(f, &Localized::from((lng, T::ErrTime)).to_string())?;
    f.write_str(",\"errType\":")?;
    js_json_str(f, &Localized::from((lng, T::ErrType)).to_string())?;
    f.write_str(",\"errOverlap\":")?;
    js_json_str(f, overlap_pattern(lng))?;
    f.write_str(",\"days\":{")?;
    let full = days_full(lng);
    for (i, day) in DAY_KEYS.iter().enumerate() {
        if i > 0 {
            f.write_char(',')?;
        }
        js_json_str(f, day)?;
        f.write_char(':')?;
        js_json_str(f, full[i])?;
    }
    f.write_str("}};")
}

fn row_html<'a>(
    day: &'a str,
    idx: usize,
    e: &'a Entry,
    errs: &'a FormErrors,
) -> impl fmt::Display + 'a {
    struct RowHtml<'a> {
        day: &'a str,
        idx: usize,
        e: &'a Entry,
        errs: &'a FormErrors,
    }

    impl<'a> fmt::Display for RowHtml<'a> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let bad = self
                .errs
                .bad_weeks
                .get(&(self.day, self.idx))
                .copied()
                .unwrap_or_default();
            f.write_str("<p class=\"row\" data-day=\"")?;
            fmt::Display::fmt(&self.day, f)?;
            f.write_str("\"><span class=\"weeks\">")?;
            for w in 1..=5 {
                f.write_str("<input type=\"checkbox\" id=\"")?;
                fmt::Display::fmt(&self.day, f)?;
                f.write_str("_w")?;
                fmt::Display::fmt(&self.idx, f)?;
                f.write_char('_')?;
                fmt::Display::fmt(&w, f)?;
                f.write_str("\" name=\"")?;
                fmt::Display::fmt(&self.day, f)?;
                f.write_str("_weeks_")?;
                fmt::Display::fmt(&self.idx, f)?;
                f.write_str("\" value=\"")?;
                fmt::Display::fmt(&w, f)?;
                f.write_char('"')?;
                if self.e.weeks.contains(week_of(w)) {
                    f.write_str(" checked")?;
                }
                if bad.contains(week_of(w)) {
                    f.write_str(" class=\"bad\"")?;
                }
                f.write_char('>')?;

                f.write_str("<label for=\"")?;
                fmt::Display::fmt(&self.day, f)?;
                f.write_str("_w")?;
                fmt::Display::fmt(&self.idx, f)?;
                f.write_char('_')?;
                fmt::Display::fmt(&w, f)?;
                f.write_char('"')?;
                if bad.contains(week_of(w)) {
                    f.write_str(" class=\"bad\"")?;
                }
                f.write_char('>')?;
                fmt::Display::fmt(&w, f)?;
                f.write_str("</label>")?;
            }
            f.write_str("</span> <span class=\"field\">")?;
            f.write_str("<input type=\"text\" name=\"")?;
            fmt::Display::fmt(&self.day, f)?;
            f.write_str("_type_")?;
            fmt::Display::fmt(&self.idx, f)?;
            f.write_str("\" value=\"")?;
            fmt::Display::fmt(&esc(&self.e.kind), f)?;
            f.write_str("\">")?;
            f.write_str("<button type=\"submit\" name=\"del\" value=\"")?;
            fmt::Display::fmt(&self.day, f)?;
            f.write_char(':')?;
            fmt::Display::fmt(&self.idx, f)?;
            f.write_str("\">-</button></span></p>")?;
            if !self.errs.fields.is_empty() {
                let row_key = format!("{}:{}", self.day, self.idx);
                if let Some(e) = self.errs.fields.get(&row_key) {
                    f.write_str("<span class=\"err\">")?;
                    fmt::Display::fmt(&esc(e), f)?;
                    f.write_str("</span>")?;
                }
            }
            Ok(())
        }
    }

    RowHtml { day, idx, e, errs }
}

pub struct AdminFormHtml(pub AdminForm, pub FormErrors);

impl LocalizedDisplay for AdminFormHtml {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<h1>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::TitleAdmin))), f)?;
        f.write_str("</h1>")?;
        if let Some(form_err) = self.1.fields.get("form") {
            f.write_str("<p class=\"err\">")?;
            fmt::Display::fmt(&esc(form_err), f)?;
            f.write_str("</p>")?;
        }
        f.write_str("<form method=\"post\" action=\"/admin\" id=\"admin-form\">")?;
        f.write_str("<p><label>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::PickupTimeLabel))), f)?;
        f.write_str("<br><input type=\"text\" name=\"pickup_time\" value=\"")?;
        fmt::Display::fmt(&esc(&self.0.pickup_time), f)?;
        f.write_str("\"></label>")?;
        if let Some(err) = self.1.fields.get("pickup_time") {
            f.write_str("<span class=\"err\">")?;
            fmt::Display::fmt(&esc(err), f)?;
            f.write_str("</span>")?;
        }
        f.write_str("<p><label>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::TzLabel))), f)?;
        f.write_str("<br><select name=\"timezone\">")?;
        for tz in TZ_VARIANTS.iter() {
            f.write_str("<option value=\"")?;
            fmt::Display::fmt(&esc(tz), f)?;
            f.write_str("\"")?;
            if tz.name() == self.0.timezone {
                f.write_str(" selected")?;
            }
            f.write_char('>')?;
            fmt::Display::fmt(&esc(tz), f)?;
            f.write_str("</option>")?;
        }
        f.write_str("</select></label>")?;
        if let Some(err) = self.1.fields.get("timezone") {
            f.write_str("<span class=\"err\">")?;
            fmt::Display::fmt(&esc(err), f)?;
            f.write_str("</span>")?;
        }
        f.write_str("</p>")?;
        f.write_str("<p><label>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::LangLabel))), f)?;
        f.write_str("<br><select name=\"default_lang\">")?;
        f.write_str("<option value=\"\"")?;
        if self.0.default_lang.is_empty() {
            f.write_str(" selected")?;
        }
        f.write_str(">Auto (en)</option>")?;
        f.write_str("<option value=\"it\"")?;
        if self.0.default_lang == "it" {
            f.write_str(" selected")?;
        }
        f.write_str(">Italiano</option>")?;
        f.write_str("<option value=\"en\"")?;
        if self.0.default_lang == "en" {
            f.write_str(" selected")?;
        }
        f.write_str(">English</option>")?;
        f.write_str("</select></label>")?;
        if let Some(err) = self.1.fields.get("default_lang") {
            f.write_str("<span class=\"err\">")?;
            fmt::Display::fmt(&esc(err), f)?;
            f.write_str("</span>")?;
        }
        f.write_str("</p>")?;

        let full = days_full(lng);
        for (di, day) in DAY_KEYS.iter().enumerate() {
            let mut day_entries: Vec<&Entry> =
                self.0.entries.iter().filter(|e| e.day == *day).collect();
            day_entries.sort_by_key(|e| sort_key(e));

            f.write_str("<h3>")?;
            f.write_str(full[di])?;
            f.write_str("<button type=\"submit\" class=\"add-btn\" name=\"add\" value=\"")?;
            f.write_str(day)?;
            f.write_str("\">+</button></h3>")?;

            for (idx, e) in day_entries.iter().enumerate() {
                fmt::Display::fmt(&row_html(day, idx, e, &self.1), f)?;
            }
        }

        f.write_str("<p><em>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::EmptyHint))), f)?;
        f.write_str(concat!(
            "</em></p>",
            "<p><button type=\"submit\" name=\"save\">"
        ))?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::Save))), f)?;
        f.write_str(concat!("</button></p>", "</form>"))?;
        f.write_str("<script>")?;
        write_admin_i18n_json(lng, f)?;
        f.write_str(include_str!(concat!(env!("OUT_DIR"), "/admin.min.js")))?;
        f.write_str("</script>")
    }
}
