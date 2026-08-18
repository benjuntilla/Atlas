/**
 * Public types.
 *
 * These are camelCase; the wire is snake_case. The mapping is written out
 * per field in `client.ts` rather than done by a generic deep-converter:
 * a converter has to guess at edge cases, breaks the moment one field does
 * not follow the rule, and erases the types that make this SDK worth using.
 */

// --- auth -------------------------------------------------------------------

export interface RegisterParams {
  email: string;
  password: string;
}

export interface RegisterResult {
  userId: string;
}

export interface LoginParams {
  email: string;
  password: string;
  /** Optional position stamped into the token's claims. Send both or neither. */
  lat?: number;
  lng?: number;
}

export interface Session {
  token: string;
  /** Unix epoch seconds. */
  expiresAt: number;
}

export interface Claims {
  userId: string;
  sessionId: string;
  /** (0, 0) when no position was supplied at login — the wire has no nulls here. */
  lastLat: number;
  lastLng: number;
  issuedAt: number;
  expiresAt: number;
}

// --- geo --------------------------------------------------------------------

export interface LocationParams {
  lat: number;
  lng: number;
  /** Unix epoch seconds. Omit to let the platform stamp "now". */
  recordedAt?: number;
}

export interface NearbyParams {
  lat: number;
  lng: number;
  /** Metres. Must be > 0 and <= 50000. */
  radiusM: number;
  /** Free-form; used as a metrics label server-side. */
  role?: string;
  /** 1-100. Omit for the server default of 20. */
  limit?: number;
}

export interface NearbyUser {
  userId: string;
  lat: number;
  lng: number;
  distanceM: number;
  /** ELO-style safety score; 1500 is neutral. */
  safetyScore: number;
}

export interface LatLng {
  lat: number;
  lng: number;
}

export interface RouteCandidate {
  routeId: string;
  /** At least two points. */
  points: LatLng[];
}

export interface ScoredRoute {
  routeId: string;
  score: number;
}

export interface RouteScore {
  bestRouteId: string;
  score: number;
  allScores: ScoredRoute[];
}

export interface CreateGeofenceParams {
  label?: string;
  centerLat: number;
  centerLng: number;
  /** Metres. Must be > 0 and <= 50000. */
  radiusM: number;
}

export interface Geofence {
  id: string;
  userId: string;
  /** Empty string when unlabelled — the wire has no null here. */
  label: string;
  centerLat: number;
  centerLng: number;
  radiusM: number;
  active: boolean;
  createdAt: number;
}

export interface GeofenceCheck {
  triggered: boolean;
  geofenceIds: string[];
}

// --- payments ---------------------------------------------------------------

export interface Wallet {
  balanceCents: number;
  currency: string;
}

export interface CreateTransactionParams {
  toUserId: string;
  /** Must be > 0. */
  amountCents: number;
  /**
   * Supply this to make retries safe across process restarts. When it is
   * omitted the SDK generates one per call, which protects against its own
   * transport-level retries but not against your application retrying
   * after a crash.
   */
  idempotencyKey?: string;
  rideId?: string;
}

export interface Transaction {
  transactionId: string;
  /** "pending" | "settled" | "failed" | "refunded" */
  status: string;
}
