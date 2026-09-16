/// The web/no-`dart:io` half of `checkout_platform.dart`'s conditional
/// import — see that file's doc comment on [isNativeMobileHost] for why
/// this needs a conditional import at all rather than a `kIsWeb` runtime
/// check: `dart:io` is not merely inert on web, it does not exist there,
/// so a library that is compiled into the web bundle must never even
/// import it.
library;

/// Always `false` here: there is no Android/iOS/macOS to be one of on a
/// platform with no `dart:io`.
bool get isNativeMobileHost => false;
