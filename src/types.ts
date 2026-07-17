export type ViewId = "home" | "proxies" | "profiles" | "settings";

export type ConnectionStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "disconnecting"
  | "error";

export type TransportMode = "system" | "tun";
export type ProxyMode = "rule" | "global" | "direct";
export type DnsMode = "fake-ip" | "redir-host";

export interface ConnectionSnapshot {
  status: ConnectionStatus;
  transportMode: TransportMode;
  proxyMode: ProxyMode;
  connectedSince: string | null;
  activeGroupName: string | null;
  activeProxyName: string | null;
  errorMessage: string | null;
}

export interface ProfileSummary {
  id: string;
  name: string;
  sourceKind: "subscription" | "file";
  maskedSource: string;
  updatedAt: string;
  lastValidAt: string;
  isValid: boolean;
  proxyCount: number;
  ruleCount: number;
}

export type DelayState = "idle" | "testing" | "available" | "timeout";

export interface ProxyNode {
  id: string;
  name: string;
  protocol: string;
  delayMs: number | null;
  delayState: DelayState;
}

export interface ProxyGroup {
  id: string;
  name: string;
  kind: "select" | "url-test" | "fallback";
  selectedProxyId: string;
  proxies: ProxyNode[];
}

export interface AppSettings {
  launchOnStartup: boolean;
  autoConnect: boolean;
  dnsMode: DnsMode;
}

export interface RuntimeSummary {
  coreVersion: string;
  appVersion: string;
  localPort: number | null;
  controllerHealthy: boolean;
  helperInstalled: boolean;
  platformLabel: string;
}

export interface AppSnapshot {
  connection: ConnectionSnapshot;
  profile: ProfileSummary | null;
  proxyGroups: ProxyGroup[];
  settings: AppSettings;
  runtime: RuntimeSummary;
  isMock: boolean;
}

export interface SettingsPatch {
  launchOnStartup?: boolean;
  autoConnect?: boolean;
  dnsMode?: DnsMode;
}

export interface DiagnosticExportResult {
  path: string;
}

export interface RepairResult {
  summary: string;
}
