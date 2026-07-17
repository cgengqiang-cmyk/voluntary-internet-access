import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { viaBridge } from "./bridge";
import type {
  AppSettings,
  AppSnapshot,
  DnsMode,
  ProxyGroup,
  ProxyMode,
  SettingsPatch,
  TransportMode,
  ViewId,
} from "./types";
import "./App.css";

type IconName =
  | "home"
  | "nodes"
  | "profile"
  | "settings"
  | "power"
  | "shield"
  | "globe"
  | "route"
  | "refresh"
  | "check"
  | "chevron"
  | "bolt"
  | "upload"
  | "link"
  | "file"
  | "clock"
  | "lock"
  | "server"
  | "download"
  | "repair"
  | "trash"
  | "info"
  | "x"
  | "warning";

interface ToastMessage {
  id: number;
  tone: "success" | "error" | "info";
  message: string;
}

const navItems: Array<{ id: ViewId; label: string; icon: IconName }> = [
  { id: "home", label: "首页", icon: "home" },
  { id: "proxies", label: "节点", icon: "nodes" },
  { id: "profiles", label: "配置", icon: "profile" },
  { id: "settings", label: "设置", icon: "settings" },
];

const pageMeta: Record<ViewId, { eyebrow: string; title: string; description: string }> = {
  home: { eyebrow: "连接概览", title: "首页", description: "安全、清晰地管理当前连接" },
  proxies: { eyebrow: "代理策略", title: "节点", description: "按代理组选择节点，需要时再测速" },
  profiles: { eyebrow: "配置管理", title: "配置", description: "导入、校验并更新当前使用的配置" },
  settings: { eyebrow: "偏好与维护", title: "设置", description: "启动行为、DNS 与本机维护" },
};

const modeOptions: Array<{ value: ProxyMode; label: string }> = [
  { value: "rule", label: "规则" },
  { value: "global", label: "全局" },
  { value: "direct", label: "直连" },
];

