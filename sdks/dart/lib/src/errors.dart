/// The stable `code` from the gateway's error envelope.
///
/// Branch on this, never on the message: messages are written for humans
/// and change without warning, while these are part of the contract.
enum AtlasErrorCode {
  invalidArgument('invalid_argument'),
  unauthenticated('unauthenticated'),
  permissionDenied('permission_denied'),
  notFound('not_found'),
  alreadyExists('already_exists'),
  failedPrecondition('failed_precondition'),
  rateLimited('rate_limited'),
  unavailable('unavailable'),
  internal('internal'),

  /// A code this SDK version does not know.
  ///
  /// Present so a gateway that adds a code does not become a parse
  /// failure here: an SDK that refuses to deserialize an unfamiliar error
  /// is strictly worse at reporting errors than one that passes it on.
  unknown('unknown');

  const AtlasErrorCode(this.wire);

  final String wire;

  static AtlasErrorCode fromWire(String? value) {
    for (final code in AtlasErrorCode.values) {
      if (code.wire == value) return code;
    }
    return AtlasErrorCode.unknown;
  }
}

/// The gateway answered with an error envelope.
class AtlasError implements Exception {
  AtlasError({
    required this.code,
    required this.message,
    required this.status,
  });

  final AtlasErrorCode code;
  final String message;
  final int status;

  /// Whether retrying the identical request might succeed.
  ///
  /// Says nothing about whether it is *safe* to retry — that depends on
  /// the request's idempotency, which the transport decides.
  bool get isRetryable =>
      code == AtlasErrorCode.unavailable || code == AtlasErrorCode.rateLimited;

  @override
  String toString() => 'AtlasError(${code.wire}, $status): $message';
}

/// The request never produced a response: DNS, TCP, TLS, or timeout.
///
/// Deliberately a different type from [AtlasError]. A caller retrying or
/// alerting needs to tell "the service rejected this" from "we could not
/// ask" — the first is about the request, the second about the network,
/// and treating them alike produces both spurious alerts and missed
/// outages.
class AtlasConnectionError implements Exception {
  AtlasConnectionError(this.message);

  final String message;

  bool get isRetryable => true;

  @override
  String toString() => 'AtlasConnectionError: $message';
}
