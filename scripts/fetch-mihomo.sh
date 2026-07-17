#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="v1.19.28"
url="https://github.com/MetaCubeX/mihomo/releases/download/${version}/mihomo-darwin-arm64-${version}.gz"
expected="40cdae2fab4b18df15f40eaa9dc3af70ab3d8be7f77164ae1e5f1af3a2a4fb44"
expected_executable="55b7286331cb30a54b2564013b02b84a0c280e8b690bd1e5da4b9d4f4ca007ac"
destination="${repo_root}/src-tauri/binaries/mihomo-aarch64-apple-darwin"
archive="$(mktemp -t via-mihomo.XXXXXX.gz)"
trap 'rm -f "$archive"' EXIT

curl --fail --location --proto '=https' --proto-redir '=https' --max-redirs 5 --output "$archive" "$url"
actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
if [[ "$actual" != "$expected" ]]; then
  echo "Mihomo SHA-256 mismatch. Expected $expected, got $actual" >&2
  exit 1
fi

mkdir -p "$(dirname "$destination")"
gzip -dc "$archive" > "$destination"
actual_executable="$(shasum -a 256 "$destination" | awk '{print $1}')"
if [[ "$actual_executable" != "$expected_executable" ]]; then
  rm -f "$destination"
  echo "Mihomo executable SHA-256 mismatch. Expected $expected_executable, got $actual_executable" >&2
  exit 1
fi
chmod 0755 "$destination"
echo "Staged Mihomo $version at $destination"
