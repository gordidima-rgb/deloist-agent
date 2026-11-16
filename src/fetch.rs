//! Загрузка страниц для бота. Адрес часто подсказывает не человек, а текст со страницы, поэтому
//! бот не должен дотягиваться до этой машины и локальной сети: ни напрямую, ни через редирект,
//! ни через домен, который указывает на внутренний адрес.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::Url;

const MAX_REDIRECTS: usize = 5;
const MAX_BODY: usize = 2 * 1024 * 1024;

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let internal = ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0);
    // 198.18.0.0/15 не закрываем: так работает fake-ip у VPN-клиентов, и внутренней сети там нет.
    !internal
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let [first, second, ..] = ip.segments();
    let internal = ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xffc0) == 0xfec0
        || (first == 0x2001 && second == 0x0db8)
        || first == 0x2002
        || (first == 0x0064 && second == 0xff9b);
    !internal
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

/// Адреса хоста, если все они разрешены. Если хоть один закрыт, не ходим совсем.
async fn resolve(url: &Url, allowed: &(dyn Fn(IpAddr) -> bool + Sync)) -> Result<Vec<SocketAddr>, String> {
    let host = url.host_str().ok_or("в адресе нет хоста")?;
    let port = url.port_or_known_default().ok_or("в адресе нет порта")?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((bare, port))
            .await
            .map_err(|e| format!("не нашёл адрес {bare}: {e}"))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(format!("не нашёл адрес {bare}"));
    }
    if addrs.iter().any(|addr| !allowed(addr.ip())) {
        return Err(format!("адрес {bare} закрыт: это эта машина или локальная сеть"));
    }
    Ok(addrs)
}

pub struct Page {
    pub status: u16,
    pub kind: String,
    pub body: String,
}

pub async fn fetch_page(start: &str, allowed: &(dyn Fn(IpAddr) -> bool + Sync)) -> Result<Page, String> {
    let mut url = Url::parse(start).map_err(|_| "нужен http(s) URL".to_string())?;
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Err("нужен http(s) URL".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("адрес с логином и паролем не поддерживается".into());
        }
        let addrs = resolve(&url, allowed).await?;

        // Соединяемся именно с проверенным адресом: повторный DNS-запрос мог бы вернуть другой.
        // Прокси отключён, иначе адрес выбирал бы он. Редиректы проверяем сами.
        let host = url.host_str().unwrap_or_default().to_string();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(20))
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(url.clone())
            .header("user-agent", "deloist/0.2 (+local agent)")
            .header("accept", "text/html,application/json,text/plain")
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if response.status().is_redirection() {
            let target = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("редирект без адреса")?;
            url = url.join(target).map_err(|_| "битый адрес в редиректе".to_string())?;
            continue;
        }

        let status = response.status().as_u16();
        let kind = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let mut bytes: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
            if bytes.len() >= MAX_BODY {
                bytes.truncate(MAX_BODY);
                break;
            }
        }
        return Ok(Page { status, kind, body: String::from_utf8_lossy(&bytes).into_owned() });
    }
    Err(format!("больше {MAX_REDIRECTS} редиректов"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, StatusCode};
    use axum::routing::get;
    use axum::Router;

    #[test]
    fn internal_addresses_are_closed() {
        for ip in [
            "127.0.0.1", "127.1.2.3", "0.0.0.0", "10.0.0.1", "172.16.0.1", "192.168.1.1", "169.254.169.254",
            "100.64.0.1", "224.0.0.1", "255.255.255.255", "::1", "::", "fc00::1", "fd12::1", "fe80::1",
            "::ffff:127.0.0.1", "::ffff:192.168.0.1", "2002:7f00:1::", "64:ff9b::7f00:1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} должен быть закрыт");
        }
    }

    #[test]
    fn public_addresses_are_open() {
        for ip in ["1.1.1.1", "77.88.8.8", "8.8.8.8", "198.18.0.5", "2a00:1450:4001::1"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} должен быть открыт");
        }
    }

    #[tokio::test]
    async fn loopback_is_refused_in_every_spelling() {
        for url in ["http://127.0.0.1:1/", "http://localhost:1/", "http://2130706433:1/", "http://[::1]:1/", "http://0x7f.1:1/"] {
            let error = fetch_page(url, &is_public_ip).await.err().unwrap_or_else(|| panic!("{url} не закрыт"));
            assert!(error.contains("закрыт"), "{url}: {error}");
        }
    }

    #[tokio::test]
    async fn other_schemes_and_credentials_are_refused() {
        assert!(fetch_page("file:///C:/Windows/win.ini", &is_public_ip).await.is_err());
        assert!(fetch_page("ftp://example.com/", &is_public_ip).await.is_err());
        assert!(fetch_page("http://user:pass@example.com/", &is_public_ip).await.is_err());
    }

    #[tokio::test]
    async fn every_redirect_hop_is_checked() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let inner = format!("http://127.0.0.2:{port}/secret");
        let app = Router::new()
            .route("/ok", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/final")]) }))
            .route("/final", get(|| async { "конец" }))
            .route("/secret", get(|| async { "секрет" }))
            .route("/loop", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/loop")]) }))
            .route(
                "/evil",
                get(move || {
                    let inner = inner.clone();
                    async move { (StatusCode::FOUND, [(header::LOCATION, inner)]) }
                }),
            );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // В тесте «публичным» считаем только 127.0.0.1, а 127.0.0.2 играет внутреннюю сеть.
        let allowed = |ip: IpAddr| ip == IpAddr::V4(Ipv4Addr::LOCALHOST);
        let page = fetch_page(&format!("http://127.0.0.1:{port}/ok"), &allowed).await.unwrap();
        assert_eq!(page.body, "конец");

        let error = fetch_page(&format!("http://127.0.0.1:{port}/evil"), &allowed).await.err().unwrap();
        assert!(error.contains("закрыт"), "{error}");

        let error = fetch_page(&format!("http://127.0.0.1:{port}/loop"), &allowed).await.err().unwrap();
        assert!(error.contains("редирект"), "{error}");
    }
}
