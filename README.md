# unirate-rocket

[Rocket](https://rocket.rs) integration for the
[UniRate API](https://unirateapi.com) — free, real-time currency exchange
rates, conversion, and VAT rates.

Attach a fairing and mount a ready-made set of routes to get four JSON
endpoints backed by the official
[`unirate-api`](https://crates.io/crates/unirate-api) client:

| Route         | Query params           | Returns                                  |
|---------------|------------------------|------------------------------------------|
| `GET /rate`       | `from`, `to`           | Single rate, or every rate for `from` when `to` is omitted |
| `GET /convert`    | `from`, `to`, `amount` | Converted amount                         |
| `GET /currencies` | —                      | List of supported currency codes         |
| `GET /vat`        | `country` (optional)   | VAT for one country, or all countries    |

This crate does **not** re-implement the UniRate HTTP client. It depends on the
published [`unirate-api`](https://crates.io/crates/unirate-api) crate for the
`Client`, and only adds the Rocket wiring — so client parity lives in one place.

## Install

```toml
[dependencies]
unirate-rocket = "0.1"
unirate-api = "0.1"
rocket = { version = "0.5", features = ["json"] }
```

## Quick start

```rust
use rocket::launch;
use unirate_rocket::{unirate_routes, UniRateFairing};

#[launch]
fn rocket() -> _ {
    let key = std::env::var("UNIRATE_API_KEY").expect("UNIRATE_API_KEY");
    rocket::build()
        .attach(UniRateFairing::new(key))
        .mount("/", unirate_routes())
}
```

Then (Rocket defaults to port 8000):

```bash
curl 'http://127.0.0.1:8000/rate?from=USD&to=EUR'
# {"from":"USD","to":"EUR","rate":0.9321}

curl 'http://127.0.0.1:8000/convert?from=USD&to=EUR&amount=100'
# {"from":"USD","to":"EUR","amount":100.0,"result":93.21}

curl 'http://127.0.0.1:8000/currencies'
# {"currencies":["USD","EUR","GBP", ...]}

curl 'http://127.0.0.1:8000/vat?country=DE'
# {"country":"DE","vat_data":{"country_code":"DE","country_name":"Germany","vat_rate":19.0}}
```

A full runnable example lives in [`examples/server.rs`](examples/server.rs):

```bash
UNIRATE_API_KEY=your-key cargo run --example server
```

Get a free API key at <https://unirateapi.com>.

## API

### `unirate_routes() -> Vec<rocket::Route>`

Returns the four routes above for mounting with
[`Rocket::mount`](https://docs.rs/rocket/0.5/rocket/struct.Rocket.html#method.mount).
Mount them at any base path — the handlers read the `Client` from managed
[`State`](https://docs.rs/rocket/0.5/rocket/struct.State.html):

```rust
use rocket::routes;
use unirate_api::Client;
use unirate_rocket::unirate_routes;

let client = Client::new("your-api-key");
let rocket = rocket::build()
    .manage(client)
    .mount("/currency", unirate_routes());
// now GET /currency/rate, /currency/convert, ...
```

### `UniRateFairing`

A [`Fairing`](https://docs.rs/rocket/0.5/rocket/fairing/trait.Fairing.html) that
constructs and manages a single `Client` in Rocket's state on ignite. Use
`UniRateFairing::new(api_key)` for the common case, or
`UniRateFairing::with_client(client)` to supply a pre-configured `Client`
(custom base URL or timeout via `Client::builder`).

### Query parameters

- **`/rate`** — `from` (defaults to `USD`), `to` (optional; omit to get all
  rates for `from`). Currency codes are uppercased before the upstream call.
- **`/convert`** — `from` (defaults to `USD`), `to` (**required**), `amount`
  (defaults to `1`). A missing `to` yields `422 Unprocessable Entity`.
- **`/currencies`** — no parameters.
- **`/vat`** — `country` (optional ISO-3166 alpha-2; omit to get all countries).

## Error handling

Upstream UniRate errors are mapped to the matching HTTP status with a JSON body
of the form:

```json
{ "error": { "status": 401, "message": "Missing or invalid API key" } }
```

| Upstream condition                    | HTTP status              | Message |
|---------------------------------------|--------------------------|---------|
| Invalid request parameters (400)      | `400 Bad Request`        | `Invalid request parameters` |
| Missing / invalid API key (401)       | `401 Unauthorized`       | `Missing or invalid API key` |
| Pro-gated endpoint on free tier (403) | `403 Forbidden`          | `Endpoint requires a Pro subscription` |
| Currency not found (404)              | `404 Not Found`          | `Currency not found or no data available` |
| Rate limit exceeded (429)             | `429 Too Many Requests`  | `Rate limit exceeded` |
| Service unavailable (503)             | `503 Service Unavailable`| `Service unavailable` |
| Other upstream status                 | that status              | upstream body |
| Transport / decode failure            | `502 Bad Gateway`        | `Failed to reach the UniRate API` |

A missing required query parameter (e.g. `to` on `/convert`) is rejected by
Rocket's query guard with `422 Unprocessable Entity` before the upstream is
called.

## Rate limits

Free-tier keys are rate limited by UniRate; a `429` from upstream surfaces as
`429 Too Many Requests`. Historical and time-series data are Pro-gated and
return `403 Forbidden` on the free tier — this crate only exposes the free-tier
routes (`/rate`, `/convert`, `/currencies`, `/vat`).

## Related clients

- [`unirate-api`](https://crates.io/crates/unirate-api) — the underlying Rust client
- Official clients for Python, Node/TS, Go, Java, Ruby, PHP, .NET, and Swift

## License

MIT © 2026 Unirate Team. See [LICENSE](LICENSE).
