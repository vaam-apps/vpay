import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// D6: "No `print`, no `debugPrint`, no `log` anywhere in `lib/`" — the same
/// technique `sdks/stripe-js/src/client.test.ts`'s
/// "has no console call anywhere in the shipping source" uses: read the
/// package's own shipping source rather than trust a review to catch a
/// `print(url)` left in from debugging, which would put a `client_secret`
/// (it rides in every request's query string) into a device's logcat, an
/// Xcode console or a browser devtools panel.
void main() {
  test('no print, debugPrint or log call anywhere in lib/', () {
    final libDir = Directory('lib');
    expect(libDir.existsSync(), isTrue, reason: 'lib/ must exist to scan it');

    final offenders = <String>[];
    for (final entity in libDir.listSync(recursive: true)) {
      if (entity is! File || !entity.path.endsWith('.dart')) {
        continue;
      }
      final source = entity.readAsStringSync();
      // Comments stripped first, the same way stripe-js's test does it: this
      // very file's own prose mentions `print(` and a match there would be
      // a false positive someone "fixes" by weakening the pattern instead
      // of by reading it.
      final code = source
          .replaceAll(RegExp(r'/\*[\s\S]*?\*/'), '')
          .replaceAll(RegExp(r'(^|[^:])//.*$', multiLine: true), r'$1');

      // The lookbehind excludes a longer identifier (`dialog(`, `sprint(`)
      // but deliberately **not** a member expression. Until 2026-09-14 `.`
      // was in this character class, which meant the one spelling a
      // developer actually reaches for — `import 'dart:developer' as
      // developer; developer.log(url)`, or `foundation.debugPrint(url)` —
      // walked straight past this gate. A qualified call puts exactly the
      // same `client_secret` in exactly the same logcat.
      if (RegExp(r'(?<![A-Za-z0-9_])print\s*\(').hasMatch(code) ||
          RegExp(r'(?<![A-Za-z0-9_])debugPrint\s*\(').hasMatch(code) ||
          RegExp(r'(?<![A-Za-z0-9_])log\s*\(').hasMatch(code)) {
        offenders.add(entity.path);
      }
    }

    expect(
      offenders,
      isEmpty,
      reason: 'these files call print/debugPrint/log: $offenders',
    );
  });
}
