use crate::Cookie;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserId {
    Firefox,
    Chrome,
    Chromium,
    Brave,
    Edge,
    Arc,
}

impl BrowserId {
    pub fn slug(self) -> &'static str {
        match self {
            BrowserId::Firefox => "firefox",
            BrowserId::Chrome => "chrome",
            BrowserId::Chromium => "chromium",
            BrowserId::Brave => "brave",
            BrowserId::Edge => "edge",
            BrowserId::Arc => "arc",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("XDG home directory unavailable")]
    NoHome,
    #[error("profile not found: {0}")]
    ProfileNotFound(PathBuf),
    #[error("Chromium-family encrypted values are not supported until libsecret integration lands")]
    EncryptedValuesUnsupported,
}

pub trait Source {
    fn id(&self) -> BrowserId;
    fn detect(&self) -> Option<PathBuf>;
    fn list_cookies(&self, domain_filter: Option<&str>) -> Result<Vec<Cookie>, Error>;
}

pub fn discover_sources() -> Vec<Box<dyn Source>> {
    let mut out: Vec<Box<dyn Source>> = vec![Box::new(crate::firefox::Firefox::new())];
    for id in [
        BrowserId::Chrome,
        BrowserId::Chromium,
        BrowserId::Brave,
        BrowserId::Edge,
        BrowserId::Arc,
    ] {
        out.push(Box::new(crate::chromium::Chromium::new(id)));
    }
    out
}
