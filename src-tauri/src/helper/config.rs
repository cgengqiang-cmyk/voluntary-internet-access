use std::{fs, io::Write};

use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use super::{HelperError, HelperLayout, HelperResult};

pub const MAX_TUN_CONFIG_BYTES: usize = 2 * 1024 * 1024;
const MAX_YAML_NODES: usize = 100_000;
const MAX_YAML_DEPTH: usize = 64;

pub(crate) fn install_config(
    layout: &HelperLayout,
    yaml: &str,
    expected_sha256: &str,
) -> HelperResult<()> {
    validate_tun_config(yaml.as_bytes())?;
    if digest(yaml.as_bytes()) != expected_sha256 {
        return Err(HelperError::InvalidRequest(
            "configuration digest does not match its contents".to_string(),
        ));
    }
    fs::create_dir_all(&layout.runtime_dir)
        .map_err(|source| HelperError::io("create fixed helper runtime directory", source))?;

    let mut temporary = tempfile::NamedTempFile::new_in(&layout.runtime_dir)
        .map_err(|source| HelperError::io("create temporary privileged configuration", source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| HelperError::io("restrict privileged configuration", source))?;
    }
    temporary
        .write_all(yaml.as_bytes())
        .map_err(|source| HelperError::io("write privileged configuration", source))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|source| HelperError::io("flush privileged configuration", source))?;
    temporary
        .persist(&layout.config)
        .map_err(|error| HelperError::io("replace privileged configuration", error.error))?;
    Ok(())
}

