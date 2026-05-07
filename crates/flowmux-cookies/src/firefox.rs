//! Firefox stores cookies at:
//!
//!   ~/.mozilla/firefox/<profile>/cookies.sqlite
//!
//! Schema (firefox 100+):
//!
//!   CREATE TABLE moz_cookies (
//!     id INTEGER PRIMARY KEY,
//!     originAttributes TEXT NOT NULL,
//!     name TEXT, value TEXT, host TEXT, path TEXT,
//!     expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER,
//!     isSecure INTEGER, isHttpOnly INTEGER,
//!     inBrowserElement INTEGER, sameSite INTEGER, ...
//!   );
//!
//! Values are plaintext.

use crate::cookie::{Cookie, SameSite};
use crate::source::{BrowserId, Error, Source};
use std::path::PathBuf;

pub struct Firefox;

impl Firefox {
    pub fn new() -> Self { Self }
}

impl Default for Firefox {
    fn default() -> Self { Self::new() }
}

impl Source for Firefox {
    fn id(&self) -> BrowserId { BrowserId::Firefox }

    fn detect(&self) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let base = home.join(".mozilla/firefox");
        if !base.is_dir() {
            return None;
        }
        // Look for the default profile (or first one with a cookies db).
        for entry in std::fs::read_dir(&base).ok()?.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let cookies = p.join("cookies.sqlite");
            if cookies.exists() {
                return Some(cookies);
            }
        }
        None
    }

    fn list_cookies(&self, domain_filter: Option<&str>) -> Result<Vec<Cookie>, Error> {
        let path = self.detect().ok_or_else(|| {
            Error::ProfileNotFound(PathBuf::from("~/.mozilla/firefox/<profile>/cookies.sqlite"))
        })?;
        // Open read-only to avoid locking the live profile.
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        let mut sql = String::from(
            "SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite \
             FROM moz_cookies",
        );
        if domain_filter.is_some() {
            sql.push_str(" WHERE host LIKE ?1");
        }
        let mut stmt = conn.prepare(&sql)?;
        let map = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Cookie> {
            let expiry: i64 = row.get(4).unwrap_or(0);
            let same_site: i64 = row.get(7).unwrap_or(0);
            Ok(Cookie {
                host: row.get(0)?,
                name: row.get(1)?,
                value: row.get(2)?,
                path: row.get(3)?,
                expires_at: if expiry > 0 {
                    chrono::DateTime::from_timestamp(expiry, 0)
                } else {
                    None
                },
                secure: row.get::<_, i64>(5)? != 0,
                http_only: row.get::<_, i64>(6)? != 0,
                same_site: match same_site {
                    1 => SameSite::Lax,
                    2 => SameSite::Strict,
                    3 => SameSite::None,
                    _ => SameSite::NoRestriction,
                },
            })
        };
        let rows: Vec<Cookie> = match domain_filter {
            Some(f) => stmt
                .query_map([format!("%{f}%")], map)?
                .filter_map(Result::ok)
                .collect(),
            None => stmt.query_map([], map)?.filter_map(Result::ok).collect(),
        };
        Ok(rows)
    }
}
