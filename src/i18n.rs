#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    It,
    En,
}

impl Lang {
    pub fn from_req(cookie: Option<&str>, accept_language: Option<&str>) -> Lang {
        if let Some(c) = cookie.and_then(parse_cookie_lang) {
            return c;
        }
        if accept_language
            .map(|a| a.to_ascii_lowercase().starts_with("it"))
            .unwrap_or(false)
        {
            Lang::It
        } else {
            Lang::En
        }
    }

    pub fn from_env() -> Lang {
        match std::env::var("LANG") {
            Ok(l) if l.to_ascii_lowercase().starts_with("it") => Lang::It,
            _ => Lang::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::It => "it",
            Lang::En => "en",
        }
    }
}

fn parse_cookie_lang(cookie: &str) -> Option<Lang> {
    cookie.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == "lang").then(|| match v {
            "it" => Lang::It,
            _ => Lang::En,
        })
    })
}

pub fn t(lng: Lang, key: &str) -> &'static str {
    match lng {
        Lang::It => match key {
            "title_home" => "Differenziata",
            "title_admin" => "Backoffice",
            "nav_home" => "Home",
            "nav_admin" => "Backoffice",
            "now_open" => "Adesso puoi buttare: {}",
            "window_until" => "Finestra aperta fino a {}.",
            "pause" => "Pausa: nessun ritiro in corso. Prossima raccolta da {}.",
            "today_pickup" => "Oggi ({}) ritirano: {} alle {}",
            "today_none" => "Oggi ({}) nessun ritiro.",
            "week" => "{}ª settimana",
            "col_day" => "Giorno",
            "col_type" => "Tipo",
            "col_window" => "Finestra",
            "pickup_time_label" => "Ora ritiro (globale, HH:MM)",
            "tz_label" => "Timezone",
            "empty_hint" => "Una riga per ogni ritiro: spunta le settimane del mese (1-5) e scrivi il tipo. Il + duplica il giorno, il - elimina la riga. Campo tipo vuoto = riga ignorata.",
            "save" => "Salva",
            "err_tz" => "timezone non valida",
            "err_time" => "pickup_time non valido (atteso HH:MM)",
            "err_io" => "errore salvataggio database",
            "err_overlap" => "Sovrapposizione: {} della {}ª settimana già assegnato.",
            _ => "",
        },
        Lang::En => match key {
            "title_home" => "Waste collection",
            "title_admin" => "Backoffice",
            "nav_home" => "Home",
            "nav_admin" => "Backoffice",
            "now_open" => "You can throw now: {}",
            "window_until" => "Window open until {}.",
            "pause" => "Pause: no pickup running. Next pickup from {}.",
            "today_pickup" => "Today ({}) pickup: {} at {}",
            "today_none" => "Today ({}) no pickup.",
            "week" => "Week {}",
            "col_day" => "Day",
            "col_type" => "Type",
            "col_window" => "Window",
            "pickup_time_label" => "Pickup time (global, HH:MM)",
            "tz_label" => "Timezone",
            "empty_hint" => "One row per pickup: tick the weeks of the month (1-5) and write the type. The + duplicates the day, the - removes the row. Empty type field = row ignored.",
            "save" => "Save",
            "err_tz" => "invalid timezone",
            "err_time" => "invalid pickup_time (expected HH:MM)",
            "err_io" => "database save error",
            "err_overlap" => "Overlap: {} of week {} already assigned.",
            _ => "",
        },
    }
}

pub fn days(lng: Lang) -> [&'static str; 7] {
    match lng {
        Lang::It => ["Lun", "Mar", "Mer", "Gio", "Ven", "Sab", "Dom"],
        Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    }
}

pub fn days_full(lng: Lang) -> [&'static str; 7] {
    match lng {
        Lang::It => ["Lunedì", "Martedì", "Mercoledì", "Giovedì", "Venerdì", "Sabato", "Domenica"],
        Lang::En => ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    }
}

pub fn fill(tpl: &str, parts: &[&str]) -> String {
    let mut out = tpl.to_string();
    for p in parts {
        out = out.replacen("{}", p, 1);
    }
    out
}
