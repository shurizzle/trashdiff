use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write};
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use actix_web::http::header::{COOKIE, LOCATION};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use bytes::Bytes;
use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc, Weekday};
use chrono_tz::{TZ_VARIANTS, Tz};
use clap::{Parser, Subcommand};
use http::{Method, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use serde::{Deserialize, Serialize};

mod i18n;

use i18n::{Lang, days, days_full};

use crate::i18n::{Localized, LocalizedDisplay, LocalizedRef, T, esc};

const DAY_KEYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

fn day_index(wd: Weekday) -> usize {
    wd.num_days_from_monday() as usize
}

fn day_index_of(day: &str) -> usize {
    DAY_KEYS
        .iter()
        .position(|d| *d == day)
        .unwrap_or(DAY_KEYS.len())
}

fn sort_key(e: &Entry) -> (bool, u32) {
    (
        e.weeks.is_empty(),
        e.weeks.iter().min().copied().unwrap_or(0),
    )
}

fn lang_of(req: &HttpRequest) -> Lang {
    let cookie = req
        .headers()
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let al = req
        .headers()
        .get(actix_web::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    Lang::from_req(cookie.as_deref(), al.as_deref())
}

#[derive(Serialize, Deserialize)]
struct Db {
    timezone: String,
    pickup_time: String,
    #[serde(default)]
    schedule: Vec<Entry>,
}

#[derive(Deserialize)]
struct DbOld {
    timezone: String,
    pickup_time: String,
    #[serde(default)]
    schedule: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    day: String,
    weeks: Vec<u32>,
    #[serde(rename = "type")]
    kind: String,
}

fn default_db() -> Db {
    Db {
        timezone: "Europe/Rome".to_string(),
        pickup_time: "05:00".to_string(),
        schedule: Vec::new(),
    }
}

fn migrate_old(old: DbOld) -> Db {
    let schedule = old
        .schedule
        .into_iter()
        .map(|(day, kind)| Entry {
            day,
            weeks: vec![1, 2, 3, 4, 5],
            kind,
        })
        .collect();
    Db {
        timezone: old.timezone,
        pickup_time: old.pickup_time,
        schedule,
    }
}

fn week_of_month(date: chrono::NaiveDate) -> u32 {
    (date.day() - 1) / 7 + 1
}

struct State {
    db_path: PathBuf,
    timezone: Tz,
    pickup_time: NaiveTime,
    schedule: Vec<Entry>,
}

impl State {
    fn load(db_path: PathBuf) -> Result<State, String> {
        let db = if db_path.exists() {
            let mut f = File::open(&db_path)
                .map_err(|e| format!("impossibile aprire {:?}: {e}", db_path))?;
            f.lock_shared()
                .map_err(|e| format!("lock {:?}: {e}", db_path))?;
            let mut raw = String::new();
            f.read_to_string(&mut raw)
                .map_err(|e| format!("impossibile leggere {:?}: {e}", db_path))?;
            drop(f);
            match toml::from_str::<Db>(&raw) {
                Ok(db) => db,
                Err(_) => {
                    let old: DbOld = toml::from_str(&raw)
                        .map_err(|e| format!("database {:?} non valido: {e}", db_path))?;
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
            .map_err(|e| format!("timezone '{}' non valida: {e}", db.timezone))?;
        let pickup_time = NaiveTime::parse_from_str(&db.pickup_time, "%H:%M").map_err(|e| {
            format!(
                "pickup_time '{}' non valido (atteso HH:MM): {e}",
                db.pickup_time
            )
        })?;
        Ok(State {
            db_path,
            timezone,
            pickup_time,
            schedule: db.schedule,
        })
    }

    fn save_file(db_path: &PathBuf, db: &Db) -> Result<(), String> {
        let raw = toml::to_string_pretty(db).map_err(|e| format!("serializzazione: {e}"))?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(db_path)
            .map_err(|e| format!("apertura {:?}: {e}", db_path))?;
        f.lock().map_err(|e| format!("lock {:?}: {e}", db_path))?;
        f.set_len(0)
            .map_err(|e| format!("scrittura {:?}: {e}", db_path))?;
        io::Write::write_all(&mut f, raw.as_bytes())
            .map_err(|e| format!("scrittura {:?}: {e}", db_path))?;
        f.sync_all()
            .map_err(|e| format!("sync {:?}: {e}", db_path))?;
        Ok(())
    }

    fn type_for(&self, date: chrono::NaiveDate) -> String {
        let day = DAY_KEYS[day_index(date.weekday())];
        let week = week_of_month(date);
        self.schedule
            .iter()
            .find(|e| e.day == day && e.weeks.contains(&week))
            .map(|e| e.kind.clone())
            .unwrap_or_default()
    }

    fn boundary(&self, date: chrono::NaiveDate) -> DateTime<Tz> {
        date.and_time(self.pickup_time)
            .and_local_timezone(self.timezone)
            .earliest()
            .expect("boundary locale non risolvibile")
    }

    fn next_boundary(&self, now: DateTime<Tz>) -> (chrono::NaiveDate, Weekday, String) {
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

struct AppState(PathBuf);

#[allow(clippy::result_large_err)]
fn load_or_500(data: &AppState) -> Result<State, HttpResponse> {
    State::load(data.0.clone()).map_err(|e| {
        HttpResponse::InternalServerError()
            .content_type("text/plain; charset=utf-8")
            .body(e)
    })
}

struct Page<Title: LocalizedDisplay, Body: LocalizedDisplay>(Title, Body);

impl<Title: LocalizedDisplay, Body: LocalizedDisplay> LocalizedDisplay for Page<Title, Body> {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let other_lang = if lng == Lang::It { Lang::En } else { Lang::It };
        f.write_str(concat!("<!doctype html>", "<html lang=\""))?;
        fmt::Display::fmt(&lng, f)?;
        f.write_str(concat!(
            "\" style=\"color-scheme:light dark\">",
            "<head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">",
            "<title>"
        ))?;
        fmt::Display::fmt(&esc(LocalizedRef::from((lng, &self.0))), f)?;
        f.write_str(concat!(
            "</title>",
            "<style>",
            "body{font-family:system-ui,sans-serif;max-width:640px;margin:2rem auto;padding:0 1rem;color:#222}",
            "table{border-collapse:collapse;width:100%}",
            "th,td{border:1px solid #ccc;padding:.4rem .6rem;text-align:left}",
            "th{background:#f2f2f2}",
            ".now{background:#e7f6e7;padding:.8rem;border-radius:.5rem;margin:1rem 0}",
            "nav a{margin-right:1rem}",
            ".err{background:#fdecea;color:#b00020;padding:.6rem;border-radius:.5rem}",
            "input[type=text]{width:100%;box-sizing:border-box;padding:.4rem}",
            ".field{display:flex}",
            ".field input[type=text]{flex:1;width:auto;border-right:none;border-radius:.3rem 0 0 .3rem}",
            ".field button{border-radius:0 .3rem .3rem 0}",
            ".add-btn{padding:.05rem .5rem;font-size:.75rem;vertical-align:middle}",
            ".weeks input[type=checkbox]{position:absolute;opacity:0;pointer-events:none}",
            ".weeks label{display:inline-block;padding:.15rem .55rem;margin:0 .2rem .2rem 0;border-radius:.3rem;cursor:pointer;background:#888;color:#fff;font-size:.9rem}",
            ".weeks input:checked + label{background:#2e7d32}",
            ".weeks input:checked + label.bad{background:#c62828}",
            "form p{margin:.6rem 0}",
            "button{padding:.5rem 1.2rem;cursor:pointer}",
            "@media (prefers-color-scheme:dark) {",
            "body{color:#ddd;background:#1e1e1e}",
            "th,td{border-color:#444}",
            "th{background:#2a2a2a}",
            ".now{background:#1e3a2b}",
            ".err{background:#4a1f1f;color:#ffb4ab}",
            "a{color:#8ab4f8}",
            "}",
            "</style></head><body>",
        ))?;

        f.write_str("<nav><a href=\"/\">")?;
        fmt::Display::fmt(&Localized::from((lng, T::NavHome)), f)?;
        f.write_str("</a><a href=\"/admin\">")?;
        fmt::Display::fmt(&Localized::from((lng, T::NavAdmin)), f)?;
        f.write_str(concat!(
            "</a>",
            "<span style=\"float:right\"><a href=\"/lang/"
        ))?;
        fmt::Display::fmt(&other_lang, f)?;
        f.write_str("\">")?;
        fmt::Debug::fmt(&other_lang, f)?;
        f.write_str("</a></span></nav>")?;
        fmt::Display::fmt(&LocalizedRef::from((lng, &self.1)), f)?;
        f.write_str("</body></html>")
    }
}

pub struct HomeHtml<'a>(&'a State);

impl<'a> LocalizedDisplay for HomeHtml<'a> {
    fn fmt(&self, lng: Lang, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let now = Utc::now().with_timezone(&self.0.timezone);
        let pickup = self.0.pickup_time;
        let (open_date, _owd, open_type) = self.0.next_boundary(now);
        let today_type = self.0.type_for(now.date_naive());

        let open_dt = open_date
            .and_time(pickup)
            .and_local_timezone(self.0.timezone)
            .earliest()
            .expect("boundary locale non risolvibile");

        f.write_str("<h1>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::TitleHome))), f)?;
        f.write_str("</h1>")?;

        f.write_str("<div class=\"now\"><p>")?;
        if open_type.is_empty() {
            fmt::Display::fmt(&esc(Localized::from((lng, T::Pause(open_dt)))), f)?;
        } else {
            f.write_str("<strong>")?;
            fmt::Display::fmt(&esc(Localized::from((lng, T::NowOpen(&open_type)))), f)?;
            f.write_str("</strong></p><p>")?;
            fmt::Display::fmt(&esc(Localized::from((lng, T::WindowUntil(open_dt)))), f)?;
        }
        f.write_str("</p></div>")?;

        f.write_str("<p>")?;
        if today_type.is_empty() {
            fmt::Display::fmt(
                &esc(Localized::from((lng, T::TodayNone(self.0.timezone)))),
                f,
            )?;
        } else {
            fmt::Display::fmt(
                &esc(Localized::from((
                    lng,
                    T::TodayPickup(self.0.timezone, &today_type, pickup),
                ))),
                f,
            )?;
        }
        f.write_str(concat!("</p>", "<h2>"))?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::Week(now.date_naive())))), f)?;
        f.write_str(concat!("</h2>", "<table><tr><th>"))?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColDay))), f)?;
        f.write_str("</th><th>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColType))), f)?;
        f.write_str("</th><th>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::ColWindow))), f)?;
        f.write_str("</th></tr>")?;
        let dnames = days(lng);
        let monday = now.date_naive() - Duration::days(day_index(now.weekday()) as i64);
        for i in 0..7 {
            let d = monday + Duration::days(i);
            let ty = self.0.type_for(d);
            let wi = day_index(d.weekday());

            f.write_str("<tr><td>")?;
            f.write_str(dnames[wi])?;
            f.write_str("</td><td>")?;
            if ty.is_empty() {
                f.write_char('—')?;
            } else {
                fmt::Display::fmt(&esc(&ty), f)?;
            }
            f.write_str("</td><td>")?;
            f.write_str(dnames[(wi + 6) % 7])?;
            f.write_char(' ')?;
            fmt::Display::fmt(&pickup.format("%H:%M"), f)?;
            f.write_str(" → ")?;
            f.write_str(dnames[wi])?;
            f.write_char(' ')?;
            fmt::Display::fmt(&pickup.format("%H:%M"), f)?;
            f.write_str("</td></tr>")?;
        }
        f.write_str("</table>")
    }
}

