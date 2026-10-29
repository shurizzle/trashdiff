use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod admin;
mod auth;
mod i18n;
#[cfg(feature = "mcp")]
mod mcp;
mod minify;
mod schedule;
mod transport;
mod views;
#[cfg(feature = "http")]
mod web;

#[cfg(feature = "mcp")]
use crate::mcp::mcp_run;
#[cfg(feature = "cgi")]
use crate::transport::cgi_run;
use crate::transport::cli_cmd;
#[cfg(feature = "fcgi")]
use crate::transport::fcgi_run;
#[cfg(feature = "scgi")]
use crate::transport::scgi_run;
#[cfg(feature = "http")]
use crate::web::serve;

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
    #[cfg(feature = "http")]
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
    #[cfg(feature = "cgi")]
    Cgi {
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Run a FastCGI server (e.g. nginx fastcgi_pass)
    #[cfg(feature = "fcgi")]
    Fcgi {
        /// Bind address (host:port)
        #[arg(long, default_value = "127.0.0.1:9000")]
        bind: String,
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Run an MCP server over stdio (Model Context Protocol)
    #[cfg(feature = "mcp")]
    Mcp {
        /// Path to database file
        #[arg(long, default_value = "trashdb.toml", env = "TRASHDIFF_DB")]
        db: PathBuf,
    },
    /// Run an SCGI server (e.g. nginx mod_scgi)
    #[cfg(feature = "scgi")]
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
    #[cfg(feature = "cgi")]
    {
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
        #[cfg(feature = "http")]
        Command::Http { bind, db } => {
            let rt = actix_web::rt::Runtime::new()?;
            rt.block_on(serve(bind, db))
        }
        #[cfg(feature = "cgi")]
        Command::Cgi { db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(cgi_run(db))
        }
        #[cfg(feature = "fcgi")]
        Command::Fcgi { bind, db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(fcgi_run(bind, db))
        }
        #[cfg(feature = "scgi")]
        Command::Scgi { bind, db } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(scgi_run(bind, db))
        }
        #[cfg(feature = "mcp")]
        Command::Mcp { db } => match mcp_run(db) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::admin::*;
    use crate::auth::Auth;
    use crate::i18n::{Lang, Localized, T};
    use crate::minify::{minify_css, minify_js};
    use crate::schedule::*;
    use crate::transport::*;
    use crate::views::*;

    use std::path::PathBuf;
    #[cfg(any(feature = "fcgi", feature = "scgi"))]
    use std::sync::Arc;

    #[cfg(any(feature = "fcgi", feature = "scgi"))]
    use bytes::Bytes;
    use chrono::{DateTime, NaiveTime, Utc, Weekday};
    use chrono_tz::Tz;
    use ecow::EcoString;
    #[cfg(feature = "scgi")]
    use http_body_util::BodyExt;
    #[cfg(any(feature = "fcgi", feature = "scgi"))]
    use http_body_util::combinators::BoxBody;

    fn w(ws: &[u32]) -> Week {
        ws.iter().fold(Week::empty(), |acc, &n| acc | week_of(n))
    }

    fn state() -> State {
        state_with_db(PathBuf::from("/nonexistent"))
    }

    fn auth() -> Auth {
        Auth::new(Some("pw".to_string()))
    }

    fn basic(password: &str) -> String {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        format!("Basic {}", STANDARD.encode(format!("user:{password}")))
    }

    fn state_with_db(db_path: PathBuf) -> State {
        let schedule = vec![
            Entry {
                day: "monday".to_string(),
                weeks: w(&[1]),
                kind: "Carta".into(),
            },
            Entry {
                day: "tuesday".to_string(),
                weeks: w(&[1]),
                kind: "Umido".into(),
            },
        ];
        State {
            db_path,
            timezone: "Europe/Rome".parse().unwrap(),
            pickup_time: NaiveTime::parse_from_str("17:00", "%H:%M").unwrap(),
            schedule,
            default_lang: None,
            custom_js: None,
            custom_css: None,
        }
    }

    fn at(date: &str, time: &str, st: &State) -> DateTime<Tz> {
        let naive =
            chrono::NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M")
                .unwrap();
        naive.and_local_timezone(st.timezone).earliest().unwrap()
    }

    #[test]
    fn theme_parses_cookie_and_cycles() {
        assert_eq!(Theme::from_req(None), Theme::Auto);
        assert_eq!(Theme::from_req(Some("theme=light")), Theme::Light);
        assert_eq!(Theme::from_req(Some("lang=it; theme=dark")), Theme::Dark);
        assert_eq!(Theme::from_req(Some("theme=bogus")), Theme::Auto);
        assert_eq!(Theme::Auto.next(), Theme::Light);
        assert_eq!(Theme::Light.next(), Theme::Dark);
        assert_eq!(Theme::Dark.next(), Theme::Auto);
    }

    #[test]
    fn minify_js_compresses_or_falls_back() {
        let src = "const  answer = 1 + 2;\nconsole.log( answer );";
        let out = minify_js(src);
        assert!(!out.contains('\n'));
        assert!(out.len() < src.len());
        assert!(out.contains("console.log"));
        // unparsable input falls back to the trimmed source
        assert_eq!(minify_js("function ("), "function (");
    }

    #[test]
    fn minify_css_compresses_or_falls_back() {
        let src = "body { color : red ; }";
        let out = minify_css(src);
        assert!(!out.contains('\n'));
        assert!(out.len() < src.len());
        assert!(out.contains("color:red"));
        // unparsable input falls back to the trimmed source
        assert_eq!(minify_css("}"), "}");
    }

    #[test]
    fn page_appends_custom_js_and_css() {
        let page = |js: Option<&str>, css: Option<&str>| {
            Localized::from((
                Lang::En,
                Page(
                    T::TitleHome,
                    T::Save,
                    Theme::Auto,
                    js.map(EcoString::from),
                    css.map(EcoString::from),
                ),
            ))
            .to_string()
        };
        let js = |js| page(js, None);
        // inline code -> minified child of a <script> tag, last before </body>
        assert!(js(Some("alert(1)")).contains("<script>alert(1)"));
        assert!(js(Some("alert(1)")).contains("</script></body>"));
        // surrounding whitespace is trimmed before minifying
        assert!(js(Some("  alert(1)  ")).contains("<script>alert(1)"));
        // unparsable code falls back to the trimmed original
        assert!(js(Some("  function (  ")).contains("<script>function (</script></body>"));
        // absolute / protocol-relative / relative URLs -> src attribute
        assert!(
            js(Some("https://example.com/a.js"))
                .contains("<script src=\"https://example.com/a.js\"></script></body>")
        );
        assert!(
            js(Some("//cdn.example.com/a.js"))
                .contains("<script src=\"//cdn.example.com/a.js\"></script></body>")
        );
        assert!(js(Some("/js/a.js")).contains("<script src=\"/js/a.js\"></script></body>"));
        assert!(js(Some("./a.js")).contains("<script src=\"./a.js\"></script></body>"));
        // absent / blank -> nothing injected
        assert!(!js(None).contains("<script></script>"));
        assert!(!js(Some("   ")).contains("src=\"\""));

        // inline CSS -> minified <style> in <head>, after the built-in one
        let inline_css = page(None, Some("body { color : red ; }"));
        assert!(inline_css.contains("<style>body{color:red"));
        assert!(inline_css.contains("</style></head><body>"));
        // unparsable CSS falls back to the trimmed original
        assert!(page(None, Some("  }  ")).contains("<style>}</style>"));
        // URL -> <link rel="stylesheet">
        assert!(
            page(None, Some("https://example.com/a.css"))
                .contains("<link rel=\"stylesheet\" href=\"https://example.com/a.css\">")
        );
        assert!(
            page(None, Some("/css/a.css"))
                .contains("<link rel=\"stylesheet\" href=\"/css/a.css\">")
        );
    }

    #[test]
    fn home_view_serializes_now_and_week() {
        let st = state();
        // Monday 2024-01-01, week 1: paper collected, Tuesday organic, rest empty
        let now = at("2024-01-01", "16:00", &st).with_timezone(&Utc);
        let json = serde_json::to_value(home_view(&st, now)).unwrap();
        assert_eq!(json["timezone"], "Europe/Rome");
        assert_eq!(json["pickup_time"], "17:00");
        assert_eq!(json["now"]["kind"], "Carta");
        assert_eq!(json["now"]["until"], "2024-01-01T17:00:00+01:00");
        assert!(json.get("today").is_none());
        let week = json["week"].as_array().unwrap();
        assert_eq!(week.len(), 7);
        assert_eq!(week[0], "Carta");
        assert_eq!(week[1], "Umido");
        assert!(week[2..].iter().all(|v| v.is_null()));
        // Wednesday 2024-01-03: pause until today's 17:00
        let now = at("2024-01-03", "10:00", &st).with_timezone(&Utc);
        let json = serde_json::to_value(home_view(&st, now)).unwrap();
        assert!(json["now"]["kind"].is_null());
        assert_eq!(json["now"]["until"], "2024-01-03T17:00:00+01:00");
    }

    #[test]
    fn home_view_window_crosses_week_boundary() {
        let st = state();
        // Sunday evening 2023-12-31: open window is Monday 2024-01-01 paper,
        // a day outside the serialized week (which ends Sunday)
        let now = at("2023-12-31", "18:00", &st).with_timezone(&Utc);
        let view = home_view(&st, now);
        assert_eq!(view.now.kind.as_deref(), Some("Carta"));
        assert_eq!(view.now.until, at("2024-01-01", "17:00", &st));
        let week = serde_json::to_value(&view).unwrap()["week"]
            .as_array()
            .unwrap()
            .clone();
        assert!(week.iter().all(|v| v.is_null()));
    }

    #[test]
    fn admin_json_groups_rows_by_weekday() {
        let mut st = state();
        st.schedule.push(Entry {
            day: "monday".to_string(),
            weeks: w(&[2]),
            kind: "Plastica".into(),
        });
        st.default_lang = Some(Lang::En);
        st.custom_js = Some("alert(1)".into());
        st.custom_css = Some("body{color:red}".into());
        let json = serde_json::to_value(admin_json(&st)).unwrap();
        assert_eq!(json["timezone"], "Europe/Rome");
        assert_eq!(json["pickup_time"], "17:00");
        assert_eq!(json["default_lang"], "en");
        assert_eq!(json["custom_js"], "alert(1)");
        assert_eq!(json["custom_css"], "body{color:red}");
        let schedule = json["schedule"].as_array().unwrap();
        assert_eq!(schedule.len(), 7);
        let monday = schedule[0].as_array().unwrap();
        assert_eq!(monday.len(), 2);
        assert_eq!(monday[0]["weeks"], serde_json::json!([1]));
        assert_eq!(monday[0]["type"], "Carta");
        assert_eq!(monday[1]["weeks"], serde_json::json!([2]));
        assert_eq!(monday[1]["type"], "Plastica");
        let tuesday = schedule[1].as_array().unwrap();
        assert_eq!(tuesday.len(), 1);
        assert_eq!(tuesday[0]["type"], "Umido");
        assert!(
            schedule[2..]
                .iter()
                .all(|d| d.as_array().unwrap().is_empty())
        );
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
            weeks: w(&[2]),
            kind: "Carta".into(),
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
        let mut f = AdminForm {
            timezone: "Europe/Rome".to_string(),
            pickup_time: "17:00".to_string(),
            entries: vec![
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[1, 2]),
                    kind: "Carta".into(),
                },
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[2, 3]),
                    kind: "Plastica".into(),
                },
            ],
            action: "save".to_string(),
            default_lang: String::new(),
            custom_js: EcoString::new(),
            custom_css: EcoString::new(),
        };
        let errs = validate_and_save(&PathBuf::from("/nonexistent"), &mut f, Lang::It).unwrap_err();
        assert!(errs.fields.contains_key("monday:1"));
        assert_eq!(errs.bad_weeks.get(&("monday", 1)), Some(&week_of(2)));
    }

    #[test]
    fn empty_type_rejected() {
        let mut f = AdminForm {
            timezone: "Europe/Rome".to_string(),
            pickup_time: "17:00".to_string(),
            entries: vec![
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[1]),
                    kind: "Carta".into(),
                },
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[2]),
                    kind: EcoString::new(),
                },
            ],
            action: "save".to_string(),
            default_lang: String::new(),
            custom_js: EcoString::new(),
            custom_css: EcoString::new(),
        };
        let errs = validate_and_save(&PathBuf::from("/nonexistent"), &mut f, Lang::It).unwrap_err();
        assert!(errs.fields.contains_key("monday:1"));
        assert!(errs.bad_weeks.is_empty());
        // zero weeks but empty type: row ignored, no error
        let mut f2 = AdminForm {
            entries: vec![Entry {
                day: "monday".to_string(),
                weeks: Week::empty(),
                kind: EcoString::new(),
            }],
            ..f
        };
        let path =
            std::env::temp_dir().join(format!("trashdiff_empty_type_{}", std::process::id()));
        assert!(validate_and_save(&path, &mut f2, Lang::It).is_ok());
        std::fs::remove_file(&path).ok();
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
            assert_eq!(e.weeks, Week::all());
        }
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[[schedule]]"));
        assert!(raw.contains("monday"));
        std::fs::remove_file(&path).ok();
    }

    #[cfg(feature = "fcgi")]
    use cegla_fcgi::protocol::{
        codec::{Decoder, Encoder},
        constants::{RecordType, Role},
        name_value_pair::NameValuePair,
        record::Record,
    };
    #[cfg(feature = "fcgi")]
    use futures_util::{SinkExt, StreamExt};
    #[cfg(feature = "fcgi")]
    use tokio_util::codec::{FramedRead, FramedWrite};

    #[cfg(feature = "fcgi")]
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
                    let lng = lang_from_headers(&headers, &st);
                    let theme = theme_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(
                        &st,
                        &auth(),
                        lng,
                        theme,
                        &method,
                        &path,
                        &headers,
                        body,
                    ));
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

    #[cfg(feature = "fcgi")]
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
                    let lng = lang_from_headers(&headers, &st);
                    let theme = theme_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(
                        &st,
                        &auth(),
                        lng,
                        theme,
                        &method,
                        &path,
                        &headers,
                        body,
                    ));
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
        let auth = basic("pw");
        for (k, v) in [
            ("REQUEST_METHOD", "POST"),
            ("REQUEST_URI", "/admin"),
            ("CONTENT_LENGTH", &body.len().to_string()),
            ("HTTP_AUTHORIZATION", auth.as_str()),
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

    #[cfg(feature = "scgi")]
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

    #[cfg(feature = "scgi")]
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
        let lng = lang_from_headers(&headers, st);
        let theme = theme_from_headers(&headers);
        Ok(route_cgi(
            st,
            &auth(),
            lng,
            theme,
            &method,
            &path,
            &headers,
            body,
        ))
    }

    #[cfg(feature = "scgi")]
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

    #[cfg(feature = "scgi")]
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
        let auth = basic("pw");
        let netstring = scgi_netstring(
            &[
                ("REQUEST_METHOD", "POST"),
                ("REQUEST_URI", "/admin"),
                ("CONTENT_LENGTH", &body.len().to_string()),
                ("HTTP_AUTHORIZATION", auth.as_str()),
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

    #[cfg(feature = "scgi")]
    async fn run_scgi_post(st: State, body: &[u8]) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client_io, server_io) = tokio::io::duplex(4096);
        let st = Arc::new(st);
        let handle = tokio::spawn(async move {
            cegla_scgi::server::server_handle_scgi(server_io, move |request| {
                let st = Arc::clone(&st);
                async move { scgi_handler(&st, request).await }
            })
            .await
            .unwrap();
        });
        let (mut reader, mut writer) = tokio::io::split(client_io);
        let netstring = scgi_netstring(
            &[
                ("REQUEST_METHOD", "POST"),
                ("REQUEST_URI", "/admin"),
                ("CONTENT_LENGTH", &body.len().to_string()),
                ("HTTP_AUTHORIZATION", "Bearer pw"),
            ],
            body,
        );
        writer.write_all(&netstring).await.unwrap();
        drop(writer);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).await.unwrap();
        handle.await.unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn temp_db() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "trashdiff_json_{}_{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn admin_write_full_replace_roundtrips() {
        let db = temp_db();
        let body = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","default_lang":null,"custom_js":"alert(1)","custom_css":"body{color:red}","schedule":[[{"weeks":[1,3],"type":"Carta"}],[],[],[],[],[],[]]}"#;
        match admin_json_write(&db, body, Lang::En) {
            JsonWrite::Saved => {}
            other => panic!("expected Saved, got {other:?}"),
        }
        let raw = std::fs::read_to_string(&db).unwrap();
        assert!(raw.contains("pickup_time = \"08:00\""), "raw:\n{raw}");
        assert!(raw.contains("custom_js = \"alert(1)\""), "raw:\n{raw}");
        assert!(
            raw.contains("custom_css = \"body{color:red}\""),
            "raw:\n{raw}"
        );
        assert!(raw.contains("type = \"Carta\""));
        assert!(raw.contains("\n    1,"));
        assert!(raw.contains("\n    3,"));
        assert!(!raw.contains("tuesday"));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn custom_resources_roundtrip_through_admin_form() {
        let db = temp_db();
        let mut f = AdminForm {
            timezone: "Europe/Rome".to_string(),
            pickup_time: "17:00".to_string(),
            entries: Vec::new(),
            action: "save".to_string(),
            default_lang: String::new(),
            custom_js: "  console.log('x');\n".into(),
            custom_css: "  body { color: red; }\n".into(),
        };
        validate_and_save(&db, &mut f, Lang::En).unwrap();
        let st = State::load(db.clone()).unwrap();
        // stored trimmed; minified only when rendered
        assert_eq!(st.custom_js.as_deref(), Some("console.log('x');"));
        assert_eq!(st.custom_css.as_deref(), Some("body { color: red; }"));
        // empty textareas clear both keys
        let mut f = AdminForm {
            custom_js: EcoString::new(),
            custom_css: EcoString::new(),
            ..f
        };
        validate_and_save(&db, &mut f, Lang::En).unwrap();
        let st = State::load(db.clone()).unwrap();
        assert_eq!(st.custom_js, None);
        assert_eq!(st.custom_css, None);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn admin_write_rejects_overlap() {
        let db = PathBuf::from("/nonexistent");
        let body = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","schedule":[[{"weeks":[1],"type":"Carta"},{"weeks":[1],"type":"Umido"}],[],[],[],[],[],[]]}"#;
        match admin_json_write(&db, body, Lang::En) {
            JsonWrite::Invalid(errs) => {
                let w: WriteErrors = errs.into();
                assert!(w.fields.contains_key("monday:1"));
                assert_eq!(w.overlaps["monday:1"], vec![1]);
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[cfg(any(feature = "cgi", feature = "fcgi", feature = "scgi"))]
    #[test]
    fn route_cgi_gates_admin_per_scheme() {
        use bytes::Bytes;
        let db = temp_db();
        let st = state_with_db(db.clone());
        let auth = auth();
        let basic_auth = basic("pw");
        let run = |method: http::Method, path: &str, cred: Option<&str>, body: &[u8]| {
            let mut headers = http::HeaderMap::new();
            if let Some(v) = cred {
                headers.insert(
                    http::header::AUTHORIZATION,
                    http::HeaderValue::from_str(v).unwrap(),
                );
            }
            let resp = route_cgi(
                &st,
                &auth,
                Lang::En,
                Theme::Auto,
                &method,
                path,
                &headers,
                Bytes::from(body.to_vec()),
            );
            (
                resp.status(),
                resp.headers().get("WWW-Authenticate").cloned(),
            )
        };

        // Backoffice page: Basic only.
        assert_eq!(run(http::Method::GET, "/admin", None, b"").0, 401);
        assert_eq!(
            run(http::Method::GET, "/admin", Some("Bearer pw"), b"").0,
            401
        );
        assert_eq!(
            run(http::Method::GET, "/admin", Some(&basic_auth), b"").0,
            200
        );

        // JSON write API: Bearer only.
        let json = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","schedule":[[],[],[],[],[],[],[]]}"#;
        let (status, challenge) = run(http::Method::POST, "/admin", None, json);
        assert_eq!(status, 401);
        assert_eq!(challenge.unwrap(), "Bearer realm=\"trashdiff\"");
        assert_eq!(
            run(http::Method::POST, "/admin", Some(&basic_auth), json).0,
            401
        );
        assert_eq!(
            run(http::Method::POST, "/admin", Some("Bearer pw"), json).0,
            200
        );

        // HTML form: Basic only.
        let form = b"pickup_time=19:30&timezone=Nope";
        let (status, _) = run(http::Method::POST, "/admin", Some("Bearer pw"), form);
        assert_eq!(status, 401);
        assert_eq!(
            run(http::Method::POST, "/admin", Some(&basic_auth), form).0,
            400
        );

        // Public reads stay open.
        assert_eq!(run(http::Method::GET, "/", None, b"").0, 200);
        assert_eq!(run(http::Method::GET, "/home.json", None, b"").0, 200);
        assert_eq!(run(http::Method::GET, "/admin.json", None, b"").0, 200);

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn body_is_json_detects_ct_and_brace() {
        assert!(body_is_json(Some("application/json"), b"x"));
        assert!(body_is_json(None, b"  {\"a\":1}"));
        assert!(!body_is_json(None, b"pickup_time=17:00"));
        assert!(!body_is_json(Some("text/plain"), b"pickup_time=17:00"));
    }

    #[cfg(feature = "scgi")]
    #[tokio::test]
    async fn scgi_roundtrip_post_admin_json_saves() {
        let db = temp_db();
        let body = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","schedule":[[{"weeks":[1],"type":"Carta"}],[],[],[],[],[],[]]}"#;
        let response = run_scgi_post(state_with_db(db.clone()), body).await;
        assert!(response.contains("Status: 200"));
        assert!(response.contains("application/json"));
        assert!(response.contains("\"type\":\"Carta\""));
        assert!(response.contains("\"timezone\":\"Europe/Rome\""));
        let raw = std::fs::read_to_string(&db).unwrap();
        assert!(raw.contains("pickup_time = \"08:00\""));
        let _ = std::fs::remove_file(&db);
    }

    #[cfg(feature = "scgi")]
    #[tokio::test]
    async fn scgi_roundtrip_post_admin_json_rejects_bad_body() {
        let response = run_scgi_post(state(), br#"{"timezone":5}"#).await;
        assert!(response.contains("Status: 400"));
        assert!(response.contains("error"));
        assert!(response.contains("invalid JSON"));
    }

    #[cfg(feature = "mcp")]
    fn mcp_db(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("trashdiff_mcp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[cfg(feature = "mcp")]
    fn mcp_call(id: u32, name: &str, args: serde_json::Value) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": args },
        })
        .to_string()
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_handshake_and_tools_list() {
        use crate::mcp::handle;
        let db = mcp_db("list");
        assert_eq!(
            handle(
                &db,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                false,
            ),
            None
        );
        let init = handle(
            &db,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            false,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "trashdiff");
        assert_eq!(
            init["result"]["capabilities"]["tools"],
            serde_json::json!({})
        );

        let list = handle(
            &db,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            false,
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["what_to_throw", "schedule_read", "schedule_write"]);

        let bad = handle(&db, r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#, false).unwrap();
        assert_eq!(bad["error"]["code"], -32601);
        let garbage = handle(&db, "{", false).unwrap();
        assert_eq!(garbage["error"]["code"], -32700);
        let unknown = handle(&db, &mcp_call(4, "nope", serde_json::json!({})), false).unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
        let _ = std::fs::remove_file(&db);
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_write_requires_auth() {
        use crate::mcp::{handle, is_write_request};
        let db = mcp_db("unauth");
        let call = mcp_call(
            1,
            "schedule_write",
            serde_json::json!({
                "timezone": "Europe/Rome",
                "pickup_time": "09:00",
                "schedule": [[],[],[],[],[],[],[]]
            }),
        );
        assert!(is_write_request(&call));
        assert!(!is_write_request(&mcp_call(
            2,
            "schedule_read",
            serde_json::json!({})
        )));
        assert!(!is_write_request("{"));

        let refused = handle(&db, &call, false).unwrap();
        assert_eq!(refused["error"]["code"], -32001);
        assert!(!db.exists(), "unauthorized write must not create the file");

        let read = handle(
            &db,
            &mcp_call(3, "schedule_read", serde_json::json!({})),
            false,
        )
        .unwrap();
        assert!(read["result"].get("isError").is_none());

        let ok = handle(&db, &call, true).unwrap();
        assert!(ok["result"].get("isError").is_none());
        let _ = std::fs::remove_file(&db);
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn mcp_reads_writes_and_rejects_overlap() {
        use crate::mcp::handle;
        let db = mcp_db("write");

        let read = handle(
            &db,
            &mcp_call(1, "schedule_read", serde_json::json!({})),
            true,
        )
        .unwrap();
        let text = read["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Europe/Rome"));
        assert!(text.contains("\"pickup_time\": \"05:00\""));

        let args = serde_json::json!({
            "timezone": "Europe/Rome",
            "pickup_time": "07:30",
            "default_lang": "it",
            "custom_js": null,
            "custom_css": null,
            "schedule": [[{"weeks":[1,3],"type":"Carta"}],[],[],[],[],[],[]]
        });
        let saved = handle(&db, &mcp_call(2, "schedule_write", args), true).unwrap();
        assert!(saved["result"].get("isError").is_none());
        let st = State::load(db.clone()).unwrap();
        assert_eq!(st.pickup_time.format("%H:%M").to_string(), "07:30");
        assert_eq!(st.schedule[0].kind, "Carta");
        assert_eq!(st.schedule[0].weeks, w(&[1, 3]));

        let overlap = serde_json::json!({
            "timezone": "Europe/Rome",
            "pickup_time": "07:30",
            "schedule": [
                [{"weeks":[1],"type":"Carta"},{"weeks":[1],"type":"Umido"}],
                [],[],[],[],[],[]
            ]
        });
        let bad = handle(&db, &mcp_call(3, "schedule_write", overlap), true).unwrap();
        assert_eq!(bad["result"]["isError"], true);
        let text = bad["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("overlaps"));
        let _ = std::fs::remove_file(&db);
    }

    #[cfg(all(feature = "http", feature = "mcp"))]
    #[actix_web::test]
    async fn mcp_http_endpoint_roundtrip() {
        use crate::web::{AppState, mcp_no_stream, mcp_post};
        use actix_web::{App, test, web};
        let db = mcp_db("http");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(AppState {
                    db: db.clone(),
                    auth: auth(),
                }))
                .route("/mcp", web::post().to(mcp_post))
                .route("/mcp", web::get().to(mcp_no_stream)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/mcp")
            .insert_header(("content-type", "application/json"))
            .set_payload(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        assert!(
            resp.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("application/json")
        );
        let raw = test::read_body(resp).await;
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "trashdiff");

        let note = test::TestRequest::post()
            .uri("/mcp")
            .set_payload(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .to_request();
        assert_eq!(test::call_service(&app, note).await.status(), 202);

        let batch = test::TestRequest::post()
            .uri("/mcp")
            .set_payload("[{}]")
            .to_request();
        let raw = test::read_body(test::call_service(&app, batch).await).await;
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["error"]["code"], -32600);

        let get = test::TestRequest::get().uri("/mcp").to_request();
        assert_eq!(
            test::call_service(&app, get).await.status(),
            actix_web::http::StatusCode::METHOD_NOT_ALLOWED
        );

        let write = mcp_call(
            2,
            "schedule_write",
            serde_json::json!({
                "timezone": "Europe/Rome",
                "pickup_time": "09:00",
                "schedule": [[],[],[],[],[],[],[]]
            }),
        );
        let req = test::TestRequest::post()
            .uri("/mcp")
            .insert_header(("content-type", "application/json"))
            .set_payload(write.clone())
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
        assert_eq!(
            resp.headers().get("www-authenticate").unwrap(),
            "Bearer realm=\"trashdiff\""
        );

        let req = test::TestRequest::post()
            .uri("/mcp")
            .insert_header(("content-type", "application/json"))
            .insert_header(("authorization", "Bearer pw"))
            .set_payload(write)
            .to_request();
        let raw = test::read_body(test::call_service(&app, req).await).await;
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert!(v["result"].get("isError").is_none());

        let _ = std::fs::remove_file(&db);
    }

    #[cfg(feature = "http")]
    #[actix_web::test]
    async fn admin_post_requires_password() {
        use crate::web::{AppState, admin_get, admin_post};
        use actix_web::{App, test, web};
        let db = temp_db();
        let json = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","schedule":[[],[],[],[],[],[],[]]}"#;
        let form = b"pickup_time=19:30&timezone=Nope";
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(AppState {
                    db: db.clone(),
                    auth: auth(),
                }))
                .route("/admin", web::get().to(admin_get))
                .route("/admin", web::post().to(admin_post)),
        )
        .await;

        let post = |ct: &str, body: &[u8], auth: Option<&str>| {
            let mut req = test::TestRequest::post()
                .uri("/admin")
                .insert_header(("content-type", ct));
            if let Some(v) = auth {
                req = req.insert_header(("authorization", v.to_string()));
            }
            req.set_payload(body.to_vec()).to_request()
        };
        let challenge = |resp: &actix_web::dev::ServiceResponse| {
            resp.headers()
                .get("www-authenticate")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        };

        // JSON write API: Bearer only.
        let resp = test::call_service(&app, post("application/json", json, None)).await;
        assert_eq!(resp.status(), 401);
        assert!(challenge(&resp).starts_with("Bearer"));
        assert!(!db.exists(), "unauthorized write must not create the file");
        assert_eq!(
            test::call_service(&app, post("application/json", json, Some(&basic("pw"))))
                .await
                .status(),
            401
        );
        assert_eq!(
            test::call_service(&app, post("application/json", json, Some("Bearer wrong")))
                .await
                .status(),
            401
        );
        assert!(!db.exists());
        assert_eq!(
            test::call_service(&app, post("application/json", json, Some("Bearer pw")))
                .await
                .status(),
            200
        );
        assert!(db.exists());

        // HTML form: Basic only.
        let resp = test::call_service(
            &app,
            post("application/x-www-form-urlencoded", form, Some("Bearer pw")),
        )
        .await;
        assert_eq!(resp.status(), 401);
        assert!(challenge(&resp).starts_with("Basic"));
        let resp = test::call_service(
            &app,
            post(
                "application/x-www-form-urlencoded",
                form,
                Some(&basic("pw")),
            ),
        )
        .await;
        // Authorized: reaches validation, which rejects the timezone.
        assert_eq!(resp.status(), 400);

        // The backoffice page itself: Basic only.
        let get = |auth: Option<&str>| {
            let mut req = test::TestRequest::get().uri("/admin");
            if let Some(v) = auth {
                req = req.insert_header(("authorization", v.to_string()));
            }
            req.to_request()
        };
        let resp = test::call_service(&app, get(None)).await;
        assert_eq!(resp.status(), 401);
        assert!(challenge(&resp).starts_with("Basic"));
        assert_eq!(
            test::call_service(&app, get(Some("Bearer pw")))
                .await
                .status(),
            401
        );
        assert_eq!(
            test::call_service(&app, get(Some(&basic("pw"))))
                .await
                .status(),
            200
        );

        let _ = std::fs::remove_file(&db);
    }
}
