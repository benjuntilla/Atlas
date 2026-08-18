# @atlas/sdk

TypeScript client for the Atlas gateway. Zero runtime dependencies — it
uses the platform `fetch`, so it runs on Node 20.3+, Deno, Bun, and in
browsers. (The floor is 20.3 rather than 18 because the transport uses
`AbortSignal.any` for composing timeouts with caller-supplied signals.)

```bash
npm install @atlas/sdk
```

## Usage

```ts
import { AtlasClient, AtlasError } from '@atlas/sdk';

const atlas = new AtlasClient({ baseUrl: 'https://api.atlas.dev' });

await atlas.auth.register({ email: 'rider@example.com', password: 'hunter2!' });
await atlas.auth.login({ email: 'rider@example.com', password: 'hunter2!' });
// The token is stored on the client; later calls are authenticated.

await atlas.geo.updateLocation({ lat: 51.5074, lng: -0.1278 });

const { users } = await atlas.geo.nearby({
  lat: 51.5074,
  lng: -0.1278,
  radiusM: 500,
});

const fence = await atlas.geo.createGeofence({
  label: 'home',
  centerLat: 51.5074,
  centerLng: -0.1278,
  radiusM: 250,
});

// Deposits are the only way money enters the platform.
await atlas.payments.deposit({ amountCents: 10_000 });
const { balanceCents } = await atlas.payments.wallet();
```

## Errors

Every failure throws `AtlasError` with a stable `code`. Branch on `code`,
not on `status` — the code is derived from the backend's gRPC status and
does not move when an HTTP status does.

```ts
try {
  await atlas.payments.createTransaction({ toUserId, amountCents: 2_500 });
} catch (err) {
  if (err instanceof AtlasError && err.code === 'failed_precondition') {
    // insufficient funds, or the wallet is in the wrong state
  }
}
```

`AtlasConnectionError` is thrown separately when no HTTP response arrived
at all — DNS, TCP, TLS, or a timeout. It is a distinct type because a
caller asking "did the server see my request?" cannot answer that from a
status code.

## Retries

Safe requests are retried with exponential backoff and jitter. Unsafe ones
are not:

| Request | Retried |
|---|---|
| `GET`, `DELETE` | yes — idempotent by definition |
| `POST` with an `Idempotency-Key` | yes |
| `POST` without one | **no** |

`payments.createTransaction` and `payments.deposit` always send an
idempotency key, generating one if you do not supply it. That is what makes
them retryable at all.

A generated key covers retries *within a single call*. It does not survive
your process restarting, so if your application retries after a crash,
persist your own key and pass it:

```ts
await atlas.payments.createTransaction({
  toUserId,
  amountCents: 2_500,
  idempotencyKey: `fare-${rideId}`,
});
```

## Identity

No method takes a user id. The gateway derives identity from the bearer
token on every authenticated call, and its request bodies have no
`user_id` field — that absence is what stops one caller acting as another.
An SDK method that accepted a user id would imply a capability the API
does not have.

For the same reason there is no `settle` or `refund`: neither is exposed
through the gateway, because it cannot verify the caller owns the
transaction. Settlement runs server-side off ride lifecycle events.

## Options

```ts
new AtlasClient({
  baseUrl: 'https://api.atlas.dev',
  token: 'atl_...',   // resume an existing session
  timeoutMs: 10_000,  // per request
  maxRetries: 2,      // safe requests only
  fetch: customFetch, // injectable, e.g. for tracing
  headers: { 'x-request-id': id },
});
```

## Development

```bash
npm install
npm test        # builds, then runs the suite against a real local server
npm run typecheck
```

Tests bind a real HTTP server on a real port rather than mocking `fetch`,
so the actual request path — headers, retries, JSON handling — is what
gets exercised.
