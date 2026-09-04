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

## Usage

```
trashdiff http [--bind HOST:PORT] [--db PATH]
trashdiff cgi  [--db PATH]
trashdiff fcgi [--bind HOST:PORT] [--db PATH]
trashdiff scgi [--bind HOST:PORT] [--db PATH]
trashdiff cli  [--db PATH]
```

`--db` defaults to `trashdb.toml` and can be set via the `TRASHDIFF_DB`
environment variable (an explicit flag wins).

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
  `pickup_time`, `default_lang` (`null` = auto → English) and `schedule`, an
  array of 7 arrays (Monday..Sunday), each holding that weekday's rows
  `{ "weeks": [...], "type": "..." }` in the order shown on the page
- `/admin` — backoffice: one row per pickup under each weekday (weekdays are
  fixed and always shown). Tick weeks 1-5 and type the waste type; `+`
  duplicates that weekday, `-` removes the row, no JS needed. Plus global
  pickup time and timezone
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

[[schedule]]
day = "monday"
weeks = [1, 3]
type = "Carta"
```

- `timezone`: IANA name (required — servers usually run UTC, the schedule is
  wall-clock local time)
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

Window logic plus CGI/FastCGI/SCGI protocol round-trips (duplex sockets, no
web server needed).
