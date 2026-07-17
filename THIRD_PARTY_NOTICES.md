# Third-party notices

> Release status: this MVP notice identifies VIA and the separately distributed
> Mihomo core, but it is not yet a complete attribution bundle for every
> transitive Rust and npm dependency linked into the desktop application. Do not
> publicly redistribute a binary until a dependency license inventory and every
> required copyright/license/NOTICE text have been generated from the final
> `Cargo.lock` and `pnpm-lock.yaml`, reviewed, and packaged with the artifact.

## VIA

- Project: [cgengqiang-cmyk/voluntary-internet-access](https://github.com/cgengqiang-cmyk/voluntary-internet-access)
- License: [GNU General Public License v3.0 only](LICENSE), SPDX `GPL-3.0-only`

The VIA source, release source archive, and modifications are provided under GPLv3. Distributors must preserve the license and make the complete corresponding source for the distributed version available as required by GPLv3.

## Mihomo

- Project: [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo)
- Pinned version: `v1.19.28`
- License: [GNU General Public License v3.0](https://github.com/MetaCubeX/mihomo/blob/v1.19.28/LICENSE)
- Corresponding source: <https://github.com/MetaCubeX/mihomo/tree/v1.19.28>
- Release artifacts: <https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.28>

VIA distributes Mihomo as a separate process from the pinned official release artifact. Exact artifact names, official HTTPS release URLs, and pre-bundle archive/executable SHA-256 values are recorded in [`scripts/mihomo-lock.json`](scripts/mihomo-lock.json). On macOS, Tauri applies an ad-hoc code signature to the app-bundle sidecar; VIA verifies its normalized Mach-O content against the pinned pre-sign artifact and verifies the resulting code signature at runtime. No Mihomo proxy logic is modified. The tagged source link above identifies the corresponding source for the pinned executable.

Release packages must retain this notice and the GPLv3 license, and release notes must identify the exact VIA commit and Mihomo version. Downstream distributors remain responsible for satisfying GPLv3 corresponding-source obligations. VIA does not claim affiliation with or endorsement by MetaCubeX or its maintainers.

## Application dependencies

The source tree declares its direct JavaScript dependencies in `package.json`
and Rust dependencies in `src-tauri/Cargo.toml`; exact transitive versions are
locked by `pnpm-lock.yaml` and `src-tauri/Cargo.lock`. React, Tauri, and their
transitive runtime dependencies retain their own licenses and attribution
requirements. The complete generated inventory is an explicit public-release
gate in [`docs/release.md`](docs/release.md), not an implicit claim made by this
development notice.
