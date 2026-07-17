# Build and release validation

VIA packages a pinned, separately licensed Mihomo executable. A distributable build is valid only when the pinned archive and the extracted executable both match `scripts/mihomo-lock.json`.

## Supported build hosts

| Target | Required host | Output |
| --- | --- | --- |
| Windows 11 x64 | Windows x64, Rust 1.97.1+ MSVC host, Node.js 22.12+, pnpm 11.9.0 | Per-machine NSIS `.exe` (UAC required) |
| macOS 13+ Apple Silicon | Native `arm64` Mac with a local administrator account, Rust 1.97.1+, Xcode Command Line Tools, Node.js 22.12+, pnpm 11.9.0 | DMG containing an ad-hoc-signed app |

The repository currently has no production code-signing or notarization credentials. Never add a certificate, private key, Apple ID password, or notarization token to the repository.

## Current verification boundary

| Area | Evidence available in this repository/workspace | Still required before public release |
| --- | --- | --- |
| Source validation | The Windows development workspace has passed `pnpm build`, Rust formatting, all-target tests, Clippy, and `cargo check`. The main debug executable has also been launched for a UI render check. | Re-run the complete matrix on the final candidate commit; a prior local result is not evidence for a changed commit. |
| Package assembly | The build scripts and CI assemble the pinned core, release helper payload, NSIS package, and ad-hoc DMG. | Install the exact candidate artifacts and complete the physical acceptance gates below. A successful bundle build is not an installation or network test. |
| Windows behavior | Network ownership/restore and helper protocol paths have automated coverage. The per-machine NSIS design invokes recovery/helper removal from its uninstall hook. | Windows 11 x64 physical tests for system proxy, abnormal exit, installer/uninstaller UAC, first-use helper installation, TUN, auto-start/auto-connect, repair, and no-residue uninstall. |
| macOS behavior | The system-proxy adapter, helper installer, DMG configuration, and interactive harness are implemented. | This Windows workspace has not validated macOS compilation or behavior. A macOS 13+ Apple Silicon host must test Gatekeeper, Keychain prompts, proxy/TUN/DNS restoration, LaunchDaemon/group behavior, and the required in-app component removal before drag-to-trash deletion. |
| Signing | macOS bundle configuration uses ad-hoc identity `-`. | Windows Authenticode signing, Apple Developer ID signing, and Apple notarization are not configured. |
| Licenses and reporting | VIA, Mihomo, source obligations, and the current incomplete notice status are documented. | Generate and review the complete Rust/npm dependency license and attribution bundle, package all required texts, and enable a verified private vulnerability-reporting route. |

TUN helper protocol v1 accepts only an inline, revalidated configuration. The desktop converts controlled YAML providers and text rule providers to inline payloads before elevation. Binary MRS provider payloads cannot be safely inlined in v1 and are rejected in TUN mode. Stateful Tailscale nodes are also rejected at both sanitizer layers. Arbitrary subscription-supplied local provider paths are rejected in every mode.

## Reproducible local validation

