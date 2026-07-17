#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${repo_root}/src-tauri/Cargo.toml"
lock_path="${repo_root}/scripts/mihomo-lock.json"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command is not available on PATH: $1" >&2
    exit 1
  fi
}

version_at_least() {
  local actual="$1"
  local required="$2"
  local actual_major actual_minor actual_patch required_major required_minor required_patch
  IFS=. read -r actual_major actual_minor actual_patch <<<"$actual"
  IFS=. read -r required_major required_minor required_patch <<<"$required"
  (( actual_major > required_major ||
     (actual_major == required_major && actual_minor > required_minor) ||
     (actual_major == required_major && actual_minor == required_minor && actual_patch >= required_patch) ))
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "scripts/build-dist.sh must be run on macOS." >&2
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "An Apple Silicon host is required; found $(uname -m)." >&2
  exit 1
fi

for command in node pnpm cargo rustc curl shasum gzip awk mkdir mktemp chmod xcode-select xcrun codesign hdiutil plutil rmdir; do
  require_command "$command"
done

if ! xcode-select -p >/dev/null 2>&1 || ! xcrun --find clang >/dev/null 2>&1; then
  echo 'Xcode Command Line Tools are required.' >&2
  exit 1
fi

node_version="$(node -p 'process.versions.node')"
if ! version_at_least "$node_version" "22.12.0"; then
  echo "Node.js 22.12.0 or newer is required; found $(node --version)." >&2
  exit 1
fi

pnpm_version="$(pnpm --version)"
if [[ "$pnpm_version" != "11.9.0" ]]; then
  echo "pnpm 11.9.0 is required; found $pnpm_version." >&2
  exit 1
fi

rust_version="$(rustc --version | awk '{ print $2 }')"
if ! version_at_least "$rust_version" "1.97.1"; then
  echo "Rust 1.97.1 or newer is required; found $rust_version." >&2
  exit 1
fi

rust_host="$(rustc -vV | awk '/^host:/ { print $2 }')"
if [[ "$rust_host" != "aarch64-apple-darwin" ]]; then
  echo "The aarch64-apple-darwin Rust host is required; found ${rust_host:-unknown}." >&2
  exit 1
fi

cd "$repo_root"

echo "Fetching the pinned Mihomo executable and verifying both archive and executable hashes..."
bash ./scripts/fetch-mihomo.sh

core_path="${repo_root}/src-tauri/binaries/mihomo-aarch64-apple-darwin"
expected_core_hash="$(node -e 'const fs = require("fs"); const lock = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(lock.platforms["macos-aarch64"].executableSha256);' "$lock_path")"
if [[ ! -x "$core_path" ]]; then
  echo "Pinned Mihomo executable was not staged as an executable at $core_path." >&2
  exit 1
fi
actual_core_hash="$(shasum -a 256 "$core_path" | awk '{ print $1 }')"
if [[ "$actual_core_hash" != "$expected_core_hash" ]]; then
  echo "Staged Mihomo hash mismatch. Expected $expected_core_hash, got $actual_core_hash." >&2
  exit 1
fi

pnpm install --frozen-lockfile
pnpm build
cargo fmt --manifest-path "$manifest" --all -- --check
cargo test --manifest-path "$manifest" --all-targets --all-features
cargo clippy --manifest-path "$manifest" --all-targets --all-features -- -D warnings
cargo check --manifest-path "$manifest" --all-targets --all-features
pnpm tauri build --bundles dmg

shopt -s nullglob
installers=("${repo_root}"/src-tauri/target/release/bundle/dmg/*.dmg)
if (( ${#installers[@]} != 1 )); then
  echo "Expected exactly one DMG installer; found ${#installers[@]}." >&2
  exit 1
fi

mount_dir="$(mktemp -d)"
cleanup_dmg_mount_best_effort() {
  hdiutil detach "$mount_dir" >/dev/null 2>&1 ||
    hdiutil detach -force "$mount_dir" >/dev/null 2>&1 ||
    true
  if [[ -d "$mount_dir" ]]; then
    rmdir "$mount_dir" >/dev/null 2>&1 || true
  fi
}
trap cleanup_dmg_mount_best_effort EXIT

hdiutil attach -readonly -nobrowse -mountpoint "$mount_dir" "${installers[0]}" >/dev/null
app_bundles=("${mount_dir}"/*.app)
if (( ${#app_bundles[@]} != 1 )); then
  echo "Expected exactly one macOS app bundle inside the DMG; found ${#app_bundles[@]}." >&2
  exit 1
fi
app_bundle="${app_bundles[0]}"
info_plist="${app_bundle}/Contents/Info.plist"
if [[ ! -f "$info_plist" ]]; then
  echo "App bundle is missing Contents/Info.plist: $app_bundle" >&2
  exit 1
fi
main_binary="$(plutil -extract CFBundleExecutable raw -o - "$info_plist")"
if [[ "$main_binary" != "voluntary-internet-access" ]]; then
  echo "Unexpected macOS main binary '$main_binary'; expected voluntary-internet-access." >&2
  exit 1
fi

required_bundle_executables=(
  "${app_bundle}/Contents/MacOS/${main_binary}"
  "${app_bundle}/Contents/MacOS/mihomo"
  "${app_bundle}/Contents/Resources/helper-payload/install-helper.sh"
  "${app_bundle}/Contents/Resources/helper-payload/mihomo"
  "${app_bundle}/Contents/Resources/helper-payload/via-helper"
  "${app_bundle}/Contents/Resources/helper-payload/via-recovery"
)
for bundled_executable in "${required_bundle_executables[@]}"; do
  if [[ ! -f "$bundled_executable" || ! -x "$bundled_executable" ]]; then
    echo "Required executable bundle content is missing or not executable: $bundled_executable" >&2
    exit 1
  fi
done

codesign --verify --strict "${app_bundle}/Contents/MacOS/mihomo"
helper_core="${app_bundle}/Contents/Resources/helper-payload/mihomo"
helper_core_hash="$(shasum -a 256 "$helper_core" | awk '{ print $1 }')"
if [[ "$helper_core_hash" != "$expected_core_hash" ]]; then
  echo "Privileged helper Mihomo hash mismatch at $helper_core. Expected $expected_core_hash, got $helper_core_hash." >&2
  exit 1
fi

hdiutil detach "$mount_dir" >/dev/null
if [[ -d "$mount_dir" ]]; then
  rmdir "$mount_dir"
fi
trap - EXIT

echo "Verified installer(s):"
printf '  %s\n' "${installers[@]}"
