use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::{ViaError, ViaResult},
    model::ProxyMode,
};

#[derive(Clone)]
pub struct ControllerClient {
    base_url: String,
    secret: String,
    client: Client,
}

impl std::fmt::Debug for ControllerClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControllerClient")
            .field("base_url", &self.base_url)
            .field("secret", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerProxyNode {
    pub name: String,
    pub protocol: String,
    pub delay_ms: Option<u32>,
    pub available: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerProxyGroup {
    pub name: String,
    pub group_type: String,
    pub selected: String,
    pub proxies: Vec<ControllerProxyNode>,
}

impl ControllerClient {
    pub fn new(port: u16, secret: String) -> ViaResult<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|error| ViaError::Core(error.to_string()))?;
        Ok(Self {
            base_url: format!("http://127.0.0.1:{port}"),
            secret,
            client,
        })
    }

    pub async fn version(&self) -> ViaResult<String> {
        let response = self.get("/version").await?;
        let payload: Value = response
            .json()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?;
        payload
            .get("version")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| ViaError::Core("Mihomo /version 响应无效".to_string()))
    }

    pub async fn set_mode(&self, mode: ProxyMode) -> ViaResult<()> {
        let mode = match mode {
            ProxyMode::Rule => "rule",
            ProxyMode::Global => "global",
            ProxyMode::Direct => "direct",
        };
        self.request(reqwest::Method::PATCH, "/configs")
            .json(&json!({ "mode": mode }))
            .send()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?
            .error_for_status()
            .map_err(|error| ViaError::Core(error.to_string()))?;
        Ok(())
    }

    pub async fn proxy_groups(&self) -> ViaResult<Vec<ControllerProxyGroup>> {
        let response = self.get("/proxies").await?;
        let payload: Value = response
            .json()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?;
        let proxies = payload
            .get("proxies")
            .and_then(Value::as_object)
            .ok_or_else(|| ViaError::Core("Mihomo /proxies 响应无效".to_string()))?;

        let mut groups = Vec::new();
        for (name, value) in proxies {
            let Some(all) = value.get("all").and_then(Value::as_array) else {
                continue;
            };
            let selected = value
                .get("now")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let group_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Selector")
                .to_string();
            let nodes = all
                .iter()
                .filter_map(Value::as_str)
                .map(|node_name| {
                    let history = proxies
                        .get(node_name)
                        .and_then(|node| node.get("history"))
                        .and_then(Value::as_array);
                    let delay_ms = history
                        .and_then(|entries| entries.last())
                        .and_then(|entry| entry.get("delay"))
                        .and_then(Value::as_u64)
                        .and_then(|delay| u32::try_from(delay).ok())
                        .filter(|delay| *delay > 0);
                    ControllerProxyNode {
                        name: node_name.to_string(),
                        protocol: proxies
                            .get(node_name)
                            .and_then(|node| node.get("type"))
                            .and_then(Value::as_str)
                            .unwrap_or("Proxy")
                            .to_string(),
                        delay_ms,
                        available: delay_ms.is_some(),
                    }
                })
                .collect();
            groups.push(ControllerProxyGroup {
                name: name.clone(),
                group_type,
                selected,
                proxies: nodes,
            });
        }
        groups.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(groups)
    }

    pub async fn select_proxy(&self, group: &str, proxy: &str) -> ViaResult<()> {
        let path = format!("/proxies/{}", encode_component(group));
        self.request(reqwest::Method::PUT, &path)
            .json(&json!({ "name": proxy }))
            .send()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?
            .error_for_status()
            .map_err(|error| ViaError::Core(error.to_string()))?;
        Ok(())
    }

    pub async fn test_delay(&self, proxy: &str) -> ViaResult<u32> {
        let path = format!(
            "/proxies/{}/delay?url={}&timeout=5000&expected=204",
            encode_component(proxy),
            encode_component("https://www.gstatic.com/generate_204")
        );
        let response = self.get(&path).await?;
        let payload: Value = response
            .json()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?;
        payload
            .get("delay")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| ViaError::Core("测速响应无效".to_string()))
    }

    async fn get(&self, path: &str) -> ViaResult<reqwest::Response> {
        let response = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .map_err(|error| ViaError::Core(error.to_string()))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(ViaError::Core("Mihomo 控制密钥被拒绝".to_string()));
        }
        response
            .error_for_status()
            .map_err(|error| ViaError::Core(error.to_string()))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{}", self.base_url, path))
            .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
    }
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_component;

    #[test]
    fn encodes_proxy_names_as_rfc3986_components() {
        assert_eq!(encode_component("香港 / A"), "%E9%A6%99%E6%B8%AF%20%2F%20A");
        assert_eq!(encode_component("a+b"), "a%2Bb");
    }
}
