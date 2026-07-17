use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::{StatusCode, header::LOCATION, redirect::Policy};
use url::Url;

use crate::error::{ViaError, ViaResult};

const MAX_REDIRECTS: usize = 5;

pub struct FetchResult {
    pub bytes: Vec<u8>,
}

pub async fn fetch_https(url: &Url, max_bytes: usize) -> ViaResult<FetchResult> {
    let mut current = url.clone();

    for redirect_index in 0..=MAX_REDIRECTS {
        ensure_secure_url(&current)?;
        let host = current
            .host_str()
            .ok_or_else(|| ViaError::InvalidInput("URL 缺少主机名".to_string()))?;
        let port = current.port_or_known_default().unwrap_or(443);
        let address = resolve_public_address(host, port).await?;

        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .user_agent("VIA/0.1 (+https://github.com/cgengqiang-cmyk/voluntary-internet-access)")
            .resolve(host, address)
            .build()
            .map_err(|error| ViaError::Network(error.to_string()))?;

        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|error| ViaError::Network(format!("HTTPS 下载失败：{error}")))?;

        if response.status().is_redirection() {
            if redirect_index == MAX_REDIRECTS {
                return Err(ViaError::Network("HTTPS 重定向次数过多".to_string()));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| ViaError::Network("重定向响应缺少 Location".to_string()))?
                .to_str()
                .map_err(|_| ViaError::Network("重定向地址编码无效".to_string()))?;
            current = current
                .join(location)
                .map_err(|error| ViaError::Network(format!("重定向地址无效：{error}")))?;
            continue;
        }

        if response.status() != StatusCode::OK {
            return Err(ViaError::Network(format!(
                "服务器返回 HTTP {}",
                response.status().as_u16()
            )));
        }

        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(ViaError::Network(format!(
                "下载内容超过 {} 字节限制",
                max_bytes
            )));
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| ViaError::Network(error.to_string()))?;
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                return Err(ViaError::Network(format!(
                    "下载内容超过 {} 字节限制",
                    max_bytes
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(FetchResult { bytes });
    }

    Err(ViaError::Network("无法完成 HTTPS 下载".to_string()))
}

fn ensure_secure_url(url: &Url) -> ViaResult<()> {
    if url.scheme() != "https" {
        return Err(ViaError::InvalidInput(
            "订阅和 Provider 只允许 HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ViaError::InvalidInput(
            "URL 不允许使用 userinfo 凭据".to_string(),
        ));
    }
    if url.host_str().is_none() {
        return Err(ViaError::InvalidInput("URL 缺少主机名".to_string()));
    }
    Ok(())
}

async fn resolve_public_address(host: &str, port: u16) -> ViaResult<SocketAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(ViaError::InvalidInput(
                "URL 不允许访问本机或私有网络地址".to_string(),
            ));
        }
        return Ok(SocketAddr::new(ip, port));
    }

    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| ViaError::Network(format!("DNS 解析失败：{error}")))?;
    addresses
        .into_iter()
        .find(|address| is_public_ip(address.ip()))
        .ok_or_else(|| ViaError::InvalidInput("主机只解析到私有或保留地址".to_string()))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || ip.is_unspecified()
    {
        return false;
    }
    let octets = ip.octets();
    !matches!(
        octets,
        [100, 64..=127, _, _] | [192, 0, 0, _] | [198, 18..=19, _, _] | [240..=255, _, _, _]
    )
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_and_userinfo() {
        assert!(ensure_secure_url(&Url::parse("http://example.com/a").unwrap()).is_err());
        assert!(ensure_secure_url(&Url::parse("https://u:p@example.com/a").unwrap()).is_err());
        assert!(ensure_secure_url(&Url::parse("https://example.com/a").unwrap()).is_ok());
    }

    #[test]
    fn blocks_private_and_reserved_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.1.1",
            "192.168.1.1",
            "198.18.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} must be blocked");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
