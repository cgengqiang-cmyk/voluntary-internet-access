import { invoke } from "@tauri-apps/api/core";
import type {
  AppSnapshot,
  DiagnosticExportResult,
  ProxyMode,
  RepairResult,
  SettingsPatch,
  TransportMode,
} from "./types";

type CommandMap = {
  get_app_snapshot: { args: undefined; result: AppSnapshot };
  set_connection: { args: { enabled: boolean }; result: AppSnapshot };
  set_transport_mode: { args: { mode: TransportMode }; result: AppSnapshot };
  set_proxy_mode: { args: { mode: ProxyMode }; result: AppSnapshot };
  import_subscription: { args: { url: string }; result: AppSnapshot };
  import_profile_file: {
    args: { path: string; contents?: string };
    result: AppSnapshot;
  };
  refresh_profile: { args: undefined; result: AppSnapshot };
  select_proxy: {
    args: { groupId: string; proxyId: string };
    result: AppSnapshot;
  };
  test_proxy_delay: {
    args: { groupId: string; proxyId: string };
    result: AppSnapshot;
  };
  update_settings: { args: { settings: SettingsPatch }; result: AppSnapshot };
  export_diagnostics: { args: undefined; result: DiagnosticExportResult };
  repair_network: { args: undefined; result: RepairResult };
  uninstall_components: { args: undefined; result: RepairResult };
};

type CommandName = keyof CommandMap;

const now = new Date("2026-07-17T02:36:00.000Z").toISOString();

const initialMockSnapshot: AppSnapshot = {
  connection: {
    status: "disconnected",
    transportMode: "system",
    proxyMode: "rule",
    connectedSince: null,
    activeGroupName: "节点选择",
    activeProxyName: "香港 · HKG 01",
    errorMessage: null,
  },
  profile: {
    id: "profile-demo",
    name: "个人订阅",
    sourceKind: "subscription",
    maskedSource: "https://sub.••••••.com/••••••••",
    updatedAt: now,
    lastValidAt: now,
    isValid: true,
    proxyCount: 8,
    ruleCount: 12642,
  },
  proxyGroups: [
    {
      id: "primary",
      name: "节点选择",
      kind: "select",
      selectedProxyId: "hk-01",
      proxies: [
        { id: "hk-01", name: "香港 · HKG 01", protocol: "VLESS", delayMs: 48, delayState: "available" },
        { id: "sg-01", name: "新加坡 · SGP 01", protocol: "VLESS", delayMs: 76, delayState: "available" },
        { id: "jp-01", name: "日本 · NRT 01", protocol: "Trojan", delayMs: 92, delayState: "available" },
        { id: "us-01", name: "美国 · LAX 01", protocol: "VLESS", delayMs: null, delayState: "idle" },
      ],
    },
    {
      id: "streaming",
      name: "流媒体",
      kind: "select",
      selectedProxyId: "sg-media",
      proxies: [
        { id: "sg-media", name: "新加坡 · Media", protocol: "Trojan", delayMs: 83, delayState: "available" },
        { id: "jp-media", name: "日本 · Media", protocol: "VLESS", delayMs: null, delayState: "idle" },
        { id: "us-media", name: "美国 · Media", protocol: "Hysteria2", delayMs: 168, delayState: "available" },
      ],
    },
  ],
  settings: {
    launchOnStartup: false,
    autoConnect: false,
    dnsMode: "fake-ip",
  },
  runtime: {
    coreVersion: "Mihomo v1.19.x",
    appVersion: "VIA 0.1.0",
    localPort: null,
    controllerHealthy: false,
    helperInstalled: false,
    platformLabel: "浏览器预览",
  },
  isMock: true,
};

let mockSnapshot = structuredClone(initialMockSnapshot);

function delay(ms = 360): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function cloneSnapshot(): AppSnapshot {
  return structuredClone(mockSnapshot);
}

function maskedSubscriptionLabel(rawUrl: string): string {
  try {
    const parsed = new URL(rawUrl);
    const parts = parsed.hostname.split(".");
    const suffix = parts.length > 1 ? parts[parts.length - 1] : "site";
    return `${parsed.protocol}//••••••.${suffix}/••••••••`;
  } catch {
    return "已安全保存的订阅";
  }
}

