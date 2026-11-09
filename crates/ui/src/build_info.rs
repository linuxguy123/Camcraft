//! Identification embedded when the UI crate is compiled.

#[cfg(test)]
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(not(test))]
pub(crate) const VERSION: &str = env!("CAMCRAFT_BUILD_LABEL");
pub(crate) const BUILT_UTC: &str = env!("CAMCRAFT_BUILD_UTC");
pub(crate) const SOURCE_SHA: &str = env!("CAMCRAFT_SOURCE_SHA");

// The toolbar snapshots need a stable label across Cargo version bumps.
// Actual application builds show the package version or the release label.
#[cfg(test)]
pub(crate) const TOOLBAR_VERSION: &str = "0.14.0";
#[cfg(not(test))]
pub(crate) const TOOLBAR_VERSION: &str = VERSION;

#[cfg(test)]
const TOOLBAR_BUILD_UTC: &str = "2000-01-01 00:00 UTC";
#[cfg(not(test))]
const TOOLBAR_BUILD_UTC: &str = BUILT_UTC;

#[cfg(not(test))]
pub(crate) fn short_source() -> String {
    if SOURCE_SHA == "Custom Build" {
        SOURCE_SHA.to_owned()
    } else {
        let modified = if SOURCE_SHA.ends_with("-dirty") { "*" } else { "" };
        format!("{}{modified}", &SOURCE_SHA[..8])
    }
}

#[cfg(test)]
pub(crate) fn short_source() -> String {
    "test".to_owned()
}

pub(crate) fn toolbar_lines(compact: bool) -> (String, String) {
    let top = format!("v{TOOLBAR_VERSION} · {}", short_source());
    let bottom = if compact {
        format!("{} UTC", TOOLBAR_BUILD_UTC.get(5..16).unwrap_or(TOOLBAR_BUILD_UTC))
    } else {
        format!("Built {TOOLBAR_BUILD_UTC}")
    };
    (top, bottom)
}
