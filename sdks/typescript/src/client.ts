import { Http } from './http.js';
import type {
  Claims,
  CreateGeofenceParams,
  CreateTransactionParams,
  Geofence,
  GeofenceCheck,
  LocationParams,
  LoginParams,
  NearbyParams,
  NearbyUser,
  RegisterParams,
  RegisterResult,
  RouteCandidate,
  RouteScore,
  Session,
  Transaction,
  Wallet,
} from './types.js';

export interface AtlasClientOptions {
  /** Gateway origin, e.g. https://api.atlas.dev. The /v1 prefix is added. */
  baseUrl: string;
  /** Existing bearer token. `login()` sets this for you. */
  token?: string;
  /** Per-request timeout. Default 10s, matching the gateway's own upstream deadline. */
  timeoutMs?: number;
  /** Retries for safe requests only. Default 2. */
  maxRetries?: number;
  fetch?: typeof globalThis.fetch;
  headers?: Record<string, string>;
}

/**
 * Client for the Atlas gateway.
 *
 * ```ts
 * const atlas = new AtlasClient({ baseUrl: 'https://api.atlas.dev' });
 * await atlas.auth.login({ email, password });   // token stored on the client
 * const { users } = await atlas.geo.nearby({ lat, lng, radiusM: 500 });
 * ```
 *
 * # Identity
 *
 * No method takes a user id. The gateway derives it from the token on
 * every authenticated call, and its request bodies have no `user_id`
 * field at all — that absence is what stops one caller acting as another.
 * An SDK that accepted a user id would be implying a capability the API
 * does not have.
 */
export class AtlasClient {
  private readonly http: Http;
  private token: string | undefined;

  readonly auth: AuthApi;
  readonly geo: GeoApi;
  readonly payments: PaymentsApi;

  constructor(opts: AtlasClientOptions) {
    this.http = new Http({
      baseUrl: opts.baseUrl,
      timeoutMs: opts.timeoutMs ?? 10_000,
      maxRetries: opts.maxRetries ?? 2,
      ...(opts.fetch ? { fetch: opts.fetch } : {}),
      ...(opts.headers ? { headers: opts.headers } : {}),
    });
    this.token = opts.token;

    this.auth = new AuthApi(this.http, this);
    this.geo = new GeoApi(this.http, this);
    this.payments = new PaymentsApi(this.http, this);
  }

  /** The current bearer token, if any. */
  getToken(): string | undefined {
    return this.token;
  }

  /** Set or clear the token. `login()` calls this; `logout()` clears it. */
  setToken(token: string | undefined): void {
    this.token = token;
  }
}

/** Internal: namespaces read the token through this at call time, never cache it. */
interface TokenHolder {
  getToken(): string | undefined;
  setToken(token: string | undefined): void;
}

// --- auth -------------------------------------------------------------------

class AuthApi {
  constructor(
    private readonly http: Http,
    private readonly holder: TokenHolder,
  ) {}

  /** Create an account. Does not log in — call `login` next. */
  async register(params: RegisterParams): Promise<RegisterResult> {
    const res = await this.http.request<{ user_id: string }>({
      method: 'POST',
      path: '/v1/auth/register',
      body: { email: params.email, password: params.password },
    });
    return { userId: res.user_id };
  }

  /**
   * Exchange credentials for a token, and store it on the client so
   * subsequent calls are authenticated.
   */
  async login(params: LoginParams): Promise<Session> {
    // The gateway rejects one coordinate without the other, so send both
    // or neither rather than letting a half-filled form 400.
    const hasPosition = params.lat !== undefined && params.lng !== undefined;
    const res = await this.http.request<{ token: string; expires_at: number }>({
      method: 'POST',
      path: '/v1/auth/login',
      body: {
        email: params.email,
        password: params.password,
        ...(hasPosition ? { lat: params.lat, lng: params.lng } : {}),
      },
    });
    this.holder.setToken(res.token);
    return { token: res.token, expiresAt: res.expires_at };
  }

