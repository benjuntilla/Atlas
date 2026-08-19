# atlas_sdk

Dart client for the Atlas gateway. **No runtime dependencies** — it uses
`dart:io`'s `HttpClient`, so adopting it does not force a `package:http`
version on your app.

```dart
import 'package:atlas_sdk/atlas_sdk.dart';

final atlas = AtlasClient(
  baseUrl: 'https://api.atlas.dev',
  projectKey: Platform.environment['ATLAS_KEY']!,
);

await atlas.auth.login(email: 'rider@example.com', password: 'hunter2!');

final users = await atlas.geo.nearby(lat: 51.5074, lng: -0.1278, radiusM: 500);
await atlas.geo.castSafetyVote(lat: 51.5074, lng: -0.1278, verdict: Verdict.safe);
final wallet = await atlas.payments.wallet();
```

## Do not ship this inside a Flutter app

Every call carries two credentials:

| | Says | From | Lives |
|---|---|---|---|
| `projectKey` | which application is calling | `atlas keys create` | your server, in an env var |
| bearer token | which of your users is calling | `auth.login()` | per user, per session |

**The project key is a server-side secret.** Anyone holding it can act on
your entire project — read every user, move money between wallets — and a
key compiled into a mobile app can be extracted from the binary in
minutes. This is the obvious thing to do with a Dart SDK, which is why the
warning is here rather than buried in a doc comment.

Put this client behind your own API, and let your app talk to that.

A missing key throws at construction rather than producing a 401 on every
call, because it is a configuration mistake and the stack trace should
point at the line that has to change.

## What the types say

**No method takes a user id or a project id.** The gateway derives both
from the credentials, and its request bodies have no fields for them —
that absence is what stops one caller acting as another.

`Me.emailVerifiedAt` is `int?`, not a `0` sentinel: 0 is a real timestamp
and a caller doing date maths on it renders 1970. There is an
`isEmailVerified` getter for the common case.

`NearbyUser` carries `safetyVoteCount` alongside `safetyScore`, with a
`hasSafetyData` getter. 1500 from nobody voting and 1500 from a hundred
evenly split voters are different facts, and a UI that renders them
identically is claiming knowledge it does not have.

`toString()` on the client redacts credentials. The default would print
your project key into the first crash report.

## Errors

`AtlasError` carries a stable `AtlasErrorCode` — branch on that, never on
the message. `AtlasConnectionError` is a separate type on purpose: "the
service rejected this" and "we could not ask" need different handling, and
conflating them produces both spurious alerts and missed outages.

`isRetryable` reports whether retrying *could* help. Whether it is *safe*
to retry is a different question, and the transport answers it: GET and
DELETE are replayed, POST is not — except deposits and transactions, which
carry an idempotency key that makes replay safe on the server side. The
same key is reused across attempts; a fresh one per retry would
double-charge. Keys are generated with `Random.secure()`, because a
predictable key could collide with another client's, and on this endpoint
a collision means one caller's payment is reported to another as an
idempotent replay.

## Development

```bash
dart pub get
dart test
dart analyze
```

Tests bind a real HTTP server on a real port rather than mocking the
transport, so headers, query encoding, retries, and JSON handling are
exercised as they actually run.
