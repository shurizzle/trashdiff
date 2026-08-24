# trashdiff

Weekly waste collection schedule. Tracks what gets picked up when, and when
you can put each waste type out.

## Concept

Each weekday has a pickup type and a global pickup time. The window for the
type collected on day D opens at the pickup time on D-1 and closes at the
pickup time on D.

Example: `monday = "Carta"`, `pickup_time = "17:00"` → paper can be put out
from Sunday 17:00 to Monday 17:00.

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

- `/` — what to throw now, today's pickup, weekly table
- `/admin` — backoffice: per-day waste types (free text, empty = no pickup),
  global pickup time, timezone
- `EN`/`IT` toggle — language switch, persisted in a cookie
- Dark mode — follows the OS theme (`prefers-color-scheme`)

## Database file

TOML, created with defaults on first run. Edited from the backoffice.

```toml
timezone    = "Europe/Rome"
pickup_time = "17:00"

[schedule]
monday    = "Carta"
tuesday   = "Umido"
```

- `timezone`: IANA name (required — servers usually run UTC, the schedule is
  wall-clock local time)
- `schedule`: weekday (`monday`..`sunday`) → waste type; missing/empty = no
  pickup that day

## Test

```sh
cargo test
```

Window logic plus CGI/FastCGI/SCGI protocol round-trips (duplex sockets, no
web server needed).
