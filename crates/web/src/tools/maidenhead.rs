//! Maidenhead Locator System (grid squares).
//!
//! Converts a latitude/longitude into a Maidenhead grid locator, the coordinate shorthand
//! hams use to describe location (and to compute distance/bearing for contacts). The
//! encoding alternates longitude and latitude across successive pairs of increasing
//! resolution:
//!
//! | Pair | Symbols | Longitude step | Latitude step |
//! |------|---------|----------------|---------------|
//! | 1 (field)      | A–R | 20°   | 10°   |
//! | 2 (square)     | 0–9 | 2°    | 1°    |
//! | 3 (subsquare)  | a–x | 5′    | 2.5′  |
//! | 4 (ext. square)| 0–9 | 30″   | 15″   |

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MaidenheadError {
    #[error("latitude {0} is out of range (must be between -90 and 90)")]
    Latitude(f64),
    #[error("longitude {0} is out of range (must be between -180 and 180)")]
    Longitude(f64),
}

/// Convert a coordinate to a Maidenhead locator with the given number of pairs
/// (1–4; 3 pairs = the common 6-character locator such as `FN31pr`).
pub fn to_locator(lat: f64, lon: f64, pairs: usize) -> Result<String, MaidenheadError> {
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return Err(MaidenheadError::Latitude(lat));
    }
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        return Err(MaidenheadError::Longitude(lon));
    }
    let pairs = pairs.clamp(1, 4);

    // Shift into positive ranges: longitude 0..360, latitude 0..180.
    let mut lon_rem = lon + 180.0;
    let mut lat_rem = lat + 90.0;
    // Nudge the exact upper bounds down so the field index can't overflow past R.
    if lon_rem >= 360.0 {
        lon_rem = 359.999_999;
    }
    if lat_rem >= 180.0 {
        lat_rem = 179.999_999;
    }

    let mut out = String::with_capacity(pairs * 2);

    // Pair 1: field (letters A–R).
    let lon_field = ((lon_rem / 20.0) as usize).min(17);
    let lat_field = ((lat_rem / 10.0) as usize).min(17);
    out.push((b'A' + lon_field as u8) as char);
    out.push((b'A' + lat_field as u8) as char);
    lon_rem -= lon_field as f64 * 20.0;
    lat_rem -= lat_field as f64 * 10.0;

    // Pair 2: square (digits 0–9).
    if pairs >= 2 {
        let lon_sq = ((lon_rem / 2.0) as usize).min(9);
        let lat_sq = ((lat_rem / 1.0) as usize).min(9);
        out.push((b'0' + lon_sq as u8) as char);
        out.push((b'0' + lat_sq as u8) as char);
        lon_rem -= lon_sq as f64 * 2.0;
        lat_rem -= lat_sq as f64 * 1.0;
    }

    // Pair 3: subsquare (letters a–x).
    if pairs >= 3 {
        let lon_step = 2.0 / 24.0;
        let lat_step = 1.0 / 24.0;
        let lon_ss = ((lon_rem / lon_step) as usize).min(23);
        let lat_ss = ((lat_rem / lat_step) as usize).min(23);
        out.push((b'a' + lon_ss as u8) as char);
        out.push((b'a' + lat_ss as u8) as char);
        lon_rem -= lon_ss as f64 * lon_step;
        lat_rem -= lat_ss as f64 * lat_step;
    }

    // Pair 4: extended square (digits 0–9).
    if pairs >= 4 {
        let lon_step = 2.0 / 24.0 / 10.0;
        let lat_step = 1.0 / 24.0 / 10.0;
        let lon_es = ((lon_rem / lon_step) as usize).min(9);
        let lat_es = ((lat_rem / lat_step) as usize).min(9);
        out.push((b'0' + lon_es as u8) as char);
        out.push((b'0' + lat_es as u8) as char);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_locations() {
        // ARRL HQ, Newington CT — a canonical reference point.
        assert_eq!(to_locator(41.714775, -72.727260, 3).unwrap(), "FN31pr");
        // The origin (0°, 0°) sits at the corner of field JJ.
        assert_eq!(to_locator(0.0, 0.0, 3).unwrap(), "JJ00aa");
    }

    #[test]
    fn respects_pair_count() {
        assert_eq!(to_locator(41.714775, -72.727260, 1).unwrap(), "FN");
        assert_eq!(to_locator(41.714775, -72.727260, 2).unwrap(), "FN31");
        assert_eq!(to_locator(41.714775, -72.727260, 4).unwrap().len(), 8);
        // Out-of-band pair counts are clamped into 1..=4.
        assert_eq!(to_locator(0.0, 0.0, 99).unwrap().len(), 8);
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(
            to_locator(95.0, 0.0, 3),
            Err(MaidenheadError::Latitude(95.0))
        );
        assert_eq!(
            to_locator(0.0, 200.0, 3),
            Err(MaidenheadError::Longitude(200.0))
        );
        assert!(to_locator(f64::NAN, 0.0, 3).is_err());
    }

    #[test]
    fn handles_boundaries() {
        // The extreme corners must not panic or overflow past the valid symbol range.
        assert_eq!(to_locator(90.0, 180.0, 3).unwrap().len(), 6);
        assert_eq!(to_locator(-90.0, -180.0, 3).unwrap(), "AA00aa");
    }
}
