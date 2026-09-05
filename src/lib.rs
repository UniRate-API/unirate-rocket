//! # unirate-rocket
//!
//! [Rocket](https://rocket.rs) integration for the
//! [UniRate API](https://unirateapi.com) — free, real-time currency exchange
//! rates, conversion, and VAT rates.
//!
//! This crate exposes a ready-made set of routes via [`unirate_routes`] plus a
//! [`UniRateFairing`] that manages a single [`unirate_api::Client`] in Rocket's
//! state. Together they add four `GET` routes:
//!
//! | Route         | Query params            | Backing client call                              |
//! |---------------|-------------------------|--------------------------------------------------|
//! | `/rate`       | `from`, `to`            | [`Client::get_rate`] / [`Client::get_all_rates`] |
//! | `/convert`    | `from`, `to`, `amount`  | [`Client::convert`]                              |
//! | `/currencies` | —                       | [`Client::get_supported_currencies`]             |
//! | `/vat`        | `country` (optional)    | [`Client::get_vat_rate`] / [`Client::get_vat_rates`] |
//!
//! UniRate errors are mapped to the matching HTTP status with a JSON error body
//! via [`ApiError`], which implements Rocket's
//! [`Responder`](rocket::response::Responder).
//!
//! ## Client sourcing
//!
//! The UniRate HTTP client itself is **not** re-implemented here — this crate
//! depends on the published [`unirate-api`](https://crates.io/crates/unirate-api)
//! crate (`unirate_api::Client`) and only adds the Rocket wiring. That keeps the
//! client parity in one place and this crate small.
//!
//! ## Quick start
//!
//! ```no_run
//! use rocket::launch;
//! use unirate_rocket::{unirate_routes, UniRateFairing};
//!
//! #[launch]
//! fn rocket() -> _ {
//!     let key = std::env::var("UNIRATE_API_KEY").expect("UNIRATE_API_KEY");
//!     rocket::build()
//!         .attach(UniRateFairing::new(key))
//!         .mount("/", unirate_routes())
//! }
//! ```
//!
//! The [`Client`] is managed by the [`UniRateFairing`], so the handlers read it
//! from [`rocket::State<Client>`]. If you'd rather manage the client yourself,
//! skip the fairing and call [`rocket::Rocket::manage`] with a [`Client`]:
//!
//! ```no_run
//! use rocket::routes;
//! use unirate_api::Client;
//! use unirate_rocket::unirate_routes;
//!
//! let client = Client::new("your-api-key");
//! let rocket = rocket::build()
//!     .manage(client)
//!     .mount("/currency", unirate_routes());
//! // now GET /currency/rate, /currency/convert, ...
//! ```

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

use std::collections::HashMap;

use rocket::fairing::{self, Fairing, Info, Kind};
use rocket::http::{ContentType, Status};
use rocket::request::Request;
use rocket::response::{self, Responder, Response};
use rocket::serde::json::Json;
use rocket::{get, routes, Build, Rocket, State};
use serde::Serialize;
use serde_json::json;
use unirate_api::{Client, UniRateError, VatRate, VatRateResponse, VatRatesResponse};

/// Return the UniRate routes for mounting on a Rocket instance.
///
/// Mount them at any base path with [`rocket::Rocket::mount`]. The handlers read
/// the [`Client`] from [`rocket::State`], so make sure a [`Client`] is managed —
/// either by attaching [`UniRateFairing`] or by calling
/// [`rocket::Rocket::manage`] yourself.
///
/// Routes:
/// - `GET /rate?from=USD&to=EUR` — single rate. Omit `to` to get every rate for
///   `from` as a `{ "rates": { .. } }` map.
/// - `GET /convert?from=USD&to=EUR&amount=100` — converted amount.
/// - `GET /currencies` — list of supported currency codes.
/// - `GET /vat?country=DE` — VAT for one country. Omit `country` for all.
pub fn unirate_routes() -> Vec<rocket::Route> {
    routes![
        rate_handler,
        convert_handler,
        currencies_handler,
        vat_handler
    ]
}

// ---------------------------------------------------------------------------
// Fairing
// ---------------------------------------------------------------------------