function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  const paths: Record<IconName, ReactNode> = {
    home: <><path d="m3 10 9-7 9 7"/><path d="M5 9v11h14V9"/><path d="M9 20v-6h6v6"/></>,
    nodes: <><circle cx="5" cy="6" r="2"/><circle cx="19" cy="6" r="2"/><circle cx="12" cy="18" r="2"/><path d="m6.7 7.1 4.1 8.1M17.3 7.1l-4.1 8.1M7 6h10"/></>,
    profile: <><path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5M9 13h6M9 17h4"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1a1.7 1.7 0 0 0 1.9.3A1.7 1.7 0 0 0 10 3V2.8h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z"/></>,
    power: <><path d="M12 2v10"/><path d="M18.4 5.6a9 9 0 1 1-12.8 0"/></>,
    shield: <><path d="M12 3 5 6v5c0 4.7 2.9 8 7 10 4.1-2 7-5.3 7-10V6z"/><path d="m9 12 2 2 4-4"/></>,
    globe: <><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/></>,
    route: <><circle cx="6" cy="18" r="2"/><circle cx="18" cy="6" r="2"/><path d="M8 18h3a2 2 0 0 0 2-2V8a2 2 0 0 1 2-2h1"/></>,
    refresh: <><path d="M20 7v5h-5"/><path d="M19 12a7 7 0 1 0-2 5"/></>,
    check: <path d="m5 12 4 4L19 6"/>,
    chevron: <path d="m9 18 6-6-6-6"/>,
    bolt: <path d="m13 2-8 12h7l-1 8 8-12h-7z"/>,
    upload: <><path d="M12 16V4M7 9l5-5 5 5"/><path d="M4 15v5h16v-5"/></>,
    link: <><path d="m10 13 4-4"/><path d="M8.5 15.5 7 17a4 4 0 0 1-6-6l3-3a4 4 0 0 1 5.5 0M15.5 8.5 17 7a4 4 0 0 1 6 6l-3 3a4 4 0 0 1-5.5 0"/></>,
    file: <><path d="M6 3h9l4 4v14H6z"/><path d="M14 3v5h5"/></>,
    clock: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
    lock: <><rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></>,
    server: <><rect x="3" y="4" width="18" height="6" rx="2"/><rect x="3" y="14" width="18" height="6" rx="2"/><path d="M7 7h.01M7 17h.01"/></>,
    download: <><path d="M12 3v13M7 11l5 5 5-5"/><path d="M4 20h16"/></>,
    repair: <><path d="M14.7 6.3a4 4 0 0 0-5-5L12 3.6 9.6 6 7.3 3.7a4 4 0 0 0 5 5L19 15.4a2 2 0 0 1-2.8 2.8l-6.7-6.7"/><circle cx="5" cy="19" r="2"/></>,
    trash: <><path d="M4 7h16M9 3h6l1 4H8zM6 7l1 14h10l1-14M10 11v6M14 11v6"/></>,
    info: <><circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/></>,
    x: <path d="m6 6 12 12M18 6 6 18"/>,
    warning: <><path d="M12 3 2.5 20h19z"/><path d="M12 9v4M12 16h.01"/></>,
  };
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled = false,
  label,
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          type="button"
          role="radio"
          aria-checked={value === option.value}
          className={value === option.value ? "segment active" : "segment"}
          disabled={disabled}
          onClick={() => onChange(option.value)}
          key={option.value}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function Toggle({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      className={checked ? "toggle active" : "toggle"}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

function Spinner({ small = false }: { small?: boolean }) {
  return <span className={small ? "spinner spinner-small" : "spinner"} aria-hidden="true" />;
}

function formatTime(value: string | null): string {
  if (!value) return "尚未更新";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "时间未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function connectionLabel(status: AppSnapshot["connection"]["status"]): string {
  const labels = {
    disconnected: "未连接",
    connecting: "正在连接",
    connected: "已连接",
    disconnecting: "正在断开",
    error: "连接异常",
  };
  return labels[status];
}

function sanitizeError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message
    .replace(/https?:\/\/\S+/gi, "[已隐藏地址]")
    .replace(/(token|secret|key)=([^\s&]+)/gi, "$1=[已隐藏]")
    .slice(0, 180);
}

function LoadingScreen() {
  return (
    <div className="app-shell app-loading" aria-label="正在读取应用状态" aria-busy="true">
      <aside className="sidebar skeleton-sidebar">
        <div className="skeleton skeleton-brand" />
        {[1, 2, 3, 4].map((item) => <div className="skeleton skeleton-nav" key={item} />)}
      </aside>
      <main className="main-content">
        <div className="skeleton skeleton-title" />
        <div className="skeleton skeleton-hero" />
        <div className="skeleton-grid">
          <div className="skeleton skeleton-card" />
          <div className="skeleton skeleton-card" />
        </div>
      </main>
    </div>
  );
}

function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<ViewId>("home");
  const [busyKeys, setBusyKeys] = useState<Set<string>>(new Set());
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const [selectedGroupId, setSelectedGroupId] = useState<string>("");
  const [uninstallOpen, setUninstallOpen] = useState(false);
  const toastId = useRef(0);

  const pushToast = useCallback((tone: ToastMessage["tone"], message: string) => {
    const id = ++toastId.current;
    setToasts((current) => [...current, { id, tone, message }]);
    window.setTimeout(() => {
      setToasts((current) => current.filter((toast) => toast.id !== id));
    }, 4200);
  }, []);

  const loadSnapshot = useCallback(async () => {
    setLoadError(null);
    try {
      const result = await viaBridge.getSnapshot();
      setSnapshot(result);
      setSelectedGroupId((current) => current || result.proxyGroups[0]?.id || "");
    } catch (error) {
      setLoadError(sanitizeError(error));
    }
  }, []);

  useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  useEffect(() => {
    let disposed = false;
    const timer = window.setInterval(() => {
      void viaBridge.getSnapshot().then((result) => {
        if (!disposed) setSnapshot(result);
      }).catch(() => {
        // Command actions surface errors explicitly; background refresh stays quiet.
      });
    }, 2000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  const setBusy = useCallback((key: string, value: boolean) => {
    setBusyKeys((current) => {
      const next = new Set(current);
      if (value) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);

  const runSnapshotAction = useCallback(async (
    key: string,
    action: () => Promise<AppSnapshot>,
    successMessage?: string,
  ) => {
    setBusy(key, true);
    try {
      const result = await action();
      setSnapshot(result);
      if (successMessage) pushToast("success", successMessage);
      return true;
    } catch (error) {
      pushToast("error", sanitizeError(error));
      return false;
    } finally {
      setBusy(key, false);
    }
  }, [pushToast, setBusy]);

  const updateSettings = useCallback((patch: SettingsPatch, successMessage?: string) => {
    void runSnapshotAction("settings", () => viaBridge.updateSettings(patch), successMessage);
  }, [runSnapshotAction]);

  if (!snapshot && !loadError) return <LoadingScreen />;

  if (!snapshot) {
    return (
      <main className="fatal-state">
        <div className="fatal-icon"><Icon name="warning" size={24} /></div>
        <h1>无法读取应用状态</h1>
        <p>{loadError || "后端暂时没有响应"}</p>
        <button className="button primary" type="button" onClick={() => void loadSnapshot()}>
          <Icon name="refresh" />重新尝试
        </button>
      </main>
    );
  }

  const isConnected = snapshot.connection.status === "connected";
  const currentMeta = pageMeta[activeView];

  const handleConnection = async () => {
    const enabled = !isConnected;
    setSnapshot((current) => current ? {
      ...current,
      connection: {
        ...current.connection,
        status: enabled ? "connecting" : "disconnecting",
      },
    } : current);
    const success = await runSnapshotAction(
      "connection",
      () => viaBridge.setConnection(enabled),
      enabled ? "连接已建立" : "已断开并恢复系统网络",
    );
    if (!success) await loadSnapshot();
  };

  const handleTransport = (mode: TransportMode) => {
    const helperHint = mode === "tun" && !snapshot.runtime.helperInstalled;
    void runSnapshotAction(
      "transport",
      () => viaBridge.setTransportMode(mode),
      helperHint ? "请按系统提示授权安装 TUN 辅助服务" : `已切换到${mode === "tun" ? "TUN" : "系统代理"}`,
    );
  };

  const handleProxyMode = (mode: ProxyMode) => {
    void runSnapshotAction("proxy-mode", () => viaBridge.setProxyMode(mode));
  };

  const handleSelectProxy = (groupId: string, proxyId: string) => {
    void runSnapshotAction(`select-${groupId}`, () => viaBridge.selectProxy(groupId, proxyId), "节点已切换");
  };

  const handleTestDelay = (groupId: string, proxyId: string) => {
    void runSnapshotAction(`delay-${proxyId}`, () => viaBridge.testProxyDelay(groupId, proxyId));
  };

  const handleRefresh = () => {
    void runSnapshotAction("refresh-profile", () => viaBridge.refreshProfile(), "配置已校验并更新");
  };

  const handleExport = async () => {
    setBusy("export", true);
    try {
      const result = await viaBridge.exportDiagnostics();
      pushToast("success", `脱敏诊断包已保存：${result.path}`);
    } catch (error) {
      pushToast("error", sanitizeError(error));
    } finally {
      setBusy("export", false);
    }
  };

  const handleRepair = async () => {
    setBusy("repair", true);
    try {
      const result = await viaBridge.repairNetwork();
      pushToast("success", result.summary);
      await loadSnapshot();
    } catch (error) {
      pushToast("error", sanitizeError(error));
    } finally {
      setBusy("repair", false);
    }
  };

  const handleUninstall = async () => {
    setBusy("uninstall", true);
    try {
      const result = await viaBridge.uninstallComponents();
      pushToast("success", result.summary);
      setUninstallOpen(false);
      await loadSnapshot();
    } catch (error) {
      pushToast("error", sanitizeError(error));
    } finally {
      setBusy("uninstall", false);
    }
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand-block">
          <div className="brand-mark" aria-hidden="true"><span>V</span></div>
          <div className="brand-copy">
            <strong>我自愿开启</strong>
            <span>国际互联网访问</span>
          </div>
        </div>

        <nav className="primary-nav" aria-label="主导航">
          {navItems.map((item) => (
            <button
              type="button"
              className={activeView === item.id ? "nav-item active" : "nav-item"}
              aria-current={activeView === item.id ? "page" : undefined}
              onClick={() => setActiveView(item.id)}
              key={item.id}
            >
              <Icon name={item.icon} size={19} />
              <span>{item.label}</span>
              {item.id === "home" && <i className={isConnected ? "status-dot online" : "status-dot"} />}
            </button>
          ))}
        </nav>

        <div className="sidebar-spacer" />
        <div className="sidebar-status">
          <div className={isConnected ? "mini-shield online" : "mini-shield"}>
            <Icon name="shield" size={17} />
          </div>
          <div>
            <span>网络保护</span>
            <strong>{isConnected ? "运行正常" : "当前未启用"}</strong>
          </div>
        </div>
        <div className="sidebar-version">
          <span>{snapshot.runtime.appVersion}</span>
          {snapshot.isMock && <span className="preview-badge">预览</span>}
        </div>
      </aside>

      <main className="main-content">
        <header className="page-header">
          <div>
            <span className="eyebrow">{currentMeta.eyebrow}</span>
            <div className="title-line">
              <h1>{currentMeta.title}</h1>
              <span>{currentMeta.description}</span>
            </div>
          </div>
          <div className="runtime-pill" title="Mihomo 控制器状态">
            <span className={snapshot.runtime.controllerHealthy ? "runtime-dot healthy" : "runtime-dot"} />
            <div>
              <strong>{snapshot.runtime.controllerHealthy ? "内核运行中" : "内核已停止"}</strong>
              <span>{snapshot.runtime.coreVersion}</span>
            </div>
          </div>
        </header>

        <div className="page-scroll" key={activeView}>
          {activeView === "home" && (
            <HomeView
              snapshot={snapshot}
              busyKeys={busyKeys}
              onConnection={() => void handleConnection()}
              onTransport={handleTransport}
              onProxyMode={handleProxyMode}
              onOpenProfiles={() => setActiveView("profiles")}
              onOpenProxies={() => setActiveView("proxies")}
              onRefresh={handleRefresh}
            />
          )}
          {activeView === "proxies" && (
            <ProxiesView
              groups={snapshot.proxyGroups}
              selectedGroupId={selectedGroupId}
              onSelectGroup={setSelectedGroupId}
              busyKeys={busyKeys}
              onSelectProxy={handleSelectProxy}
              onTestDelay={handleTestDelay}
              onOpenProfiles={() => setActiveView("profiles")}
            />
          )}
          {activeView === "profiles" && (
            <ProfilesView
              snapshot={snapshot}
              busyKeys={busyKeys}
              runSnapshotAction={runSnapshotAction}
              onRefresh={handleRefresh}
              pushToast={pushToast}
            />
          )}
          {activeView === "settings" && (
            <SettingsView
              settings={snapshot.settings}
              runtime={snapshot.runtime}
              busyKeys={busyKeys}
              onUpdate={updateSettings}
              onExport={() => void handleExport()}
              onRepair={() => void handleRepair()}
              onUninstall={() => setUninstallOpen(true)}
            />
          )}
        </div>
      </main>

      <div className="toast-stack" aria-live="polite" aria-atomic="false">
        {toasts.map((toast) => (
          <div className={`toast ${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"} key={toast.id}>
            <span className="toast-icon"><Icon name={toast.tone === "success" ? "check" : toast.tone === "error" ? "warning" : "info"} size={16} /></span>
            <span>{toast.message}</span>
            <button type="button" aria-label="关闭提示" onClick={() => setToasts((current) => current.filter((item) => item.id !== toast.id))}><Icon name="x" size={15} /></button>
          </div>
        ))}
      </div>

      {uninstallOpen && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => setUninstallOpen(false)}>
          <section className="modal" role="alertdialog" aria-modal="true" aria-labelledby="uninstall-title" onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-icon danger"><Icon name="trash" size={22} /></div>
            <h2 id="uninstall-title">移除运行组件？</h2>
            <p>将先断开连接、恢复系统网络，再移除高权限辅助服务。订阅凭据和应用本体不会在此步骤中删除。</p>
            <div className="modal-actions">
              <button className="button ghost" type="button" onClick={() => setUninstallOpen(false)}>取消</button>
              <button className="button danger" type="button" disabled={busyKeys.has("uninstall")} onClick={() => void handleUninstall()}>
                {busyKeys.has("uninstall") && <Spinner small />}确认移除
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function HomeView({
  snapshot,
  busyKeys,
  onConnection,
  onTransport,
  onProxyMode,
  onOpenProfiles,
  onOpenProxies,
  onRefresh,
}: {
  snapshot: AppSnapshot;
  busyKeys: Set<string>;
  onConnection: () => void;
  onTransport: (mode: TransportMode) => void;
  onProxyMode: (mode: ProxyMode) => void;
  onOpenProfiles: () => void;
  onOpenProxies: () => void;
  onRefresh: () => void;
}) {
  const connected = snapshot.connection.status === "connected";
  const transitional = snapshot.connection.status === "connecting" || snapshot.connection.status === "disconnecting";
  return (
    <div className="home-layout page-enter">
      <section className={connected ? "connection-card connected" : "connection-card"}>
        <div className="connection-ambient" />
        <div className="connection-topline">
          <span className={connected ? "connection-badge online" : "connection-badge"}>
            <i />{connectionLabel(snapshot.connection.status)}
          </span>
          <span className="fail-open-label"><Icon name="shield" size={15} />故障时自动恢复网络</span>
        </div>
        <div className="connection-core">
          <button
            className={connected ? "power-button active" : "power-button"}
            type="button"
            aria-label={connected ? "断开代理连接" : "建立代理连接"}
            aria-pressed={connected}
            disabled={transitional || busyKeys.has("connection") || !snapshot.profile}
            onClick={onConnection}
          >
            <span className="power-halo" />
            {transitional || busyKeys.has("connection") ? <Spinner /> : <Icon name="power" size={31} />}
          </button>
          <div className="connection-copy">
            <h2>{connected ? "连接已受到保护" : snapshot.profile ? "准备就绪" : "请先导入配置"}</h2>
            <p>{connected ? `${snapshot.connection.activeProxyName || "订阅默认节点"} · ${snapshot.connection.proxyMode === "rule" ? "规则分流" : snapshot.connection.proxyMode === "global" ? "全局代理" : "全部直连"}` : snapshot.profile ? "点击按钮开始连接" : "导入经过校验的订阅或 YAML 文件后即可连接"}</p>
          </div>
        </div>
        <div className="connection-controls">
          <div className="control-block">
            <label>接管方式</label>
            <Segmented
              label="接管方式"
              value={snapshot.connection.transportMode}
              options={[{ value: "system", label: "系统代理" }, { value: "tun", label: "TUN 模式" }]}
              disabled={busyKeys.has("transport") || transitional}
              onChange={onTransport}
            />
          </div>
          <div className="control-divider" />
          <div className="control-block control-block-wide">
            <label>代理模式</label>
            <Segmented
              label="代理模式"
              value={snapshot.connection.proxyMode}
              options={modeOptions}
              disabled={busyKeys.has("proxy-mode") || transitional}
              onChange={onProxyMode}
            />
          </div>
        </div>
      </section>

      <div className="home-grid">
        <section className="card profile-overview">
          <div className="card-heading">
            <div className="heading-icon violet"><Icon name="profile" /></div>
            <div><span>当前配置</span><h3>{snapshot.profile?.name || "尚未配置"}</h3></div>
            {snapshot.profile && <span className="valid-badge"><Icon name="check" size={13} />有效</span>}
          </div>
          {snapshot.profile ? (
            <>
              <p className="masked-source">{snapshot.profile.maskedSource}</p>
              <div className="meta-row">
                <span><Icon name="clock" size={14} />{formatTime(snapshot.profile.updatedAt)} 更新</span>
                <span>{snapshot.profile.proxyCount} 个节点</span>
                <span>{snapshot.profile.ruleCount.toLocaleString("zh-CN")} 条规则</span>
              </div>
              <div className="card-actions">
                <button className="text-button" type="button" onClick={onOpenProfiles}>管理配置<Icon name="chevron" size={15} /></button>
                <button className="icon-button" aria-label="刷新配置" title="刷新配置" type="button" disabled={busyKeys.has("refresh-profile")} onClick={onRefresh}>
                  {busyKeys.has("refresh-profile") ? <Spinner small /> : <Icon name="refresh" size={16} />}
                </button>
              </div>
            </>
          ) : (
            <div className="empty-compact">
              <p>订阅地址仅会存入系统凭据库。</p>
              <button className="button secondary compact" type="button" onClick={onOpenProfiles}><Icon name="upload" size={16} />导入配置</button>
            </div>
          )}
        </section>

        <section className="card route-overview">
          <div className="card-heading">
            <div className="heading-icon blue"><Icon name="route" /></div>
            <div><span>当前路由</span><h3>{snapshot.connection.activeProxyName || "跟随配置"}</h3></div>
          </div>
          <div className="route-path" aria-label="当前路由路径">
            <div><span className="route-node"><Icon name="globe" size={16} /></span><small>此设备</small></div>
            <i className={connected ? "route-line active" : "route-line"} />
            <div><span className="route-node accent"><Icon name="server" size={16} /></span><small>{snapshot.connection.activeGroupName || "代理组"}</small></div>
            <i className={connected ? "route-line active" : "route-line"} />
            <div><span className="route-node"><Icon name="shield" size={16} /></span><small>互联网</small></div>
          </div>
          <div className="card-actions route-actions">
            <span className="port-label">{snapshot.runtime.localPort ? `本地端口 ${snapshot.runtime.localPort}` : "连接后自动分配端口"}</span>
            <button className="text-button" type="button" onClick={onOpenProxies}>选择节点<Icon name="chevron" size={15} /></button>
          </div>
        </section>
      </div>
    </div>
  );
}

function ProxiesView({
  groups,
  selectedGroupId,
  onSelectGroup,
  busyKeys,
  onSelectProxy,
  onTestDelay,
  onOpenProfiles,
}: {
  groups: ProxyGroup[];
  selectedGroupId: string;
  onSelectGroup: (groupId: string) => void;
  busyKeys: Set<string>;
  onSelectProxy: (groupId: string, proxyId: string) => void;
  onTestDelay: (groupId: string, proxyId: string) => void;
  onOpenProfiles: () => void;
}) {
  const activeGroup = groups.find((group) => group.id === selectedGroupId) || groups[0];
  const selectedProxy = activeGroup?.proxies.find((proxy) => proxy.id === activeGroup.selectedProxyId);

  if (!activeGroup) {
    return (
      <section className="card empty-state page-enter">
        <div className="empty-icon"><Icon name="nodes" size={25} /></div>
        <h2>当前没有可选节点</h2>
        <p>导入一份有效配置后，代理组和节点会显示在这里。</p>
        <button className="button primary" type="button" onClick={onOpenProfiles}>前往导入配置</button>
      </section>
    );
  }

  return (
    <div className="proxy-layout page-enter">
      <aside className="group-panel card" aria-label="代理组">
        <div className="panel-label">代理组</div>
        {groups.map((group) => {
          const selected = group.proxies.find((proxy) => proxy.id === group.selectedProxyId);
          return (
            <button type="button" className={group.id === activeGroup.id ? "group-item active" : "group-item"} onClick={() => onSelectGroup(group.id)} key={group.id}>
              <span className="group-icon"><Icon name={group.kind === "url-test" ? "bolt" : "nodes"} size={17} /></span>
              <span className="group-copy"><strong>{group.name}</strong><small>{selected?.name || "未选择"}</small></span>
              <Icon name="chevron" size={15} />
            </button>
          );
        })}
        <div className="group-note"><Icon name="info" size={15} /><span>节点不会自动测速，减少不必要的探测流量。</span></div>
      </aside>

      <section className="node-panel card">
        <div className="node-panel-header">
          <div>
            <span className="panel-label">{activeGroup.kind === "select" ? "手动选择" : "自动策略"}</span>
            <h2>{activeGroup.name}</h2>
            <p>当前使用：{selectedProxy?.name || "未选择"}</p>
          </div>
          <span className="node-count">{activeGroup.proxies.length} 个节点</span>
        </div>
        <div className="node-list" role="radiogroup" aria-label={`${activeGroup.name}节点`}>
          {activeGroup.proxies.map((proxy) => {
            const selected = proxy.id === activeGroup.selectedProxyId;
            const delayBusy = busyKeys.has(`delay-${proxy.id}`);
            const selectBusy = busyKeys.has(`select-${activeGroup.id}`);
            const delayTone = proxy.delayMs === null ? "unknown" : proxy.delayMs < 100 ? "fast" : proxy.delayMs < 180 ? "medium" : "slow";
            return (
              <div className={selected ? "node-row selected" : "node-row"} key={proxy.id}>
                <button
                  type="button"
                  className="node-select"
                  role="radio"
                  aria-checked={selected}
                  disabled={selectBusy}
                  onClick={() => onSelectProxy(activeGroup.id, proxy.id)}
                >
                  <span className={selected ? "radio-mark active" : "radio-mark"}>{selected && <span />}</span>
                  <span className="node-copy"><strong>{proxy.name}</strong><small>{proxy.protocol}</small></span>
                  {selected && <span className="selected-label">使用中</span>}
                </button>
                <button
                  className={`delay-button ${delayTone}`}
                  type="button"
                  disabled={delayBusy}
                  aria-label={`测试 ${proxy.name} 延迟`}
                  onClick={() => onTestDelay(activeGroup.id, proxy.id)}
                >
                  {delayBusy ? <Spinner small /> : <><Icon name="bolt" size={14} /><span>{proxy.delayMs === null ? "测速" : `${proxy.delayMs} ms`}</span></>}
                </button>
              </div>
            );
          })}
        </div>
      </section>
    </div>
  );
}

function ProfilesView({
  snapshot,
  busyKeys,
  runSnapshotAction,
  onRefresh,
  pushToast,
}: {
  snapshot: AppSnapshot;
  busyKeys: Set<string>;
  runSnapshotAction: (key: string, action: () => Promise<AppSnapshot>, successMessage?: string) => Promise<boolean>;
  onRefresh: () => void;
  pushToast: (tone: ToastMessage["tone"], message: string) => void;
}) {
  const urlInputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const importSubscription = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const input = urlInputRef.current;
    const url = input?.value.trim() || "";
    if (!url) {
      pushToast("error", "请输入 HTTPS 订阅地址");
      input?.focus();
      return;
    }
    // Do not retain subscription credentials in React state or the input DOM.
    if (input) input.value = "";
    const success = await runSnapshotAction("import-url", () => viaBridge.importSubscription(url), "订阅已安全导入并通过校验");
    if (!success) input?.focus();
  };

  const importFile = async (file: File) => {
    if (!/\.ya?ml$/i.test(file.name)) {
      pushToast("error", "请选择 .yaml 或 .yml 文件");
      return;
    }
    if (file.size > 5 * 1024 * 1024) {
      pushToast("error", "配置文件不能超过 5 MB");
      return;
    }
    const contents = await file.text();
    await runSnapshotAction("import-file", () => viaBridge.importProfileFile(file.name, contents), "本地配置已导入并通过校验");
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  return (
    <div className="profiles-layout page-enter">
      <section className="card current-profile-card">
        <div className="section-heading">
          <div><span className="panel-label">正在使用</span><h2>当前配置</h2></div>
          {snapshot.profile && <span className={snapshot.profile.isValid ? "valid-badge" : "valid-badge invalid"}><Icon name={snapshot.profile.isValid ? "check" : "warning"} size={13} />{snapshot.profile.isValid ? "校验通过" : "校验失败"}</span>}
        </div>
        {snapshot.profile ? (
          <div className="profile-detail">
            <div className="profile-main-row">
              <div className="profile-file-icon"><Icon name={snapshot.profile.sourceKind === "subscription" ? "link" : "file"} size={21} /></div>
              <div><h3>{snapshot.profile.name}</h3><p>{snapshot.profile.maskedSource}</p></div>
              <button className="button secondary compact" type="button" disabled={busyKeys.has("refresh-profile")} onClick={onRefresh}>
                {busyKeys.has("refresh-profile") ? <Spinner small /> : <Icon name="refresh" size={15} />}刷新
              </button>
            </div>
            <dl className="profile-stats">
              <div><dt>节点</dt><dd>{snapshot.profile.proxyCount}</dd></div>
              <div><dt>规则</dt><dd>{snapshot.profile.ruleCount.toLocaleString("zh-CN")}</dd></div>
              <div><dt>最近更新</dt><dd>{formatTime(snapshot.profile.updatedAt)}</dd></div>
              <div><dt>最后有效版本</dt><dd>{formatTime(snapshot.profile.lastValidAt)}</dd></div>
            </dl>
            <div className="safe-backup-note"><Icon name="shield" size={16} /><span>更新失败时继续使用最后一个有效版本，不会中断现有连接。</span></div>
          </div>
        ) : (
          <div className="profile-empty"><div className="empty-icon"><Icon name="profile" /></div><div><h3>还没有配置</h3><p>在下方导入订阅或本地 YAML。</p></div></div>
        )}
      </section>

      <div className="import-grid">
        <section className="card import-card">
          <div className="heading-icon blue"><Icon name="link" /></div>
          <h3>订阅地址</h3>
          <p>仅接受 HTTPS。地址会写入系统凭据库，不会显示在日志中。</p>
          <form onSubmit={(event) => void importSubscription(event)}>
            <label className="sr-only" htmlFor="subscription-url">HTTPS 订阅地址</label>
            <div className="input-with-button">
              <input id="subscription-url" ref={urlInputRef} type="url" inputMode="url" autoComplete="off" spellCheck={false} placeholder="https://example.com/••••••" disabled={busyKeys.has("import-url")} />
              <button className="button primary compact" type="submit" disabled={busyKeys.has("import-url")}>
                {busyKeys.has("import-url") ? <Spinner small /> : <Icon name="download" size={15} />}导入
              </button>
            </div>
          </form>
        </section>

        <section className="card import-card">
          <div className="heading-icon violet"><Icon name="file" /></div>
          <h3>本地 YAML</h3>
          <p>适用于离线配置。文件会先校验，再复制到应用的专用目录。</p>
          <input
            className="sr-only"
            ref={fileInputRef}
            type="file"
            accept=".yaml,.yml,application/yaml,text/yaml"
            onChange={(event) => {
              const file = event.currentTarget.files?.[0];
              if (file) void importFile(file);
            }}
          />
          <button className="button secondary file-button" type="button" disabled={busyKeys.has("import-file")} onClick={() => fileInputRef.current?.click()}>
            {busyKeys.has("import-file") ? <Spinner small /> : <Icon name="upload" size={16} />}选择 YAML 文件
          </button>
        </section>
      </div>

      <div className="security-strip"><Icon name="lock" size={17} /><div><strong>配置会经过安全覆盖</strong><span>监听地址、控制密钥、端口、TUN 与文件路径均由 VIA 管理；订阅无法扩大本机访问权限。</span></div></div>
    </div>
  );
}

function SettingsView({
  settings,
  runtime,
  busyKeys,
  onUpdate,
  onExport,
  onRepair,
  onUninstall,
}: {
  settings: AppSettings;
  runtime: AppSnapshot["runtime"];
  busyKeys: Set<string>;
  onUpdate: (patch: SettingsPatch, successMessage?: string) => void;
  onExport: () => void;
  onRepair: () => void;
  onUninstall: () => void;
}) {
  const settingsBusy = busyKeys.has("settings");
  const dnsOptions: Array<{ value: DnsMode; label: string }> = [
    { value: "fake-ip", label: "Fake-IP" },
    { value: "redir-host", label: "兼容模式" },
  ];
  return (
    <div className="settings-layout page-enter">
      <section className="settings-card card">
        <div className="settings-section-title"><div className="heading-icon blue"><Icon name="power" /></div><div><h2>启动与连接</h2><p>控制应用随系统启动后的行为</p></div></div>
        <div className="setting-row">
          <div><strong>开机启动</strong><span>登录系统后自动在托盘运行 VIA</span></div>
          <Toggle checked={settings.launchOnStartup} disabled={settingsBusy} label="开机启动" onChange={(value) => onUpdate({ launchOnStartup: value })} />
        </div>
        <div className="setting-row">
          <div><strong>启动后自动连接</strong><span>使用最后一个已验证配置；失败时自动恢复网络</span></div>
          <Toggle checked={settings.autoConnect} disabled={settingsBusy} label="启动后自动连接" onChange={(value) => onUpdate({ autoConnect: value, launchOnStartup: value ? true : settings.launchOnStartup })} />
        </div>
      </section>

      <section className="settings-card card">
        <div className="settings-section-title"><div className="heading-icon violet"><Icon name="globe" /></div><div><h2>DNS 模式</h2><p>TUN 模式下的域名解析策略</p></div></div>
        <div className="dns-setting">
          <Segmented label="DNS 模式" value={settings.dnsMode} options={dnsOptions} disabled={settingsBusy} onChange={(value) => onUpdate({ dnsMode: value }, `已切换到${value === "fake-ip" ? " Fake-IP" : "兼容模式"}`)} />
          <p>{settings.dnsMode === "fake-ip" ? "推荐。分流响应更快，遇到局域网域名或特殊应用时可切换兼容模式。" : "使用 Redir-Host，提高游戏、局域网服务和部分特殊应用的兼容性。"}</p>
        </div>
      </section>

      <section className="settings-card card maintenance-card">
        <div className="settings-section-title"><div className="heading-icon amber"><Icon name="repair" /></div><div><h2>诊断与恢复</h2><p>仅记录受控事件，不保存内核原始输出，最多保留 7 天</p></div></div>
        <div className="maintenance-list">
          <div className="maintenance-row">
            <div><strong>导出诊断包</strong><span>导出版本、崩溃信息和脱敏日志，不包含订阅令牌</span></div>
            <button className="button secondary compact" type="button" disabled={busyKeys.has("export")} onClick={onExport}>{busyKeys.has("export") ? <Spinner small /> : <Icon name="download" size={15} />}导出</button>
          </div>
          <div className="maintenance-row">
            <div><strong>修复系统网络</strong><span>停止 Mihomo、撤销 TUN，并恢复原系统代理</span></div>
            <button className="button secondary compact" type="button" disabled={busyKeys.has("repair")} onClick={onRepair}>{busyKeys.has("repair") ? <Spinner small /> : <Icon name="repair" size={15} />}立即修复</button>
          </div>
          <div className="maintenance-row danger-row">
            <div><strong>移除运行组件</strong><span>安全断开后移除 TUN 辅助服务与运行组件</span></div>
            <button className="button danger-ghost compact" type="button" onClick={onUninstall}><Icon name="trash" size={15} />移除</button>
          </div>
        </div>
      </section>

      <footer className="about-line">
        <span>{runtime.appVersion}</span><i />
        <span>{runtime.coreVersion}</span><i />
        <span>{runtime.platformLabel}</span><i />
        <span>{runtime.helperInstalled ? "辅助服务已安装" : "辅助服务未安装"}</span>
      </footer>
    </div>
  );
}

export default App;