async fn home(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let lng = lang_of(&req);
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(Localized::from((lng, Page(T::TitleHome, HomeHtml(&st)))).to_string())
}

fn admin_form_from_state(st: &State) -> AdminForm {
    AdminForm {
        timezone: st.timezone.to_string(),
        pickup_time: st.pickup_time.format("%H:%M").to_string(),
        entries: st.schedule.clone(),
        action: String::new(),
    }
}

async fn admin_get(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let lng = lang_of(&req);
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(
            Localized::from((
                lng,
                Page(
                    T::TitleAdmin,
                    AdminFormHtml(admin_form_from_state(&st), FormErrors::default()),
                ),
            ))
            .to_string(),
        )
}

struct AdminForm {
    timezone: String,
    pickup_time: String,
    entries: Vec<Entry>,
    action: String,
}

#[derive(Default)]
struct FormErrors {
    fields: HashMap<String, String>,
    bad_weeks: HashMap<String, Vec<u32>>,
}

fn row_html2<'a>(
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
            let row_key = format!("{}:{}", self.day, self.idx);
            let bad = self
                .errs
                .bad_weeks
                .get(&row_key)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            f.write_str("<p><span class=\"weeks\">")?;
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
                if self.e.weeks.contains(&w) {
                    f.write_str(" checked")?;
                }
                if bad.contains(&w) {
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
                if bad.contains(&w) {
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
            if let Some(e) = self.errs.fields.get(&row_key) {
                f.write_str("<span class=\"err\">")?;
                fmt::Display::fmt(&esc(e), f)?;
                f.write_str("</span>")?;
            }
            Ok(())
        }
    }

    RowHtml { day, idx, e, errs }
}

