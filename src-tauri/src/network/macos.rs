use std::collections::BTreeMap;

#[cfg(target_os = "macos")]
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};

use super::{LoopbackProxyTarget, NetworkError, NetworkResult, ProxyAdapter};

#[cfg(target_os = "macos")]
const NETWORKSETUP: &str = "/usr/sbin/networksetup";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacProxyEndpoint {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
    pub authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacAutoProxyState {
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacNetworkServiceProxy {
    pub service: String,
    pub service_enabled: bool,
    pub web: MacProxyEndpoint,
    pub secure_web: MacProxyEndpoint,
    pub auto_proxy: MacAutoProxyState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacOsProxySnapshot {
    pub services: Vec<MacNetworkServiceProxy>,
}

/// macOS `networksetup` system-proxy adapter.
///
/// Every command is executed through `Command::args`; neither service names nor
/// URLs ever enter a shell string. The command plan is unit tested on Windows,
/// but the actual mutations must still pass the project's macOS 13+ physical-
/// machine network-recovery script before release.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacOsProxyAdapter;

impl MacOsProxyAdapter {
    pub fn new() -> Self {
        Self
    }

    #[cfg(target_os = "macos")]
    fn list_services(&self) -> NetworkResult<Vec<(String, bool)>> {
        let output = run_networksetup(&["-listallnetworkservices".to_string()])?;
        let text = output_text("enumerate macOS network services", output)?;
        parse_network_services(&text)
    }

    #[cfg(target_os = "macos")]
    fn read_proxy(
        &self,
        operation: &'static str,
        option: &'static str,
        service: &str,
    ) -> NetworkResult<MacProxyEndpoint> {
        let output = run_networksetup(&[option.to_string(), service.to_string()])?;
        let text = output_text(operation, output)?;
        parse_proxy_endpoint(operation, &text)
    }

    #[cfg(target_os = "macos")]
    fn read_auto_proxy(&self, service: &str) -> NetworkResult<MacAutoProxyState> {
        let output = run_networksetup(&["-getautoproxyurl".to_string(), service.to_string()])?;
        let text = output_text("read macOS auto proxy URL", output)?;
        parse_auto_proxy("read macOS auto proxy URL", &text)
    }

    fn reject_authenticated_baseline(baseline: &MacOsProxySnapshot) -> NetworkResult<()> {
        for service in baseline
            .services
            .iter()
            .filter(|service| service.service_enabled)
        {
            if service.web.authenticated {
                return Err(NetworkError::AuthenticatedProxyBaseline {
                    service: service.service.clone(),
                    proxy_kind: "HTTP",
                });
            }
            if service.secure_web.authenticated {
                return Err(NetworkError::AuthenticatedProxyBaseline {
                    service: service.service.clone(),
                    proxy_kind: "HTTPS",
                });
            }
        }
        Ok(())
    }
}

impl ProxyAdapter for MacOsProxyAdapter {
    type Snapshot = MacOsProxySnapshot;

    #[cfg(target_os = "macos")]
    fn capture(&self) -> NetworkResult<Self::Snapshot> {
        let mut services = Vec::new();
        for (service, service_enabled) in self.list_services()? {
            let web = self.read_proxy("read macOS web proxy", "-getwebproxy", &service)?;
            let secure_web = self.read_proxy(
                "read macOS secure web proxy",
                "-getsecurewebproxy",
                &service,
            )?;
            let auto_proxy = self.read_auto_proxy(&service)?;
            services.push(MacNetworkServiceProxy {
                service,
                service_enabled,
                web,
                secure_web,
                auto_proxy,
            });
        }
        Ok(MacOsProxySnapshot { services })
    }

    #[cfg(not(target_os = "macos"))]
    fn capture(&self) -> NetworkResult<Self::Snapshot> {
        Err(NetworkError::Unsupported {
            operation: "capture macOS system proxy",
            reason: "macOS networksetup is unavailable on this host".to_string(),
        })
    }

    fn build_via_owned_state(
        &self,
        baseline: &Self::Snapshot,
        target: LoopbackProxyTarget,
    ) -> NetworkResult<Self::Snapshot> {
        target.validate()?;
        Self::reject_authenticated_baseline(baseline)?;

        let endpoint = MacProxyEndpoint {
            enabled: true,
            server: target.host().to_string(),
            port: target.mixed_port(),
            authenticated: false,
        };
        Ok(MacOsProxySnapshot {
            services: baseline
                .services
                .iter()
                .map(|service| {
                    if service.service_enabled {
                        MacNetworkServiceProxy {
                            service: service.service.clone(),
                            service_enabled: true,
                            web: endpoint.clone(),
                            secure_web: endpoint.clone(),
                            // Preserve the PAC URL itself, but keep it disabled
                            // for the duration of VIA's ownership.
                            auto_proxy: MacAutoProxyState {
                                enabled: false,
                                url: service.auto_proxy.url.clone(),
                            },
                        }
                    } else {
                        // Disabled services are outside VIA's mutation scope.
                        service.clone()
                    }
                })
                .collect(),
        })
    }

    #[cfg(target_os = "macos")]
    fn apply_snapshot(&self, via_owned_state: &Self::Snapshot) -> NetworkResult<()> {
        execute_plan(&plan_write(via_owned_state)?)
    }

    #[cfg(not(target_os = "macos"))]
    fn apply_snapshot(&self, _via_owned_state: &Self::Snapshot) -> NetworkResult<()> {
        Err(NetworkError::Unsupported {
            operation: "apply macOS system proxy",
            reason: "macOS networksetup is unavailable on this host".to_string(),
        })
    }

    #[cfg(target_os = "macos")]
    fn restore_snapshot(&self, baseline: &Self::Snapshot) -> NetworkResult<()> {
        // `build_via_owned_state` rejects authenticated baselines before the
        // first mutation. Keep the same guard for old or manually created
        // leases so no unknown password is silently destroyed.
        Self::reject_authenticated_baseline(baseline)?;
        execute_plan(&plan_write(baseline)?)
    }

    #[cfg(not(target_os = "macos"))]
    fn restore_snapshot(&self, _baseline: &Self::Snapshot) -> NetworkResult<()> {
        Err(NetworkError::Unsupported {
            operation: "restore macOS system proxy",
            reason: "macOS networksetup is unavailable on this host".to_string(),
        })
    }

    fn merge_recovery_state(
        &self,
        current: &Self::Snapshot,
        baseline: &Self::Snapshot,
        via_owned_state: &Self::Snapshot,
    ) -> NetworkResult<Self::Snapshot> {
        if current.services.len() != baseline.services.len()
            || current.services.len() != via_owned_state.services.len()
        {
            return Err(NetworkError::adapter(
                "merge macOS proxy recovery state",
                "network service set changed while VIA owned the proxy",
            ));
        }

        let mut services = Vec::with_capacity(current.services.len());
        for current_service in &current.services {
            let baseline_service = baseline
                .services
                .iter()
                .find(|service| service.service == current_service.service)
                .ok_or_else(|| {
                    NetworkError::adapter(
                        "merge macOS proxy recovery state",
                        "baseline network service is missing",
                    )
                })?;
            let owned_service = via_owned_state
                .services
                .iter()
                .find(|service| service.service == current_service.service)
                .ok_or_else(|| {
                    NetworkError::adapter(
                        "merge macOS proxy recovery state",
                        "owned network service is missing",
                    )
                })?;
            if current_service.service_enabled != owned_service.service_enabled {
                return Err(NetworkError::adapter(
                    "merge macOS proxy recovery state",
                    format!(
                        "network service `{}` was enabled or disabled externally",
                        current_service.service
                    ),
                ));
            }
            services.push(MacNetworkServiceProxy {
                service: current_service.service.clone(),
                service_enabled: baseline_service.service_enabled,
                web: merge_endpoint(
                    &current_service.web,
                    &baseline_service.web,
                    &owned_service.web,
                ),
                secure_web: merge_endpoint(
                    &current_service.secure_web,
                    &baseline_service.secure_web,
                    &owned_service.secure_web,
                ),
                auto_proxy: MacAutoProxyState {
                    enabled: restore_if_owned(
                        current_service.auto_proxy.enabled,
                        baseline_service.auto_proxy.enabled,
                        owned_service.auto_proxy.enabled,
                    ),
                    url: restore_if_owned_ref(
                        &current_service.auto_proxy.url,
                        &baseline_service.auto_proxy.url,
                        &owned_service.auto_proxy.url,
                    ),
                },
            });
        }
        Ok(MacOsProxySnapshot { services })
    }
}

fn merge_endpoint(
    current: &MacProxyEndpoint,
    baseline: &MacProxyEndpoint,
    owned: &MacProxyEndpoint,
) -> MacProxyEndpoint {
    MacProxyEndpoint {
        enabled: restore_if_owned(current.enabled, baseline.enabled, owned.enabled),
        server: restore_if_owned_ref(&current.server, &baseline.server, &owned.server),
        port: restore_if_owned(current.port, baseline.port, owned.port),
        authenticated: restore_if_owned(
            current.authenticated,
            baseline.authenticated,
            owned.authenticated,
        ),
    }
}

fn restore_if_owned<T: Copy + PartialEq>(current: T, baseline: T, owned: T) -> T {
    if current == owned { baseline } else { current }
}

fn restore_if_owned_ref<T: Clone + PartialEq>(current: &T, baseline: &T, owned: &T) -> T {
    if current == owned {
        baseline.clone()
    } else {
        current.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkSetupCommand {
    arguments: Vec<String>,
}

impl NetworkSetupCommand {
    fn new(arguments: impl IntoIterator<Item = String>) -> Self {
        Self {
            arguments: arguments.into_iter().collect(),
        }
    }
}

/// Produce one globally ordered mutation plan:
/// 1. disable HTTP, HTTPS, and PAC for every enabled service;
/// 2. write endpoints/URLs while disabled;
/// 3. enable only the exact desired states.
fn plan_write(snapshot: &MacOsProxySnapshot) -> NetworkResult<Vec<NetworkSetupCommand>> {
    MacOsProxyAdapter::reject_authenticated_baseline(snapshot)?;
    let enabled_services: Vec<&MacNetworkServiceProxy> = snapshot
        .services
        .iter()
        .filter(|service| service.service_enabled)
        .collect();
    let mut commands = Vec::new();

    for service in &enabled_services {
        commands.push(state_command("-setwebproxystate", &service.service, false));
        commands.push(state_command(
            "-setsecurewebproxystate",
            &service.service,
            false,
        ));
        commands.push(state_command("-setautoproxystate", &service.service, false));
    }

    for service in &enabled_services {
        commands.push(endpoint_command(
            "-setwebproxy",
            &service.service,
            &service.web,
        ));
        // `networksetup -setwebproxy` enables the proxy as a side effect.
        // Return to fail-open state until the final enable phase.
        commands.push(state_command("-setwebproxystate", &service.service, false));
        commands.push(endpoint_command(
            "-setsecurewebproxy",
            &service.service,
            &service.secure_web,
        ));
        commands.push(state_command(
            "-setsecurewebproxystate",
            &service.service,
            false,
        ));
        if !service.auto_proxy.url.is_empty() {
            commands.push(NetworkSetupCommand::new([
                "-setautoproxyurl".to_string(),
                service.service.clone(),
                service.auto_proxy.url.clone(),
            ]));
            // PAC URL assignment can also enable PAC on supported releases.
            commands.push(state_command("-setautoproxystate", &service.service, false));
        }
    }

    for service in &enabled_services {
        commands.push(state_command(
            "-setwebproxystate",
            &service.service,
            service.web.enabled,
        ));
        commands.push(state_command(
            "-setsecurewebproxystate",
            &service.service,
            service.secure_web.enabled,
        ));
        commands.push(state_command(
            "-setautoproxystate",
            &service.service,
            service.auto_proxy.enabled,
        ));
    }

    Ok(commands)
}

fn state_command(option: &str, service: &str, enabled: bool) -> NetworkSetupCommand {
    NetworkSetupCommand::new([
        option.to_string(),
        service.to_string(),
        if enabled { "on" } else { "off" }.to_string(),
    ])
}

fn endpoint_command(
    option: &str,
    service: &str,
    endpoint: &MacProxyEndpoint,
) -> NetworkSetupCommand {
    NetworkSetupCommand::new([
        option.to_string(),
        service.to_string(),
        endpoint.server.clone(),
        endpoint.port.to_string(),
        // Explicitly keep authenticated proxy support off. Authenticated
        // baselines are rejected before a plan is created.
        "off".to_string(),
    ])
}

#[cfg(target_os = "macos")]
fn execute_plan(plan: &[NetworkSetupCommand]) -> NetworkResult<()> {
    for command in plan {
        let output = run_networksetup(&command.arguments)?;
        output_text("write macOS system proxy", output)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_networksetup(arguments: &[String]) -> NetworkResult<Output> {
    Command::new(NETWORKSETUP)
        .args(arguments)
        .output()
        .map_err(|error| NetworkError::adapter("execute networksetup", error.to_string()))
}

#[cfg(target_os = "macos")]
fn output_text(operation: &'static str, output: Output) -> NetworkResult<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(NetworkError::adapter(
            operation,
            if stderr.is_empty() {
                format!("networksetup exited with {}", output.status)
            } else {
                stderr
            },
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| NetworkError::adapter(operation, error.to_string()))
}

fn parse_network_services(output: &str) -> NetworkResult<Vec<(String, bool)>> {
    let mut services = Vec::new();
    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("An asterisk") {
            continue;
        }
        let (name, enabled) = match line.strip_prefix('*') {
            Some(disabled) => (disabled.trim(), false),
            None => (line, true),
        };
        if name.is_empty() {
            return Err(NetworkError::adapter(
                "parse macOS network services",
                "networksetup returned an empty service name",
            ));
        }
        services.push((name.to_string(), enabled));
    }
    services.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(services)
}

fn parse_proxy_endpoint(operation: &'static str, output: &str) -> NetworkResult<MacProxyEndpoint> {
    let fields = parse_fields(output);
    let enabled = parse_bool(
        operation,
        "Enabled",
        required(&fields, operation, "Enabled")?,
    )?;
    let server = required(&fields, operation, "Server")?.to_string();
    let port = required(&fields, operation, "Port")?
        .parse::<u16>()
        .map_err(|error| {
            NetworkError::adapter(operation, format!("invalid proxy port: {error}"))
        })?;
    let authenticated = fields
        .get("Authenticated Proxy Enabled")
        .copied()
        .map(|value| parse_bool(operation, "Authenticated Proxy Enabled", value))
        .transpose()?
        .unwrap_or(false);

    Ok(MacProxyEndpoint {
        enabled,
        server,
        port,
        authenticated,
    })
}

fn parse_auto_proxy(operation: &'static str, output: &str) -> NetworkResult<MacAutoProxyState> {
    let fields = parse_fields(output);
    let raw_url = required(&fields, operation, "URL")?;
    Ok(MacAutoProxyState {
        enabled: parse_bool(
            operation,
            "Enabled",
            required(&fields, operation, "Enabled")?,
        )?,
        // Some releases print `(null)` when no PAC URL exists. Normalize it so
        // restoration never creates a literal `(null)` URL.
        url: if raw_url.eq_ignore_ascii_case("(null)") {
            String::new()
        } else {
            raw_url.to_string()
        },
    })
}

fn parse_fields(output: &str) -> BTreeMap<&str, &str> {
    output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

fn required<'a>(
    fields: &'a BTreeMap<&str, &str>,
    operation: &'static str,
    key: &'static str,
) -> NetworkResult<&'a str> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| NetworkError::adapter(operation, format!("networksetup omitted {key}")))
}

fn parse_bool(operation: &'static str, field: &'static str, value: &str) -> NetworkResult<bool> {
    match value.to_ascii_lowercase().as_str() {
        "yes" | "on" | "1" | "true" => Ok(true),
        "no" | "off" | "0" | "false" => Ok(false),
        _ => Err(NetworkError::adapter(
            operation,
            format!("invalid {field} value: {value}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(enabled: bool, server: &str, port: u16) -> MacProxyEndpoint {
        MacProxyEndpoint {
            enabled,
            server: server.to_string(),
            port,
            authenticated: false,
        }
    }

    fn service(name: &str, service_enabled: bool) -> MacNetworkServiceProxy {
        MacNetworkServiceProxy {
            service: name.to_string(),
            service_enabled,
            web: endpoint(true, "127.0.0.1", 17890),
            secure_web: endpoint(true, "127.0.0.1", 17890),
            auto_proxy: MacAutoProxyState {
                enabled: false,
                url: "https://pac.example.invalid/config.pac".to_string(),
            },
        }
    }

    #[test]
    fn parses_networksetup_proxy_and_pac_output() {
        let endpoint = parse_proxy_endpoint(
            "test",
            "Enabled: Yes\nServer: 127.0.0.1\nPort: 17890\nAuthenticated Proxy Enabled: 0\n",
        )
        .unwrap();
        let pac = parse_auto_proxy(
            "test",
            "URL: https://pac.example.invalid/proxy.pac?next=https://example.invalid\nEnabled: No\n",
        )
        .unwrap();

        assert_eq!(endpoint, super::tests::endpoint(true, "127.0.0.1", 17890));
        assert_eq!(
            pac,
            MacAutoProxyState {
                enabled: false,
                url: "https://pac.example.invalid/proxy.pac?next=https://example.invalid"
                    .to_string(),
            }
        );
        assert_eq!(
            parse_auto_proxy("test", "URL: (null)\nEnabled: No\n").unwrap(),
            MacAutoProxyState {
                enabled: false,
                url: String::new(),
            }
        );
    }

    #[test]
    fn parses_enabled_and_disabled_network_services() {
        let parsed = parse_network_services(
            "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\n*USB 10/100/1000 LAN\n",
        )
        .unwrap();

        assert_eq!(
            parsed,
            vec![
                ("USB 10/100/1000 LAN".to_string(), false),
                ("Wi-Fi".to_string(), true),
            ]
        );
    }

    #[test]
    fn command_plan_is_disable_then_write_then_enable_and_skips_disabled_services() {
        let enabled = service("Wi-Fi; $(not-a-shell)", true);
        let disabled = service("Disabled Ethernet", false);
        let plan = plan_write(&MacOsProxySnapshot {
            services: vec![enabled, disabled],
        })
        .unwrap();

        let arguments: Vec<Vec<String>> =
            plan.into_iter().map(|command| command.arguments).collect();
        assert_eq!(arguments.len(), 12);
        assert_eq!(
            arguments[0],
            vec!["-setwebproxystate", "Wi-Fi; $(not-a-shell)", "off"]
        );
        assert_eq!(
            arguments[3],
            vec![
                "-setwebproxy",
                "Wi-Fi; $(not-a-shell)",
                "127.0.0.1",
                "17890",
                "off"
            ]
        );
        assert_eq!(
            arguments[7],
            vec![
                "-setautoproxyurl",
                "Wi-Fi; $(not-a-shell)",
                "https://pac.example.invalid/config.pac"
            ]
        );
        assert_eq!(
            arguments[11],
            vec!["-setautoproxystate", "Wi-Fi; $(not-a-shell)", "off"]
        );
        assert!(
            arguments
                .iter()
                .flatten()
                .all(|argument| !argument.contains("Disabled Ethernet"))
        );
    }

    #[test]
    fn every_enabled_service_is_disabled_before_any_endpoint_write() {
        let plan = plan_write(&MacOsProxySnapshot {
            services: vec![service("Ethernet", true), service("Wi-Fi", true)],
        })
        .unwrap();
        let arguments: Vec<Vec<String>> =
            plan.into_iter().map(|command| command.arguments).collect();

        let first_endpoint = arguments
            .iter()
            .position(|arguments| arguments[0] == "-setwebproxy")
            .unwrap();
        assert_eq!(first_endpoint, 6);
        assert!(
            arguments[..first_endpoint]
                .iter()
                .all(|arguments| arguments[0].ends_with("proxystate") && arguments[2] == "off")
        );
        assert!(
            arguments[arguments.len() - 6..]
                .iter()
                .all(|arguments| arguments[0].ends_with("proxystate"))
        );
    }

    #[test]
    fn authenticated_baseline_is_rejected_before_a_plan_exists() {
        let mut authenticated = service("Wi-Fi", true);
        authenticated.web.authenticated = true;
        let snapshot = MacOsProxySnapshot {
            services: vec![authenticated],
        };

        assert!(matches!(
            plan_write(&snapshot).unwrap_err(),
            NetworkError::AuthenticatedProxyBaseline {
                proxy_kind: "HTTP",
                ..
            }
        ));
        assert!(matches!(
            MacOsProxyAdapter
                .build_via_owned_state(&snapshot, LoopbackProxyTarget::new(17890).unwrap()),
            Err(NetworkError::AuthenticatedProxyBaseline { .. })
        ));
    }

    #[test]
    fn owned_state_changes_only_enabled_services_and_disables_pac() {
        let enabled = service("Wi-Fi", true);
        let disabled = service("Disabled Ethernet", false);
        let baseline = MacOsProxySnapshot {
            services: vec![enabled, disabled.clone()],
        };

        let owned = MacOsProxyAdapter
            .build_via_owned_state(&baseline, LoopbackProxyTarget::new(19090).unwrap())
            .unwrap();

        assert_eq!(owned.services[0].web, endpoint(true, "127.0.0.1", 19090));
        assert!(!owned.services[0].auto_proxy.enabled);
        assert_eq!(owned.services[1], disabled);
    }

    #[test]
    fn recovery_merge_restores_owned_fields_but_preserves_external_endpoint() {
        let baseline = MacOsProxySnapshot {
            services: vec![MacNetworkServiceProxy {
                service: "Wi-Fi".to_string(),
                service_enabled: true,
                web: endpoint(false, "baseline", 8080),
                secure_web: endpoint(false, "baseline", 8443),
                auto_proxy: MacAutoProxyState {
                    enabled: true,
                    url: "https://baseline/pac".to_string(),
                },
            }],
        };
        let owned = MacOsProxyAdapter
            .build_via_owned_state(&baseline, LoopbackProxyTarget::new(17890).unwrap())
            .unwrap();
        let mut current = owned.clone();
        current.services[0].web.server = "external".to_string();
        current.services[0].web.port = 9090;

        let merged = MacOsProxyAdapter
            .merge_recovery_state(&current, &baseline, &owned)
            .unwrap();

        assert_eq!(
            merged.services[0].web.enabled,
            baseline.services[0].web.enabled
        );
        assert_eq!(merged.services[0].web.server, "external");
        assert_eq!(merged.services[0].web.port, 9090);
        assert_eq!(
            merged.services[0].secure_web,
            baseline.services[0].secure_web
        );
        assert_eq!(
            merged.services[0].auto_proxy,
            baseline.services[0].auto_proxy
        );
    }
}
