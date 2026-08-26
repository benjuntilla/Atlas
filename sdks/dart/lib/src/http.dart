import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'errors.dart';

const String keyHeader = 'X-Atlas-Key';

/// The transport: headers, retries, timeouts.
///
/// Built on `dart:io`'s HttpClient rather than package:http so the SDK has
/// no runtime dependencies — a package that pulls in `http` forces that
/// version choice on every Flutter app that adopts it.
class Http {
  Http({
    required String baseUrl,
    required this.projectKey,
    required this.timeout,
    required this.maxRetries,
    HttpClient? client,
  })  : baseUrl = baseUrl.endsWith('/')
            ? baseUrl.substring(0, baseUrl.length - 1)
            : baseUrl,
        _client = client ?? HttpClient();

  final String baseUrl;
  final String projectKey;
  final Duration timeout;
  final int maxRetries;
  final HttpClient _client;
  final Random _random = Random();

  String? token;

  void close() => _client.close(force: true);

  /// Send a request, retrying only when it is safe to.
  ///
  /// [replayable] marks a POST the caller has made idempotent by
  /// supplying an idempotency key. Without it a POST is sent exactly once:
  /// replaying a non-idempotent write is how one deposit becomes two.
  Future<dynamic> send({
    required String method,
    required String path,
    Map<String, dynamic>? body,
    Map<String, String>? query,
    bool authenticated = true,
    bool replayable = false,
  }) async {
    final idempotent = method == 'GET' || method == 'DELETE' || replayable;
    final attempts = idempotent ? maxRetries + 1 : 1;

    Object? lastError;
    for (var attempt = 0; attempt < attempts; attempt++) {
      if (attempt > 0) {
        // Exponential backoff with jitter. Without jitter every client
        // that failed together retries together, and the recovering
        // service is hit by a synchronised wave.
        final base = 100 * (1 << (attempt - 1));
        await Future<void>.delayed(
          Duration(milliseconds: base + _random.nextInt(base ~/ 2 + 1)),
        );
      }

      try {
        return await _attempt(
          method: method,
          path: path,
          body: body,
          query: query,
          authenticated: authenticated,
        );
      } on AtlasError catch (e) {
        if (!e.isRetryable || attempt + 1 == attempts) rethrow;
        lastError = e;
      } on AtlasConnectionError catch (e) {
        if (attempt + 1 == attempts) rethrow;
        lastError = e;
      }
    }
    throw lastError ?? AtlasConnectionError('no attempt was made');
  }

  Future<dynamic> _attempt({
    required String method,
    required String path,
    Map<String, dynamic>? body,
    Map<String, String>? query,
    required bool authenticated,
  }) async {
    final uri = Uri.parse('$baseUrl$path').replace(
      queryParameters: (query == null || query.isEmpty) ? null : query,
    );

    HttpClientResponse response;
    String text;
    try {
      final request = await _client.openUrl(method, uri).timeout(timeout);

      // Sent on every request, including register and login: creating a
      // user means creating them in a project.
      request.headers.set(keyHeader, projectKey);
      if (authenticated && token != null) {
        request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
      }
      if (body != null) {
        request.headers.contentType = ContentType.json;
        request.write(jsonEncode(body));
      }

      response = await request.close().timeout(timeout);
      text = await response.transform(utf8.decoder).join().timeout(timeout);
    } on TimeoutException {
      throw AtlasConnectionError('request to $uri timed out after $timeout');
    } on SocketException catch (e) {
      throw AtlasConnectionError('could not reach $uri: ${e.message}');
    } on HttpException catch (e) {
      throw AtlasConnectionError('could not reach $uri: ${e.message}');
    }

    if (response.statusCode >= 200 && response.statusCode < 300) {
      if (text.trim().isEmpty) return null;
      return jsonDecode(text);
    }
    throw _toError(response.statusCode, text);
  }

  AtlasError _toError(int status, String body) {
    try {
      final decoded = jsonDecode(body);
      if (decoded is Map<String, dynamic>) {
        final envelope = decoded['error'];
        if (envelope is Map<String, dynamic>) {
          return AtlasError(
            code: AtlasErrorCode.fromWire(envelope['code'] as String?),
            message: envelope['message'] as String? ?? 'unknown error',
            status: status,
          );
        }
      }
    } on FormatException {
      // Falls through: a non-JSON body means something between the caller
      // and the gateway answered — a proxy, an ingress 502 — and the
      // status is the only reliable signal.
    }
    return AtlasError(
      code: _codeForStatus(status),
      message: body.isEmpty
          ? 'HTTP $status'
          : body.substring(0, body.length < 200 ? body.length : 200),
      status: status,
    );
  }

  AtlasErrorCode _codeForStatus(int status) => switch (status) {
        401 => AtlasErrorCode.unauthenticated,
        403 => AtlasErrorCode.permissionDenied,
        404 => AtlasErrorCode.notFound,
        409 => AtlasErrorCode.alreadyExists,
        429 => AtlasErrorCode.rateLimited,
        503 => AtlasErrorCode.unavailable,
        _ => AtlasErrorCode.unknown,
      };
}