struct AdminFormHtml(AdminForm, FormErrors);

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
        f.write_str("<form method=\"post\" action=\"/admin\">")?;
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
            fmt::Display::fmt(&esc(tz), f)?;
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
                fmt::Display::fmt(&row_html2(day, idx, e, &self.1), f)?;
            }
        }

        f.write_str("<p><em>")?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::EmptyHint))), f)?;
        f.write_str(concat!(
            "</em></p>",
            "<p><button type=\"submit\" name=\"save\">"
        ))?;
        fmt::Display::fmt(&esc(Localized::from((lng, T::Save))), f)?;
        f.write_str(concat!("</button></p>", "</form>"))
    }
}

fn validate_and_save(db_path: &PathBuf, f: &AdminForm, lng: Lang) -> Result<(), FormErrors> {
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
    let mut schedule: Vec<Entry> = Vec::new();
    let mut seen: HashSet<(String, u32)> = HashSet::new();
    for day in DAY_KEYS {
        let mut day_entries: Vec<&Entry> = f.entries.iter().filter(|e| e.day == *day).collect();
        day_entries.sort_by_key(|e| sort_key(e));
        for (idx, e) in day_entries.iter().enumerate() {
            if !e.kind.trim().is_empty() {
                let di = day_index_of(day);
                let mut weeks: Vec<u32> = e
                    .weeks
                    .iter()
                    .copied()
                    .filter(|w| (1..=5).contains(w))
                    .collect();
                weeks.sort_unstable();
                weeks.dedup();
                if !weeks.is_empty() {
                    for w in &weeks {
                        if !seen.insert((e.day.clone(), *w)) {
                            let key = format!("{day}:{idx}");
                            errs.fields.entry(key.clone()).or_insert_with(|| {
                                Localized::from((lng, T::ErrOverlap(days_full(lng)[di], *w)))
                                    .to_string()
                            });
                            errs.bad_weeks.entry(key).or_default().push(*w);
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
    }
    if !errs.fields.is_empty() || !errs.bad_weeks.is_empty() {
        return Err(errs);
    }
    let db = Db {
        timezone: f.timezone.to_string(),
        pickup_time: f.pickup_time.to_string(),
        schedule,
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
fn process_admin(
    mut f: AdminForm,
    db_path: &PathBuf,
    lng: Lang,
) -> Result<Option<AdminForm>, (AdminForm, FormErrors)> {
    if let Some(day) = f.action.strip_prefix("add:") {
        let mut covered: HashSet<u32> = HashSet::new();
        for e in f.entries.iter().filter(|e| e.day == day) {
            covered.extend(e.weeks.iter().copied());
        }
        let weeks: Vec<u32> = (1..=5).filter(|w| !covered.contains(w)).collect();
        f.entries.push(Entry {
            day: day.to_string(),
            weeks,
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

async fn admin_post(req: HttpRequest, data: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let lng = lang_of(&req);
    let f = form_from_body(&body);
    match process_admin(f, &data.0, lng) {
        Ok(Some(form)) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(
                Localized::from((
                    lng,
                    Page(T::TitleAdmin, AdminFormHtml(form, FormErrors::default())),
                ))
                .to_string(),
            ),
        Ok(None) => HttpResponse::SeeOther()
            .insert_header((LOCATION, "/"))
            .finish(),
        Err((form, errs)) => HttpResponse::BadRequest()
            .content_type("text/html; charset=utf-8")
            .body(
                Localized::from((lng, Page(T::TitleAdmin, AdminFormHtml(form, errs)))).to_string(),
            ),
    }
}

async fn switch_lang(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let code = path.into_inner();
    let lng = if code == "en" { "en" } else { "it" };
    let back = req
        .headers()
        .get(actix_web::http::header::REFERER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("/")
        .to_string();
    HttpResponse::SeeOther()
        .insert_header((LOCATION, back))
        .insert_header((
            actix_web::http::header::SET_COOKIE,
            format!("lang={lng}; Path=/; Max-Age=31536000; SameSite=Lax"),
        ))
        .finish()
}

#[derive(Parser)]
#[command(
    name = "trashdiff",
    version,
    about = "Weekly waste collection schedule"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server
    Http {
        /// Bind address (host:port)
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Show what to throw right now
    Cli {
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Serve a single request over CGI (e.g. from Apache/nginx)
    Cgi {
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Run a FastCGI server (e.g. nginx fastcgi_pass)
    Fcgi {
        /// Bind address (host:port)
        #[arg(long, default_value = "127.0.0.1:9000")]
        bind: String,
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Run an SCGI server (e.g. nginx mod_scgi)
    Scgi {
        /// Bind address (host:port)
        #[arg(long, default_value = "127.0.0.1:4000")]
        bind: String,
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
}

fn main() -> std::io::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let no_real_args = argv.iter().skip(1).all(|a| a.trim().is_empty());
    if no_real_args
        && std::env::var("GATEWAY_INTERFACE")
            .map(|g| g.starts_with("CGI/"))
            .unwrap_or(false)
    {
        let db = std::env::var("TRASHDIFF_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("trashdb.toml"));
        let rt = tokio::runtime::Runtime::new()?;
        return rt.block_on(cgi_run(db));
    }
    let cli = Cli::parse();
    match cli.command {
        Command::Cli { db } => match cli_cmd(db) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Command::Http { bind, db } => {
            let rt = actix_web::rt::Runtime::new()?;
            rt.block_on(serve(bind, db))
        }
        Command::Cgi { db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(cgi_run(db))
        }
        Command::Fcgi { bind, db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(fcgi_run(bind, db))
        }
        Command::Scgi { bind, db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(scgi_run(bind, db))
        }
    }
}

fn cli_cmd(db_path: PathBuf) -> Result<(), String> {
    let st = State::load(db_path)?;
    let lng = Lang::from_env();
    let now = Utc::now().with_timezone(&st.timezone);
    let (open_date, _wd, open_type) = st.next_boundary(now);
    let open_dt = open_date
        .and_time(st.pickup_time)
        .and_local_timezone(st.timezone)
        .earliest()
        .expect("boundary locale non risolvibile");
    if open_type.is_empty() {
        println!("{}", Localized::from((lng, T::Pause(open_dt))));
    } else {
        println!(
            "{} ({})",
            Localized::from((lng, T::NowOpen(&open_type))),
            Localized::from((lng, T::WindowUntil(open_dt)))
        );
    }
    Ok(())
}

async fn serve(bind: String, db_path: PathBuf) -> std::io::Result<()> {
    let data = web::Data::new(AppState(db_path));
    println!("trashdiff listening on http://{bind}");
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .route("/", web::get().to(home))
            .route("/admin", web::get().to(admin_get))
            .route("/admin", web::post().to(admin_post))
            .route("/lang/{code}", web::get().to(switch_lang))
    })
    .bind(bind)?
    .run()
    .await
}

fn lang_from_headers(h: &http::HeaderMap) -> Lang {
    let cookie = h.get(http::header::COOKIE).and_then(|v| v.to_str().ok());
    let al = h
        .get(http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    Lang::from_req(cookie, al)
}

fn form_from_body(body: &[u8]) -> AdminForm {
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
                .map(|vs| vs.iter().filter_map(|w| w.parse::<u32>().ok()).collect())
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
    }
}

fn boxed_body(b: Bytes) -> BoxBody<Bytes, std::io::Error> {
    Full::new(b)
        .map_err(|_| std::io::Error::other("body error"))
        .boxed()
}

fn respond(status: StatusCode, html: String) -> http::Response<BoxBody<Bytes, std::io::Error>> {
    http::Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(boxed_body(Bytes::from(html)))
        .unwrap()
}

fn redirect(
    location: &str,
    cookie: Option<&str>,
) -> http::Response<BoxBody<Bytes, std::io::Error>> {
    let mut b = http::Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("Location", location);
    if let Some(c) = cookie {
        b = b.header("Set-Cookie", c);
    }
    b.body(boxed_body(Bytes::new())).unwrap()
}

fn route_cgi(
    st: &State,
    lng: Lang,
    method: &Method,
    path: &str,
    headers: &http::HeaderMap,
    body: Bytes,
) -> http::Response<BoxBody<Bytes, std::io::Error>> {
    let path = path.trim_end_matches('/');
    if path.ends_with("/lang/en") || path.ends_with("/lang/it") {
        let code = if path.ends_with("/lang/en") {
            "en"
        } else {
            "it"
        };
        let back = headers
            .get(http::header::REFERER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("/");
        return redirect(
            back,
            Some(&format!(
                "lang={code}; Path=/; Max-Age=31536000; SameSite=Lax"
            )),
        );
    }
    if path == "/admin" || path.ends_with("/admin") {
        if *method == Method::POST {
            let f = form_from_body(&body);
            return match process_admin(f, &st.db_path, lng) {
                Ok(Some(form)) => {
                    let html = Localized::from((
                        lng,
                        Page(T::TitleAdmin, AdminFormHtml(form, FormErrors::default())),
                    ))
                    .to_string();
                    respond(StatusCode::OK, html)
                }
                Ok(None) => redirect("/", None),
                Err((form, errs)) => {
                    let html =
                        Localized::from((lng, Page(T::TitleAdmin, AdminFormHtml(form, errs))))
                            .to_string();
                    respond(StatusCode::BAD_REQUEST, html)
                }
            };
        }
        let html = Localized::from((
            lng,
            Page(
                T::TitleAdmin,
                AdminFormHtml(admin_form_from_state(st), FormErrors::default()),
            ),
        ))
        .to_string();
        return respond(StatusCode::OK, html);
    }
    let html = Localized::from((lng, Page(T::TitleHome, HomeHtml(st)))).to_string();
    respond(StatusCode::OK, html)
}

async fn cgi_run(db: PathBuf) -> std::io::Result<()> {
    let st = State::load(db).map_err(std::io::Error::other)?;
    let has_body = std::env::var("CONTENT_LENGTH")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        > 0;
    // cegla reads stdin to EOF; without a request body the gateway keeps the
    // pipe open forever, so feed an empty stream unless a body is expected.
    let stdin: Box<dyn tokio::io::AsyncRead + Unpin> = if has_body {
        Box::new(tokio::io::stdin())
    } else {
        Box::new(tokio::io::empty())
    };
    cegla_cgi::server::handle_request(
        stdin,
        tokio::io::stdout(),
        tokio::io::stderr(),
        move |request, _stderr| async move {
            let method = request.method().clone();
            let path = std::env::var("PATH_INFO").unwrap_or_default();
            let path = if path.is_empty() {
                request.uri().path().to_string()
            } else {
                path
            };
            let headers = request.headers().clone();
            let body = request
                .into_body()
                .collect()
                .await
                .map_err(std::io::Error::other)?
                .to_bytes();
            let lng = lang_from_headers(&headers);
            let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> =
                Ok(route_cgi(&st, lng, &method, &path, &headers, body));
            resp
        },
    )
    .await
}

struct TokioRt;

impl cegla_fcgi::server::Runtime for TokioRt {
    fn spawn(&self, future: impl std::future::Future + Send + 'static) {
        tokio::spawn(async move {
            future.await;
        });
    }
}

async fn read_body_capped<B>(mut body: B, content_length: usize) -> Result<Bytes, std::io::Error>
where
    B: BodyExt + Unpin,
    B::Data: AsRef<[u8]>,
    B::Error: Into<std::io::Error>,
{
    if content_length == 0 {
        return Ok(Bytes::new());
    }
    let mut buf = Vec::with_capacity(content_length);
    while buf.len() < content_length {
        let frame = match body.frame().await {
            Some(Ok(f)) => f,
            Some(Err(e)) => return Err(e.into()),
            None => break,
        };
        if let Ok(data) = frame.into_data() {
            let chunk = data.as_ref();
            let need = content_length - buf.len();
            buf.extend_from_slice(&chunk[..chunk.len().min(need)]);
        }
    }
    // cegla-fcgi keeps the stdin channel open past the last chunk; dropping the
    // body avoids waiting for an EOF that never comes.
    drop(body);
    Ok(Bytes::from(buf))
}

async fn fcgi_run(bind: String, db: PathBuf) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("trashdiff fcgi listening on {bind}");
    loop {
        let (stream, _) = listener.accept().await?;
        let db = db.clone();
        tokio::spawn(async move {
            let _ =
                cegla_fcgi::server::server_handle_fcgi(stream, TokioRt, move |request, _stderr| {
                    let db = db.clone();
                    async move {
                        let method = request.method().clone();
                        let path = request.uri().path().to_string();
                        let headers = request.headers().clone();
                        let content_length = headers
                            .get(http::header::CONTENT_LENGTH)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(0);
                        let body = read_body_capped(request.into_body(), content_length).await?;
                        let lng = lang_from_headers(&headers);
                        let st = State::load(db).map_err(std::io::Error::other)?;
                        let resp: Result<
                            http::Response<BoxBody<Bytes, std::io::Error>>,
                            std::io::Error,
                        > = Ok(route_cgi(&st, lng, &method, &path, &headers, body));
                        resp
                    }
                })
                .await;
        });
    }
}

async fn scgi_run(bind: String, db: PathBuf) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("trashdiff scgi listening on {bind}");
    loop {
        let (stream, _) = listener.accept().await?;
        let db = db.clone();
        tokio::spawn(async move {
            let _ = cegla_scgi::server::server_handle_scgi(stream, move |request| {
                let db = db.clone();
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_string();
                    let headers = request.headers().clone();
                    let content_length = headers
                        .get(http::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    let body = read_body_capped(request.into_body(), content_length).await?;
                    let lng = lang_from_headers(&headers);
                    let st = State::load(db).map_err(std::io::Error::other)?;
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, &method, &path, &headers, body));
                    resp
                }
            })
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn state() -> State {
        let schedule = vec![
            Entry {
                day: "monday".to_string(),
                weeks: vec![1],
                kind: "Carta".to_string(),
            },
            Entry {
                day: "tuesday".to_string(),
                weeks: vec![1],
                kind: "Umido".to_string(),
            },
        ];
        State {
            db_path: PathBuf::from("/nonexistent"),
            timezone: "Europe/Rome".parse().unwrap(),
            pickup_time: NaiveTime::parse_from_str("17:00", "%H:%M").unwrap(),
            schedule,
        }
    }

    fn at(date: &str, time: &str, st: &State) -> DateTime<Tz> {
        let naive =
            chrono::NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M")
                .unwrap();
        naive.and_local_timezone(st.timezone).earliest().unwrap()
    }

    #[test]
    fn window_open_before_pickup() {
        let st = state();
        // Mon 2024-01-01: paper window open since Sun 17:00
        let (d, wd, t) = st.next_boundary(at("2024-01-01", "16:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(wd, Weekday::Mon);
        assert_eq!(t, "Carta");
    }

    #[test]
    fn window_open_sunday_evening() {
        let st = state();
        // Sunday evening: Monday's window already open
        let (d, wd, t) = st.next_boundary(at("2023-12-31", "18:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(wd, Weekday::Mon);
        assert_eq!(t, "Carta");
    }

    #[test]
    fn window_closed_after_pickup() {
        let st = state();
        // Mon 18:00: pickup passed, next window = Tuesday (Organic)
        let (d, wd, t) = st.next_boundary(at("2024-01-01", "18:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 2).unwrap());
        assert_eq!(wd, Weekday::Tue);
        assert_eq!(t, "Umido");
    }

    #[test]
    fn empty_day_is_pause() {
        let st = state();
        // Wednesday not configured -> next pickup Wednesday (empty = pause)
        let (d, wd, t) = st.next_boundary(at("2024-01-03", "10:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 3).unwrap());
        assert_eq!(wd, Weekday::Wed);
        assert_eq!(t, "");
    }

    #[test]
    fn week2_only_skips_week1() {
        let mut st = state();
        st.schedule = vec![Entry {
            day: "monday".to_string(),
            weeks: vec![2],
            kind: "Carta".to_string(),
        }];
        // 2024-01-01 is Monday of week 1: not collected -> pause
        let (d, _wd, t) = st.next_boundary(at("2024-01-01", "10:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert_eq!(t, "");
        // 2024-01-08 is Monday of week 2: collected
        let (d, _wd, t) = st.next_boundary(at("2024-01-08", "10:00", &st));
        assert_eq!(d, chrono::NaiveDate::from_ymd_opt(2024, 1, 8).unwrap());
        assert_eq!(t, "Carta");
    }

    #[test]
    fn overlap_rejected() {
        let f = AdminForm {
            timezone: "Europe/Rome".to_string(),
            pickup_time: "17:00".to_string(),
            entries: vec![
                Entry {
                    day: "monday".to_string(),
                    weeks: vec![1, 2],
                    kind: "Carta".to_string(),
                },
                Entry {
                    day: "monday".to_string(),
                    weeks: vec![2, 3],
                    kind: "Plastica".to_string(),
                },
            ],
            action: "save".to_string(),
        };
        let errs = validate_and_save(&PathBuf::from("/nonexistent"), &f, Lang::It).unwrap_err();
        assert!(errs.fields.contains_key("monday:1"));
        assert_eq!(errs.bad_weeks.get("monday:1").unwrap(), &[2]);
    }

    #[test]
    fn load_migrates_old_format() {
        let path = std::env::temp_dir().join(format!("trashdiff_migrate_{}", std::process::id()));
        std::fs::write(
            &path,
            "timezone = \"Europe/Rome\"\npickup_time = \"17:00\"\n\n[schedule]\nmonday = \"Carta\"\ntuesday = \"Umido\"\n",
        )
        .unwrap();
        let st = State::load(path.clone()).unwrap();
        assert_eq!(st.schedule.len(), 2);
        for e in &st.schedule {
            assert_eq!(e.weeks, vec![1, 2, 3, 4, 5]);
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[[schedule]]"));
        assert!(raw.contains("monday"));
        std::fs::remove_file(&path).ok();
    }

    use cegla_fcgi::protocol::{
        codec::{Decoder, Encoder},
        constants::{RecordType, Role},
        name_value_pair::NameValuePair,
        record::Record,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_util::codec::{FramedRead, FramedWrite};

    #[tokio::test]
    async fn fcgi_roundtrip_serves_home() {
        let (client_io, server_io) = tokio::io::duplex(1024);
        let st = Arc::new(state());
        let handle = tokio::spawn(async move {
            cegla_fcgi::server::server_handle_fcgi(server_io, TokioRt, move |request, _stderr| {
                let st = Arc::clone(&st);
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_string();
                    let headers = request.headers().clone();
                    let content_length = headers
                        .get(http::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    let body = read_body_capped(request.into_body(), content_length).await?;
                    let lng = lang_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, &method, &path, &headers, body));
                    resp
                }
            })
            .await
            .unwrap();
        });

        let (client_reader, client_writer) = tokio::io::split(client_io);
        let mut client_read = FramedRead::new(client_reader, Decoder::default());
        let mut client_write = FramedWrite::new(client_writer, Encoder);

        client_write
            .send(Record::new(
                RecordType::BeginRequest as u8,
                1,
                vec![0, Role::Responder as u8, 0, 0, 0, 0, 0, 0],
            ))
            .await
            .unwrap();
        let mut params = Vec::new();
        params.extend_from_slice(
            &NameValuePair::new(b"REQUEST_METHOD".to_vec(), b"GET".to_vec()).encode(),
        );
        params.extend_from_slice(
            &NameValuePair::new(b"REQUEST_URI".to_vec(), b"/".to_vec()).encode(),
        );
        client_write
            .send(Record::new(RecordType::Params as u8, 1, params))
            .await
            .unwrap();
        client_write
            .send(Record::new(RecordType::Params as u8, 1, vec![]))
            .await
            .unwrap();
        client_write
            .send(Record::new(RecordType::Stdin as u8, 1, vec![]))
            .await
            .unwrap();

        let mut out = Vec::new();
        loop {
            let record = client_read.next().await.unwrap().unwrap();
            if record.record_type == RecordType::Stdout as u8 {
                if record.content.is_empty() {
                    break;
                }
                out.extend_from_slice(&record.content);
            }
        }
        let end = client_read.next().await.unwrap().unwrap();
        assert_eq!(end.record_type, RecordType::EndRequest as u8);

        let body = String::from_utf8_lossy(&out);
        assert!(body.contains("Content-Type: text/html"));
        assert!(body.contains("Waste collection"));

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn fcgi_roundtrip_post_admin_validation() {
        let (client_io, server_io) = tokio::io::duplex(1024);
        let st = Arc::new(state());
        let handle = tokio::spawn(async move {
            cegla_fcgi::server::server_handle_fcgi(server_io, TokioRt, move |request, _stderr| {
                let st = Arc::clone(&st);
                async move {
                    let method = request.method().clone();
                    let path = request.uri().path().to_string();
                    let headers = request.headers().clone();
                    let content_length = headers
                        .get(http::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    let body = read_body_capped(request.into_body(), content_length).await?;
                    let lng = lang_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, &method, &path, &headers, body));
                    resp
                }
            })
            .await
            .unwrap();
        });

        let (client_reader, client_writer) = tokio::io::split(client_io);
        let mut client_read = FramedRead::new(client_reader, Decoder::default());
        let mut client_write = FramedWrite::new(client_writer, Encoder);

        let body = b"pickup_time=19:30&timezone=Nope&day_monday=X";
        client_write
            .send(Record::new(
                RecordType::BeginRequest as u8,
                1,
                vec![0, Role::Responder as u8, 0, 0, 0, 0, 0, 0],
            ))
            .await
            .unwrap();
        let mut params = Vec::new();
        for (k, v) in [
            ("REQUEST_METHOD", "POST"),
            ("REQUEST_URI", "/admin"),
            ("CONTENT_LENGTH", &body.len().to_string()),
        ] {
            params.extend_from_slice(
                &NameValuePair::new(k.as_bytes().to_vec(), v.as_bytes().to_vec()).encode(),
            );
        }
        client_write
            .send(Record::new(RecordType::Params as u8, 1, params))
            .await
            .unwrap();
        client_write
            .send(Record::new(RecordType::Params as u8, 1, vec![]))
            .await
            .unwrap();
        client_write
            .send(Record::new(RecordType::Stdin as u8, 1, body.to_vec()))
            .await
            .unwrap();
        client_write
            .send(Record::new(RecordType::Stdin as u8, 1, vec![]))
            .await
            .unwrap();

        let mut out = Vec::new();
        loop {
            let record = client_read.next().await.unwrap().unwrap();
            if record.record_type == RecordType::Stdout as u8 {
                if record.content.is_empty() {
                    break;
                }
                out.extend_from_slice(&record.content);
            }
        }

        let response = String::from_utf8_lossy(&out);
        assert!(response.contains("Status: 400"));
        assert!(response.contains("invalid timezone"));

        handle.await.unwrap();
    }

    fn scgi_netstring(pairs: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut env = Vec::new();
        for (k, v) in pairs {
            env.extend_from_slice(k.as_bytes());
            env.push(0);
            env.extend_from_slice(v.as_bytes());
            env.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(env.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(&env);
        out.push(b',');
        out.extend_from_slice(body);
        out
    }

    async fn scgi_handler<B>(
        st: &State,
        request: http::Request<B>,
    ) -> Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error>
    where
        B: BodyExt + Unpin,
        B::Data: AsRef<[u8]>,
        B::Error: Into<std::io::Error>,
    {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        let headers = request.headers().clone();
        let content_length = headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let body = read_body_capped(request.into_body(), content_length).await?;
        let lng = lang_from_headers(&headers);
        Ok(route_cgi(st, lng, &method, &path, &headers, body))
    }

    #[tokio::test]
    async fn scgi_roundtrip_serves_home() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client_io, server_io) = tokio::io::duplex(1024);
        let st = Arc::new(state());
        let handle = tokio::spawn(async move {
            cegla_scgi::server::server_handle_scgi(server_io, move |request| {
                let st = Arc::clone(&st);
                async move { scgi_handler(&st, request).await }
            })
            .await
            .unwrap();
        });
        let (mut reader, mut writer) = tokio::io::split(client_io);
        let netstring = scgi_netstring(&[("REQUEST_METHOD", "GET"), ("REQUEST_URI", "/")], b"");
        writer.write_all(&netstring).await.unwrap();
        drop(writer);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.contains("Content-Type: text/html"));
        assert!(response.contains("Waste collection"));
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn scgi_roundtrip_post_admin_validation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client_io, server_io) = tokio::io::duplex(1024);
        let st = Arc::new(state());
        let handle = tokio::spawn(async move {
            cegla_scgi::server::server_handle_scgi(server_io, move |request| {
                let st = Arc::clone(&st);
                async move { scgi_handler(&st, request).await }
            })
            .await
            .unwrap();
        });
        let (mut reader, mut writer) = tokio::io::split(client_io);
        let body = b"pickup_time=19:30&timezone=Nope&day_monday=X";
        let netstring = scgi_netstring(
            &[
                ("REQUEST_METHOD", "POST"),
                ("REQUEST_URI", "/admin"),
                ("CONTENT_LENGTH", &body.len().to_string()),
            ],
            body,
        );
        writer.write_all(&netstring).await.unwrap();
        drop(writer);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(response.contains("Status: 400"));
        assert!(response.contains("invalid timezone"));
        handle.await.unwrap();
    }
}
