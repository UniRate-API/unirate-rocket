//! Integration tests for the `unirate-rocket` routes.
//!
//! Each test spins up a `wiremock::MockServer` standing in for the upstream
//! UniRate API, points a `unirate_api::Client` at it, mounts the routes on a
//! Rocket instance, and drives requests through Rocket's blocking local client.
//! No network access.

use std::time::Duration;

use rocket::http::Status;
use rocket::local::blocking::Client as LocalClient;
use rocket::{Build, Rocket};
use serde_json::Value;
use unirate_api::Client;
use unirate_rocket::unirate_routes;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a Rocket instance whose managed client points at the mock upstream.
fn rocket_for(server: &MockServer) -> Rocket<Build> {
    let client = Client::builder("test-key")
        .base_url(server.uri())
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client builds");
    rocket::build().manage(client).mount("/", unirate_routes())
}

/// Fire a single blocking GET through the Rocket instance; return (status, JSON).
///
/// Uses a fresh Tokio runtime to run the wiremock setup closure, then dispatches
/// synchronously via Rocket's blocking local client.
fn call(rocket: Rocket<Build>, uri: &str) -> (Status, Value) {
    let client = LocalClient::tracked(rocket).expect("rocket launches");
    let response = client.get(uri).dispatch();
    let status = response.status();
    let body = response.into_string().unwrap_or_default();
    let json: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, json)
}

/// Mount a single stub responding to every GET with the given status/body.
async fn stub(server: &MockServer, status: u16, body: &str) {
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(server)
        .await;
}

/// Run an async setup closure on a fresh current-thread runtime.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds")
        .block_on(f)
}

// ---------------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------------

#[test]
fn rate_single_returns_rate() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/rates"))
            .and(query_param("from", "USD"))
            .and(query_param("to", "EUR"))
            .and(query_param("api_key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"rate": 0.9321}"#))
            .expect(1)
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/rate?from=usd&to=eur");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["from"], "USD");
    assert_eq!(body["to"], "EUR");
    assert!((body["rate"].as_f64().unwrap() - 0.9321).abs() < 1e-6);
}

#[test]
fn rate_without_to_returns_all_rates() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/rates"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"rates": {"EUR": 0.9, "GBP": "0.8"}}"#),
            )
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/rate?from=USD");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["from"], "USD");
    assert!((body["rates"]["EUR"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert!((body["rates"]["GBP"].as_f64().unwrap() - 0.8).abs() < 1e-6);
}

#[test]
fn rate_defaults_from_to_usd() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/rates"))
            .and(query_param("from", "USD"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"rates": {"EUR": 0.9}}"#))
            .expect(1)
            .mount(&server),
    );

    // No `from` supplied → should default to USD.
    let (status, body) = call(rocket_for(&server), "/rate");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["from"], "USD");
}

#[test]
fn convert_returns_result() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/convert"))
            .and(query_param("from", "USD"))
            .and(query_param("to", "EUR"))
            .and(query_param("amount", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"result": 93.21}"#))
            .expect(1)
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/convert?from=usd&to=eur&amount=100");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["from"], "USD");
    assert_eq!(body["to"], "EUR");
    assert!((body["amount"].as_f64().unwrap() - 100.0).abs() < 1e-6);
    assert!((body["result"].as_f64().unwrap() - 93.21).abs() < 0.01);
}

#[test]
fn convert_defaults_amount_to_one() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/convert"))
            .and(query_param("amount", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"result": 0.93}"#))
            .expect(1)
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/convert?to=EUR");
    assert_eq!(status, Status::Ok);
    assert!((body["amount"].as_f64().unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn convert_missing_to_is_422() {
    // `to` is required; Rocket's query guard fails to forward and, with no other
    // matching route, returns 422 Unprocessable Entity before we hit the client.
    let server = block_on(MockServer::start());
    let (status, _body) = call(rocket_for(&server), "/convert?from=USD");
    assert_eq!(status, Status::UnprocessableEntity);
}

#[test]
fn currencies_returns_list() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/currencies"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"currencies": ["USD", "EUR", "GBP", "BTC"]}"#),
            )
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/currencies");
    assert_eq!(status, Status::Ok);
    assert_eq!(
        body["currencies"],
        serde_json::json!(["USD", "EUR", "GBP", "BTC"])
    );
}

#[test]
fn vat_single_country_returns_rate() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/vat/rates"))
            .and(query_param("country", "DE"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"country": "DE", "vat_data": {"country_code": "DE", "country_name": "Germany", "vat_rate": 19.0}}"#,
            ))
            .expect(1)
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/vat?country=de");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["country"], "DE");
    assert!((body["vat_data"]["vat_rate"].as_f64().unwrap() - 19.0).abs() < 1e-6);
}