  /** Revoke the current token and clear it from the client. */
  async logout(): Promise<{ success: boolean }> {
    const res = await this.http.request<{ success: boolean }>({
      method: 'POST',
      path: '/v1/auth/logout',
      token: this.holder.getToken(),
    });
    // Cleared regardless of the reported result: the token is either
    // revoked or was already invalid, and keeping it helps nobody.
    this.holder.setToken(undefined);
    return res;
  }

  /** The current token's claims. Useful for checking a session is still live. */
  async me(): Promise<Claims> {
    const res = await this.http.request<{
      user_id: string;
      session_id: string;
      last_lat: number;
      last_lng: number;
      issued_at: number;
      expires_at: number;
    }>({
      method: 'GET',
      path: '/v1/auth/me',
      token: this.holder.getToken(),
    });
    return {
      userId: res.user_id,
      sessionId: res.session_id,
      lastLat: res.last_lat,
      lastLng: res.last_lng,
      issuedAt: res.issued_at,
      expiresAt: res.expires_at,
    };
  }
}

// --- geo --------------------------------------------------------------------

class GeoApi {
  constructor(
    private readonly http: Http,
    private readonly holder: TokenHolder,
  ) {}

  /** Record the caller's position. */
  async updateLocation(params: LocationParams): Promise<{ ok: boolean }> {
    return this.http.request<{ ok: boolean }>({
      method: 'POST',
      path: '/v1/geo/locations',
      token: this.holder.getToken(),
      body: {
        lat: params.lat,
        lng: params.lng,
        ...(params.recordedAt !== undefined
          ? { recorded_at: params.recordedAt }
          : {}),
      },
    });
  }

  /** Find other users within `radiusM` metres, nearest first. */
  async nearby(params: NearbyParams): Promise<{ users: NearbyUser[] }> {
    const res = await this.http.request<{
      users: Array<{
        user_id: string;
        lat: number;
        lng: number;
        distance_m: number;
        safety_score: number;
      }>;
    }>({
      method: 'GET',
      path: '/v1/geo/nearby',
      token: this.holder.getToken(),
      query: {
        lat: params.lat,
        lng: params.lng,
        radius_m: params.radiusM,
        role: params.role,
        limit: params.limit,
      },
    });
    return {
      users: res.users.map((u) => ({
        userId: u.user_id,
        lat: u.lat,
        lng: u.lng,
        distanceM: u.distance_m,
        safetyScore: u.safety_score,
      })),
    };
  }

  /** Score route candidates against the safety graph. Highest score wins. */
  async scoreRoute(candidates: RouteCandidate[]): Promise<RouteScore> {
    const res = await this.http.request<{
      best_route_id: string;
      score: number;
      all_scores: Array<{ route_id: string; score: number }>;
    }>({
      method: 'POST',
      path: '/v1/geo/routes/score',
      token: this.holder.getToken(),
      body: {
        candidates: candidates.map((c) => ({
          route_id: c.routeId,
          points: c.points.map((p) => ({ lat: p.lat, lng: p.lng })),
        })),
      },
    });
    return {
      bestRouteId: res.best_route_id,
      score: res.score,
      allScores: res.all_scores.map((s) => ({
        routeId: s.route_id,
        score: s.score,
      })),
    };
  }

  async createGeofence(params: CreateGeofenceParams): Promise<Geofence> {
    const res = await this.http.request<WireGeofence>({
      method: 'POST',
      path: '/v1/geo/geofences',
      token: this.holder.getToken(),
      body: {
        ...(params.label !== undefined ? { label: params.label } : {}),
        center_lat: params.centerLat,
        center_lng: params.centerLng,
        radius_m: params.radiusM,
      },
    });
    return toGeofence(res);
  }

