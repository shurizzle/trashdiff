use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod admin;
mod i18n;
mod schedule;
mod transport;
mod views;
mod web;

use crate::transport::{cgi_run, cli_cmd, fcgi_run, scgi_run};
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

#[cfg(test)]
mod tests {
    use crate::admin::*;
    use crate::i18n::Lang;
    use crate::schedule::*;
    use crate::transport::*;
    use crate::views::*;

    use std::path::PathBuf;
    use std::sync::Arc;

    use bytes::Bytes;
    use chrono::{DateTime, NaiveTime, Utc, Weekday};
    use chrono_tz::Tz;
    use http_body_util::BodyExt;
    use http_body_util::combinators::BoxBody;

    fn w(ws: &[u32]) -> Week {
        ws.iter().fold(Week::empty(), |acc, &n| acc | week_of(n))
    }

    fn state() -> State {
        state_with_db(PathBuf::from("/nonexistent"))
    }

    fn state_with_db(db_path: PathBuf) -> State {
        let schedule = vec![
            Entry {
                day: "monday".to_string(),
                weeks: w(&[1]),
                kind: "Carta".to_string(),
            },
            Entry {
                day: "tuesday".to_string(),
                weeks: w(&[1]),
                kind: "Umido".to_string(),
            },
        ];
        State {
            db_path,
            timezone: "Europe/Rome".parse().unwrap(),
            pickup_time: NaiveTime::parse_from_str("17:00", "%H:%M").unwrap(),
            schedule,
            default_lang: None,
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
            kind: "Plastica".to_string(),
        });
        st.default_lang = Some(Lang::En);
        let json = serde_json::to_value(admin_json(&st)).unwrap();
        assert_eq!(json["timezone"], "Europe/Rome");
        assert_eq!(json["pickup_time"], "17:00");
        assert_eq!(json["default_lang"], "en");
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
                    weeks: w(&[1, 2]),
                    kind: "Carta".to_string(),
                },
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[2, 3]),
                    kind: "Plastica".to_string(),
                },
            ],
            action: "save".to_string(),
            default_lang: String::new(),
        };
        let errs = validate_and_save(&PathBuf::from("/nonexistent"), &f, Lang::It).unwrap_err();
        assert!(errs.fields.contains_key("monday:1"));
        assert_eq!(errs.bad_weeks.get(&("monday", 1)), Some(&week_of(2)));
    }

    #[test]
    fn empty_type_rejected() {
        let f = AdminForm {
            timezone: "Europe/Rome".to_string(),
            pickup_time: "17:00".to_string(),
            entries: vec![
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[1]),
                    kind: "Carta".to_string(),
                },
                Entry {
                    day: "monday".to_string(),
                    weeks: w(&[2]),
                    kind: String::new(),
                },
            ],
            action: "save".to_string(),
            default_lang: String::new(),
        };
        let errs = validate_and_save(&PathBuf::from("/nonexistent"), &f, Lang::It).unwrap_err();
        assert!(errs.fields.contains_key("monday:1"));
        assert!(errs.bad_weeks.is_empty());
        // zero weeks but empty type: row ignored, no error
        let f2 = AdminForm {
            entries: vec![Entry {
                day: "monday".to_string(),
                weeks: Week::empty(),
                kind: String::new(),
            }],
            ..f
        };
        let path =
            std::env::temp_dir().join(format!("trashdiff_empty_type_{}", std::process::id()));
        assert!(validate_and_save(&path, &f2, Lang::It).is_ok());
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
                    let lng = lang_from_headers(&headers, &st);
                    let theme = theme_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, theme, &method, &path, &headers, body));
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
                    let lng = lang_from_headers(&headers, &st);
                    let theme = theme_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, theme, &method, &path, &headers, body));
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
        let lng = lang_from_headers(&headers, st);
        let theme = theme_from_headers(&headers);
        Ok(route_cgi(st, lng, theme, &method, &path, &headers, body))
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
        let body = br#"{"timezone":"Europe/Rome","pickup_time":"08:00","default_lang":null,"schedule":[[{"weeks":[1,3],"type":"Carta"}],[],[],[],[],[],[]]}"#;
        match admin_json_write(&db, body, Lang::En) {
            JsonWrite::Saved => {}
            other => panic!("expected Saved, got {other:?}"),
        }
        let raw = std::fs::read_to_string(&db).unwrap();
        assert!(raw.contains("pickup_time = \"08:00\""), "raw:\n{raw}");
        assert!(raw.contains("type = \"Carta\""));
        assert!(raw.contains("\n    1,"));
        assert!(raw.contains("\n    3,"));
        assert!(!raw.contains("tuesday"));
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

    #[test]
    fn body_is_json_detects_ct_and_brace() {
        assert!(body_is_json(Some("application/json"), b"x"));
        assert!(body_is_json(None, b"  {\"a\":1}"));
        assert!(!body_is_json(None, b"pickup_time=17:00"));
        assert!(!body_is_json(Some("text/plain"), b"pickup_time=17:00"));
    }

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

    #[tokio::test]
    async fn scgi_roundtrip_post_admin_json_rejects_bad_body() {
        let response = run_scgi_post(state(), br#"{"timezone":5}"#).await;
        assert!(response.contains("Status: 400"));
        assert!(response.contains("error"));
        assert!(response.contains("invalid JSON"));
    }
}
