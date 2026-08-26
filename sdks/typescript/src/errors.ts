/**
 * Errors thrown by the SDK.
 *
 * The gateway returns one envelope for every failure:
 *
 *   { "error": { "code": "invalid_argument", "message": "radius_m must be > 0" } }
 *
 * `code` is the stable part. It is derived from the backend's gRPC status
 * and deliberately does not move when an HTTP status does, so branching on
 * it is safe in a way that branching on `status` is not.
 */

/** Every `code` the gateway can return. */
export type AtlasErrorCode =
  | 'invalid_argument'
  | 'out_of_range'
  | 'unauthenticated'
  | 'permission_denied'
  | 'not_found'
  | 'already_exists'
  | 'aborted'
  | 'failed_precondition'
  | 'resource_exhausted'
  | 'cancelled'
  | 'deadline_exceeded'
  | 'unavailable'
  | 'unimplemented'
  | 'internal';

export class AtlasError extends Error {
  /** Stable machine-readable code. Branch on this. */
  readonly code: AtlasErrorCode | string;
  /** HTTP status that carried the error. */
  readonly status: number;
  /** Request path, for logging. Never includes the bearer token. */
  readonly path: string;

  constructor(opts: {
    code: string;
    message: string;
    status: number;
    path: string;
  }) {
    super(opts.message);
    this.name = 'AtlasError';
    this.code = opts.code;
    this.status = opts.status;
    this.path = opts.path;
  }

  /**
   * Whether retrying the identical request could plausibly succeed.
   *
   * Note this is about the *error*, not about the request: retrying a
   * non-idempotent POST is unsafe even when this returns true. The
   * transport takes both into account — see `http.ts`.
   */
  get retryable(): boolean {
    return (
      this.code === 'unavailable' ||
      this.code === 'deadline_exceeded' ||
      this.code === 'resource_exhausted' ||
      this.status >= 500
    );
  }
}

/** The request never produced an HTTP response: DNS, TCP, TLS, or timeout. */
export class AtlasConnectionError extends Error {
  readonly path: string;
  override readonly cause: unknown;

  constructor(message: string, path: string, cause: unknown) {
    super(message);
    this.name = 'AtlasConnectionError';
    this.path = path;
    this.cause = cause;
  }
}
