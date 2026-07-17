# 我自愿开启国际互联网访问

`VIA` 是一个面向 Windows 11 x64 与 macOS 13+ Apple Silicon 的精简 Mihomo 桌面客户端。它把权限边界、系统网络状态恢复和最小化界面放在首位；macOS 支持目前仍属于必须经过 Apple Silicon 真机验收的候选实现。

> 这是未经独立安全审计的 MVP。Windows 包没有代码签名；macOS 包仅使用 ad-hoc 签名且没有公证。不要把尚未通过真机验收的构建当作生产 VPN，也不要在只能远程连接的机器上测试崩溃恢复或 TUN。

## 首版范围

- 导入一个 HTTPS 订阅 URL 或本地 YAML；更新失败时保留最后一个有效配置。
- 系统代理与可选 TUN，规则/全局/直连模式，代理组选择与手动测速。
- 桌面进程保持普通用户权限；Windows 的 per-machine 应用安装以及首次启用 TUN 时的 helper 安装会分别显示 UAC。macOS 的 helper 安装会请求管理员授权，系统代理在当前 MVP 中要求本地管理员账户并仍须真机验证授权行为。
- 订阅秘密保存到 Windows Credential Manager / macOS Keychain。
- 本地脱敏日志、故障开放恢复、网络修复和 helper 卸载路径。
- 不提供节点或订阅服务，不开放局域网代理，不收集遥测，也不静默更新。

TUN helper v1 会把受控缓存中的 YAML Provider 或文本规则 Provider 验证后内联，再把配置交给高权限进程。MRS 是二进制格式，当前不能安全内联，因此 **TUN 模式会拒绝 MRS Provider**；普通系统代理模式不受这项 helper 限制。会创建持久状态目录的 Tailscale 节点以及订阅内直接指定的任意本地文件 Provider 会在所有模式中被拒绝。

## 验证状态

| 层级 | 已验证 | 尚未验证/发版门槛 |
| --- | --- | --- |
| 前端与 Rust | Windows 开发工作区已通过前端构建、格式检查、全部目标测试、Clippy 和 `cargo check` | 每个候选提交仍须由 CI 在 Windows x64 与 macOS arm64 重新执行 |
| Windows 桌面 | Debug 主程序已启动并完成界面渲染检查；网络事务和 helper 协议有自动化测试 | Windows 11 真机上的 NSIS 安装/卸载、真实系统代理、崩溃恢复、UAC、TUN、开机启动与无残留检查 |
| macOS | 已配置 arm64 构建、ad-hoc DMG、系统代理适配器、LaunchDaemon/helper 和交互验收脚本 | 当前 Windows 工作区不能替代 macOS 13+ Apple Silicon 真机构建、`networksetup` 管理员授权、首次 helper 组生效、代理/TUN/DNS 与卸载验收 |
| 发布身份 | 固定 Mihomo 版本和两级 SHA-256 校验 | Windows Authenticode、Apple Developer ID 与 notarization 均未配置 |

“构建成功”只证明包可以组装，不证明它能安全修改并恢复真实网络状态。完整门槛见 [`docs/release.md`](docs/release.md)。

## 安装与卸载边界

- Windows 使用 per-machine NSIS 安装，安装和卸载都会显示 UAC。卸载器会先运行恢复工具，再移除 TUN helper 与应用拥有的运行组件；验收时仍要确认系统代理、DNS、路由、计划任务和 `C:\ProgramData\VoluntaryInternetAccess` 没有残留。
- macOS 使用 DMG 拖放安装。把 `.app` 直接拖入废纸篓 **不能** 获得 root 权限，也不会自动删除 LaunchDaemon/helper。删除应用前，必须先在 VIA 设置中点击“移除运行组件”、确认管理员授权和成功提示，然后退出并删除应用。如果应用已经先被删除，应重新安装并打开同版本应用完成移除，不要盲目手工删除系统目录。

## 开发

需要 Node.js 22.12+、**恰好 pnpm 11.9.0**、Rust 1.97.1+，以及对应平台的 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)。

Windows PowerShell：

```powershell
pnpm install --frozen-lockfile
Set-ExecutionPolicy -Scope Process Bypass
.\scripts\fetch-mihomo.ps1
pnpm tauri dev
```

macOS Apple Silicon：

```bash
pnpm install --frozen-lockfile
bash scripts/fetch-mihomo.sh
pnpm tauri dev
```

只测试普通系统代理时无需安装 helper。若要从开发构建测试 TUN，先运行 `pnpm prepare:tauri` 生成并暂存 helper payload；随后由应用在用户明确授权时调用平台安装脚本。不要手动把开发目录中的任意二进制复制到高权限目录。

## 构建安装包

```powershell
.\scripts\build-dist.ps1
```

```bash
bash scripts/build-dist.sh
```

Windows 输出位于 `src-tauri\target\release\bundle\nsis`，macOS 输出位于 `src-tauri/target/release/bundle/dmg`。脚本会验证固定的 Mihomo 归档与可执行文件哈希、锁定的前端依赖和 Rust 检查，但不会安装、发布、使用生产证书签名或更改网络设置。

架构与信任边界见 [`docs/architecture.md`](docs/architecture.md)，安全报告方式见 [`SECURITY.md`](SECURITY.md)，第三方许可证见 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。

## 许可证

VIA 源码使用 [GNU GPLv3](LICENSE)，SPDX 标识为 `GPL-3.0-only`。随包分发的 Mihomo 来自固定版本的官方可执行文件，代理逻辑未修改；macOS 应用内副本会按平台要求重新做 ad-hoc 代码签名。Mihomo 同样采用 GPLv3；版本、许可证、对应源码和官方发布地址记录在 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)。
