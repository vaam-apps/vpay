#!/usr/bin/env bash
# Builds the sealing module the Node SDK loads: `sdks/rust/wasm`, compiled to
# WebAssembly and wrapped for Node by `wasm-bindgen --target nodejs`.
#
#   scripts/build-wasm.sh              -> sdks/nodejs/wasm       (ships in the npm package)
#   scripts/build-wasm.sh --responder  -> sdks/nodejs/wasm-test  (the server half, for vitest only)
#
# Both are build outputs and gitignored. `wasm-bindgen`'s command-line tool must
# be exactly the version of the `wasm-bindgen` crate in Cargo.lock: the two
# agree on a private interface, and a mismatch fails at load time with an error
# that names neither (`just wasm-build` installs nothing; CI installs the
# version this script prints when it disagrees).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
cd "$root"

features=()
out="$root/sdks/nodejs/wasm"
if [ "${1:-}" = "--responder" ]; then
	features=(--features responder)
	out="$root/sdks/nodejs/wasm-test"
fi

wanted="$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/version = |"/, ""); print; exit}' Cargo.lock)"
if ! command -v wasm-bindgen >/dev/null 2>&1; then
	echo "build-wasm: FAIL — wasm-bindgen is not on PATH." >&2
	echo "build-wasm: install the version Cargo.lock pins: cargo install wasm-bindgen-cli --locked --version $wanted" >&2
	exit 1
fi
found="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$found" != "$wanted" ]; then
	echo "build-wasm: FAIL — wasm-bindgen $found is installed, Cargo.lock pins the crate at $wanted." >&2
	echo "build-wasm: cargo install wasm-bindgen-cli --locked --version $wanted --force" >&2
	exit 1
fi
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# Size over speed: this is a signing call, not a hot loop, and the module is
# embedded in the glue and parsed on every `require`.
CARGO_PROFILE_RELEASE_OPT_LEVEL=s \
	cargo build --release --locked --target wasm32-unknown-unknown -p vpay-sdk-wasm "${features[@]}"

rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target nodejs --out-dir "$out" --out-name vpay_sdk_wasm \
	"${CARGO_TARGET_DIR:-$root/target}/wasm32-unknown-unknown/release/vpay_sdk_wasm.wasm"
# wasm-bindgen's Node glue reads its `.wasm` from `${__dirname}` at load time.
# A bundler that inlines the glue (Next's Turbopack does, for a workspace
# package) rewrites `__dirname` to a build-time path that does not exist where
# the bundle runs, and every call to vpay then fails with ENOENT. So the module
# carries its own bytes: the read becomes a base64 literal, and the glue needs
# no file, no path and no `fs` wherever it ends up. Each of the two lines must
# be found exactly once, so a wasm-bindgen that words them differently fails
# this build rather than shipping a module that still reads from disk.
glue="$out/vpay_sdk_wasm.js"
b64="$out/.wasm.b64"
base64 <"$out/vpay_sdk_wasm_bg.wasm" | tr -d '\n' >"$b64"
awk -v b64file="$b64" '
	$0 == "const wasmPath = `${__dirname}/vpay_sdk_wasm_bg.wasm`;" { path++; next }
	$0 == "const wasmBytes = require('"'"'fs'"'"').readFileSync(wasmPath);" {
		bytes++
		getline literal <b64file
		print "const wasmBytes = Buffer.from(\"" literal "\", \"base64\");"
		next
	}
	{ print }
	END { if (path != 1 || bytes != 1) exit 1 }
' "$glue" >"$glue.embedded" || {
	echo "build-wasm: FAIL — wasm-bindgen's glue no longer reads its module the way this script rewrites." >&2
	exit 1
}
mv "$glue.embedded" "$glue"
rm -f "$b64"
# The SDK is ESM ("type": "module"); wasm-bindgen emits CommonJS. A nested
# package.json is what tells Node which one this directory is.
printf '{ "type": "commonjs" }\n' >"$out/package.json"
echo "build-wasm: wrote $out"
