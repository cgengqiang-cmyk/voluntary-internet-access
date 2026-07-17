#!/bin/sh
set -eu

ACTION="${1:-install}"
PAYLOAD_ROOT="${2:-}"
CLIENT_USER="${3:-}"
case "$ACTION" in
  install|remove|start|stop|status) ;;
  *) echo "usage: $0 {install|remove|start|stop|status} [payload-root] [client-user]" >&2; exit 2 ;;
esac

if [ "$ACTION" != "status" ] && [ "$(/usr/bin/id -u)" -ne 0 ]; then
  [ -n "$CLIENT_USER" ] || CLIENT_USER=$(/usr/bin/id -un)
  exec /usr/bin/sudo -- "$0" "$ACTION" "$PAYLOAD_ROOT" "$CLIENT_USER"
fi

SCRIPT_DIR=$(/usr/bin/dirname "$0")
REPOSITORY_ROOT=$(cd "$SCRIPT_DIR/.." && /bin/pwd -P)
INSTALL_ROOT="/Library/Application Support/VoluntaryInternetAccess"
CORE_DIR="$INSTALL_ROOT/core"
RUNTIME_DIR="$INSTALL_ROOT/runtime"
HELPER="$INSTALL_ROOT/via-helper"
RECOVERY="$INSTALL_ROOT/via-recovery"
CORE="$CORE_DIR/mihomo"
TOKEN="$INSTALL_ROOT/helper.auth"
SOCKET_DIR="/var/run/voluntary-internet-access"
PLIST="/Library/LaunchDaemons/io.github.cgengqiang-cmyk.voluntary-internet-access.helper.plist"
LABEL="io.github.cgengqiang-cmyk.voluntary-internet-access.helper"
if [ -n "$PAYLOAD_ROOT" ]; then
  PAYLOAD_ROOT=$(cd "$PAYLOAD_ROOT" && /bin/pwd -P)
  BUILT_HELPER="$PAYLOAD_ROOT/via-helper"
  BUILT_RECOVERY="$PAYLOAD_ROOT/via-recovery"
  BUNDLED_CORE="$PAYLOAD_ROOT/mihomo"
else
  BUILT_HELPER="$REPOSITORY_ROOT/src-tauri/target/release/via-helper"
  BUILT_RECOVERY="$REPOSITORY_ROOT/src-tauri/target/release/via-recovery"
  BUNDLED_CORE="$REPOSITORY_ROOT/src-tauri/binaries/mihomo-aarch64-apple-darwin"
fi
EXPECTED_CORE_SHA256="55b7286331cb30a54b2564013b02b84a0c280e8b690bd1e5da4b9d4f4ca007ac"

validate_client_user() {
  case "$CLIENT_USER" in
    ""|root|*[!A-Za-z0-9._-]*)
      echo "invalid non-root client user" >&2
      exit 64
      ;;
  esac
  client_uid=$(/usr/bin/id -u "$CLIENT_USER" 2>/dev/null) || {
    echo "client user does not exist" >&2
    exit 64
  }
  [ "$client_uid" -ne 0 ] || { echo "root cannot be a helper client" >&2; exit 64; }
}

if [ "$ACTION" != "status" ]; then
  validate_client_user
fi

assert_fixed_paths() {
  [ "$INSTALL_ROOT" = "/Library/Application Support/VoluntaryInternetAccess" ] || exit 70
  [ "$PLIST" = "/Library/LaunchDaemons/io.github.cgengqiang-cmyk.voluntary-internet-access.helper.plist" ] || exit 70
}

ensure_group() {
  if ! /usr/bin/dscl . -read /Groups/_via >/dev/null 2>&1; then
    /usr/sbin/dseditgroup -o create -r "VIA privileged helper clients" _via
  else
    existing_members=$(/usr/bin/dscl . -read /Groups/_via GroupMembership 2>/dev/null | /usr/bin/sed 's/^GroupMembership:[[:space:]]*//' || true)
    for existing_member in $existing_members; do
      if [ "$existing_member" != "$CLIENT_USER" ]; then
        /usr/sbin/dseditgroup -o edit -d "$existing_member" -t user _via
      fi
    done
  fi
  /usr/sbin/dseditgroup -o edit -a "$CLIENT_USER" -t user _via
}

write_plist() {
  /usr/bin/printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0"><dict>' \
    '  <key>Label</key><string>io.github.cgengqiang-cmyk.voluntary-internet-access.helper</string>' \
    '  <key>ProgramArguments</key><array>' \
    '    <string>/Library/Application Support/VoluntaryInternetAccess/via-helper</string>' \
    '    <string>serve</string>' \
    '  </array>' \
    '  <key>RunAtLoad</key><true/>' \
    '  <key>KeepAlive</key><true/>' \
    '  <key>ProcessType</key><string>Interactive</string>' \
    '</dict></plist>' > "$PLIST"
  /usr/sbin/chown root:wheel "$PLIST"
  /bin/chmod 0644 "$PLIST"
  /usr/bin/plutil -lint "$PLIST" >/dev/null
}

