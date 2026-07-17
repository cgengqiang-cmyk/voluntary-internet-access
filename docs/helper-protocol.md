# VIA privileged helper protocol

This document describes protocol version 1 used by `via-helper`. The helper is
only required for TUN mode. Windows system-proxy mode remains a user-mode
transaction. The macOS adapter currently invokes `networksetup` from the
ordinary desktop process; the MVP therefore requires a local administrator
account and treats authorization behavior as a physical-Mac release gate.

## Trust boundary

The desktop process is treated as unprivileged, even after it proves possession
of the install-time token. It can request an allowlisted operation and can send
sanitized YAML contents, but it cannot choose any of the following:

- helper, Mihomo, configuration, lease, socket, or log paths;
- an executable, command, working directory, service, device name, or PID;
- a listening address exposed beyond loopback;
- a local file provider or a proxy field that references a key/certificate or
  another filesystem path.

The helper uses these fixed locations:

| Item | Windows 11 x64 | macOS 13+ arm64 |
| --- | --- | --- |
| Root | `C:\ProgramData\VoluntaryInternetAccess` | `/Library/Application Support/VoluntaryInternetAccess` |
| Core | `core\mihomo.exe` | `core/mihomo` |
| TUN config | `runtime\tun.yaml` | `runtime/tun.yaml` |
| Token | `helper.auth` | `helper.auth` |
| Lease | `runtime\helper-lease.json` | `runtime/helper-lease.json` |
| IPC | `\\.\pipe\via-helper-v1` | `/var/run/voluntary-internet-access/helper-v1.sock` |

Before every launch, the helper rejects symlinks, canonicalizes the exact fixed
files, hashes Mihomo, parses the configuration again, and runs `mihomo -t`.
The pinned Mihomo executable hashes correspond to v1.19.28.

## Installation and authentication

`scripts/install-helper.ps1` installs an elevated, per-user Scheduled Task on
Windows. `scripts/install-helper.sh` installs a root LaunchDaemon and a dedicated
`_via` client group on macOS. Both scripts:

1. copy only the fixed helper, recovery, and core filenames from the staged build payload;
2. verify the pinned Mihomo SHA-256 before and after copying;
3. generate a 32-byte random token once;
4. restrict the token to the installing user/administrators on Windows or
   `root:_via` on macOS;
5. place the executable and core under an administrator-owned directory.

The token is 64 lowercase hexadecimal characters. It is included in every
request and compared without an early byte mismatch. The transport is local;
the token is never returned in a response or written to helper logs.

## Framing and schema

Each connection carries exactly one request and one response. A frame is a
four-byte unsigned big-endian JSON length followed by that many UTF-8 bytes.
The maximum frame is the 2 MiB configuration limit plus 128 KiB of protocol
overhead. Unknown JSON fields and unknown operations are rejected.

Common request fields:

```json
{
  "version": 1,
  "request_id": "e85a9c76-9f19-4a61-b28d-c465e842a3f3",
  "auth_token": "<64 lowercase hex characters>",
  "operation": "status"
}
```

Responses repeat `version` and `request_id`, include an `ok` boolean, and use a
tagged `result`. Errors contain a stable code and a redacted message.

## Allowlisted operations

| Operation | Additional request data | Effect |
| --- | --- | --- |
| `status` | none | Reports install/running state without returning paths or credentials. |
| `install` | `config_yaml`, `config_sha256` | Validates and atomically stages YAML at the fixed TUN config path. It does **not** install a service or executable. |
| `remove` | `confirm: true` | Stops TUN and deletes only the fixed staged config. The platform script removes binaries and service registration. |
| `start_tun` | `session_id`, config hash, controller secret, heartbeat timeout | Confirms the secret matches the fixed managed config, then starts only the pinned core without placing the secret in process arguments. Timeout must be 5-60 seconds. |
| `heartbeat` | active `session_id` | Renews the dirty helper lease. |
| `stop_tun` | active `session_id` | Stops only the child owned by the active session and clears its lease. |
| `restore` | optional `session_id` | Fail-open recovery. With no session it is the authenticated standalone-recovery path. |

`start_tun` writes and flushes a checksummed dirty lease before reporting
success. The helper checks the child and heartbeat every second. A dead child or
expired heartbeat stops TUN and clears the lease. On helper restart, a stale PID
is terminated only after its executable path is proven to be the fixed pinned
core; PID reuse can therefore never kill an unrelated process.

The desktop should heartbeat every 2 seconds with a 10-15 second timeout. It
must treat a rejected heartbeat as disconnected and invoke recovery.

## Standalone recovery

`via-recovery` starts no Tauri runtime or WebView. It computes VIA's fixed
per-user lease path, reuses the conditional `ProxyTransaction` recovery, and
then sends an authenticated `restore` to the helper. Its only options are:

```text
via-recovery [--json] [--proxy-only | --tun-only]
```

The proxy transaction compares each tracked field with VIA's recorded owned
value. Fields that VIA still owns are restored to baseline; fields changed by
another application or the user are preserved. A changed macOS service set or
service enabled state is treated as a recovery error instead of being guessed.

## Current validation limits

- The helper itself rejects every `file` provider. Before elevation, the desktop
  resolves only app-controlled provider cache paths, rejects symlinks and path
  escapes, validates YAML providers or text rule providers, and converts them to
  inline payloads.
  Binary MRS providers cannot be represented by this protocol and are rejected
  in TUN mode. User-supplied local provider paths are never passed through.
- The Windows Scheduled Task and macOS LaunchDaemon installers are implemented
  but have not been physically exercised in this workspace. macOS ad-hoc
  distribution, LaunchDaemon approval, `_via` group behavior, and TUN routing
  require the repository's physical Mac acceptance script. A fresh user must
  be tested from the first `_via` group creation through the first helper
  connection; if macOS does not refresh the running login session's group
  membership, the UI instructs the user to log out and back in before retrying.
- The macOS system-proxy adapter implements capture, conditional mutation, and
  restoration for HTTP, HTTPS, and PAC state, while refusing baselines it cannot
  safely reproduce. It still requires a local administrator account and
  physical macOS acceptance with both a clean network setup and a pre-existing
  manual proxy baseline.
- A production Windows hardening pass should place the child in a kill-on-close
  Job Object. The restart path already cleans a stale child using a checksummed
  lease and verified executable path, while the heartbeat handles ordinary app
  crashes.
