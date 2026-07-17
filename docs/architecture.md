# VIA architecture

## Trust boundaries

1. The React WebView is untrusted presentation code. It can call only typed Tauri commands exposed to the `main` window.
2. The Tauri Rust backend owns profile fetching, credential access, sanitization, validation, state transitions, logs, and ordinary system-proxy mode.
3. `via-helper` is installed only when TUN is first enabled. It runs with elevated privileges and accepts only versioned, authenticated, allowlisted IPC messages.
4. Mihomo is a pinned, hash-verified, separate GPLv3 executable. User-mode and TUN-mode copies live in different trust locations.

## Profile and connection transactions

Every transition is serialized. Import and refresh first run a bounded profile
transaction:

1. Download or read the profile into staging.
2. Sanitize it and download HTTPS providers as the ordinary user.
3. Run `mihomo -t` with a hard timeout against the staged effective profile.
4. Atomically promote the validated source and effective profile to active and
   last-valid storage. A failed update leaves the prior last-valid profile intact.

Connecting is a separate network transaction:

1. Rebuild a managed runtime profile from the active/last-valid data.
2. Start Mihomo on free loopback ports with a random controller secret.
3. Prove readiness with `/version` and a listener probe.
4. Atomically write a dirty network ownership lease containing the baseline and VIA-owned target.
5. Apply system proxy or request TUN activation from the helper.
6. If any later step fails, stop the owned process and conditionally restore the lease.

Unexpected process exit, lost helper heartbeat, explicit exit, startup recovery, the in-app “移除运行组件” action, and the Windows NSIS uninstall hook all use the same idempotent restoration path. Restoration never requires a working Mihomo controller. macOS drag-to-trash deletion cannot execute this elevated cleanup, so the in-app action must complete before the app bundle is removed.

## Configuration policy

The application preserves only proxy definitions, proxy groups, rules, sub-rules, and separately validated provider content. VIA generates all listeners, ports, controller settings, TUN, DNS, paths, log level, and LAN policy. Provider URLs are fetched outside the privileged Mihomo process so every redirect can be required to remain HTTPS. Before TUN elevation, controlled YAML providers and text rule providers are validated and converted to inline payloads; binary MRS providers are rejected by helper protocol v1 because they cannot be safely represented inline.

## Platform layout

- User settings: non-secret preferences only. A fresh controller secret is written to the managed runtime profile for each launch so it never appears in process arguments; it is excluded from diagnostics and logs.
- User data: sanitized active and last-valid profiles plus the recovery lease.
- User cache: subscription/provider staging.
- User logs: redacted, rotated, seven-day retention.
- OS credential store: subscription URL/token.
- Protected helper area: elevated helper, TUN-mode Mihomo, version/hash manifest, and root/Admin-owned runtime files.

Windows uses a local-only Named Pipe and verifies the elevated server PID image before transmitting authenticated requests. macOS uses a root-owned, `_via`-group-restricted Unix Domain Socket with peer credential verification. The Mihomo controller remains a separate random loopback HTTP endpoint because its native socket modes do not authenticate the controller secret.
