import { test, describe, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, type Server, type IncomingMessage } from 'node:http';
import { AtlasClient, AtlasError, AtlasConnectionError } from '../src/index.js';

/**
 * Tests run against a real HTTP server on a real port, so the actual fetch
 * path, header handling, retry loop, and JSON parsing are all exercised. A
 * mocked fetch would test the mock.
 */

interface Recorded {
  method: string;
  url: string;
  headers: Record<string, string | string[] | undefined>;
  body: string;
}

let recorded: Recorded[] = [];
/** Queue of responses; each request pops one. Falls back to 200 {}. */
let responses: Array<{ status: number; body: string; contentType?: string }> = [];

const server: Server = createServer((req: IncomingMessage, res) => {
  const chunks: Buffer[] = [];
  req.on('data', (c: Buffer) => chunks.push(c));
  req.on('end', () => {
    recorded.push({
      method: req.method ?? '',
      url: req.url ?? '',
      headers: req.headers,
      body: Buffer.concat(chunks).toString(),
    });
    const next = responses.shift() ?? { status: 200, body: '{}' };
    res.writeHead(next.status, {
      'content-type': next.contentType ?? 'application/json',
    });
    res.end(next.body);
  });
});

// Started with top-level await rather than a root `before` hook.
//
// On the oldest Node this package supports (20.3), a root-level `before`
// does not reliably complete before the tests inside top-level
// `describe` blocks begin, so `baseUrl` was still undefined when the
// first client was constructed — the whole suite failed on 20.3 while
// passing on 22. Module evaluation, by contrast, is guaranteed to finish
// before any test runs on every supported version.
await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
const addr = server.address();
if (typeof addr === 'string' || addr === null) throw new Error('no port');
const baseUrl = `http://127.0.0.1:${addr.port}`;
// A listening socket would otherwise keep the process alive if `after`
// never runs; the suite should exit on its own either way.
server.unref();

after(async () => {
  await new Promise<void>((resolve, reject) =>
    server.close((e) => (e ? reject(e) : resolve())),
  );
});

function reset(): void {
  recorded = [];
  responses = [];
}

const TEST_PROJECT_KEY = 'atl_test_0123456789abcdef0123456789abcdef';

function client(opts: { token?: string; maxRetries?: number } = {}): AtlasClient {
  return new AtlasClient({
    baseUrl,
    projectKey: TEST_PROJECT_KEY,
    maxRetries: opts.maxRetries ?? 0,
    timeoutMs: 2_000,
    ...(opts.token ? { token: opts.token } : {}),
  });
}

describe('auth', () => {
  test('register maps snake_case to camelCase', async () => {
    reset();
    responses.push({ status: 201, body: '{"user_id":"u-1"}' });
    const out = await client().auth.register({
      email: 'a@b.dev',
      password: 'pw',
    });
    assert.equal(out.userId, 'u-1');
    assert.equal(recorded[0]?.url, '/v1/auth/register');
    assert.deepEqual(JSON.parse(recorded[0]!.body), {
      email: 'a@b.dev',
      password: 'pw',
    });
  });

  test('login stores the token for later calls', async () => {
    reset();
    responses.push({ status: 200, body: '{"token":"tok-1","expires_at":99}' });
    responses.push({
      status: 200,
      body: '{"user_id":"u-1","session_id":"s-1","last_lat":0,"last_lng":0,"issued_at":1,"expires_at":99}',
    });

    const c = client();
    const session = await c.auth.login({ email: 'a@b.dev', password: 'pw' });
    assert.equal(session.token, 'tok-1');
    assert.equal(session.expiresAt, 99);
    assert.equal(c.getToken(), 'tok-1');

    await c.auth.me();
    assert.equal(recorded[1]?.headers['authorization'], 'Bearer tok-1');
  });

  test('login omits coordinates unless both are given', async () => {
    reset();
    responses.push({ status: 200, body: '{"token":"t","expires_at":1}' });
    // The gateway 400s on one coordinate without the other, so a
    // half-filled form must not be forwarded as-is.
    await client().auth.login({ email: 'a@b.dev', password: 'pw', lat: 51.5 });
    assert.deepEqual(JSON.parse(recorded[0]!.body), {
      email: 'a@b.dev',
      password: 'pw',
    });

    reset();
    responses.push({ status: 200, body: '{"token":"t","expires_at":1}' });
    await client().auth.login({
      email: 'a@b.dev',
      password: 'pw',
      lat: 51.5,
      lng: -0.1,
    });
    assert.deepEqual(JSON.parse(recorded[0]!.body), {
      email: 'a@b.dev',
      password: 'pw',
      lat: 51.5,
      lng: -0.1,
    });
  });

  test('logout clears the token even when the server says false', async () => {
    reset();
    responses.push({ status: 200, body: '{"success":false}' });
    const c = client({ token: 'tok-1' });
    await c.auth.logout();
    assert.equal(c.getToken(), undefined);
  });
});