install_helper() {
  assert_fixed_paths
  [ -n "$CLIENT_USER" ] || { echo "cannot identify the installing user" >&2; exit 1; }
  for source in "$BUILT_HELPER" "$BUILT_RECOVERY" "$BUNDLED_CORE"; do
    [ -f "$source" ] || { echo "missing build artifact: $source" >&2; exit 1; }
  done
  actual=$(/usr/bin/shasum -a 256 "$BUNDLED_CORE" | /usr/bin/awk '{print $1}')
  [ "$actual" = "$EXPECTED_CORE_SHA256" ] || { echo "Mihomo SHA-256 mismatch" >&2; exit 1; }
  expected_helper=$(/usr/bin/shasum -a 256 "$BUILT_HELPER" | /usr/bin/awk '{print $1}')
  expected_recovery=$(/usr/bin/shasum -a 256 "$BUILT_RECOVERY" | /usr/bin/awk '{print $1}')

  ensure_group
  stop_helper
  /bin/mkdir -p "$CORE_DIR" "$RUNTIME_DIR" "$SOCKET_DIR"
  /usr/bin/install -o root -g wheel -m 0755 "$BUILT_HELPER" "$HELPER"
  /usr/bin/install -o root -g _via -m 0750 "$BUILT_RECOVERY" "$RECOVERY"
  /usr/bin/install -o root -g wheel -m 0755 "$BUNDLED_CORE" "$CORE"
  copied_helper=$(/usr/bin/shasum -a 256 "$HELPER" | /usr/bin/awk '{print $1}')
  copied_recovery=$(/usr/bin/shasum -a 256 "$RECOVERY" | /usr/bin/awk '{print $1}')
  [ "$copied_helper" = "$expected_helper" ] || { echo "installed helper SHA-256 mismatch" >&2; exit 1; }
  [ "$copied_recovery" = "$expected_recovery" ] || { echo "installed recovery SHA-256 mismatch" >&2; exit 1; }
  copied=$(/usr/bin/shasum -a 256 "$CORE" | /usr/bin/awk '{print $1}')
  [ "$copied" = "$EXPECTED_CORE_SHA256" ] || { echo "installed Mihomo SHA-256 mismatch" >&2; exit 1; }

  if [ ! -f "$TOKEN" ]; then
    /usr/bin/openssl rand -hex 32 > "$TOKEN"
  fi
  /usr/sbin/chown root:_via "$INSTALL_ROOT" "$RUNTIME_DIR" "$TOKEN" "$SOCKET_DIR"
  /bin/chmod 0750 "$INSTALL_ROOT" "$RUNTIME_DIR" "$SOCKET_DIR"
  /bin/chmod 0640 "$TOKEN"
  write_plist
  /bin/launchctl bootstrap system "$PLIST"
  /bin/launchctl kickstart -k system/$LABEL
  echo "VIA privileged helper installed and started."
}

stop_helper() {
  if /bin/launchctl print system/$LABEL >/dev/null 2>&1; then
    [ -x "$RECOVERY" ] || { echo "cannot stop an installed helper without its recovery tool" >&2; exit 1; }
    /usr/bin/sudo -u "$CLIENT_USER" -- "$RECOVERY" --tun-only
    /bin/launchctl bootout system/$LABEL
  fi
}

remove_helper() {
  assert_fixed_paths
  if /bin/launchctl print system/$LABEL >/dev/null 2>&1; then
    ensure_group
  fi
  stop_helper
  /bin/rm -f "$PLIST"
  if [ -e "$INSTALL_ROOT" ]; then
    resolved=$(cd "$INSTALL_ROOT" && /bin/pwd -P)
    [ "$resolved" = "$INSTALL_ROOT" ] || { echo "refusing to remove unexpected path: $resolved" >&2; exit 70; }
    /bin/rm -rf -- "$resolved"
  fi
  if [ -e "$SOCKET_DIR" ]; then
    resolved_socket=$(cd "$SOCKET_DIR" && /bin/pwd -P)
    [ "$resolved_socket" = "$SOCKET_DIR" ] || { echo "refusing to remove unexpected socket path" >&2; exit 70; }
    /bin/rm -rf -- "$resolved_socket"
  fi
  if /usr/bin/dscl . -read /Groups/_via >/dev/null 2>&1; then
    /usr/sbin/dseditgroup -o delete _via
  fi
  echo "VIA privileged helper and its client group were removed."
}

status_helper() {
  if /bin/launchctl print system/$LABEL >/dev/null 2>&1; then
    echo "helper is loaded"
  else
    echo "helper is not loaded"
  fi
}

case "$ACTION" in
  install) install_helper ;;
  remove) remove_helper ;;
  start) /bin/launchctl bootstrap system "$PLIST" 2>/dev/null || true; /bin/launchctl kickstart -k system/$LABEL ;;
  stop) stop_helper ;;
  status) status_helper ;;
esac
