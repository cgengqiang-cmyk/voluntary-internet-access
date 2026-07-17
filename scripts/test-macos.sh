#!/usr/bin/env bash
set -euo pipefail

# Interactive, read-only network-state harness. The only state-changing command
# in this script is `open`; proxy/TUN changes are performed explicitly by the
# tester in VIA so their authorization remains visible.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
default_app="${repo_root}/src-tauri/target/release/bundle/macos/我自愿开启国际互联网访问.app"
app_path="${1:-$default_app}"
helper_label="${VIA_HELPER_LABEL:-io.github.cgengqiang-cmyk.voluntary-internet-access.helper}"
helper_path="${VIA_HELPER_PATH:-/Library/Application Support/VoluntaryInternetAccess/via-helper}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required macOS command is unavailable: $1" >&2
    exit 1
  fi
}

pause_for_tester() {
  printf '\n%s\n' "$1"
  printf '完成后按回车继续：'
  read -r _
}

capture_services() {
  networksetup -listallnetworkservices | tail -n +2 | while IFS= read -r service; do
    [[ -z "$service" || "$service" == \** ]] && continue
    printf '\n## %s\n' "$service"
    "$@" "$service" 2>&1 || printf '<unavailable>\n'
  done
}

capture_proxy() {
  local output="$1"
  {
    echo '# scutil --proxy'
    scutil --proxy
    echo '# networksetup configured web proxies'
    capture_services networksetup -getwebproxy
    echo '# networksetup configured secure web proxies'
    capture_services networksetup -getsecurewebproxy
    echo '# networksetup configured SOCKS proxies'
    capture_services networksetup -getsocksfirewallproxy
  } >"$output" 2>&1
}

capture_dns() {
  local output="$1"
  {
    echo '# scutil --dns'
    scutil --dns
    echo '# networksetup configured DNS servers'
    capture_services networksetup -getdnsservers
  } >"$output" 2>&1
}

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This test requires macOS on Apple Silicon; found $(uname -s) $(uname -m)." >&2
  exit 1
fi

for command in sw_vers open codesign spctl scutil networksetup tail cmp diff grep pgrep launchctl stat awk sed mktemp rm rmdir; do
  require_command "$command"
done

mac_major="$(sw_vers -productVersion | awk -F. '{ print $1 }')"
if [[ "$mac_major" -lt 13 ]]; then
  echo "macOS 13 or newer is required; found $(sw_vers -productVersion)." >&2
  exit 1
fi

if [[ ! -d "$app_path" ]]; then
  echo "App bundle not found: $app_path" >&2
  echo "Build it first with: bash scripts/build-dist.sh" >&2
  exit 1
fi

if [[ ! -t 0 ]]; then
  echo 'This is an interactive physical-machine test; run it from Terminal.' >&2
  exit 1
fi

umask 077
temp_dir="$(mktemp -d -t via-macos-test)"
baseline_proxy="${temp_dir}/baseline-proxy.txt"
baseline_dns="${temp_dir}/baseline-dns.txt"
crash_proxy="${temp_dir}/crash-proxy.txt"
final_proxy="${temp_dir}/final-proxy.txt"
final_dns="${temp_dir}/final-dns.txt"
cleanup() {
  rm -f "$baseline_proxy" "$baseline_dns" "$crash_proxy" "$final_proxy" "$final_dns"
  rmdir "$temp_dir" 2>/dev/null || true
}
trap cleanup EXIT
result=0

echo "VIA macOS physical acceptance test"
echo "App: $app_path"
echo "macOS: $(sw_vers -productVersion), architecture: $(uname -m)"
echo
echo 'Safety: do not run the crash/TUN portions over a remote-only connection.'
echo 'Disconnect other VPN/proxy tools first and keep this Terminal open.'
echo 'This build is ad-hoc signed, not notarized. Gatekeeper may reject a downloaded DMG.'
echo 'Use Finder > right-click > Open only if you built or independently verified this source.'
echo 'The script will not remove quarantine attributes and will not call sudo for you.'

echo
echo 'Code-signature report (ad-hoc is expected):'
codesign -dv --verbose=2 "$app_path" 2>&1 || true
if ! codesign --verify --deep --strict "$app_path"; then
  echo 'FAIL: the app bundle does not have a valid self-consistent code signature.' >&2
  result=1
fi
echo
echo 'Gatekeeper assessment (a rejection is expected for an unnotarized build):'
spctl --assess --type execute --verbose=2 "$app_path" 2>&1 || true