describe('geo', () => {
  test('nearby sends snake_case query params and maps the response', async () => {
    reset();
    responses.push({
      status: 200,
      body: '{"users":[{"user_id":"u-2","lat":1,"lng":2,"distance_m":42.5,"safety_score":1500}]}',
    });
    const { users } = await client({ token: 't' }).geo.nearby({
      lat: 1,
      lng: 2,
      radiusM: 500,
      limit: 10,
    });

    const url = new URL(baseUrl + recorded[0]!.url);
    assert.equal(url.pathname, '/v1/geo/nearby');
    assert.equal(url.searchParams.get('radius_m'), '500');
    assert.equal(url.searchParams.get('limit'), '10');
    // Omitted optional params must not appear as the string "undefined".
    assert.equal(url.searchParams.has('role'), false);

    assert.equal(users[0]?.userId, 'u-2');
    assert.equal(users[0]?.distanceM, 42.5);
    assert.equal(users[0]?.safetyScore, 1500);
  });

  test('scoreRoute maps nested route candidates both ways', async () => {
    reset();
    responses.push({
      status: 200,
      body: '{"best_route_id":"r-1","score":1600,"all_scores":[{"route_id":"r-1","score":1600}]}',
    });
    const out = await client({ token: 't' }).geo.scoreRoute([
      { routeId: 'r-1', points: [{ lat: 1, lng: 2 }, { lat: 3, lng: 4 }] },
    ]);

    assert.deepEqual(JSON.parse(recorded[0]!.body), {
      candidates: [
        { route_id: 'r-1', points: [{ lat: 1, lng: 2 }, { lat: 3, lng: 4 }] },
      ],
    });
    assert.equal(out.bestRouteId, 'r-1');
    assert.equal(out.allScores[0]?.routeId, 'r-1');
  });

  test('deleteGeofence url-encodes the id', async () => {
    reset();
    responses.push({ status: 200, body: '{"deleted":true}' });
    // A caller passing something odd must not be able to escape the path.
    await client({ token: 't' }).geo.deleteGeofence('a/../b');
    assert.equal(recorded[0]?.url, '/v1/geo/geofences/a%2F..%2Fb');
  });
});

describe('payments', () => {
  test('createTransaction always sends an Idempotency-Key', async () => {
    reset();
    responses.push({
      status: 201,
      body: '{"transaction_id":"tx-1","status":"pending"}',
    });
    const out = await client({ token: 't' }).payments.createTransaction({
      toUserId: 'u-2',
      amountCents: 500,
    });

    const key = recorded[0]?.headers['idempotency-key'];
    assert.ok(typeof key === 'string' && key.length > 0, 'key must be sent');
    assert.equal(out.transactionId, 'tx-1');
    // No user id in the body: identity comes from the token.
    assert.deepEqual(JSON.parse(recorded[0]!.body), {
      to_user_id: 'u-2',
      amount_cents: 500,
    });
  });

  test('deposit sends an idempotency key and maps the balance back', async () => {
    reset();
    responses.push({
      status: 201,
      body: '{"transaction_id":"tx-9","status":"settled","balance_cents":10000}',
    });
    const out = await client({ token: 't' }).payments.deposit({ amountCents: 10_000 });

    assert.equal(recorded[0]?.url, '/v1/payments/deposits');
    assert.ok(typeof recorded[0]?.headers['idempotency-key'] === 'string');
    // No toUserId: a caller can only ever top up themselves.
    assert.deepEqual(JSON.parse(recorded[0]!.body), { amount_cents: 10_000 });
    assert.equal(out.status, 'settled');
    assert.equal(out.balanceCents, 10_000);
  });

  test('a supplied idempotency key is used verbatim', async () => {
    reset();
    responses.push({ status: 201, body: '{"transaction_id":"t","status":"pending"}' });
    await client({ token: 't' }).payments.createTransaction({
      toUserId: 'u-2',
      amountCents: 500,
      idempotencyKey: 'mine-123',
    });
    assert.equal(recorded[0]?.headers['idempotency-key'], 'mine-123');
  });
});

