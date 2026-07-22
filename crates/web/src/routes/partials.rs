//! HTMX partial handlers — endpoints that return HTML *fragments* to be swapped into the
//! page, rather than full documents.

use actix_web::{post, web};
use askama::Template;
use askama_web::WebTemplate;
use serde::Deserialize;

use crate::tools::{callsign, maidenhead};

#[derive(Debug, Deserialize)]
pub struct GridForm {
    // Accept raw strings so we can return a friendly inline error instead of a 400 when
    // the input isn't a valid number.
    lat: String,
    lon: String,
    pairs: Option<usize>,
}

/// Rendered fragment for the Maidenhead grid tool result.
#[derive(Template, WebTemplate)]
#[template(path = "partials/grid_result.html")]
struct GridResultTemplate {
    locator: Option<String>,
    error: Option<String>,
    lat: String,
    lon: String,
}

impl GridResultTemplate {
    fn failure(lat: &str, lon: &str, message: impl Into<String>) -> Self {
        Self {
            locator: None,
            error: Some(message.into()),
            lat: lat.to_string(),
            lon: lon.to_string(),
        }
    }
}

/// `POST /tools/grid` — compute a Maidenhead locator and return the result fragment,
/// swapped into `#grid-result` by HTMX.
#[post("/tools/grid")]
pub async fn grid_tool(form: web::Form<GridForm>) -> GridResultTemplate {
    let form = form.into_inner();

    let lat = match form.lat.trim().parse::<f64>() {
        Ok(v) => v,
        Err(_) => {
            return GridResultTemplate::failure(&form.lat, &form.lon, "Latitude must be a number.");
        }
    };
    let lon = match form.lon.trim().parse::<f64>() {
        Ok(v) => v,
        Err(_) => {
            return GridResultTemplate::failure(
                &form.lat,
                &form.lon,
                "Longitude must be a number.",
            );
        }
    };
    let pairs = form.pairs.unwrap_or(3);

    match maidenhead::to_locator(lat, lon, pairs) {
        Ok(locator) => GridResultTemplate {
            locator: Some(locator),
            error: None,
            lat: form.lat,
            lon: form.lon,
        },
        Err(e) => GridResultTemplate::failure(&form.lat, &form.lon, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct CallsignForm {
    callsign: String,
}

/// Rendered fragment for the callsign lookup result. Doubles as the live "hint" shown
/// under the callsign field in the logbook add-form.
#[derive(Template, WebTemplate)]
#[template(path = "partials/callsign_result.html")]
struct CallsignResultTemplate {
    callsign: String,
    country: Option<String>,
    continent: Option<String>,
    error: Option<String>,
}

/// `POST /tools/callsign` — resolve a callsign's country/continent and return a fragment.
#[post("/tools/callsign")]
pub async fn callsign_tool(form: web::Form<CallsignForm>) -> CallsignResultTemplate {
    let raw = form.into_inner().callsign;
    let normalized = callsign::normalize(&raw);

    // Empty input → an empty fragment (nothing to show yet).
    if normalized.is_empty() {
        return CallsignResultTemplate {
            callsign: raw,
            country: None,
            continent: None,
            error: None,
        };
    }

    match callsign::lookup(&normalized) {
        Ok(info) => CallsignResultTemplate {
            callsign: info.callsign,
            country: Some(info.country),
            continent: Some(info.continent),
            error: None,
        },
        Err(e) => CallsignResultTemplate {
            callsign: normalized,
            country: None,
            continent: None,
            error: Some(e.to_string()),
        },
    }
}
