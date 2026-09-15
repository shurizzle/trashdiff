use std::path::PathBuf;

use bytes::Bytes;
use chrono::Utc;
use http::{Method, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};

use crate::admin::{
    JsonWrite, WriteErrors, admin_json_write, body_is_json, form_from_body, process_admin,
};
use crate::i18n::{Lang, Localized, T};
use crate::schedule::State;
use crate::views::{
    AdminFormHtml, FormErrors, HomeHtml, Page, Theme, admin_form_from_state, admin_json, home_view,
};

pub fn lang_from_headers(h: &http::HeaderMap, st: &State) -> Lang {
    let cookie = h.get(http::header::COOKIE).and_then(|v| v.to_str().ok());
    let al = h
        .get(http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    Lang::from_req(cookie, al, st.default_lang.unwrap_or(Lang::En))
}

pub fn theme_from_headers(h: &http::HeaderMap) -> Theme {
    let cookie = h.get(http::header::COOKIE).and_then(|v| v.to_str().ok());
    Theme::from_req(cookie)
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

fn respond_json_status(
    status: StatusCode,
    json: String,
) -> http::Response<BoxBody<Bytes, std::io::Error>> {
    http::Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .body(boxed_body(Bytes::from(json)))
        .unwrap()
}

fn respond_json(json: String) -> http::Response<BoxBody<Bytes, std::io::Error>> {
    respond_json_status(StatusCode::OK, json)
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

pub fn route_cgi(
    st: &State,
    lng: Lang,
    theme: Theme,
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
    if path.ends_with("/theme/auto")
        || path.ends_with("/theme/light")
        || path.ends_with("/theme/dark")
    {
        let code = ["auto", "light", "dark"]
            .into_iter()
            .find(|c| path.ends_with(&format!("/theme/{c}")))
            .unwrap_or("auto");
        let back = headers
            .get(http::header::REFERER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("/");
        return redirect(
            back,
            Some(&format!(
                "theme={code}; Path=/; Max-Age=31536000; SameSite=Lax"
            )),
        );
    }
    if path == "/admin" || path.ends_with("/admin") {
        if *method == Method::POST {
            let ct = headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok());
            if body_is_json(ct, &body) {
                return match admin_json_write(&st.db_path, &body, lng) {
                    JsonWrite::ParseErr(msg) => respond_json_status(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "error": msg }).to_string(),
                    ),
                    JsonWrite::Saved => match State::load(st.db_path.clone()) {
                        Ok(fresh) => {
                            respond_json(serde_json::to_string(&admin_json(&fresh)).unwrap())
                        }
                        Err(e) => respond_json_status(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            serde_json::json!({ "error": e }).to_string(),
                        ),
                    },
                    JsonWrite::Invalid(errs) => respond_json_status(
                        StatusCode::BAD_REQUEST,
                        serde_json::to_string(&WriteErrors::from(errs)).unwrap(),
                    ),
                };
            }
            let f = form_from_body(&body);
            return match process_admin(f, &st.db_path, lng) {
                Ok(Some(form)) => {
                    let html = Localized::from((
                        lng,
                        Page(
                            T::TitleAdmin,
                            AdminFormHtml(form, FormErrors::default()),
                            theme,
                        ),
                    ))
                    .to_string();
                    respond(StatusCode::OK, html)
                }
                Ok(None) => redirect("/", None),
                Err((form, errs)) => {
                    let html = Localized::from((
                        lng,
                        Page(T::TitleAdmin, AdminFormHtml(form, errs), theme),
                    ))
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
                theme,
            ),
        ))
        .to_string();
        return respond(StatusCode::OK, html);
    }
    if path == "/home.json" || path.ends_with("/home.json") {
        return respond_json(serde_json::to_string(&home_view(st, Utc::now())).unwrap());
    }
    if path == "/admin.json" || path.ends_with("/admin.json") {
        return respond_json(serde_json::to_string(&admin_json(st)).unwrap());
    }
    let view = home_view(st, Utc::now());
    let html = Localized::from((lng, Page(T::TitleHome, HomeHtml(&view), theme))).to_string();
    respond(StatusCode::OK, html)
}

pub async fn cgi_run(db: PathBuf) -> std::io::Result<()> {
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
            let lng = lang_from_headers(&headers, &st);
            let theme = theme_from_headers(&headers);
            let resp: Result<http::Response<BoxBody<Bytes, std::io::Error>>, std::io::Error> =
                Ok(route_cgi(&st, lng, theme, &method, &path, &headers, body));
            resp
        },
    )
    .await
}

pub struct TokioRt;

impl cegla_fcgi::server::Runtime for TokioRt {
    fn spawn(&self, future: impl std::future::Future + Send + 'static) {
        tokio::spawn(async move {
            future.await;
        });
    }
}

pub async fn read_body_capped<B>(mut body: B, content_length: usize) -> Result<Bytes, std::io::Error>
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

pub async fn fcgi_run(bind: String, db: PathBuf) -> std::io::Result<()> {
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
                        let st = State::load(db).map_err(std::io::Error::other)?;
                        let lng = lang_from_headers(&headers, &st);
                        let theme = theme_from_headers(&headers);
                        let resp: Result<
                            http::Response<BoxBody<Bytes, std::io::Error>>,
                            std::io::Error,
                        > = Ok(route_cgi(&st, lng, theme, &method, &path, &headers, body));
                        resp
                    }
                })
                .await;
        });
    }
}

pub async fn scgi_run(bind: String, db: PathBuf) -> std::io::Result<()> {
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
                    let st = State::load(db).map_err(std::io::Error::other)?;
                    let lng = lang_from_headers(&headers, &st);
                    let theme = theme_from_headers(&headers);
                    let resp: Result<
                        http::Response<BoxBody<Bytes, std::io::Error>>,
                        std::io::Error,
                    > = Ok(route_cgi(&st, lng, theme, &method, &path, &headers, body));
                    resp
                }
            })
            .await;
        });
    }
}

pub fn cli_cmd(db_path: PathBuf) -> Result<(), String> {
    let st = State::load(db_path)?;
    let lng = Lang::from_env();
    let now = Utc::now().with_timezone(&st.timezone);
    let (open_date, _wd, open_type) = st.next_boundary(now);
    let open_dt = open_date
        .and_time(st.pickup_time)
        .and_local_timezone(st.timezone)
        .earliest()
        .expect("local boundary not resolvable");
    if open_type.is_empty() {
        println!("{}", Localized::from((lng, T::Pause(open_dt))));
    } else {
        println!(
            "{} ({})",
            Localized::from((lng, T::NowOpen(open_type))),
            Localized::from((lng, T::WindowUntil(open_dt)))
        );
    }
    Ok(())
}