  async listGeofences(opts: { activeOnly?: boolean } = {}): Promise<Geofence[]> {
    const res = await this.http.request<{ geofences: WireGeofence[] }>({
      method: 'GET',
      path: '/v1/geo/geofences',
      token: this.holder.getToken(),
      query: { active_only: opts.activeOnly },
    });
    return res.geofences.map(toGeofence);
  }

  /**
   * Deactivate a geofence. This is a soft delete: the fence stops matching
   * but stays visible to `listGeofences({ activeOnly: false })`.
   */
  async deleteGeofence(geofenceId: string): Promise<{ deleted: boolean }> {
    return this.http.request<{ deleted: boolean }>({
      method: 'DELETE',
      path: `/v1/geo/geofences/${encodeURIComponent(geofenceId)}`,
      token: this.holder.getToken(),
    });
  }

  /** Which of the caller's geofences contain this point right now. */
  async checkGeofences(point: {
    lat: number;
    lng: number;
  }): Promise<GeofenceCheck> {
    const res = await this.http.request<{
      triggered: boolean;
      geofence_ids: string[];
    }>({
      method: 'POST',
      path: '/v1/geo/geofences/check',
      token: this.holder.getToken(),
      body: { lat: point.lat, lng: point.lng },
    });
    return { triggered: res.triggered, geofenceIds: res.geofence_ids };
  }
}

interface WireGeofence {
  id: string;
  user_id: string;
  label: string;
  center_lat: number;
  center_lng: number;
  radius_m: number;
  active: boolean;
  created_at: number;
}

function toGeofence(g: WireGeofence): Geofence {
  return {
    id: g.id,
    userId: g.user_id,
    label: g.label,
    centerLat: g.center_lat,
    centerLng: g.center_lng,
    radiusM: g.radius_m,
    active: g.active,
    createdAt: g.created_at,
  };
}

// --- payments ---------------------------------------------------------------

class PaymentsApi {
  constructor(
    private readonly http: Http,
    private readonly holder: TokenHolder,
  ) {}

  /** The caller's own wallet. There is no way to read anyone else's. */
  async wallet(): Promise<Wallet> {
    const res = await this.http.request<{
      balance_cents: number;
      currency: string;
    }>({
      method: 'GET',
      path: '/v1/payments/wallet',
      token: this.holder.getToken(),
    });
    return { balanceCents: res.balance_cents, currency: res.currency };
  }

  /**
   * Move money from the caller's wallet to another user.
   *
   * An idempotency key is always sent — supplied by you, or generated
   * here. That is what lets the transport retry this call at all; without
   * a key it would be the one request in the SDK that can never be
   * repeated safely. A generated key covers retries within this call, but
   * only a key you persist covers your own process crashing and retrying.
   *
   * There is deliberately no `settle` or `refund` here: those are not
   * exposed through the gateway, because it has no way to check that the
   * caller owns the transaction. Settlement is driven server-side by ride
   * lifecycle events.
   */
  async createTransaction(
    params: CreateTransactionParams,
  ): Promise<Transaction> {
    const idempotencyKey = params.idempotencyKey ?? randomKey();
    const res = await this.http.request<{
      transaction_id: string;
      status: string;
    }>({
      method: 'POST',
      path: '/v1/payments/transactions',
      token: this.holder.getToken(),
      // Sent as a header rather than in the body: it is the conventional
      // location, and the gateway prefers the header when both are present.
      headers: { 'Idempotency-Key': idempotencyKey },
      body: {
        to_user_id: params.toUserId,
        amount_cents: params.amountCents,
        ...(params.rideId !== undefined ? { ride_id: params.rideId } : {}),
      },
    });
    return { transactionId: res.transaction_id, status: res.status };
  }
}

function randomKey(): string {
  // crypto.randomUUID is available in Node 18+ and every browser that has
  // fetch, so it needs no polyfill in any environment this SDK supports.
  return globalThis.crypto.randomUUID();
}
