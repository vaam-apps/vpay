/// D6: "`toString()` is overridden on every type that holds a session URL, a
/// session secret or an intent secret, rendering `[N chars redacted]`."
///
/// One function, used by every `toString()` override in this package, so the
/// rendering cannot drift between types — the same reason
/// `sdks/stripe-js/src/client.ts` builds its `[INSPECT_CUSTOM]`/`toJSON`
/// pair from one shared shape rather than one per class.
///
/// `null` is rendered as the literal `null` rather than `[0 chars
/// redacted]`, because "there is no secret here" and "there is a
/// zero-length one" are different facts and a reader of a log line should
/// not have to guess which.
String redacted(String? value) {
  if (value == null) {
    return 'null';
  }
  return '[${value.length} chars redacted]';
}