/// A Rocket [`Fairing`] that manages a single [`unirate_api::Client`] in state.
///
/// Attach it to your Rocket instance so the [`unirate_routes`] handlers can read
/// the client from [`rocket::State`]:
///
/// ```no_run
/// use unirate_rocket::{unirate_routes, UniRateFairing};
///
/// let rocket = rocket::build()
///     .attach(UniRateFairing::new(std::env::var("UNIRATE_API_KEY").unwrap()))
///     .mount("/", unirate_routes());
/// ```
///
/// If you already have a configured [`Client`] (custom base URL or timeout), use
/// [`UniRateFairing::with_client`].
#[derive(Debug)]
pub struct UniRateFairing {
    client: Client,
}

impl UniRateFairing {
    /// Build a fairing that manages a [`Client`] created from `api_key`.
    pub fn new(api_key: impl Into<String>) -> Self {
        UniRateFairing {
            client: Client::new(api_key),
        }
    }

    /// Build a fairing around a pre-configured [`Client`].
    ///
    /// Use this when you need a custom base URL or timeout (via
    /// [`Client::builder`]).
    pub fn with_client(client: Client) -> Self {
        UniRateFairing { client }
    }
}

#[rocket::async_trait]
impl Fairing for UniRateFairing {
    fn info(&self) -> Info {
        Info {
            name: "UniRate Client",
            kind: Kind::Ignite,
        }
    }

