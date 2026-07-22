//! Callsign lookup.
//!
//! Given an amateur-radio callsign, this validates its shape and resolves the DXCC
//! entity (country) and continent from the callsign prefix, using a built-in table.
//!
//! The table below is a **curated starter set** covering common entities — it is not the
//! full ~340-entity DXCC list, and prefix allocations have exceptions the simple
//! longest-prefix match here does not model. It is structured to be easily extended (or
//! swapped for a `cty.dat` parser or an online lookup such as callook.info / HamQTH) as
//! the application grows. Lookups are pure and offline, which keeps them fast and testable.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallsignInfo {
    /// The normalized callsign (uppercased, whitespace trimmed).
    pub callsign: String,
    pub country: String,
    pub continent: String,
    /// The prefix that matched in the table.
    pub prefix: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CallsignError {
    #[error("please enter a callsign")]
    Empty,
    #[error("'{0}' is not a valid callsign")]
    Invalid(String),
    #[error("could not determine the country for '{0}'")]
    UnknownPrefix(String),
}

/// Uppercase and trim a callsign.
pub fn normalize(raw: &str) -> String {
    raw.trim().to_ascii_uppercase()
}

/// A loose structural check for an amateur callsign.
///
/// Intentionally permissive: accepts an optional prefix/suffix separated by `/` (e.g.
/// `W1AW/4`, `DL/W1AW`), requires the core token to be 3–10 alphanumerics containing at
/// least one letter and one digit. This is not a full ITU validator, but it rejects
/// obvious junk without being fussy about edge cases.
pub fn is_valid(callsign: &str) -> bool {
    let cs = normalize(callsign);
    if cs.is_empty() {
        return false;
    }
    // Consider each `/`-separated part; at least one part must look like a callsign core.
    cs.split('/').any(|part| {
        let len = part.chars().count();
        (3..=10).contains(&len)
            && part.chars().all(|c| c.is_ascii_alphanumeric())
            && part.chars().any(|c| c.is_ascii_alphabetic())
            && part.chars().any(|c| c.is_ascii_digit())
    })
}

/// Resolve country & continent for a callsign.
pub fn lookup(raw: &str) -> Result<CallsignInfo, CallsignError> {
    let callsign = normalize(raw);
    if callsign.is_empty() {
        return Err(CallsignError::Empty);
    }
    if !is_valid(&callsign) {
        return Err(CallsignError::Invalid(callsign));
    }

    // Use the most specific (longest) part for prefix matching — for `DL/W1AW` the
    // operating prefix `DL` is what matters, but our simple approach just tries the
    // callsign core. Take the longest `/`-separated part as the core.
    let core = callsign
        .split('/')
        .max_by_key(|p| p.chars().count())
        .unwrap_or(&callsign);

    match match_prefix(core) {
        Some(entry) => Ok(CallsignInfo {
            callsign: callsign.clone(),
            country: entry.country.to_string(),
            continent: entry.continent.to_string(),
            prefix: entry.prefix.to_string(),
        }),
        None => Err(CallsignError::UnknownPrefix(callsign)),
    }
}

struct PrefixEntry {
    prefix: &'static str,
    country: &'static str,
    continent: &'static str,
}

const fn e(prefix: &'static str, country: &'static str, continent: &'static str) -> PrefixEntry {
    PrefixEntry {
        prefix,
        country,
        continent,
    }
}

/// Find the entry whose prefix is the longest match against the start of `core`.
fn match_prefix(core: &str) -> Option<&'static PrefixEntry> {
    PREFIXES
        .iter()
        .filter(|entry| core.starts_with(entry.prefix))
        .max_by_key(|entry| entry.prefix.len())
}

