//! Загрузка по HTTPS через системный WinHTTP: прокси и корневые сертификаты берутся из Windows,
//! а сам загрузчик остаётся маленьким (свой TLS и reqwest добавили бы мегабайты).

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::*;

use crate::w;

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Разбор адреса: схема, хост, порт и путь с параметрами.
pub fn split_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(format!("адрес не http(s): {url}"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().map_err(|_| "порт в адресе".to_string())?),
        None => (authority, if secure { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err("в адресе нет хоста".to_string());
    }
    Ok((secure, host.to_string(), port, path.to_string()))
}

fn query_number(request: *mut c_void, what: u32) -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        WinHttpQueryHeaders(request, what | WINHTTP_QUERY_FLAG_NUMBER, null(), &mut value as *mut u32 as *mut c_void, &mut size, null_mut())
    };
    (ok != 0).then_some(value)
}

/// Читает ответ кусками. `sink` получает кусок и общий размер (если сервер его назвал);
/// сверх `limit` байт не читаем.
pub fn get(url: &str, limit: u64, sink: &mut dyn FnMut(&[u8], Option<u64>) -> Result<(), String>) -> Result<(), String> {
    let (secure, host, port, path) = split_url(url)?;
    unsafe {
        let session = Handle(WinHttpOpen(w("Deloist-Setup").as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            return Err("нет доступа к сети (WinHTTP)".to_string());
        }
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000);
        let connection = Handle(WinHttpConnect(session.0, w(&host).as_ptr(), port, 0));
        if connection.0.is_null() {
            return Err(format!("не удалось подключиться к {host}"));
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { 0 };
        let request = Handle(WinHttpOpenRequest(connection.0, w("GET").as_ptr(), w(&path).as_ptr(), null(), null(), null(), flags));
        if request.0.is_null() {
            return Err("не удалось создать запрос".to_string());
        }
        if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(format!("нет ответа от {host}: проверь интернет"));
        }
        let status = query_number(request.0, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if !(200..300).contains(&status) {
            return Err(format!("сервер ответил HTTP {status}"));
        }
        let total = query_number(request.0, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
        if total.is_some_and(|n| n > limit) {
            return Err("файл больше допустимого".to_string());
        }
        let mut buffer = vec![0u8; 64 * 1024];
        let mut seen: u64 = 0;
        loop {
            let mut read: u32 = 0;
            if WinHttpReadData(request.0, buffer.as_mut_ptr() as *mut c_void, buffer.len() as u32, &mut read) == 0 {
                return Err("загрузка оборвалась".to_string());
            }
            if read == 0 {
                break;
            }
            seen += u64::from(read);
            if seen > limit {
                return Err("файл больше допустимого".to_string());
            }
            sink(&buffer[..read as usize], total)?;
        }
    }
    Ok(())
}

pub fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    get(url, limit, &mut |chunk, _| {
        out.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::split_url;

    #[test]
    fn splits_urls() {
        assert_eq!(split_url("https://github.com/a/b").unwrap(), (true, "github.com".into(), 443, "/a/b".into()));
        assert_eq!(split_url("http://127.0.0.1:8080/x?y=1").unwrap(), (false, "127.0.0.1".into(), 8080, "/x?y=1".into()));
        assert!(split_url("ftp://x").is_err());
        assert!(split_url("https:///x").is_err());
    }
}
