import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, readFileSync, rmSync, chmodSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";

const repository = resolve(import.meta.dirname, "..");
const tauri = join(repository, "src-tauri");
const payload = join(tauri, "helper-payload");
const windows = process.platform === "win32";
const macos = process.platform === "darwin";

if (!windows && !macos) {
  throw new Error("VIA helper payloads are supported only on Windows and macOS");
}
if (windows && process.arch !== "x64") {
  throw new Error(`Windows helper builds require x64; found ${process.arch}`);
}
if (macos && process.arch !== "arm64") {
  throw new Error(`macOS helper builds require Apple Silicon; found ${process.arch}`);
}

const cargo = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--manifest-path",
    join(tauri, "Cargo.toml"),
    "--bin",
    "via-helper",
    "--bin",
    "via-recovery",
  ],
  { cwd: repository, stdio: "inherit", shell: false },
);
if (cargo.status !== 0) {
  throw new Error(`cargo helper build failed with exit code ${cargo.status ?? "unknown"}`);
}

mkdirSync(payload, { recursive: true });
for (const file of [
  "via-helper.exe",
  "via-recovery.exe",
  "mihomo.exe",
  "install-helper.ps1",
  "via-helper",
  "via-recovery",
  "mihomo",
  "install-helper.sh",
]) {
  rmSync(join(payload, file), { force: true });
}

const lock = JSON.parse(readFileSync(join(repository, "scripts", "mihomo-lock.json"), "utf8"));
const platform = windows ? lock.platforms["windows-x86_64"] : lock.platforms["macos-aarch64"];
const core = join(repository, platform.destination);
const actualHash = createHash("sha256").update(readFileSync(core)).digest("hex");
if (actualHash !== platform.executableSha256) {
  throw new Error(`pinned Mihomo executable is missing or has the wrong SHA-256: ${core}`);
}

for (const pinnedFile of [
  join(repository, "scripts", windows ? "install-helper.ps1" : "install-helper.sh"),
  join(tauri, "src", "helper", "layout.rs"),
]) {
  if (!readFileSync(pinnedFile, "utf8").includes(platform.executableSha256)) {
    throw new Error(`Mihomo executable hash in ${pinnedFile} does not match scripts/mihomo-lock.json`);
  }
}

const suffix = windows ? ".exe" : "";
copyFileSync(join(tauri, "target", "release", `via-helper${suffix}`), join(payload, `via-helper${suffix}`));
copyFileSync(join(tauri, "target", "release", `via-recovery${suffix}`), join(payload, `via-recovery${suffix}`));
copyFileSync(core, join(payload, windows ? "mihomo.exe" : "mihomo"));
copyFileSync(
  join(repository, "scripts", windows ? "install-helper.ps1" : "install-helper.sh"),
  join(payload, windows ? "install-helper.ps1" : "install-helper.sh"),
);

if (macos) {
  for (const file of ["via-helper", "via-recovery", "mihomo", "install-helper.sh"]) {
    chmodSync(join(payload, file), 0o755);
  }
}

console.log(`Staged verified helper payload at ${payload}`);
