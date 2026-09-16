# Embedded host integrations

Each section is the shape of the integration and the commands a developer
actually runs. Verified against the example projects in the framework repo.

## Flutter

`examples/embedded-flutter`. A Rust `cdylib` crate under `native/`, bridged with
`flutter_rust_bridge` 2.x. The generated Dart (`lib/src/rust/`) and the platform
scaffolds (`android/`, `ios/`, `macos/`, …) are **deliberately not committed**.

```bash
cd examples/embedded-flutter

flutter create . --org dev.cratestack.examples --platforms=macos,ios,android

flutter_rust_bridge_codegen integrate --rust-crate-name embedded_flutter_native \
  --rust-crate-dir native --no-write-lib --no-integration-test

flutter_rust_bridge_codegen generate

flutter pub get
cargo build -p embedded_flutter_native --manifest-path native/Cargo.toml
```

Day to day: `flutter run -d macos`. In the framework repo, regenerate glue with
`just frb-generate examples/embedded-flutter`.

`flutter_rust_bridge.yaml`:

```yaml
rust_input: crate::api
rust_root: native/
dart_output: lib/src/rust/
rust_output: native/src/frb_generated.rs
```

Things that bite:

- **Flatten types at the FFI boundary.** `String`, `Vec<T>`, primitives and plain
  `#[derive(Clone)]` structs cross cleanly. `Uuid` and `DateTime<Utc>` must
  become strings or ints.
- **`mod frb_generated;` must come after inner attributes** in `native/src/lib.rs`.
  The codegen injects it at the very top, which Rust rejects.
- The crate name uses **underscores** (`embedded_flutter_native`) because
  cargokit reads it verbatim for the `lib<name>.{a,dylib}` lookup. That is a
  separate constraint from the workspace exclusion — see the SKILL for why the
  exclusion exists.
- Wrap the schema in its own module so `cratestack_schema` does not bleed into
  the crate root.
- Verified in the repo: macOS end to end; Android APK build only; **iOS untested
  and explicitly out of scope**.

**`cratestack-client-flutter` is not this.** It is the HTTP client wrapper, and
its `mod frb_generated;` sits behind an off-by-default `frb-glue` feature — which
is why it stays a normal workspace member. Copy that pattern for your own crate.

**`examples/flutter-riverpod` is also not this.** It is the generated-Dart-HTTP-
client example running against a Postgres server; it contains no
`include_embedded_schema!` and no `RusqliteRuntime`. See `cratestack-clients`.

## React Native (Expo)

`examples/embedded-expo`. A Rust `cdylib` exporting a C ABI, plus JNI shims for
Android, consumed through a **local Expo native module**.

The Rust side exports three symbols — `cratestack_init`, `cratestack_dispatch`,
`cratestack_free` — over a shared JSON envelope
(`cratestack_rusqlite::ffi::{OperationRequest, OperationResponse}`). The module
exposes two Expo Functions, `initDatabase(path)` and `dispatch(requestJson)`,
which `index.ts` wraps into `listNotes`, `createNote`, `updateNote`,
`deleteNote`, `findNote`.

**What is checked in vs generated is the thing people get wrong:**

- **Checked in, hand-completed:** `app/modules/cratestack-notes/index.ts`, the
  Swift module and podspec, the Kotlin module and `build.gradle`. These were
  originally scaffolded by `create-expo-module` and then hand-finished with the
  FFI bridge. **Do not re-run `npx create-expo-module` against that directory** —
  it overwrites the bridge.
- **Generated and gitignored:** only the app-level `app/ios/` and `app/android/`,
  produced by `npx expo prebuild`.

```bash
cd examples/embedded-expo
cargo build -p embedded-expo-native --target aarch64-apple-ios-sim --release
cargo ndk --target arm64-v8a --target armeabi-v7a --target x86 --target x86_64 \
    -o app/modules/cratestack-notes/android/src/main/jniLibs build --release -p embedded-expo-native
cd app && pnpm install && npx expo prebuild && npx expo run:android
```

Node 24+ required. Verified: Android end to end; iOS untested.

## Tauri

Two shapes, and the choice is real.

| | `tauri-web` | `tauri-native` |
| --- | --- | --- |
| Local data | wasm32 in the webview, OPFS-backed | native Rust in the shell, file-backed |
| Remote | `include_client_schema!` in the shell | same |
| Renderer | worker holding the wasm runtime | plain TS, IPC only |
| Crates | two (wasm cdylib + `src-tauri`) | one (`src-tauri`, both macros) |

**Pick `tauri-native`** for OS-native SQLite with full filesystem semantics, to
share the database file with non-webview code, or to keep the wasm toolchain out
of the build entirely. The tradeoff is that the renderer then always needs the
shell — it cannot run in a plain browser.

**Pick `tauri-web`** when the renderer should stay portable to a browser, or when
filesystem access must stay sandboxed.

`tauri-native` puts both macros in one crate, each in its own module:

```rust
mod notes_schema {
    use cratestack_macros::include_embedded_schema;
    include_embedded_schema!("notes.cstack");
}

mod articles_schema {
    use cratestack_macros::include_client_schema;
    include_client_schema!("articles.cstack");
}
```

Run from the example root, not from `web/` — the Tauri CLI walks *down* for
`tauri.conf.json`: `pnpm install && pnpm tauri dev`.

## Async Rust hosts

`examples/embedded-webhook` (axum) and `examples/embedded-daemon` (batching
daemon). Both hold `Arc<RusqliteRuntime>` in state and wrap every data call in
`tokio::task::spawn_blocking`. A CLI, an FFI `cdylib`, or wasm-in-a-worker needs
none of this — they are already synchronous hosts.
