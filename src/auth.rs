//! Write authorization. The password comes from `TRASHDIFF_PASSWORD`; the
//! expected `Authorization` scheme depends on the surface: `Basic` for the
//! HTML backoffice form, `Bearer` for the JSON write API and for MCP. Reads are
//! never gated.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub const ENV: &str = "TRASHDIFF_PASSWORD";

/// Accepted `Authorization` scheme for a write surface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// `Basic base64(user:password)` — backoffice HTML form.
    Basic,
    /// `Bearer <password>` — JSON write API and MCP.
    Bearer,
}

impl Scheme {
    pub fn name(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Bearer => "Bearer",
        }
    }
}

/// Write gate. Writes are refused when no password is configured.
#[derive(Clone)]
pub struct Auth(Option<String>);

impl Auth {
    pub fn new(password: Option<String>) -> Self {
        Self(password.filter(|p| !p.is_empty()))
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var(ENV).ok())
    }

    pub fn configured(&self) -> bool {
        self.0.is_some()
    }

    /// True when the request carries the configured password with the scheme
    /// the surface expects. `header` is the raw `Authorization` value.
    pub fn allows(&self, header: Option<&str>, scheme: Scheme) -> bool {
        let Some(want) = self.0.as_deref() else {
            return false;
        };
        let Some(value) = header else {
            return false;
        };
        match credential(value, scheme) {
            Some(got) => ct_eq(want.as_bytes(), got.as_bytes()),
            None => false,
        }
    }

    pub fn warn_if_unset(&self) {
        if !self.configured() {
            eprintln!("writes disabled: set {ENV} to enable the backoffice and MCP writes");
        }
    }
}

/// Password from an `Authorization` value using only `scheme`; other schemes
/// are refused. Basic carries `user:password`, the user part is ignored.
fn credential(value: &str, scheme: Scheme) -> Option<String> {
    let (name, rest) = value.split_once(' ')?;
    let rest = rest.trim();
    if !name.eq_ignore_ascii_case(scheme.name()) {
        return None;
    }
    match scheme {
        Scheme::Bearer => Some(rest.to_string()),
        Scheme::Basic => {
            let raw = STANDARD.decode(rest).ok()?;
            let raw = String::from_utf8(raw).ok()?;
            let (_, password) = raw.split_once(':')?;
            Some(password.to_string())
        }
    }
}

/// Constant-time byte comparison: no early exit, no length shortcut.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_password_blocks_everything() {
        let auth = Auth::new(None);
        assert!(!auth.configured());
        assert!(!auth.allows(Some("Bearer anything"), Scheme::Bearer));
        assert!(!auth.allows(Some("Basic dXNlcjpwYXNz"), Scheme::Basic));
        assert!(!auth.allows(None, Scheme::Basic));
    }

    #[test]
    fn empty_password_counts_as_unset() {
        assert!(!Auth::new(Some(String::new())).configured());
    }

    #[test]
    fn bearer_matches() {
        let auth = Auth::new(Some("s3cret".into()));
        assert!(auth.allows(Some("Bearer s3cret"), Scheme::Bearer));
        assert!(auth.allows(Some("bearer s3cret"), Scheme::Bearer));
        assert!(!auth.allows(Some("Bearer nope"), Scheme::Bearer));
        assert!(!auth.allows(Some("Bearer "), Scheme::Bearer));
    }

    /// `dXNlcjpzM2NyZXQ=` is base64 of `user:s3cret`.
    #[test]
    fn basic_matches_ignoring_user() {
        let auth = Auth::new(Some("s3cret".into()));
        assert!(auth.allows(Some("Basic dXNlcjpzM2NyZXQ="), Scheme::Basic));
        assert!(auth.allows(Some("Basic OnMzY3JldA=="), Scheme::Basic));
        assert!(!auth.allows(Some("Basic dXNlcjp3cm9uZw=="), Scheme::Basic));
        assert!(!auth.allows(Some("Basic !!!"), Scheme::Basic));
        assert!(!auth.allows(Some("Basic "), Scheme::Basic));
    }

    #[test]
    fn scheme_is_enforced_per_surface() {
        let auth = Auth::new(Some("s3cret".into()));
        // right password, wrong scheme
        assert!(!auth.allows(Some("Basic dXNlcjpzM2NyZXQ="), Scheme::Bearer));
        assert!(!auth.allows(Some("Bearer s3cret"), Scheme::Basic));
        assert!(!auth.allows(Some("Digest s3cret"), Scheme::Bearer));
        assert!(!auth.allows(Some("s3cret"), Scheme::Bearer));
        assert!(!auth.allows(None, Scheme::Bearer));
    }
}
