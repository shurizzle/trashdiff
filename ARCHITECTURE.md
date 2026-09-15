# ARCHITECTURE.md

# Architecture

This file documents the high-level architecture of the codebase.
It helps AI agents understand the project structure, key modules,
data flow, and design decisions without having to re-discover
them on every exploration.

## 1. Directory layout

Single binary crate (`trashdiff`, `Cargo.toml`); the logic is split by domain
into a handful of modules plus static assets.

| Path | Responsibility |
|---|---|
| `src/main.rs` | Entry point: `Cli`/`Command` (clap), `main()` dispatch, and the whole test module |
| `src/schedule.rs` | Domain + disk layer: `Week`, `Entry`, `Db`/`DbOld`, `State`, the window algorithm |
| `src/views.rs` | `Theme` and all view models / rendering: `Page`, `HomeHtml`/`HomeView`, `AdminForm`/`AdminFormHtml`/`FormErrors`, `AdminJson`, `home_view`, `admin_json` |
| `src/admin.rs` | Admin write path: form/JSON parsing, `validate_and_save`, `process_admin`, `AdminWrite`/`WriteErrors`/`JsonWrite` |
| `src/web.rs` | actix-web HTTP frontend: `AppState`, handlers, `serve` |
| `src/transport.rs` | Non-actix frontends: `route_cgi`, `cgi_run`/`fcgi_run`/`scgi_run`, `cli_cmd`, response helpers |
| `src/i18n.rs` | `Lang` (IT/EN), the `T` message-key enum, translation table, HTML escaping, `LocalizedDisplay` trait |
| `src/admin.js` | Progressive-enhancement JS for the backoffice (add/remove rows, client-side validation); no framework |
| `src/style.css`, `src/dark.css` | Light + dark stylesheets |
| `build.rs` | Minifies `admin.js` (oxc) and the CSS (lightningcss) into `$OUT_DIR` at build time |
| `Dockerfile`, `.github/` | Packaging / CI |

Tests are inline `#[cfg(test)]` modules: the main integration-style suite lives at
the bottom of `main.rs`, and `i18n.rs` has its own.

## 2. Key types and relationships

**Config / disk layer** (`schedule.rs`):
- `Db` — serde TOML struct: `timezone`, `pickup_time`, `schedule: Vec<Entry>`, optional `default_lang`.
- `Entry` — one schedule line: `day: String`, `weeks: Week`, `kind: String` (`#[serde(rename="type")]`).
- `DbOld` — legacy format `schedule: HashMap<String,String>` (day→type); auto-migrated by `migrate_old`.
- `State` — parsed, in-memory runtime form (not serde): `db_path`, `timezone: Tz`, `pickup_time: NaiveTime`, `schedule`, `default_lang`. Rebuilt per request via `State::load`.

**Schedule primitive**: `Week` — a `bitflags! u8` set (`FIRST..FIFTH`) with custom serde as an int list 1..5. Enables union/difference/overlap checks in validation.

**Localization** (`i18n.rs`): `Lang` (serde lowercase `"it"`/`"en"`), `T<'a>` message keys, `LocalizedT` holding a concrete language, `LocalizedDisplay` trait, wrappers `Localized`/`LocalizedRef`, plus `HtmlEscape`/`esc`.

**View/response models** (`views.rs`): `Theme` (Auto/Light/Dark cookie cycle), `Page<Title,Body>` (HTML shell), `HomeHtml`, `HomeView`/`NowWindow` (home JSON), `AdminJson`/`AdminRow`, `AdminForm`/`FormErrors`, `AdminFormHtml`.

**Relationship**: disk `Db` → `State` (per request) → `HomeView`/`AdminJson` (JSON) or `HomeHtml`/`AdminFormHtml` (HTML via `LocalizedDisplay`). Writes: form/`AdminWrite` JSON → `AdminForm` → `validate_and_save` → `Db` → `State::save_file`.

## 3. Control flow (request → response)

**Entry point** `main()` (`main.rs`):
- No args + `GATEWAY_INTERFACE` starts with `CGI/` → auto-run `transport::cgi_run`.
- Otherwise clap `Cli`/`Command` dispatches to `transport::cli_cmd` / `web::serve` / `transport::cgi_run` / `transport::fcgi_run` / `transport::scgi_run`.

**HTTP** (`web.rs`): actix-web `HttpServer` with routes — `GET /`→`home`, `GET /home.json`→`home_json_endpoint`, `GET|POST /admin`→`admin_get`/`admin_post`, `GET /admin.json`→`admin_json_endpoint`, `GET /lang/{code}`→`switch_lang`, `GET /theme/{code}`→`switch_theme`. Uses `actix_web::rt::Runtime`. Only the `PathBuf` is cached (`AppState`); `State` is reloaded per request.

**CGI / FCGI / SCGI** (`transport.rs`) share one manual router `route_cgi`: `path.trim_end_matches('/')` matched against `/lang/*`, `/theme/*` (303 + `Set-Cookie`), `/admin` (POST→JSON or form, GET→HTML), `/home.json`, `/admin.json`, else home. Transport drivers: `cgi_run` (`cegla_cgi::server::handle_request`), `fcgi_run` (`TcpListener` + `cegla_fcgi::server::server_handle_fcgi`), `scgi_run` (`server_handle_scgi`); all load `State` then call `route_cgi`. Body read via generic `read_body_capped<B>`. Responses are `http::Response<BoxBody<Bytes, io::Error>>` built by `respond`/`respond_json`/`redirect`.