/// Curated prefix → (country, continent) table. Longer, more specific prefixes win.
/// Extend freely; entries are matched by longest-prefix.
const PREFIXES: &[PrefixEntry] = &[
    // North America
    e("K", "United States", "NA"),
    e("W", "United States", "NA"),
    e("N", "United States", "NA"),
    e("AA", "United States", "NA"),
    e("AB", "United States", "NA"),
    e("AC", "United States", "NA"),
    e("AK", "Alaska", "NA"),
    e("KH6", "Hawaii", "OC"),
    e("KL", "Alaska", "NA"),
    e("VE", "Canada", "NA"),
    e("VA", "Canada", "NA"),
    e("VO", "Canada", "NA"),
    e("VY", "Canada", "NA"),
    e("XE", "Mexico", "NA"),
    e("XF", "Mexico", "NA"),
    // South America
    e("PY", "Brazil", "SA"),
    e("PP", "Brazil", "SA"),
    e("PU", "Brazil", "SA"),
    e("LU", "Argentina", "SA"),
    e("CE", "Chile", "SA"),
    e("HK", "Colombia", "SA"),
    e("YV", "Venezuela", "SA"),
    // Europe
    e("G", "England", "EU"),
    e("M", "England", "EU"),
    e("2E", "England", "EU"),
    e("GW", "Wales", "EU"),
    e("GM", "Scotland", "EU"),
    e("GI", "Northern Ireland", "EU"),
    e("EI", "Ireland", "EU"),
    e("DL", "Germany", "EU"),
    e("DK", "Germany", "EU"),
    e("DJ", "Germany", "EU"),
    e("DD", "Germany", "EU"),
    e("DF", "Germany", "EU"),
    e("DB", "Germany", "EU"),
    e("F", "France", "EU"),
    e("EA", "Spain", "EU"),
    e("EB", "Spain", "EU"),
    e("EC", "Spain", "EU"),
    e("I", "Italy", "EU"),
    e("PA", "Netherlands", "EU"),
    e("PB", "Netherlands", "EU"),
    e("PD", "Netherlands", "EU"),
    e("ON", "Belgium", "EU"),
    e("OZ", "Denmark", "EU"),
    e("SM", "Sweden", "EU"),
    e("SA", "Sweden", "EU"),
    e("LA", "Norway", "EU"),
    e("OH", "Finland", "EU"),
    e("HB9", "Switzerland", "EU"),
    e("HB0", "Liechtenstein", "EU"),
    e("OE", "Austria", "EU"),
    e("SP", "Poland", "EU"),
    e("OK", "Czech Republic", "EU"),
    e("OM", "Slovakia", "EU"),
    e("HA", "Hungary", "EU"),
    e("YO", "Romania", "EU"),
    e("LZ", "Bulgaria", "EU"),
    e("SV", "Greece", "EU"),
    e("CT", "Portugal", "EU"),
    e("EA6", "Balearic Islands", "EU"),
    e("R", "Russia", "EU"),
    e("UA", "Russia", "EU"),
    e("UR", "Ukraine", "EU"),
    e("UT", "Ukraine", "EU"),
    // Asia
    e("JA", "Japan", "AS"),
    e("JE", "Japan", "AS"),
    e("JF", "Japan", "AS"),
    e("JH", "Japan", "AS"),
    e("JR", "Japan", "AS"),
    e("7J", "Japan", "AS"),
    e("BY", "China", "AS"),
    e("BG", "China", "AS"),
    e("BA", "China", "AS"),
    e("HL", "South Korea", "AS"),
    e("DS", "South Korea", "AS"),
    e("VU", "India", "AS"),
    e("YB", "Indonesia", "AS"),
    e("9M", "Malaysia", "AS"),
    e("HS", "Thailand", "AS"),
    e("4X", "Israel", "AS"),
    e("4Z", "Israel", "AS"),
    // Oceania
    e("VK", "Australia", "OC"),
    e("ZL", "New Zealand", "OC"),
    e("KH", "Hawaii", "OC"),
    e("FK", "New Caledonia", "OC"),
    // Africa
    e("ZS", "South Africa", "AF"),
    e("SU", "Egypt", "AF"),
    e("CN", "Morocco", "AF"),
    e("5N", "Nigeria", "AF"),
    e("5Z", "Kenya", "AF"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("  w1aw "), "W1AW");
    }

    #[test]
    fn validates() {
        assert!(is_valid("W1AW"));
        assert!(is_valid("G3ABC"));
        assert!(is_valid("VE3XYZ"));
        assert!(is_valid("W1AW/4"));
        assert!(!is_valid(""));
        assert!(!is_valid("HELLO")); // no digit
        assert!(!is_valid("12345")); // no letter
        assert!(!is_valid("!!"));
    }

    #[test]
    fn resolves_countries() {
        assert_eq!(lookup("W1AW").unwrap().country, "United States");
        assert_eq!(lookup("g3abc").unwrap().country, "England");
        assert_eq!(lookup("VE3XYZ").unwrap().country, "Canada");
        assert_eq!(lookup("DL1ABC").unwrap().country, "Germany");
        assert_eq!(lookup("JA1XYZ").unwrap().country, "Japan");
        assert_eq!(lookup("VK2DEF").unwrap().country, "Australia");
    }

    #[test]
    fn longest_prefix_wins() {
        // "HB9" (Switzerland) must beat a hypothetical shorter "H" match, and "HB0" is
        // Liechtenstein.
        assert_eq!(lookup("HB9AA").unwrap().country, "Switzerland");
        assert_eq!(lookup("HB0XX").unwrap().country, "Liechtenstein");
    }

    #[test]
    fn continent_is_reported() {
        assert_eq!(lookup("W1AW").unwrap().continent, "NA");
        assert_eq!(lookup("DL1ABC").unwrap().continent, "EU");
    }

    #[test]
    fn errors() {
        assert_eq!(lookup(""), Err(CallsignError::Empty));
        assert_eq!(
            lookup("HELLO"),
            Err(CallsignError::Invalid("HELLO".to_string()))
        );
        // Valid shape but unknown prefix.
        assert!(matches!(
            lookup("Q9ZZ"),
            Err(CallsignError::UnknownPrefix(_))
        ));
    }
}