async function mockInvoke<C extends CommandName>(
  command: C,
  args: CommandMap[C]["args"],
): Promise<CommandMap[C]["result"]> {
  await delay(command === "test_proxy_delay" ? 720 : 360);

  switch (command) {
    case "get_app_snapshot":
      return cloneSnapshot() as CommandMap[C]["result"];
    case "set_connection": {
      const enabled = (args as CommandMap["set_connection"]["args"]).enabled;
      if (enabled && !mockSnapshot.profile) {
        throw new Error("请先导入一份有效配置");
      }
      mockSnapshot.connection.status = enabled ? "connected" : "disconnected";
      mockSnapshot.connection.connectedSince = enabled ? new Date().toISOString() : null;
      mockSnapshot.connection.errorMessage = null;
      mockSnapshot.runtime.localPort = enabled ? 17890 : null;
      mockSnapshot.runtime.controllerHealthy = enabled;
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "set_transport_mode": {
      const mode = (args as CommandMap["set_transport_mode"]["args"]).mode;
      mockSnapshot.connection.transportMode = mode;
      if (mode === "tun") mockSnapshot.runtime.helperInstalled = true;
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "set_proxy_mode":
      mockSnapshot.connection.proxyMode = (args as CommandMap["set_proxy_mode"]["args"]).mode;
      return cloneSnapshot() as CommandMap[C]["result"];
    case "import_subscription": {
      const url = (args as CommandMap["import_subscription"]["args"]).url;
      if (!/^https:\/\//i.test(url)) throw new Error("订阅地址必须使用 HTTPS");
      const importTime = new Date().toISOString();
      mockSnapshot.profile = {
        id: "profile-imported",
        name: "我的订阅",
        sourceKind: "subscription",
        maskedSource: maskedSubscriptionLabel(url),
        updatedAt: importTime,
        lastValidAt: importTime,
        isValid: true,
        proxyCount: 8,
        ruleCount: 12642,
      };
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "import_profile_file": {
      const fileArgs = args as CommandMap["import_profile_file"]["args"];
      if (!/\.ya?ml$/i.test(fileArgs.path)) throw new Error("请选择 YAML 配置文件");
      const importTime = new Date().toISOString();
      mockSnapshot.profile = {
        id: "profile-file",
        name: fileArgs.path.replace(/\.ya?ml$/i, ""),
        sourceKind: "file",
        maskedSource: "本地 YAML · 文件名已隐藏",
        updatedAt: importTime,
        lastValidAt: importTime,
        isValid: true,
        proxyCount: 8,
        ruleCount: 12642,
      };
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "refresh_profile":
      if (!mockSnapshot.profile) throw new Error("当前没有可刷新的配置");
      mockSnapshot.profile.updatedAt = new Date().toISOString();
      mockSnapshot.profile.lastValidAt = mockSnapshot.profile.updatedAt;
      mockSnapshot.profile.isValid = true;
      return cloneSnapshot() as CommandMap[C]["result"];
    case "select_proxy": {
      const selection = args as CommandMap["select_proxy"]["args"];
      const group = mockSnapshot.proxyGroups.find((item) => item.id === selection.groupId);
      const proxy = group?.proxies.find((item) => item.id === selection.proxyId);
      if (!group || !proxy) throw new Error("节点已不存在，请刷新配置");
      group.selectedProxyId = proxy.id;
      if (group.id === "primary") mockSnapshot.connection.activeProxyName = proxy.name;
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "test_proxy_delay": {
      const target = args as CommandMap["test_proxy_delay"]["args"];
      const group = mockSnapshot.proxyGroups.find((item) => item.id === target.groupId);
      const proxy = group?.proxies.find((item) => item.id === target.proxyId);
      if (!proxy) throw new Error("测速目标不存在");
      const seed = [...proxy.id].reduce((total, char) => total + char.charCodeAt(0), 0);
      proxy.delayMs = 36 + (seed % 148);
      proxy.delayState = "available";
      return cloneSnapshot() as CommandMap[C]["result"];
    }
    case "update_settings":
      mockSnapshot.settings = {
        ...mockSnapshot.settings,
        ...(args as CommandMap["update_settings"]["args"]).settings,
      };
      return cloneSnapshot() as CommandMap[C]["result"];
    case "export_diagnostics":
      return { path: "桌面/VIA-诊断报告-20260717.zip" } as CommandMap[C]["result"];
    case "repair_network":
      mockSnapshot.connection.status = "disconnected";
      mockSnapshot.connection.connectedSince = null;
      mockSnapshot.runtime.controllerHealthy = false;
      mockSnapshot.runtime.localPort = null;
      return { summary: "系统代理与 TUN 状态已恢复" } as CommandMap[C]["result"];
    case "uninstall_components":
      mockSnapshot.runtime.helperInstalled = false;
      mockSnapshot.connection.status = "disconnected";
      return { summary: "辅助服务与运行组件已移除" } as CommandMap[C]["result"];
    default:
      throw new Error(`未知命令：${String(command)}`);
  }
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function call<C extends CommandName>(
  command: C,
  ...payload: CommandMap[C]["args"] extends undefined
    ? [] | [undefined]
    : [CommandMap[C]["args"]]
): Promise<CommandMap[C]["result"]> {
  const args = payload[0] as CommandMap[C]["args"];
  if (!isTauriRuntime()) return mockInvoke(command, args);
  return invoke<CommandMap[C]["result"]>(command, args ?? {});
}

export const viaBridge = {
  getSnapshot: () => call("get_app_snapshot"),
  setConnection: (enabled: boolean) => call("set_connection", { enabled }),
  setTransportMode: (mode: TransportMode) => call("set_transport_mode", { mode }),
  setProxyMode: (mode: ProxyMode) => call("set_proxy_mode", { mode }),
  importSubscription: (url: string) => call("import_subscription", { url }),
  importProfileFile: (path: string, contents?: string) =>
    call("import_profile_file", { path, contents }),
  refreshProfile: () => call("refresh_profile"),
  selectProxy: (groupId: string, proxyId: string) =>
    call("select_proxy", { groupId, proxyId }),
  testProxyDelay: (groupId: string, proxyId: string) =>
    call("test_proxy_delay", { groupId, proxyId }),
  updateSettings: (settings: SettingsPatch) => call("update_settings", { settings }),
  exportDiagnostics: () => call("export_diagnostics"),
  repairNetwork: () => call("repair_network"),
  uninstallComponents: () => call("uninstall_components"),
};
