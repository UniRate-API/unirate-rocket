//! Runnable Rocket server exposing the UniRate routes.
//!
//! Usage:
//!
//! ```bash
//! UNIRATE_API_KEY=your-key cargo run --example server
//! ```
//!
//! Then, in another terminal (Rocket defaults to port 8000):
//!
//! ```bash
//! curl 'http://127.0.0.1:8000/rate?from=USD&to=EUR'
//! curl 'http://127.0.0.1:8000/convert?from=USD&to=EUR&amount=100'
//! curl 'http://127.0.0.1:8000/currencies'
//! curl 'http://127.0.0.1:8000/vat?country=DE'
//! ```

use rocket::launch;
use unirate_rocket::{unirate_routes, UniRateFairing};

#[launch]
fn rocket() -> _ {
    let key =
        std::env::var("UNIRATE_API_KEY").expect("UNIRATE_API_KEY must be set to run this example");

    rocket::build()
        .attach(UniRateFairing::new(key))
        .mount("/", unirate_routes())
}
