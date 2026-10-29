# trashdiff

Weekly waste collection schedule. Tracks what gets picked up when, and when
you can put each waste type out.

## Concept

Each schedule entry ties a waste type to a weekday and a set of weeks of the
month (1st..5th). The window for the type collected on day D opens at the
pickup time on D-1 and closes at the pickup time on D.

Example: `entry day = monday, weeks = [1, 3], type = "Carta"`, `pickup_time =
"17:00"` → paper is collected on the 1st and 3rd Monday of the month, and can
be put out from the preceding Sunday 17:00 to Monday 17:00.

## Build

```sh
cargo build --release
```

Each frontend is behind a Cargo feature (all enabled by default): `http`,
`cgi`, `fcgi`, `scgi`, `mcp`. The `cli` subcommand and the domain logic are always
built. Build only what you deploy:

```sh
cargo build --release --no-default-features --features scgi
```

`actix-web` (`http`) and the `cegla-*` crates (`cgi`/`fcgi`/`scgi`) are only
pulled in by their feature.

## Usage

```
trashdiff http [--bind HOST:PORT] [--db PATH]
trashdiff cgi  [--db PATH]
trashdiff fcgi [--bind HOST:PORT] [--db PATH]
trashdiff scgi [--bind HOST:PORT] [--db PATH]
trashdiff mcp  [--db PATH]
trashdiff cli  [--db PATH]
```

`--db` defaults to `trashdb.toml` and can be set via the `TRASHDIFF_DB`
environment variable (an explicit flag wins).

## Writes and authentication

Public, no credentials: `/`, `/home.json`, `/admin.json` and the MCP read
tools. The backoffice page (`GET /admin`) is protected with Basic like its
save, and every write is password-protected and **disabled unless
`TRASHDIFF_PASSWORD` is set** (fail closed):

```sh
TRASHDIFF_PASSWORD='change-me' trashdiff http
```

- backoffice page and form (`GET /admin`, and `POST /admin` without a JSON
  body): **Basic only** — the browser prompts for the password (any user name,
  the password is what counts);
- JSON write API (`POST /admin` with a JSON body): **Bearer only** —
  `curl -H "Authorization: Bearer $TRASHDIFF_PASSWORD"`, Basic is refused;
- MCP writes (`schedule_write`): **Bearer only**;

The scheme is enforced: a correct password under the wrong scheme is refused.
`GET`/`POST /admin` without valid credentials answers `401` with
`WWW-Authenticate: Basic realm="trashdiff"` (page and form) or
`Bearer realm="trashdiff"` (JSON); an unauthenticated `schedule_write` answers
`401` with the Bearer challenge (over stdio: JSON-RPC error `-32001`). Neither
reads nor creates the database when refused.

The password travels in clear text inside the request (Basic/Bearer are not
encrypted by themselves): terminate TLS at the reverse proxy in front of
trashdiff. It is read once at process start, so changing it requires a restart.

For `trashdiff mcp` over stdio there is no header; the client passes the
password in the process environment.

### http

Built-in web server (actix-web). Default `http://127.0.0.1:8080`.

```sh
trashdiff http
```

### cgi

Classic CGI (RFC 3875). One process per request; deploy by dropping the
binary in `cgi-bin` and pointing Apache/nginx at it.

```sh
# Apache: ScriptAlias /trashdiff/ /path/to/cgi-bin/trashdiff
TRASHDIFF_DB=/abs/path/trashdb.toml trashdiff cgi
```

### fcgi

FastCGI server, persistent process. Point nginx at it:

```sh
trashdiff fcgi --bind 127.0.0.1:9000
```

```nginx
location / {
    fastcgi_pass 127.0.0.1:9000;
    include fastcgi_params;
}
```

### scgi

SCGI server (nginx `mod_scgi`):

```sh
trashdiff scgi --bind 127.0.0.1:4000
```

```nginx
location / {
    scgi_pass 127.0.0.1:4000;
    include scgi_params;
}
```

### mcp

Model Context Protocol (JSON-RPC 2.0), for AI clients. Tools: `what_to_throw`
(same data as `/home.json`), `schedule_read` (same shape as `/admin.json`)
and `schedule_write` (full replace, same validation as `POST /admin`).

Two transports:

- **Streamable HTTP** — `POST /mcp` on the regular HTTP server. Stateless: no
  session id, no server-initiated SSE stream (`GET`/`DELETE /mcp` answer
  `405`). Requests get `200` with `application/json`; notifications get `202`
  with an empty body.

  ```json
  {
    "mcpServers": {
      "trashdiff": {
        "type": "http",
        "url": "http://127.0.0.1:8080/mcp",
        "headers": { "Authorization": "Bearer change-me" }
      }
    }
  }
  ```