#[test]
fn vat_all_countries_returns_map() {
    let server = block_on(MockServer::start());
    block_on(
        Mock::given(method("GET"))
            .and(path("/api/vat/rates"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"date": "2026-01-22", "total_countries": 2, "vat_rates": {"DE": {"country_code": "DE", "country_name": "Germany", "vat_rate": 19.0}, "FR": {"country_code": "FR", "country_name": "France", "vat_rate": 20.0}}}"#,
            ))
            .mount(&server),
    );

    let (status, body) = call(rocket_for(&server), "/vat");
    assert_eq!(status, Status::Ok);
    assert_eq!(body["total_countries"], 2);
    assert!((body["vat_rates"]["FR"]["vat_rate"].as_f64().unwrap() - 20.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------
// Error mapping — one per status
// ---------------------------------------------------------------------------

#[test]
fn upstream_400_maps_to_400() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 400, ""));
    let (status, body) = call(rocket_for(&server), "/rate?from=USD&to=EUR");
    assert_eq!(status, Status::BadRequest);
    assert_eq!(body["error"]["status"], 400);
    assert_eq!(body["error"]["message"], "Invalid request parameters");
}

#[test]
fn upstream_401_maps_to_401() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 401, ""));
    let (status, body) = call(rocket_for(&server), "/rate?from=USD&to=EUR");
    assert_eq!(status, Status::Unauthorized);
    assert_eq!(body["error"]["status"], 401);
    assert_eq!(body["error"]["message"], "Missing or invalid API key");
}

#[test]
fn upstream_403_maps_to_403_pro_gated() {
    let server = block_on(MockServer::start());
    block_on(stub(
        &server,
        403,
        r#"{"error": "Historical data access requires a Pro subscription"}"#,
    ));
    let (status, body) = call(rocket_for(&server), "/rate?from=USD&to=EUR");
    assert_eq!(status, Status::Forbidden);
    assert_eq!(body["error"]["status"], 403);
    assert_eq!(
        body["error"]["message"],
        "Endpoint requires a Pro subscription"
    );
}

#[test]
fn upstream_404_maps_to_404() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 404, ""));
    let (status, body) = call(rocket_for(&server), "/rate?from=USD&to=ZZZ");
    assert_eq!(status, Status::NotFound);
    assert_eq!(body["error"]["status"], 404);
    assert_eq!(
        body["error"]["message"],
        "Currency not found or no data available"
    );
}

#[test]
fn upstream_429_maps_to_429() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 429, ""));
    let (status, body) = call(rocket_for(&server), "/rate?from=USD&to=EUR");
    assert_eq!(status, Status::TooManyRequests);
    assert_eq!(body["error"]["status"], 429);
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
}

#[test]
fn upstream_503_maps_to_503() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 503, ""));
    let (status, body) = call(rocket_for(&server), "/currencies");
    assert_eq!(status, Status::ServiceUnavailable);
    assert_eq!(body["error"]["status"], 503);
    assert_eq!(body["error"]["message"], "Service unavailable");
}

#[test]
fn upstream_500_maps_through_with_body() {
    let server = block_on(MockServer::start());
    block_on(stub(&server, 500, r#"{"error": "boom"}"#));
    let (status, body) = call(rocket_for(&server), "/currencies");
    assert_eq!(status, Status::InternalServerError);
    assert_eq!(body["error"]["status"], 500);
    assert!(body["error"]["message"].as_str().unwrap().contains("boom"));
}

#[test]
fn transport_error_maps_to_502() {
    // Point the client at a port nothing is listening on → the upstream call
    // fails at the transport layer, which we surface as 502 Bad Gateway.
    let client = Client::builder("test-key")
        .base_url("http://127.0.0.1:1")
        .timeout(Duration::from_secs(2))
        .build()
        .expect("client builds");
    let rocket = rocket::build().manage(client).mount("/", unirate_routes());
    let (status, body) = call(rocket, "/currencies");
    assert_eq!(status, Status::BadGateway);
    assert_eq!(body["error"]["status"], 502);
}

#[test]
fn unknown_route_is_404() {
    let server = block_on(MockServer::start());
    let (status, _body) = call(rocket_for(&server), "/nope");
    assert_eq!(status, Status::NotFound);
}
