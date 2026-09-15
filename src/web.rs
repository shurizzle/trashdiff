use std::path::PathBuf;

use actix_web::http::header::{COOKIE, LOCATION};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use chrono::Utc;

use crate::admin::{
    JsonWrite, WriteErrors, admin_json_write, body_is_json, form_from_body, process_admin,
};
use crate::i18n::{Lang, Localized, T};
use crate::schedule::State;
use crate::views::{
    AdminFormHtml, FormErrors, HomeHtml, Page, Theme, admin_form_from_state, admin_json, home_view,
};

fn lang_of(req: &HttpRequest, st: &State) -> Lang {
    let cookie = req.headers().get(COOKIE).and_then(|v| v.to_str().ok());
    let al = req
        .headers()
        .get(actix_web::http::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    Lang::from_req(cookie, al, st.default_lang.unwrap_or(Lang::En))
}

fn theme_of(req: &HttpRequest) -> Theme {
    let cookie = req.headers().get(COOKIE).and_then(|v| v.to_str().ok());
    Theme::from_req(cookie)
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

async fn home(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    let lng = lang_of(&req, &st);
    let theme = theme_of(&req);
    let view = home_view(&st, Utc::now());
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(Localized::from((lng, Page(T::TitleHome, HomeHtml(&view), theme))).to_string())
}

async fn home_json_endpoint(_req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    let json = serde_json::to_string(&home_view(&st, Utc::now())).unwrap();
    HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(json)
}

fn admin_post_json(db_path: &PathBuf, lng: Lang, body: &[u8]) -> HttpResponse {
    match admin_json_write(db_path, body, lng) {
        JsonWrite::ParseErr(msg) => HttpResponse::BadRequest()
            .content_type("application/json; charset=utf-8")
            .body(serde_json::json!({ "error": msg }).to_string()),
        JsonWrite::Saved => match State::load(db_path.clone()) {
            Ok(fresh) => HttpResponse::Ok()
                .content_type("application/json; charset=utf-8")
                .body(serde_json::to_string(&admin_json(&fresh)).unwrap()),
            Err(e) => HttpResponse::InternalServerError()
                .content_type("text/plain; charset=utf-8")
                .body(e),
        },
        JsonWrite::Invalid(errs) => HttpResponse::BadRequest()
            .content_type("application/json; charset=utf-8")
            .body(serde_json::to_string(&WriteErrors::from(errs)).unwrap()),
    }
}

async fn admin_get(req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    let lng = lang_of(&req, &st);
    let theme = theme_of(&req);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(
            Localized::from((
                lng,
                Page(
                    T::TitleAdmin,
                    AdminFormHtml(admin_form_from_state(&st), FormErrors::default()),
                    theme,
                ),
            ))
            .to_string(),
        )
}

async fn admin_post(req: HttpRequest, data: web::Data<AppState>, body: web::Bytes) -> HttpResponse {
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    let lng = lang_of(&req, &st);
    let theme = theme_of(&req);
    let ct = req
        .headers()
        .get(actix_web::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if body_is_json(ct, &body) {
        return admin_post_json(&data.0, lng, &body);
    }
    let f = form_from_body(&body);
    match process_admin(f, &data.0, lng) {
        Ok(Some(form)) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(
                Localized::from((
                    lng,
                    Page(
                        T::TitleAdmin,
                        AdminFormHtml(form, FormErrors::default()),
                        theme,
                    ),
                ))
                .to_string(),
            ),
        Ok(None) => HttpResponse::SeeOther()
            .insert_header((LOCATION, "/"))
            .finish(),
        Err((form, errs)) => HttpResponse::BadRequest()
            .content_type("text/html; charset=utf-8")
            .body(
                Localized::from((lng, Page(T::TitleAdmin, AdminFormHtml(form, errs), theme)))
                    .to_string(),
            ),
    }
}

async fn admin_json_endpoint(_req: HttpRequest, data: web::Data<AppState>) -> HttpResponse {
    let st = match load_or_500(&data) {
        Ok(st) => st,
        Err(resp) => return resp,
    };
    let json = serde_json::to_string(&admin_json(&st)).unwrap();
    HttpResponse::Ok()
        .content_type("application/json; charset=utf-8")
        .body(json)
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

async fn switch_theme(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let theme = match path.into_inner().as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => "auto",
    };
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
            format!("theme={theme}; Path=/; Max-Age=31536000; SameSite=Lax"),
        ))
        .finish()
}

pub async fn serve(bind: String, db_path: PathBuf) -> std::io::Result<()> {
    let data = web::Data::new(AppState(db_path));
    println!("trashdiff listening on http://{bind}");
    HttpServer::new(move || {
        App::new()
            .app_data(data.clone())
            .route("/", web::get().to(home))
            .route("/home.json", web::get().to(home_json_endpoint))
            .route("/admin", web::get().to(admin_get))
            .route("/admin", web::post().to(admin_post))
            .route("/admin.json", web::get().to(admin_json_endpoint))
            .route("/lang/{code}", web::get().to(switch_lang))
            .route("/theme/{code}", web::get().to(switch_theme))
    })
    .bind(bind)?
    .run()
    .await
}