**Static assets**: none served — `style.min.css`/`dark.min.css` are inlined in `Page::fmt` (`views.rs`) and `admin.min.js` in `AdminFormHtml::fmt` via `include_str!`.

## 4. Data flow

- **Window algorithm** (`schedule.rs`): `week_of_month(date)` = `(day-1)/7+1`; `State::type_for(date)` finds the `Entry` for that weekday+week; `State::boundary(date)` = local date at `pickup_time` in `State.timezone`; `State::next_boundary(now)` scans 0..=7 days for the first boundary `> now`. Semantics (README "Concept"): a pickup on day D is throwable from D-1 `pickup_time` to D `pickup_time`. `home_view` (`views.rs`) builds the current Monday..Sunday week plus `now.kind`/`now.until`.
- **Load**: `State::load` opens the file, takes a **shared** lock, parses `Db` (fallback `DbOld`→migrate), parses tz/time. Missing file → defaults (`Europe/Rome`, `05:00`).
- **Save**: `validate_and_save` (`admin.rs`) builds `Db` and calls `State::save_file`: pretty TOML → `OpenOptions` → **exclusive** lock → `set_len(0)` → `write_all` → `sync_all`.
- **Admin write path**: form (`process_admin`) and JSON (`admin_json_write`) both reuse `validate_and_save`; validation rejects empty types and overlapping `(day, week)` pairs (`FormErrors`/`WriteErrors`).

## 5. Design decisions & rationale

- **One binary, thin crate root**: `main.rs` only wires CLI + runtime; concrete frontends over shared `http`-crate types live in `web.rs`/`transport.rs`; the domain is isolated in `schedule.rs`. Rationale: CGI/FCGI/SCGI are drop-in deployments (README), all reusing `route_cgi`.
- **Module split is by role, not abstraction**: no local `trait Frontend`. Genericity comes from body-typed helpers (`read_body_capped`, test `scgi_handler`) and implementing `cegla_fcgi::server::Runtime for TokioRt`.
- **TOML DB, file-locked, re-read per request**: only the `PathBuf` is cached (`AppState`), so multiple instances can share the file (README "Database file") and state can never go stale. Human-editable format.
- **`timezone` + `pickup_time`** (IANA name + naive `HH:MM`) rather than a fixed offset: servers usually run UTC, schedule is wall-clock local time.
- **Old-format auto-migration**: new `Db` tried first, `DbOld` fallback expanded to full-week entries and rewritten in place (backward compatibility).
- **JSON vs form POST on one endpoint**: `body_is_json` sniffs Content-Type or a leading `{`, giving a machine full-replace API while keeping the JS-free form path.

## 6. External dependencies

| Crate | Use |
|---|---|
| `actix-web` (optional, `http` feature) | Built-in HTTP server + routing (`web::serve`) |
| `cegla-cgi` / `cegla-fcgi` / `cegla-scgi` (`server` feature; optional, one per CGI feature) | CGI-family wire protocols; app supplies a body-generic handler |
| `tokio` (`rt`, `net`, `io-std`, `io-util`; optional) | Runtime + sockets for CGI/FCGI/SCGI |
| `bitflags` | `Week` 1..5 bitmask |
| `chrono` / `chrono-tz` | Time/date math; `Tz` IANA resolve + `TZ_VARIANTS` for the timezone `<select>` |
| `clap` (`derive`, `env`) | Subcommands + `TRASHDIFF_DB` env |
| `serde` / `serde_json` / `toml` | (De)serialization |
| `http` / `http-body-util` / `bytes` | Shared request/response types + boxed body |
| `form_urlencoded` | URL-encoded form parsing |
| `lightningcss`, `oxc_*` (build-dep) | Minify CSS/JS into `$OUT_DIR` |

## 7. Entry points / execution modes

| Mode | Entry | Default bind | Runtime |
|---|---|---|---|
| `http` | `web::serve` | `127.0.0.1:8080` | actix `rt::Runtime` |
| `cgi` | `transport::cgi_run`, auto-detected via `GATEWAY_INTERFACE` | — | tokio |
| `fcgi` | `transport::fcgi_run` | `127.0.0.1:9000` | tokio |
| `scgi` | `transport::scgi_run` | `127.0.0.1:4000` | tokio |
| `cli` | `transport::cli_cmd` | prints what can be thrown now | tokio |

`--db` defaults to `trashdb.toml` (env `TRASHDIFF_DB`; explicit flag wins).

### Build features

Each frontend sits behind a Cargo feature, all enabled by default: `http`,
`cgi`, `fcgi`, `scgi`. The `cli` subcommand is always built. Turning a feature
off removes its subcommand and its dependencies (`actix-web` for `http`,
`cegla-cgi`/`cegla-fcgi`/`cegla-scgi` + `tokio` for the CGI family), e.g.
`cargo build --no-default-features --features fcgi`. The transport code in
`transport.rs` and the `web` module are `#[cfg]`-gated accordingly; only the
`PathBuf`-loading `cli_cmd` and shared domain logic stay unconditional.
