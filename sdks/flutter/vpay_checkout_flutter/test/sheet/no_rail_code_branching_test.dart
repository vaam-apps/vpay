import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// #189's own instruction, checked mechanically rather than trusted:
/// "No rail code branched on anywhere. `if (code == "mtn_momo")` must
/// appear nowhere. A rail the SDK has never heard of still renders from its
/// spec."
///
/// This walks every `.dart` file under `lib/` (generated files excluded —
/// `pigeons/checkout.dart`'s output speaks a platform channel that has
/// nothing to do with rails) and fails if any of them contains a literal
/// rail code, or a structural pattern that reads as "branch on this rail's
/// identity". `rails.dart` itself proves the alternative is possible: it
/// drives every decision off `RailSpec.flow` and `RailField`'s *kind* —
/// values the server declares — never off `RailSpec.code`.
void main() {
  test(
    'MUTATION PROOF: no lib/ source file branches on a rail code literal',
    () {
      final Directory libDir = Directory('${_packageRoot()}/lib');
      expect(libDir.existsSync(), isTrue, reason: 'lib/ must exist to scan it');

      final List<File> dartFiles = libDir
          .listSync(recursive: true)
          .whereType<File>()
          .where((File f) => f.path.endsWith('.dart'))
          .where((File f) => !f.path.endsWith('.g.dart'))
          .toList();
      expect(dartFiles, isNotEmpty);

      // Known rail codes from the issue and the adapters this deployment
      // ships (`mtn_momo`, `orange_money`) — the exact two the issue quotes
      // as the "must appear nowhere" example. A structural check (below)
      // also guards against a *future* code the SDK has never heard of,
      // which a literal-code list alone could never catch.
      const List<String> knownRailCodeLiterals = ['mtn_momo', 'orange_money'];

      final List<String> literalViolations = [];
      final List<String> structuralViolations = [];

      // `.code ==`, `.code ==` with different spacing, `switch` on a
      // `.code`/`code` member, or a `case '...'` inside one — every shape
      // "branch on a rail's identity" could take in this codebase's own
      // style. Deliberately over-inclusive: a false positive here just
      // means a comment needs rewording, which is a far cheaper mistake
      // than a rail code silently reappearing in a branch.
      final RegExp railCodeBranch = RegExp(r'\brail\s*\.\s*code\s*==');
      final RegExp genericCodeBranch = RegExp(
        r'\bcode\s*==\s*['
        r"'"
        r'"'
        r']',
      );
      final RegExp switchOnCode = RegExp(r'switch\s*\(\s*[\w.]*\bcode\b');

      final String root = _packageRoot();
      for (final File file in dartFiles) {
        final String contents = file.readAsStringSync();
        final String relative = file.path.startsWith(root)
            ? file.path.substring(root.length + 1)
            : file.path;

        for (final String code in knownRailCodeLiterals) {
          if (contents.contains("'$code'") || contents.contains('"$code"')) {
            literalViolations.add('$relative contains the literal "$code"');
          }
        }

        if (railCodeBranch.hasMatch(contents)) {
          structuralViolations.add(
            '$relative matches `rail.code ==` — a branch on a rail\'s own '
            'identity',
          );
        }
        if (genericCodeBranch.hasMatch(contents)) {
          structuralViolations.add(
            '$relative matches `code == \'...\'` — a branch on a rail\'s '
            'own identity',
          );
        }
        if (switchOnCode.hasMatch(contents)) {
          structuralViolations.add(
            '$relative matches a `switch` keyed on a `code` member',
          );
        }
      }

      expect(literalViolations, isEmpty, reason: literalViolations.join('\n'));
      expect(
        structuralViolations,
        isEmpty,
        reason: structuralViolations.join('\n'),
      );
    },
  );

  test('this test itself proves it can fail: the same regex catches an '
      'inline fixture standing in for a real branch', () {
    const String simulatedRegression =
        "if (rail.code == 'mtn_momo') { /* ... */ }";
    final RegExp railCodeBranch = RegExp(r'\brail\s*\.\s*code\s*==');
    expect(railCodeBranch.hasMatch(simulatedRegression), isTrue);
  });
}

String _packageRoot() {
  // `test/sheet/` -> package root is two directories up.
  final String here = Directory.current.path;
  return here;
}
