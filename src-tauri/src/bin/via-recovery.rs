#[allow(dead_code, unused_imports)]
#[path = "../helper/mod.rs"]
mod helper;

use std::path::PathBuf;

use serde::Serialize;
use via_desktop_lib::network::{
    FileLeaseJournal, NativeProxyAdapter, ProxyTransaction, RecoveryOutcome,
};

#[cfg(target_os = "macos")]
use via_desktop_lib::network::MacOsProxySnapshot as NativeSnapshot;
#[cfg(windows)]
use via_desktop_lib::network::WindowsProxySnapshot as NativeSnapshot;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum RepairState {
    NotRequested,
    NoLease,
    Restored,
    AlreadyAtBaseline,
    PreservedExternalChange,
    HelperNotInstalled,
    Failed,
}

#[derive(Debug, Serialize)]
struct RepairPart {
    state: RepairState,
    message: String,
}

#[derive(Debug, Serialize)]
struct RepairReport {
    success: bool,
    proxy: RepairPart,
    tun: RepairPart,
}

#[derive(Debug, Default)]
struct Options {
    json: bool,
    proxy_only: bool,
    tun_only: bool,
}

#[tokio::main]
async fn main() {
    let options = match parse_options() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let run_proxy = !options.tun_only;
    let run_tun = !options.proxy_only;

    let proxy = if run_proxy {
        recover_proxy()
    } else {
        not_requested()
    };
    let tun = if run_tun {
        recover_tun().await
    } else {
        not_requested()
    };
    let success =
        !matches!(proxy.state, RepairState::Failed) && !matches!(tun.state, RepairState::Failed);
    let report = RepairReport {
        success,
        proxy,
        tun,
    };

    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("repair report is serializable")
        );
    } else {
        println!("系统代理：{}", report.proxy.message);
        println!("TUN：{}", report.tun.message);
    }
    if !success {
        std::process::exit(1);
    }
}

fn recover_proxy() -> RepairPart {
    let path = match network_lease_path() {
        Ok(path) => path,
        Err(message) => return failed(message),
    };
    let journal = FileLeaseJournal::<NativeSnapshot>::new(path);
    let transaction = ProxyTransaction::new(NativeProxyAdapter::new(), journal);
    match transaction.recover_stale() {
        Ok(RecoveryOutcome::NoLease) => RepairPart {
            state: RepairState::NoLease,
            message: "没有待恢复的租约".to_string(),
        },
        Ok(RecoveryOutcome::Restored) => RepairPart {
            state: RepairState::Restored,
            message: "已恢复 VIA 连接前的系统代理".to_string(),
        },
        Ok(RecoveryOutcome::AlreadyAtBaseline) => RepairPart {
            state: RepairState::AlreadyAtBaseline,
            message: "系统代理已处于连接前状态".to_string(),
        },
        Ok(RecoveryOutcome::PreservedExternalChange) => RepairPart {
            state: RepairState::PreservedExternalChange,
            message: "检测到外部修改，已保留当前系统代理".to_string(),
        },
        Err(error) => failed(format!("系统代理恢复失败：{error}")),
    }
}

async fn recover_tun() -> RepairPart {
    let client = match helper::HelperClient::from_installed() {
        Ok(client) => client,
        Err(helper::HelperError::NotInstalled(_)) => return helper_not_installed(),
        Err(helper::HelperError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return helper_not_installed();
        }
        Err(error) => return failed(format!("无法读取 helper 凭据：{error}")),
    };
    match client
        .request(helper::HelperOperation::Restore { session_id: None })
        .await
    {
        Ok(response)
            if response.ok && matches!(response.body, helper::HelperResponseBody::Restored) =>
        {
            RepairPart {
                state: RepairState::Restored,
                message: "已请求高权限 helper 停止 TUN 并清除租约".to_string(),
            }
        }
        Ok(response) => failed(format!("helper 拒绝恢复请求：{:?}", response.body)),
        Err(error) => failed(format!("TUN 恢复失败：{error}")),
    }
}

fn network_lease_path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "APPDATA 未设置，无法定位 VIA 数据目录".to_string())?;
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"))
        .ok_or_else(|| "HOME 未设置，无法定位 VIA 数据目录".to_string())?;

    Ok(base
        .join("io.github.cgengqiang-cmyk.voluntary-internet-access")
        .join("network-lease.json"))
}

fn parse_options() -> Result<Option<Options>, String> {
    let mut options = Options::default();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--json" => options.json = true,
            "--proxy-only" => options.proxy_only = true,
            "--tun-only" => options.tun_only = true,
            "--help" | "-h" => {
                println!(
                    "via-recovery {}\n\nUSAGE:\n    via-recovery [--json] [--proxy-only | --tun-only]\n\nRestores VIA-owned network state without starting a WebView.",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(None);
            }
            _ => return Err(format!("未知参数：{argument}")),
        }
    }
    if options.proxy_only && options.tun_only {
        return Err("--proxy-only 与 --tun-only 不能同时使用".to_string());
    }
    Ok(Some(options))
}

fn not_requested() -> RepairPart {
    RepairPart {
        state: RepairState::NotRequested,
        message: "未请求".to_string(),
    }
}

fn helper_not_installed() -> RepairPart {
    RepairPart {
        state: RepairState::HelperNotInstalled,
        message: "高权限 helper 未安装，无需恢复".to_string(),
    }
}

fn failed(message: String) -> RepairPart {
    RepairPart {
        state: RepairState::Failed,
        message,
    }
}
