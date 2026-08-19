# atlas-sdk

Rust client for the Atlas gateway. Async, `reqwest` + `rustls`, no system
TLS dependency — so it links in a `FROM scratch` container and in
cross-compiled binaries, which is where a system OpenSSL usually breaks.

```toml
[dependencies]
atlas-sdk = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use atlas_sdk::{AtlasClient, ClientOptions, Verdict};

# async fn example() -> atlas_sdk::Result<()> {
let atlas = AtlasClient::new(ClientOptions {
    base_url: "https://api.atlas.dev".into(),
    project_key: std::env::var("ATLAS_KEY").unwrap(),
    ..Default::default()
})?;

atlas.auth().login("rider@example.com", "hunter2!").await?;  // token stored

let users = atlas.geo().nearby(51.5074, -0.1278, 500.0).await?;
atlas.geo().cast_safety_vote(51.5074, -0.1278, Verdict::Safe).await?;
let wallet = atlas.payments().wallet().await?;
# Ok(())
# }
```

## Two credentials

| | Says | From | Lives |
|---|---|---|---|
| `project_key` | which application is calling | `atlas keys create` | your server, in an env var |
| bearer token | which of your users is calling | `auth().login()` | per user, per session |

Both are sent on every call, including register and login — creating a
user means creating them in a project. Neither substitutes for the other.

**The project key is a server-side secret.** Anyone holding it can act on
your whole project, so keep this client on your backend.

A missing key fails at construction rather than on every subsequent call,
because it is a configuration mistake and the error should point at the
line that has to change.

## What the types say

**No method takes a user id or a project id.** The gateway derives both
from the credentials, and its request bodies have no fields for them —
that absence is what stops one caller acting as another.

`Me::email_verified_at` is `Option<i64>`, not a `0` sentinel: 0 is a real
timestamp and a caller doing date maths on it renders 1970. Gate features
on `is_some()`.

`NearbyUser` carries `safety_vote_count` alongside `safety_score`. 1500
from nobody voting and 1500 from a hundred evenly split voters are
different facts, and a UI that renders them identically is claiming
knowledge it does not have.

`Debug` on `AtlasClient` and `ClientOptions` is implemented by hand and
redacts credentials. A derived one would print your project key the first
time anything logs a client.

## Errors

`Error::Api` carries a stable `ErrorCode` — branch on that, never on the
message. `Error::Connection` is deliberately separate: "the service
rejected this" and "we could not ask" need different handling, and
conflating them produces both spurious alerts and missed outages.

`Error::is_retryable()` reports whether retrying *could* help. Whether it
is *safe* to retry is a different question, and the transport answers it:
GET and DELETE are replayed, POST is not — except deposits and
transactions, which carry an idempotency key that makes replay safe on the
server side. The same key is reused across attempts; a fresh one per retry
would double-charge.

## Tests

```bash
cargo test -p atlas-sdk
```

They bind a real HTTP server on a real port rather than mocking the
transport, so headers, query encoding, retries, and JSON handling are
exercised as they actually run. A mock only proves the SDK can call a mock.