pub(crate) fn load_and_validate_config(layout: &HelperLayout) -> HelperResult<(Vec<u8>, String)> {
    layout.verify_config()?;
    let bytes = fs::read(&layout.config)
        .map_err(|source| HelperError::io("read privileged configuration", source))?;
    validate_tun_config(&bytes)?;
    let checksum = digest(&bytes);
    Ok((bytes, checksum))
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn validate_tun_config(bytes: &[u8]) -> HelperResult<()> {
    if bytes.is_empty() || bytes.len() > MAX_TUN_CONFIG_BYTES {
        return Err(HelperError::UnsafeConfiguration(format!(
            "configuration must be 1-{MAX_TUN_CONFIG_BYTES} bytes"
        )));
    }
    let document: Value = serde_yaml::from_slice(bytes)
        .map_err(|_| HelperError::UnsafeConfiguration("invalid YAML".to_string()))?;
    enforce_budget(&document)?;
    let root = document
        .as_mapping()
        .ok_or_else(|| HelperError::UnsafeConfiguration("root must be a mapping".to_string()))?;

    require_integer(root, "mixed-port", |port| port > 0)?;
    for disabled_port in ["port", "socks-port", "redir-port", "tproxy-port"] {
        require_integer(root, disabled_port, |port| port == 0)?;
    }
    require_bool(root, "allow-lan", false)?;
    require_string(root, "bind-address", |value| {
        value == "127.0.0.1" || value == "::1"
    })?;
    require_string(root, "external-controller", is_loopback_endpoint)?;
    require_string(root, "secret", |value| {
        (32..=128).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    })?;

    let tun = get(root, "tun")
        .and_then(Value::as_mapping)
        .ok_or_else(|| HelperError::UnsafeConfiguration("tun mapping is required".to_string()))?;
    require_bool(tun, "enable", true)?;
    require_bool(tun, "auto-route", true)?;
    require_bool(tun, "auto-detect-interface", true)?;
    require_bool(tun, "strict-route", true)?;
    require_string(tun, "stack", |value| value == "mixed")?;

    reject_dangerous_fields(&document, None, "root")?;
    validate_provider_sections(root)?;
    Ok(())
}

fn validate_provider_sections(root: &Mapping) -> HelperResult<()> {
    for section in ["proxy-providers", "rule-providers"] {
        let Some(providers) = get(root, section) else {
            continue;
        };
        let providers = providers.as_mapping().ok_or_else(|| {
            HelperError::UnsafeConfiguration(format!("{section} must be a mapping"))
        })?;
        for definition in providers.values() {
            let definition = definition.as_mapping().ok_or_else(|| {
                HelperError::UnsafeConfiguration(format!("{section} entry must be a mapping"))
            })?;
            if get(definition, "type").and_then(Value::as_str) != Some("inline") {
                return Err(HelperError::UnsafeConfiguration(format!(
                    "privileged {section} entries must be inline; helper-managed file providers are not implemented"
                )));
            }
        }
    }
    Ok(())
}

fn reject_dangerous_fields(
    value: &Value,
    parent: Option<&str>,
    location: &str,
) -> HelperResult<()> {
    match value {
        Value::Mapping(mapping) => {
            for (raw_key, child) in mapping {
                let key = raw_key.as_str().ok_or_else(|| {
                    HelperError::UnsafeConfiguration("all mapping keys must be strings".to_string())
                })?;
                let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
                if normalized == "type"
                    && child
                        .as_str()
                        .is_some_and(|value| value.eq_ignore_ascii_case("tailscale"))
                {
                    return Err(HelperError::UnsafeConfiguration(format!(
                        "stateful proxy type `{}` is not allowed at {location}",
                        child.as_str().unwrap_or_default()
                    )));
                }
                if (normalized == "secret" && location != "root")
                    || is_dangerous_key(&normalized, parent)
                {
                    return Err(HelperError::UnsafeConfiguration(format!(
                        "field `{key}` is not allowed at {location}"
                    )));
                }
                reject_dangerous_fields(child, Some(&normalized), &format!("{location}.{key}"))?;
            }
        }
        Value::Sequence(sequence) => {
            for (index, child) in sequence.iter().enumerate() {
                reject_dangerous_fields(child, parent, &format!("{location}[{index}]"))?;
            }
        }
        Value::Tagged(_) => {
            return Err(HelperError::UnsafeConfiguration(
                "YAML tags are not allowed".to_string(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn is_dangerous_key(key: &str, parent: Option<&str>) -> bool {
    if key == "path" {
        return !matches!(parent, Some("ws-opts" | "http-opts" | "h2-opts"));
    }
    key.ends_with("-path")
        || matches!(
            key,
            "command"
                | "commands"
                | "cmd"
                | "exec"
                | "executable"
                | "program"
                | "binary"
                | "script"
                | "cwd"
                | "working-dir"
                | "working-directory"
                | "state-dir"
                | "private-key"
                | "private-key-file"
                | "client-key"
                | "client-cert"
                | "certificate"
                | "ca-file"
                | "unix-socket"
                | "socket"
                | "interface"
                | "interface-name"
                | "routing-mark"
                | "listeners"
                | "authentication"
                | "external-ui"
                | "external-ui-url"
                | "external-controller-tls"
                | "iptables"
                | "ebpf"
        )
}

fn enforce_budget(root: &Value) -> HelperResult<()> {
    let mut nodes = 0usize;
    let mut stack = vec![(root, 0usize)];
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_YAML_NODES || depth > MAX_YAML_DEPTH {
            return Err(HelperError::UnsafeConfiguration(
                "YAML complexity limit exceeded".to_string(),
            ));
        }
        match value {
            Value::Mapping(mapping) => {
                for (key, value) in mapping {
                    stack.push((key, depth + 1));
                    stack.push((value, depth + 1));
                }
            }
            Value::Sequence(sequence) => {
                stack.extend(sequence.iter().map(|value| (value, depth + 1)));
            }
            Value::Tagged(tagged) => stack.push((&tagged.value, depth + 1)),
            _ => {}
        }
    }
    Ok(())
}

fn require_integer(
    mapping: &Mapping,
    key: &'static str,
    predicate: impl FnOnce(u64) -> bool,
) -> HelperResult<()> {
    let value = get(mapping, key)
        .and_then(Value::as_u64)
        .ok_or_else(|| HelperError::UnsafeConfiguration(format!("{key} must be an integer")))?;
    if !predicate(value) {
        return Err(HelperError::UnsafeConfiguration(format!(
            "{key} has an unsafe value"
        )));
    }
    Ok(())
}

fn require_bool(mapping: &Mapping, key: &'static str, expected: bool) -> HelperResult<()> {
    if get(mapping, key).and_then(Value::as_bool) != Some(expected) {
        return Err(HelperError::UnsafeConfiguration(format!(
            "{key} must be {expected}"
        )));
    }
    Ok(())
}

fn require_string(
    mapping: &Mapping,
    key: &'static str,
    predicate: impl FnOnce(&str) -> bool,
) -> HelperResult<()> {
    let value = get(mapping, key)
        .and_then(Value::as_str)
        .ok_or_else(|| HelperError::UnsafeConfiguration(format!("{key} must be a string")))?;
    if !predicate(value) {
        return Err(HelperError::UnsafeConfiguration(format!(
            "{key} has an unsafe value"
        )));
    }
    Ok(())
}

fn is_loopback_endpoint(value: &str) -> bool {
    let Some((host, port)) = value.rsplit_once(':') else {
        return false;
    };
    matches!(
        host.trim_matches(['[', ']']),
        "127.0.0.1" | "::1" | "localhost"
    ) && port.parse::<u16>().is_ok_and(|port| port > 0)
}

fn get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAFE: &str = r#"
mixed-port: 18900
port: 0
socks-port: 0
redir-port: 0
tproxy-port: 0
allow-lan: false
bind-address: 127.0.0.1
external-controller: 127.0.0.1:18901
secret: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
tun:
  enable: true
  stack: mixed
  auto-route: true
  auto-detect-interface: true
  strict-route: true
dns: {enable: true, nameserver: [system]}
proxies: []
rules: [MATCH,DIRECT]
"#;

    #[test]
    fn accepts_app_controlled_tun_shape() {
        validate_tun_config(SAFE.as_bytes()).unwrap();
    }

    #[test]
    fn rejects_command_and_filesystem_path_fields() {
        for fragment in [
            "proxies: [{name: bad, type: ss, command: calc.exe}]",
            "proxies: [{name: bad, type: ssh, private-key: /tmp/key}]",
            "proxies: [{name: bad, type: ss, state-dir: /tmp/state}]",
            "external-ui: /tmp/ui",
        ] {
            let candidate = format!("{SAFE}\n{fragment}\n");
            assert!(validate_tun_config(candidate.as_bytes()).is_err());
        }
    }

    #[test]
    fn rejects_tailscale_even_without_an_explicit_state_directory() {
        let candidate =
            format!("{SAFE}\nproxies: [{{name: bad, type: tailscale, auth-key: tskey-example}}]\n");
        assert!(validate_tun_config(candidate.as_bytes()).is_err());
    }

    #[test]
    fn rejects_non_loopback_controller_and_disabled_tun() {
        assert!(
            validate_tun_config(SAFE.replace("127.0.0.1:18901", "0.0.0.0:18901").as_bytes())
                .is_err()
        );
        assert!(
            validate_tun_config(SAFE.replace("enable: true", "enable: false").as_bytes()).is_err()
        );
    }

    #[test]
    fn file_providers_are_not_accepted_by_privileged_boundary() {
        let candidate = format!(
            "{SAFE}\nproxy-providers:\n  nodes: {{type: file, path: providers/nodes.yaml}}\n"
        );
        assert!(validate_tun_config(candidate.as_bytes()).is_err());
    }
}
