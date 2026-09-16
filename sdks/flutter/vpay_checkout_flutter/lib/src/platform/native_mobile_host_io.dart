/// The `dart:io` half of `checkout_platform.dart`'s conditional import —
/// see that file's doc comment on [isNativeMobileHost].
library;

import 'dart:io' show Platform;

/// `true` on the three platforms `MethodChannelVpayCheckoutPlatform`
/// implements (Lane C design doc D5): Android, iOS, macOS.
bool get isNativeMobileHost =>
    Platform.isAndroid || Platform.isIOS || Platform.isMacOS;