From a clean checkout on Windows:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\build-dist.ps1
```

The script checks the Windows/Rust architecture, downloads Mihomo from the pinned official HTTPS release URL, verifies the archive and executable hashes, installs exactly the locked frontend dependencies, runs frontend and Rust validation, builds and stages the release helper/recovery payload, and builds an NSIS installer under `src-tauri\target\release\bundle\nsis`.

From a native Apple Silicon Mac:

```bash
bash scripts/build-dist.sh
```

The script refuses Intel/Rosetta hosts, performs the same pinned-core, helper-payload, and locked-dependency checks, and builds an ad-hoc-signed DMG under `src-tauri/target/release/bundle/dmg`.

Both build scripts are fail-fast. They modify only normal build outputs, dependency caches, and the ignored staged Mihomo binary. They do not publish, sign with an identity, install the app, or change network settings.

The macOS app-bundle sidecar is re-signed ad-hoc by Tauri, so its post-bundle byte hash is expected to differ from the official artifact. The build verifies the official pre-bundle hash, the privileged helper payload retains that exact hash, and the desktop verifies normalized Mach-O content plus the app-bundle code signature at runtime.

## CI policy

`.github/workflows/ci.yml` targets Windows Server 2025 x64 and the GitHub-hosted macOS 15 ARM64 image, and explicitly fails if the runner architecture differs. It uses read-only repository permissions and pins every third-party Action to an exact commit. Each platform job invokes its complete `build-dist` script, which performs:

1. pinned Mihomo download and two-level SHA-256 verification;
2. `pnpm install --frozen-lockfile` and `pnpm build`;
3. Rust formatting, tests, Clippy, and `cargo check`;
4. release helper/recovery staging through Tauri's `beforeBuildCommand`, a complete NSIS or DMG bundle build, and platform-specific post-bundle assertions (including the NSIS uninstall hook/payload or macOS sidecar signature/helper core hash).

CI compiles and asserts complete installers, then discards them. It never uploads installer artifacts or creates a GitHub Release. There is intentionally no tag-triggered publication workflow; reviewed local candidate artifacts are the only binaries produced before the physical, licensing, and reporting gates pass.

CI compilation is not physical network validation. The Windows runner is Windows Server rather than Windows 11, and hosted runners cannot approve or exercise interactive privileged helper installation, crash recovery, real system proxy changes, TUN routes, Gatekeeper, or user keychain prompts.

## Physical acceptance gates

Before distributing any build:

- On a real Windows 11 x64 machine, launch the per-machine NSIS installer as a standard user and verify the expected UAC prompt. Validate system proxy connection, forced-exit recovery, first-use TUN elevation, auto-start/auto-connect, and “修复网络”. Finally run the NSIS uninstaller while VIA owns a test connection, approve UAC, and confirm that its recovery/helper-removal hook leaves no residual proxy, DNS, routes, scheduled task, helper process, or `C:\ProgramData\VoluntaryInternetAccess` directory.
- On a macOS 13-or-newer Apple Silicon machine, build the exact candidate commit, mount the DMG, copy the app to `/Applications`, and run the interactive harness:

  ```bash
  bash scripts/test-macos.sh "/Applications/我自愿开启国际互联网访问.app"
  ```

  The harness captures a private baseline, reports the ad-hoc signature/Gatekeeper status, guides launch and forced-exit recovery, inspects the privileged helper prerequisites, and fails if the final observable proxy or DNS state differs. It never calls `sudo` or silently removes quarantine attributes. `VIA_HELPER_PATH` and `VIA_HELPER_LABEL` can override the expected helper identifiers if the packaged helper contract changes.
- On macOS, test `networksetup` mutation and restoration from both a local administrator account and a standard account. The current MVP supports only the administrator-account path; the standard-account path must fail without leaving a dirty lease or partial proxy change. Test both administrator-password policy states.
- On a fresh macOS user that has never belonged to `_via`, install the helper and attempt the first connection immediately. If the current login session cannot see the new group, verify the UI gives the explicit log-out/log-in instruction, then confirm TUN succeeds after a new login. Uninstall must remove the helper-owned `_via` group and its membership.
- On macOS, after the harness passes, open VIA settings and click “移除运行组件”. Approve the explicit administrator prompt, verify success, quit VIA, and only then drag the app to Trash. Drag-to-trash deletion cannot elevate and therefore cannot remove the root LaunchDaemon/helper by itself. Confirm the helper process, LaunchDaemon, protected helper directory, proxy, DNS, and routes are absent before accepting the uninstall gate.
- Import both a subscription URL and a local YAML profile. Confirm invalid/oversized/unsafe profiles are rejected without replacing the last valid profile.
- Confirm rule/global/direct switching, proxy-group selection, delay tests, tray lifecycle, single-instance behavior, redacted diagnostics, and cleanup of app-owned components.
- Run the checks once with no pre-existing proxy/VPN and once with a pre-existing manual proxy baseline. VIA must restore only state it owns and must not erase an unrelated baseline.

## Unsigned and ad-hoc caveats

The Windows installer is unsigned, so SmartScreen may warn. The macOS app uses ad-hoc signing (`-`) and is not notarized, so Gatekeeper may reject a downloaded DMG. Testers must verify the source/commit and checksum before using Finder’s explicit **Open** action. Do not instruct users to globally disable Gatekeeper.

Generate checksums beside reviewed installers:

```powershell
Get-FileHash -Algorithm SHA256 .\path\to\installer.exe
```

```bash
shasum -a 256 /path/to/installer.dmg
```

Record the source commit, Mihomo version, installer SHA-256, physical test OS versions, and acceptance results in release notes. A public GitHub Release should be created manually only after both physical-platform gates pass, a private vulnerability-reporting route is verified, and the complete Rust/npm dependency license inventory has been reviewed. Attach installers, checksum files, the GPLv3 license, complete third-party attribution bundle, and a source archive or durable link to the exact VIA and Mihomo sources; never attach credentials or local diagnostics.