describe('errors', () => {
  test('the error envelope becomes an AtlasError with a stable code', async () => {
    reset();
    responses.push({
      status: 422,
      body: '{"error":{"code":"failed_precondition","message":"insufficient funds"}}',
    });
    await assert.rejects(
      () =>
        client({ token: 't' }).payments.createTransaction({
          toUserId: 'u-2',
          amountCents: 1,
        }),
      (err: unknown) => {
        assert.ok(err instanceof AtlasError);
        assert.equal(err.code, 'failed_precondition');
        assert.equal(err.status, 422);
        assert.equal(err.message, 'insufficient funds');
        assert.equal(err.retryable, false);
        return true;
      },
    );
  });

  test('a non-envelope error body still yields a useful error', async () => {
    reset();
    // What a load balancer returns when it never reached the gateway.
    responses.push({
      status: 502,
      body: '<html>502 Bad Gateway</html>',
      contentType: 'text/html',
    });
    await assert.rejects(
      () => client({ token: 't' }).payments.wallet(),
      (err: unknown) => {
        assert.ok(err instanceof AtlasError);
        assert.equal(err.status, 502);
        assert.ok(err.message.includes('502'));
        return true;
      },
    );
  });

  test('connection failures are a distinct type', async () => {
    const c = new AtlasClient({
      // Port nothing listens on.
      baseUrl: 'http://127.0.0.1:59987',
      projectKey: TEST_PROJECT_KEY,
      maxRetries: 0,
      timeoutMs: 1_000,
    });
    await assert.rejects(
      () => c.auth.register({ email: 'a@b.dev', password: 'pw' }),
      (err: unknown) => {
        assert.ok(err instanceof AtlasConnectionError);
        return true;
      },
    );
  });
});

describe('retries', () => {
  test('GET retries a 503 and then succeeds', async () => {
    reset();
    responses.push({
      status: 503,
      body: '{"error":{"code":"unavailable","message":"backend down"}}',
    });
    responses.push({ status: 200, body: '{"balance_cents":100,"currency":"USD"}' });

    const out = await client({ token: 't', maxRetries: 2 }).payments.wallet();
    assert.equal(out.balanceCents, 100);
    assert.equal(recorded.length, 2, 'should have retried once');
  });

  test('a 4xx is not retried', async () => {
    reset();
    responses.push({
      status: 401,
      body: '{"error":{"code":"unauthenticated","message":"invalid token"}}',
    });
    await assert.rejects(() =>
      client({ token: 'bad', maxRetries: 2 }).payments.wallet(),
    );
    assert.equal(recorded.length, 1, 'client errors must not be retried');
  });

  test('POST with an idempotency key is retried', async () => {
    reset();
    responses.push({
      status: 503,
      body: '{"error":{"code":"unavailable","message":"down"}}',
    });
    responses.push({
      status: 201,
      body: '{"transaction_id":"tx-1","status":"pending"}',
    });

    const out = await client({ token: 't', maxRetries: 2 }).payments.createTransaction({
      toUserId: 'u-2',
      amountCents: 500,
    });
    assert.equal(out.transactionId, 'tx-1');
    assert.equal(recorded.length, 2);
    // Critically, the same key on both attempts — otherwise the retry is a
    // second distinct transaction and the payer is charged twice.
    assert.equal(
      recorded[0]?.headers['idempotency-key'],
      recorded[1]?.headers['idempotency-key'],
    );
  });

  test('POST without an idempotency key is never retried', async () => {
    reset();
    responses.push({
      status: 503,
      body: '{"error":{"code":"unavailable","message":"down"}}',
    });
    responses.push({ status: 200, body: '{"ok":true}' });

    // updateLocation carries no key, so repeating it is not this layer's
    // call to make.
    await assert.rejects(() =>
      client({ token: 't', maxRetries: 2 }).geo.updateLocation({ lat: 1, lng: 2 }),
    );
    assert.equal(recorded.length, 1);
  });
});

describe('project key', () => {
  test('is sent on every request, authenticated or not', async () => {
    reset();
    await client().auth.register({ email: 'a@b.dev', password: 'pw' });
    reset();
    await client({ token: 't' }).payments.wallet();

    // Both calls above recorded one request each; check the second, which
    // also carries a bearer token, to prove the two headers coexist.
    assert.equal(recorded[0]?.headers['x-atlas-key'], TEST_PROJECT_KEY);
    assert.equal(recorded[0]?.headers['authorization'], 'Bearer t');
  });

  test('a missing project key fails at construction, not at call time', () => {
    assert.throws(
      // Cast because the type already forbids this; the runtime check is
      // for JavaScript callers and for a key read from an unset env var.
      () => new AtlasClient({ baseUrl, projectKey: '' } as never),
      /projectKey/,
    );
  });

  test('caller headers cannot override the project key', async () => {
    // Otherwise a stray `headers` option would silently send requests as
    // some other project, which is the sort of thing that only shows up
    // in production.
    reset();
    const c = new AtlasClient({
      baseUrl,
      projectKey: TEST_PROJECT_KEY,
      maxRetries: 0,
      headers: { 'X-Atlas-Key': 'atl_live_ffffffffffffffffffffffffffffffff' },
    });
    await c.auth.register({ email: 'a@b.dev', password: 'pw' });
    assert.equal(recorded[0]?.headers['x-atlas-key'], TEST_PROJECT_KEY);
  });
});
