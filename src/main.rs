use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

use i18n::{Lang, days, days_full, fill, t};

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
    schedule: HashMap<String, String>,
}

fn default_db() -> Db {
    let schedule = HashMap::new();
    Db {
        timezone: "Europe/Rome".to_string(),
        pickup_time: "05:00".to_string(),
        schedule,
    }
}

struct State {
    db_path: PathBuf,
    timezone: Tz,
    pickup_time: NaiveTime,
    schedule: HashMap<String, String>,
}

impl State {
    fn load(db_path: PathBuf) -> Result<State, String> {
        let db = if db_path.exists() {
            let raw = std::fs::read_to_string(&db_path)
                .map_err(|e| format!("impossibile leggere {:?}: {e}", db_path))?;
            toml::from_str::<Db>(&raw)
                .map_err(|e| format!("database {:?} non valido: {e}", db_path))?
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
        std::fs::write(db_path, raw).map_err(|e| format!("scrittura {:?}: {e}", db_path))
    }

    fn type_for(&self, wd: Weekday) -> String {
        self.schedule
            .get(DAY_KEYS[day_index(wd)])
            .cloned()
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
                return (date, date.weekday(), self.type_for(date.weekday()));
            }
        }
        unreachable!()
    }
}

#[derive(Clone)]
struct AppState(Arc<Mutex<State>>);

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(lng: Lang, title: &str, body: String) -> String {
    let home = t(lng, "nav_home");
    let admin = t(lng, "nav_admin");
    let (other_code, other_label) = if lng == Lang::It {
        ("en", "EN")
    } else {
        ("it", "IT")
    };
    format!(
        r#"<!doctype html>
<html lang="{html_lang}" style="color-scheme:light dark"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
body{{font-family:system-ui,sans-serif;max-width:640px;margin:2rem auto;padding:0 1rem;color:#222}}
table{{border-collapse:collapse;width:100%}}
th,td{{border:1px solid #ccc;padding:.4rem .6rem;text-align:left}}
th{{background:#f2f2f2}}
.now{{background:#e7f6e7;padding:.8rem;border-radius:.5rem;margin:1rem 0}}
nav a{{margin-right:1rem}}
.err{{background:#fdecea;color:#b00020;padding:.6rem;border-radius:.5rem}}
input[type=text]{{width:100%;box-sizing:border-box;padding:.4rem}}
form p{{margin:.6rem 0}}
button{{padding:.5rem 1.2rem;cursor:pointer}}
@media (prefers-color-scheme:dark) {{
body{{color:#ddd;background:#1e1e1e}}
th,td{{border-color:#444}}
th{{background:#2a2a2a}}
.now{{background:#1e3a2b}}
.err{{background:#4a1f1f;color:#ffb4ab}}
a{{color:#8ab4f8}}
}}
</style></head><body>
<nav><a href="/">{home}</a><a href="/admin">{admin}</a><span style="float:right"><a href="/lang/{other_code}">{other_label}</a></span></nav>
{body}
</body></html>"#,
        html_lang = lng.code(),
        title = title,
        home = home,
        admin = admin,
        other_code = other_code,
        other_label = other_label,
        body = body,
    )
}

fn fmt_dt(lng: Lang, dt: DateTime<Tz>) -> String {
    let day = days(lng)[day_index(dt.weekday())];
    match lng {
        Lang::It => format!("{day} {} alle {}", dt.format("%d/%m"), dt.format("%H:%M")),
        Lang::En => format!("{day} {} at {}", dt.format("%m/%d"), dt.format("%H:%M")),
    }
}

fn now_it(lng: Lang) -> String {
    match lng {
        Lang::It => Utc::now().format("%d/%m").to_string(),
        Lang::En => Utc::now().format("%m/%d").to_string(),
    }
}

fn home_html(st: &State, lng: Lang) -> String {
    let now = Utc::now().with_timezone(&st.timezone);
    let pickup = st.pickup_time;
    let (open_date, _owd, open_type) = st.next_boundary(now);
    let today_type = st.type_for(now.date_naive().weekday());

    let open_dt = open_date
        .and_time(pickup)
        .and_local_timezone(st.timezone)
        .earliest()
        .expect("boundary locale non risolvibile");

    let open_row = if open_type.is_empty() {
        let s = fmt_dt(lng, open_dt);
        format!("<p>{}</p>", esc(&fill(t(lng, "pause"), &[s.as_str()])))
    } else {
        let s = fmt_dt(lng, open_dt);
        format!(
            "<p><strong>{}</strong></p><p>{}</p>",
            esc(&fill(t(lng, "now_open"), &[open_type.as_str()])),
            esc(&fill(t(lng, "window_until"), &[s.as_str()])),
        )
    };

    let today_line = if today_type.is_empty() {
        let d = now_it(lng);
        format!("<p>{}</p>", esc(&fill(t(lng, "today_none"), &[d.as_str()])))
    } else {
        let d = now_it(lng);
        let tm = pickup.format("%H:%M").to_string();
        format!(
            "<p>{}</p>",
            esc(&fill(
                t(lng, "today_pickup"),
                &[d.as_str(), today_type.as_str(), tm.as_str()],
            ))
        )
    };

    let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let types = (0..7)
        .map(|i| st.type_for((base + Duration::days(i as i64)).weekday()))
        .collect::<Vec<_>>();
    let dnames = days(lng);
    let rows = (0..7)
        .map(|i| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{} {} → {} {}</td></tr>",
                dnames[i],
                if types[i].is_empty() {
                    "—".to_string()
                } else {
                    esc(&types[i])
                },
                dnames[(i + 6) % 7],
                pickup.format("%H:%M"),
                dnames[i],
                pickup.format("%H:%M")
            )
        })
        .collect::<String>();

    format!(
        r#"<h1>{title_home}</h1>
<div class="now">{open_row}</div>
{today_line}
<h2>{week}</h2>
<table><tr><th>{col_day}</th><th>{col_type}</th><th>{col_window}</th></tr>{rows}</table>"#,
        title_home = esc(t(lng, "title_home")),
        week = esc(t(lng, "week")),
        col_day = esc(t(lng, "col_day")),
        col_type = esc(t(lng, "col_type")),
        col_window = esc(t(lng, "col_window")),
        open_row = open_row,
        today_line = today_line,
        rows = rows
    )
}

async fn home(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let lng = lang_of(&req);
    let body = {
        let st = data.0.lock().unwrap();
        home_html(&st, lng)
    };
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(page(lng, t(lng, "title_home"), body))
}

fn admin_form_from_state(st: &State) -> AdminForm {
    let sched = &st.schedule;
    AdminForm {
        timezone: st.timezone.to_string(),
        pickup_time: st.pickup_time.format("%H:%M").to_string(),
        day_monday: sched.get("monday").cloned().unwrap_or_default(),
        day_tuesday: sched.get("tuesday").cloned().unwrap_or_default(),
        day_wednesday: sched.get("wednesday").cloned().unwrap_or_default(),
        day_thursday: sched.get("thursday").cloned().unwrap_or_default(),
        day_friday: sched.get("friday").cloned().unwrap_or_default(),
        day_saturday: sched.get("saturday").cloned().unwrap_or_default(),
        day_sunday: sched.get("sunday").cloned().unwrap_or_default(),
    }
}

async fn admin_get(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let lng = lang_of(&req);
    let form = {
        let st = data.0.lock().unwrap();
        admin_form_from_state(&st)
    };
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(page(
            lng,
            t(lng, "title_admin"),
            admin_form_html(lng, form, None),
        ))
}

#[derive(Deserialize)]
struct AdminForm {
    timezone: String,
    pickup_time: String,
    day_monday: String,
    day_tuesday: String,
    day_wednesday: String,
    day_thursday: String,
    day_friday: String,
    day_saturday: String,
    day_sunday: String,
}

fn admin_form_html(lng: Lang, f: AdminForm, err: Option<String>) -> String {
    let err_html = err
        .map(|e| format!("<p class=\"err\">{}</p>", esc(&e)))
        .unwrap_or_default();
    let full = days_full(lng);
    let days = [
        ("day_monday", full[0], f.day_monday),
        ("day_tuesday", full[1], f.day_tuesday),
        ("day_wednesday", full[2], f.day_wednesday),
        ("day_thursday", full[3], f.day_thursday),
        ("day_friday", full[4], f.day_friday),
        ("day_saturday", full[5], f.day_saturday),
        ("day_sunday", full[6], f.day_sunday),
    ];
    let fields = days
        .into_iter()
        .map(|(name, label, val)| {
            format!(
                "<p><label>{label}<br><input type=\"text\" name=\"{name}\" value=\"{}\"></label></p>",
                esc(&val)
            )
        })
        .collect::<String>();
    let tz_options = TZ_VARIANTS
        .iter()
        .map(|tz| {
            let name = tz.to_string();
            let sel = if name == f.timezone { " selected" } else { "" };
            format!(
                "<option value=\"{}\"{}>{}</option>",
                esc(&name),
                sel,
                esc(&name)
            )
        })
        .collect::<String>();
    format!(
        r#"<h1>{title}</h1>
{err_html}
<form method="post" action="/admin">
<p><label>{plabel}<br><input type="text" name="pickup_time" value="{pt}"></label></p>
<p><label>{tzlabel}<br><select name="timezone">{tzopts}</select></label></p>
{fields}
<p><em>{hint}</em></p>
<p><button type="submit">{save}</button></p>
</form>"#,
        title = esc(t(lng, "title_admin")),
        plabel = esc(t(lng, "pickup_time_label")),
        tzlabel = esc(t(lng, "tz_label")),
        hint = esc(t(lng, "empty_hint")),
        save = esc(t(lng, "save")),
        pt = esc(&f.pickup_time),
        tzopts = tz_options,
        fields = fields,
    )
}

fn validate_and_save(st: &mut State, f: &AdminForm, lng: Lang) -> Result<(), String> {
    let timezone: Tz = f
        .timezone
        .parse()
        .map_err(|e| format!("{}: {e}", t(lng, "err_tz")))?;
    let pickup_time = NaiveTime::parse_from_str(&f.pickup_time, "%H:%M")
        .map_err(|e| format!("{}: {e}", t(lng, "err_time")))?;
    let mut schedule = HashMap::new();
    for (name, val) in [
        ("monday", &f.day_monday),
        ("tuesday", &f.day_tuesday),
        ("wednesday", &f.day_wednesday),
        ("thursday", &f.day_thursday),
        ("friday", &f.day_friday),
        ("saturday", &f.day_saturday),
        ("sunday", &f.day_sunday),
    ] {
        if !val.trim().is_empty() {
            schedule.insert(name.to_string(), val.trim().to_string());
        }
    }
    let db = Db {
        timezone: f.timezone.clone(),
        pickup_time: f.pickup_time.clone(),
        schedule,
    };
    State::save_file(&st.db_path, &db).map_err(|e| format!("{}: {e}", t(lng, "err_io")))?;
    st.timezone = timezone;
    st.pickup_time = pickup_time;
    st.schedule = db.schedule;
    Ok(())
}

async fn admin_post(
    req: HttpRequest,
    data: web::Data<AppState>,
    form: web::Form<AdminForm>,
) -> HttpResponse {
    let lng = lang_of(&req);
    let f = form.into_inner();
    let result = {
        let mut st = data.0.lock().unwrap();
        validate_and_save(&mut st, &f, lng)
    };
    if let Err(e) = result {
        let body = admin_form_html(lng, f, Some(e));
        return HttpResponse::BadRequest()
            .content_type("text/html; charset=utf-8")
            .body(page(lng, t(lng, "title_admin"), body));
    }
    HttpResponse::SeeOther()
        .insert_header((LOCATION, "/"))
        .finish()
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
    let line = if open_type.is_empty() {
        let s = fmt_dt(lng, open_dt);
        fill(t(lng, "pause"), &[s.as_str()])
    } else {
        let s = fmt_dt(lng, open_dt);
        format!(
            "{} ({})",
            fill(t(lng, "now_open"), &[open_type.as_str()]),
            fill(t(lng, "window_until"), &[s.as_str()]),
        )
    };
    println!("{line}");
    Ok(())
}

async fn serve(bind: String, db_path: PathBuf) -> std::io::Result<()> {
    let state = match State::load(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let data = web::Data::new(AppState(Arc::new(Mutex::new(state))));
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
    let pairs: HashMap<String, String> = form_urlencoded::parse(body).into_owned().collect();
    AdminForm {
        timezone: pairs.get("timezone").cloned().unwrap_or_default(),
        pickup_time: pairs.get("pickup_time").cloned().unwrap_or_default(),
        day_monday: pairs.get("day_monday").cloned().unwrap_or_default(),
        day_tuesday: pairs.get("day_tuesday").cloned().unwrap_or_default(),
        day_wednesday: pairs.get("day_wednesday").cloned().unwrap_or_default(),
        day_thursday: pairs.get("day_thursday").cloned().unwrap_or_default(),
        day_friday: pairs.get("day_friday").cloned().unwrap_or_default(),
        day_saturday: pairs.get("day_saturday").cloned().unwrap_or_default(),
        day_sunday: pairs.get("day_sunday").cloned().unwrap_or_default(),
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
    st: &mut State,
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
            return match validate_and_save(st, &f, lng) {
                Ok(()) => redirect("/", None),
                Err(e) => {
                    let html = page(lng, t(lng, "title_admin"), admin_form_html(lng, f, Some(e)));
                    respond(StatusCode::BAD_REQUEST, html)
                }
            };
        }
        let html = page(
            lng,
            t(lng, "title_admin"),
            admin_form_html(lng, admin_form_from_state(st), None),
        );
        return respond(StatusCode::OK, html);
    }
    let html = page(lng, t(lng, "title_home"), home_html(st, lng));
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
            let mut st = st;
            let lng = lang_from_headers(&headers);
            let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> =
                Ok(route_cgi(&mut st, lng, &method, &path, &headers, body));
            resp
        },
    )
    .await
}

struct TokioRt;

impl cegla_fcgi::server::Runtime for TokioRt {
    fn spawn(&self, future: impl std::future::Future + Send + 'static) {
        tokio::spawn(async move { future.await; });
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
    let state = State::load(db).map_err(std::io::Error::other)?;
    let state = Arc::new(Mutex::new(state));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("trashdiff fcgi listening on {bind}");
    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = cegla_fcgi::server::server_handle_fcgi(
                stream,
                TokioRt,
                move |request, _stderr| {
                    let state = Arc::clone(&state);
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
                        let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> = {
                            let mut st = state.lock().unwrap();
                            Ok(route_cgi(&mut st, lng, &method, &path, &headers, body))
                        };
                        resp
                    }
                },
            )
            .await;
        });
    }
}

async fn scgi_run(bind: String, db: PathBuf) -> std::io::Result<()> {
    let state = State::load(db).map_err(std::io::Error::other)?;
    let state = Arc::new(Mutex::new(state));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("trashdiff scgi listening on {bind}");
    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = cegla_scgi::server::server_handle_scgi(
                stream,
                move |request| {
                    let state = Arc::clone(&state);
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
                        let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> = {
                            let mut st = state.lock().unwrap();
                            Ok(route_cgi(&mut st, lng, &method, &path, &headers, body))
                        };
                        resp
                    }
                },
            )
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut schedule = HashMap::new();
        schedule.insert("monday".to_string(), "Carta".to_string());
        schedule.insert("tuesday".to_string(), "Umido".to_string());
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
        let state = Arc::new(Mutex::new(state()));
        let handle = tokio::spawn(async move {
            cegla_fcgi::server::server_handle_fcgi(
                server_io,
                TokioRt,
                move |request, _stderr| {
                    let state = Arc::clone(&state);
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
                        let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> = {
                            let mut st = state.lock().unwrap();
                            Ok(route_cgi(&mut st, lng, &method, &path, &headers, body))
                        };
                        resp
                    }
                },
            )
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
        let state = Arc::new(Mutex::new(state()));
        let handle = tokio::spawn(async move {
            cegla_fcgi::server::server_handle_fcgi(
                server_io,
                TokioRt,
                move |request, _stderr| {
                    let state = Arc::clone(&state);
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
                        let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> = {
                            let mut st = state.lock().unwrap();
                            Ok(route_cgi(&mut st, lng, &method, &path, &headers, body))
                        };
                        resp
                    }
                },
            )
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
            params.extend_from_slice(&NameValuePair::new(k.as_bytes().to_vec(), v.as_bytes().to_vec()).encode());
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
        state: Arc<Mutex<State>>,
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
        let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> = {
            let mut st = state.lock().unwrap();
            Ok(route_cgi(&mut st, lng, &method, &path, &headers, body))
        };
        resp
    }

    #[tokio::test]
    async fn scgi_roundtrip_serves_home() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client_io, server_io) = tokio::io::duplex(1024);
        let state = Arc::new(Mutex::new(state()));
        let handle = tokio::spawn(async move {
            cegla_scgi::server::server_handle_scgi(server_io, move |request| {
                let state = Arc::clone(&state);
                async move { scgi_handler(state, request).await }
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
        let state = Arc::new(Mutex::new(state()));
        let handle = tokio::spawn(async move {
            cegla_scgi::server::server_handle_scgi(server_io, move |request| {
                let state = Arc::clone(&state);
                async move { scgi_handler(state, request).await }
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