- **stdio** — the client spawns the process. No network port.

  ```json
  {
    "mcpServers": {
      "trashdiff": {
        "command": "trashdiff",
        "args": ["mcp", "--db", "/srv/trashdb.toml"],
        "env": { "TRASHDIFF_PASSWORD": "change-me" }
      }
    }
  }
  ```

`schedule_write` requires the password (see [Writes and
authentication](#writes-and-authentication)); `what_to_throw` and
`schedule_read` stay public.

`trashdiff mcp` writes nothing to stdout except protocol frames; errors go to
stderr and to JSON-RPC error frames.

### cli

Print what can be thrown right now. Output language follows the `LANG`
environment variable.

```sh
trashdiff cli
# You can throw now: Carta (Window open until Lun 24/08 alle 17:00.)
```

## Web interface

- `/` — what to throw now, today's pickup, weekly table (current week)
- `/home.json` — same view as JSON: `timezone`, `pickup_time` (`HH:MM`),
  `now` (`kind` = waste type currently out, `null` = pause, with `until` =
  when it ends) and `week` as waste-type strings (or `null` when nothing is
  collected); `week` is the current week, 7 entries Monday..Sunday
- `/admin.json` — backoffice configuration as JSON: `timezone`,
  `pickup_time`, `default_lang` (`null` = auto → English), `custom_js` and
  `custom_css` (`null` = none; URL or inline content) and `schedule`, an
  array of 7 arrays (Monday..Sunday), each holding that weekday's rows
  `{ "weeks": [...], "type": "..." }` in the order shown on the page
- `/mcp` — Model Context Protocol endpoint (streamable HTTP, stateless); see
  the `mcp` section below
- `POST /admin` — full-replace write API: send the `admin.json` shape with
  `Content-Type: application/json` (a body starting with `{` is also
  detected). Requires `Authorization: Bearer $TRASHDIFF_PASSWORD` (Basic is
  refused), otherwise `401`. Validated like the backoffice save (type required for the ticked
  weeks, no overlapping `(day, week)` pairs, empty `weeks` rows dropped; on
  success returns `200` with the refreshed `admin.json`, invalid input gives
  `400` (`{"error": ...}` for parse failures, `{fields, overlaps}` for
  validation errors). Form posts are unchanged
- `/admin` — backoffice, password required (Basic): one row per pickup under
  each weekday (weekdays are fixed and always shown). Tick weeks 1-5 and type the waste type; `+`
  duplicates that weekday, `-` removes the row, no JS needed. Saving asks for
  the password (browser dialog); without `TRASHDIFF_PASSWORD` saving is
  disabled. Plus global
  pickup time, timezone and the custom JavaScript/CSS fields
- `EN`/`IT` toggle — language switch, persisted in a cookie
- Dark mode — follows the OS theme (`prefers-color-scheme`)

## Database file

TOML, created with defaults on first run. Edited from the backoffice. The
file is locked (shared on read, exclusive on write) and re-read on every
request, so multiple instances can run against the same database
concurrently.

```toml
timezone    = "Europe/Rome"
pickup_time = "17:00"
custom_js   = "https://example.com/analytics.js"
custom_css  = "https://example.com/custom.css"

[[schedule]]
day = "monday"
weeks = [1, 3]
type = "Carta"
```

- `timezone`: IANA name (required — servers usually run UTC, the schedule is
  wall-clock local time)
- `custom_js` (optional): added as the last element before `</body>` on every
  HTML page. If the value is a URL (`http://…`, `https://…`, `//host/…` or a
  relative reference such as `/js/a.js` or `./a.js`) it is emitted as
  `<script src="…">`; otherwise it is emitted inline as a `<script>` child.
  Inline code is minified with oxc (same pipeline as `admin.js`); if it does
  not parse, the trimmed original is emitted instead. Editable from the
  backoffice and via the `admin.json` write API.
- `custom_css` (optional): added in `<head>`, right after the built-in
  `<style>`, on every HTML page. If the value is a URL (same forms as
  `custom_js`) it is emitted as `<link rel="stylesheet" href="…">`; otherwise
  it is emitted inline as a `<style>` child. Inline CSS is minified with
  lightningcss (same pipeline as `style.css`); if it does not parse, the
  trimmed original is emitted instead. Editable from the backoffice and via
  the `admin.json` write API.
- `schedule`: list of entries; each entry has a weekday (`monday`..`sunday`),
  the weeks of the month (1-5) it applies to, and the waste type. Entries
  must not overlap on the same `(day, week)` pair — the backoffice rejects
  duplicates.
- Old single-key format (`monday = "Carta"`) is auto-detected and migrated to
  one entry covering all 5 weeks on load.

## Test

```sh
cargo test
```

Window logic, CGI/FastCGI/SCGI protocol round-trips (duplex sockets, no
web server needed), MCP JSON-RPC framing/tools and the write password gate
(Basic/Bearer parsing, `401` without credentials, refused writes never create
or read the database).