    async fn on_ignite(&self, rocket: Rocket<Build>) -> fairing::Result {
        // Clone the client into managed state. `unirate_api::Client` is cheaply
        // cloneable (it wraps a connection-pooled reqwest client).
        Ok(rocket.manage(self.client.clone()))
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /rate?from=USD&to=EUR`
///
/// With `to`: returns `{ "from", "to", "rate" }`. Without `to`: returns
/// `{ "from", "rates": { .. } }` for every rate of `from`. `from` defaults to
/// `USD`.
#[get("/rate?<from>&<to>")]
async fn rate_handler(
    client: &State<Client>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let from = from.unwrap_or("USD");
    match to {
        Some(to) => {
            let rate = client.get_rate(from, to).await?;
            Ok(Json(json!({
                "from": from.to_uppercase(),
                "to": to.to_uppercase(),
                "rate": rate,
            })))
        }
        None => {
            let rates: HashMap<String, f64> = client.get_all_rates(from).await?;
            Ok(Json(json!({
                "from": from.to_uppercase(),
                "rates": rates,
            })))
        }
    }
}

/// `GET /convert?from=USD&to=EUR&amount=100`
///
/// `to` is required (a missing `to` is a `422` from Rocket's query guard).
/// `from` defaults to `USD`, `amount` defaults to `1`.
#[get("/convert?<from>&<to>&<amount>")]
async fn convert_handler(
    client: &State<Client>,
    from: Option<&str>,
    to: &str,
    amount: Option<f64>,
) -> Result<Json<ConvertResponse>, ApiError> {
    let from = from.unwrap_or("USD");
    let amount = amount.unwrap_or(1.0);
    let result = client.convert(amount, from, to).await?;
    Ok(Json(ConvertResponse {
        from: from.to_uppercase(),
        to: to.to_uppercase(),
        amount,
        result,
    }))
}

/// `GET /currencies` — list of supported currency codes.
#[get("/currencies")]
async fn currencies_handler(client: &State<Client>) -> Result<Json<CurrenciesResponse>, ApiError> {
    let currencies = client.get_supported_currencies().await?;
    Ok(Json(CurrenciesResponse { currencies }))
}

/// `GET /vat?country=DE` — VAT for one country. Omit `country` for all.
#[get("/vat?<country>")]
async fn vat_handler(
    client: &State<Client>,
    country: Option<&str>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match country {
        Some(country) => {
            let resp = client.get_vat_rate(country).await?;
            Ok(Json(json!(VatCountry::from(resp))))
        }
        None => {
            let resp = client.get_vat_rates().await?;
            Ok(Json(json!(VatAll::from(resp))))
        }
    }
}

// ---------------------------------------------------------------------------
// Response bodies
// ---------------------------------------------------------------------------

/// Body of a successful `GET /convert`.
#[derive(Debug, Serialize)]
pub struct ConvertResponse {
    /// Source currency (uppercased).
    pub from: String,
    /// Target currency (uppercased).
    pub to: String,
    /// The input amount.
    pub amount: f64,
    /// The converted amount.
    pub result: f64,
}

/// Body of a successful `GET /currencies`.
#[derive(Debug, Serialize)]
pub struct CurrenciesResponse {
    /// Supported currency codes.
    pub currencies: Vec<String>,
}

/// VAT rate for a single country, as returned in a JSON response.
///
/// The `unirate-api` [`VatRate`] type only derives `Deserialize`, so this
/// serializable mirror is used for the outgoing HTTP body.
#[derive(Debug, Serialize)]
pub struct VatRateBody {
    /// ISO-3166 alpha-2 country code, when supplied by the upstream API.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    /// Human-readable country name, when supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_name: Option<String>,
    /// VAT rate as a percentage (e.g. `19.0`).
    pub vat_rate: f64,
}

impl From<VatRate> for VatRateBody {
    fn from(v: VatRate) -> Self {
        VatRateBody {
            country_code: v.country_code,
            country_name: v.country_name,
            vat_rate: v.vat_rate,
        }
    }
}

/// Body of a successful `GET /vat?country=XX`.
#[derive(Debug, Serialize)]
pub struct VatCountry {
    /// Echoed country code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// The country's VAT detail.
    pub vat_data: VatRateBody,
}

impl From<VatRateResponse> for VatCountry {
    fn from(r: VatRateResponse) -> Self {
        VatCountry {
            country: r.country,
            vat_data: r.vat_data.into(),
        }
    }
}

/// Body of a successful `GET /vat` (all countries).
#[derive(Debug, Serialize)]
pub struct VatAll {
    /// Snapshot date, when provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Number of countries in the response.
    pub total_countries: u32,
    /// Per-country VAT rates keyed by ISO-3166 alpha-2 code.
    pub vat_rates: HashMap<String, VatRateBody>,
}

impl From<VatRatesResponse> for VatAll {
    fn from(r: VatRatesResponse) -> Self {
        VatAll {
            date: r.date,
            total_countries: r.total_countries,
            vat_rates: r
                .vat_rates
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Wraps a [`UniRateError`] and turns it into an HTTP response via Rocket's
/// [`Responder`].
///
/// The status code follows the UniRate error → HTTP mapping:
///
/// | UniRate error                    | HTTP status |
/// |----------------------------------|-------------|
/// | `InvalidDate`                    | 400 Bad Request |
/// | `Authentication`                 | 401 Unauthorized |
/// | `Api { status: 403, .. }`        | 403 Forbidden (Pro-gated) |
/// | `InvalidCurrency`                | 404 Not Found |
/// | `RateLimit`                      | 429 Too Many Requests |
/// | `Api { status: 503, .. }`        | 503 Service Unavailable |
/// | `Api { status, .. }` (other)     | that status |
/// | `Http` / `Json` (transport)      | 502 Bad Gateway |
#[derive(Debug)]
pub struct ApiError(pub UniRateError);

impl From<UniRateError> for ApiError {
    fn from(err: UniRateError) -> Self {
        ApiError(err)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    /// Compute the outgoing (status, message) pair for this error.
    fn parts(&self) -> (Status, String) {
        match &self.0 {
            UniRateError::InvalidDate => {
                (Status::BadRequest, "Invalid request parameters".to_string())
            }
            UniRateError::Authentication => (
                Status::Unauthorized,
                "Missing or invalid API key".to_string(),
            ),
            UniRateError::InvalidCurrency => (
                Status::NotFound,
                "Currency not found or no data available".to_string(),
            ),
            UniRateError::RateLimit => (Status::TooManyRequests, "Rate limit exceeded".to_string()),
            UniRateError::Api { status, body } => {
                let code = Status::from_code(*status).unwrap_or(Status::BadGateway);
                let message = match *status {
                    403 => "Endpoint requires a Pro subscription".to_string(),
                    503 => "Service unavailable".to_string(),
                    _ if body.is_empty() => format!("Upstream API error (status {status})"),
                    _ => body.clone(),
                };
                (code, message)
            }
            // Transport / decode failures are our side of the fence — the
            // upstream call never completed cleanly, so surface a 502.
            UniRateError::Http(_) | UniRateError::Json(_) => (
                Status::BadGateway,
                "Failed to reach the UniRate API".to_string(),
            ),
            // `UniRateError` is `#[non_exhaustive]`; be defensive.
            _ => (Status::InternalServerError, "Unexpected error".to_string()),
        }
    }
}

impl<'r> Responder<'r, 'static> for ApiError {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'static> {
        let (status, message) = self.parts();
        let body = json!({
            "error": {
                "status": status.code,
                "message": message,
            }
        })
        .to_string();

        Response::build_from(body.respond_to(request)?)
            .status(status)
            .header(ContentType::JSON)
            .ok()
    }
}
