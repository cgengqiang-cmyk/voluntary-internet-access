# Security policy

VIA changes operating-system proxy and routing state. Please do not publish subscription URLs, node credentials, controller secrets, unreviewed diagnostics, or proof-of-concept details for a privilege-boundary issue in a public GitHub issue, pull request, discussion, or chat log.

Use the repository's **Security → Report a vulnerability** private-reporting flow when it is enabled. If private reporting is unavailable, do not post secret material; contact the repository owner through the owner's GitHub profile with a non-sensitive request to establish a private channel. A public binary release is blocked until a verified private reporting route is enabled. Include the VIA version/commit, operating-system version, whether system-proxy or TUN mode was active, reproduction steps, expected/actual result, and a manually reviewed redacted log bundle. Never attach a live subscription or node configuration.

This MVP has no guaranteed security-support window and has not received an independent security audit. It does not collect telemetry or crash reports automatically. Public releases must not be described as production-ready until the physical acceptance gates in [`docs/release.md`](docs/release.md) pass.

## Security invariants

- WebView code cannot launch arbitrary commands or access the filesystem/network directly.
- The Mihomo controller binds to loopback with a fresh random secret stored only in the managed runtime profile; it is never placed in process arguments, logs, or diagnostics.
- Untrusted profiles are size-limited, parsed, sanitized, tested by the pinned Mihomo core, then atomically promoted.
- Subscription and provider fetching requires HTTPS on every redirect, rejects authenticated URLs and non-public destinations, and enforces response-size limits.
- TUN helper commands are allowlisted; the helper never accepts arbitrary executable or configuration paths.
- A durable ownership lease enables fail-open restoration after abnormal termination.
- Subscription credentials and controller secrets never appear in logs or exported diagnostics.

## Known security and validation limits

- Windows packages are unsigned. macOS packages are ad-hoc signed and not notarized. Treat SmartScreen/Gatekeeper warnings as meaningful provenance checks; never disable either protection globally.
- The TUN helper installation/elevation paths and real routing cleanup still require Windows 11 and macOS Apple Silicon physical acceptance. Do not exercise crash/TUN tests over a remote-only connection.
- The macOS system-proxy adapter currently runs `networksetup` from the ordinary desktop process. The MVP requires a local administrator account, and both administrator-password policy behavior and standard-user failure handling remain physical release gates.
- macOS drag-to-trash deletion cannot remove a root-owned LaunchDaemon/helper. Users must run VIA's in-app “移除运行组件” action and approve its administrator prompt before deleting the app; otherwise privileged components can remain installed.
- TUN helper protocol v1 can inline validated YAML providers and text rule providers, but rejects binary MRS payloads and stateful Tailscale nodes. It also rejects all arbitrary local provider paths and key/certificate path fields.
- A production Windows hardening pass should place the privileged Mihomo child in a kill-on-close Job Object. The current design instead relies on a checksummed lease, verified executable path, and heartbeat-based fail-open cleanup.
- The standalone recovery tool and desktop app share a durable lease but not a cross-process mutex. Run the recovery tool only after VIA has fully exited; the NSIS uninstaller enforces that order.

## Report scope

Reports about helper authentication or authorization, path/symlink validation, profile sanitization escapes, subscription SSRF, controller-secret exposure, network-state restoration, updater/supply-chain integrity, or credential/log leakage are security-relevant. Ordinary upstream node availability, censorship behavior, or a provider's privacy policy is outside VIA's security boundary unless VIA itself mishandles the data.
