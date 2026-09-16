import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group('VpayError.fromEnvelope', () {
    test('reads the four keys vpay_api::error_envelope_with_param writes', () {
      final error = VpayError.fromEnvelope({
        'error': {
          'type': 'invalid_request_error',
          'code': 'resource_missing',
          'message': 'No such payment intent: pi_1',
          'param': 'id',
        },
      })!;

      expect(error.type, 'invalid_request_error');
      expect(error.code, 'resource_missing');
      expect(error.message, 'No such payment intent: pi_1');
      expect(error.param, 'id');
    });

    test('leaves param absent when the server omitted it', () {
      final error = VpayError.fromEnvelope({
        'error': {'type': 'api_error', 'code': 'unexpected_response'},
      })!;

      expect(error.param, isNull);
    });

    test('answers null for a body with no error envelope shape', () {
      expect(VpayError.fromEnvelope({'object': 'balance'}), isNull);
      expect(VpayError.fromEnvelope(null), isNull);
      expect(VpayError.fromEnvelope('not even a map'), isNull);
      expect(VpayError.fromEnvelope({'error': 'not a map'}), isNull);
      expect(
        VpayError.fromEnvelope({
          'error': {'code': 'x'},
        }),
        isNull,
      );
    });
  });

  group('fixed-string factories never echo a value they were not handed', () {
    test('connection() carries no code and a fixed message', () {
      final error = VpayError.connection();
      expect(error.type, 'api_connection_error');
      expect(error.code, isNull);
    });

    test('unexpectedResponse(status) carries the status and nothing else', () {
      final error = VpayError.unexpectedResponse(502);
      expect(error.message, contains('502'));
      expect(error.code, VpayClientErrorCodes.unexpectedResponse);
    });

    test('embeddedSessionNotSupported names the session id and the reason', () {
      final error = VpayError.embeddedSessionNotSupported('cs_123');
      expect(error.code, VpayClientErrorCodes.embeddedSessionNotSupported);
      expect(error.message, contains('cs_123'));
    });
  });
}