capture_proxy "$baseline_proxy"
capture_dns "$baseline_dns"
echo
echo "Baseline captured privately under $temp_dir."

if ! open "$app_path"; then
  echo 'macOS refused to launch the app. Review the Gatekeeper guidance above.' >&2
  exit 1
fi

pause_for_tester '在 VIA 中导入测试配置，选择“系统代理”，手动连接，并确认浏览器可以访问预期站点。'
echo
echo 'Current proxy state while VIA should be connected:'
scutil --proxy
if ! scutil --proxy | grep -Eq '(HTTPEnable|HTTPSEnable|SOCKSEnable) : 1'; then
  echo 'WARNING: no enabled system proxy was detected. Do not count this step as passed.' >&2
  result=1
fi

pause_for_tester '保持系统代理已连接，用“强制退出”结束 VIA；等待至少 10 秒，让故障恢复路径执行。不要手工修改系统网络设置。'
capture_proxy "$crash_proxy"
if cmp -s "$baseline_proxy" "$crash_proxy"; then
  echo 'PASS: system proxy state returned exactly to the baseline after forced exit.'
else
  echo 'WARNING: proxy state differs after forced exit.' >&2
  diff -u "$baseline_proxy" "$crash_proxy" || true
  result=1
  pause_for_tester '重新打开 VIA，运行“修复网络”，再断开连接。若仍无法恢复，请停止测试并保存诊断信息。'
fi

open "$app_path"
pause_for_tester '测试 TUN：选择 TUN，确认系统显示明确的管理员授权，并只在认可 helper 来源后授权。连接后验证浏览器与 DNS；然后回到这里。'

echo
echo 'TUN helper prerequisite/elevation evidence:'
if [[ -e "$helper_path" ]]; then
  stat -f 'owner=%Su group=%Sg mode=%Sp path=%N' "$helper_path"
  helper_owner="$(stat -f '%Su' "$helper_path")"
  if [[ "$helper_owner" != 'root' ]]; then
    echo 'WARNING: privileged helper is not owned by root.' >&2
    result=1
  fi
  helper_mode="$(stat -f '%Lp' "$helper_path")"
  if (( (8#$helper_mode & 8#022) != 0 )); then
    echo "WARNING: privileged helper is group/world writable (mode $helper_mode)." >&2
    result=1
  fi
else
  echo "WARNING: helper not found at $helper_path" >&2
  echo 'If the implementation uses another verified path, rerun with VIA_HELPER_PATH set.' >&2
  result=1
fi

if launchctl print "system/$helper_label" >/dev/null 2>&1; then
  echo "PASS: launchd has system/$helper_label loaded."
else
  echo "WARNING: launchd service system/$helper_label is not loaded." >&2
  echo 'If the implementation uses another label, rerun with VIA_HELPER_LABEL set.' >&2
  result=1
fi

if ! pgrep -fl 'via-helper'; then
  echo 'WARNING: no via-helper process was visible while TUN should be connected.' >&2
  result=1
fi
if ! pgrep -fl 'mihomo'; then
  echo 'WARNING: no Mihomo process was visible while TUN should be connected.' >&2
  result=1
fi
echo
echo 'DNS state while TUN should be connected:'
scutil --dns | sed -n '1,120p'

pause_for_tester '在 VIA 中断开 TUN，退出应用，并等待至少 10 秒。不要手工修改代理或 DNS。'
capture_proxy "$final_proxy"
capture_dns "$final_dns"

if cmp -s "$baseline_proxy" "$final_proxy"; then
  echo 'PASS: no residual system proxy configuration was detected.'
else
  echo 'FAIL: final proxy state differs from the baseline.' >&2
  diff -u "$baseline_proxy" "$final_proxy" || true
  result=1
fi

if cmp -s "$baseline_dns" "$final_dns"; then
  echo 'PASS: no residual DNS configuration was detected.'
else
  echo 'FAIL: final DNS state differs from the baseline.' >&2
  echo 'A DHCP/network-service change can also cause this; review the diff before filing a bug.' >&2
  diff -u "$baseline_dns" "$final_dns" || true
  result=1
fi

if [[ "$result" -ne 0 ]]; then
  echo 'One or more acceptance gates failed.' >&2
  echo 'If final network state differs, run VIA “修复网络”; if it remains different, restore the baseline manually and attach exported diagnostics.' >&2
  exit "$result"
fi

echo 'All observable macOS cleanup checks passed.'
