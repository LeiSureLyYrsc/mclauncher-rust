use crate::meta::{dashed_uuid, mirror_url};
use crate::model::{Account, AccountKind, McResult, Source};
use md5::{Digest, Md5};
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

/// Java `UUID.nameUUIDFromBytes("OfflinePlayer:{name}")`.
pub fn offline_uuid(name: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(format!("OfflinePlayer:{name}").as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

pub fn validate_player_name(name: &str) -> McResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("没有填写游戏名字".into());
    }
    if name.chars().count() > 16 {
        return Err("游戏名字不能超过 16 个字符".into());
    }
    if name.chars().any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err("游戏名字包含不能使用的字符".into());
    }
    Ok(name.to_string())
}

pub fn offline_account(name: &str) -> McResult<Account> {
    let name = validate_player_name(name)?;
    Ok(Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Offline,
        uuid: offline_uuid(&name),
        access_token: Uuid::new_v4().simple().to_string(),
        user_type: "legacy".into(),
        server: None,
        name,
    })
}

pub fn candidate_roots(input: &str) -> McResult<Vec<String>> {
    let mut s = input.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Err("请填写认证服务器地址".into());
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        s = format!("https://{s}");
    }
    for suffix in [
        "/authserver/authenticate",
        "/authserver/refresh",
        "/sessionserver/session/minecraft/hasJoined",
        "/authserver",
    ] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.trim_end_matches('/').to_string();
        }
    }
    let url = url::Url::parse(&s).map_err(|_| format!("认证服务器地址无效: {s}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("认证服务器只支持 http 或 https".into());
    }
    let path = url.path().trim_end_matches('/');
    let origin = url.origin().ascii_serialization();
    if path.is_empty() {
        Ok(vec![format!("{origin}/api/yggdrasil"), origin])
    } else {
        Ok(vec![s])
    }
}

pub async fn yggdrasil_login(
    http: &reqwest::Client,
    server: &str,
    username: &str,
    password: &str,
    client_token: &str,
) -> McResult<Account> {
    let username = username.trim();
    let password = password.trim();
    if username.is_empty() || password.is_empty() {
        return Err("请填写账号和密码".into());
    }
    let body = json!({
        "agent": {"name": "Minecraft", "version": 1},
        "username": username,
        "password": password,
        "requestUser": true,
        "clientToken": client_token
    });
    let roots = candidate_roots(server)?;
    let (root, value) = post_first_ok(http, &roots, "authenticate", &body).await?;
    account_from_auth(&root, &value)
}

pub async fn yggdrasil_refresh(
    http: &reqwest::Client,
    account: &Account,
    client_token: &str,
) -> McResult<Account> {
    let server = account.server.clone().ok_or("这个账号没有认证服务器")?;
    let body = json!({
        "accessToken": account.access_token,
        "clientToken": client_token,
        "requestUser": true
    });
    let roots = candidate_roots(&server)?;
    let (root, value) = post_first_ok(http, &roots, "refresh", &body).await?;
    let mut next = account_from_auth(&root, &value)?;
    next.id = account.id.clone();
    Ok(next)
}

fn account_from_auth(root: &str, value: &Value) -> McResult<Account> {
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        let msg = value
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .unwrap_or(err);
        return Err(format!("登录失败: {msg}"));
    }
    let token = value
        .get("accessToken")
        .and_then(|v| v.as_str())
        .ok_or("登录失败: 服务器没有返回 accessToken")?
        .to_string();
    let profile = value.get("selectedProfile").cloned().or_else(|| {
        value
            .get("availableProfiles")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .cloned()
    });
    let profile = profile.ok_or("登录失败: 这个账号下面没有游戏角色")?;
    let name = profile
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("登录失败: 角色没有名字")?
        .to_string();
    let id = profile
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("登录失败: 角色没有 UUID")?;
    Ok(Account {
        id: Uuid::new_v4().to_string(),
        kind: AccountKind::Yggdrasil,
        name,
        uuid: dashed_uuid(id),
        access_token: token,
        user_type: "mojang".into(),
        server: Some(root.trim_end_matches('/').to_string()),
    })
}

async fn post_first_ok(
    http: &reqwest::Client,
    roots: &[String],
    action: &str,
    body: &Value,
) -> McResult<(String, Value)> {
    let mut last = "登录失败".to_string();
    for root in roots {
        let url = format!("{}/authserver/{action}", root.trim_end_matches('/'));
        let resp = match http.post(&url).json(body).send().await {
            Ok(r) => r,
            Err(e) => {
                last = format!("无法连接认证服务器: {e}");
                continue;
            }
        };
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.as_u16() == 404 || text.trim_start().starts_with('<') {
            last = format!("认证接口不存在: {url}");
            continue;
        }
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            format!("认证服务器返回的不是 JSON ({status})")
        })?;
        return Ok((root.trim_end_matches('/').to_string(), value));
    }
    Err(last)
}

pub async fn ensure_authlib(
    http: &reqwest::Client,
    dest: &Path,
    source: Source,
) -> McResult<std::path::PathBuf> {
    if dest.exists() && dest.metadata().map(|m| m.len() > 10_000).unwrap_or(false) {
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let meta_url = mirror_url("https://authlib-injector.yushi.moe/artifact/latest.json", source);
    let meta: Value = http
        .get(&meta_url)
        .send()
        .await
        .map_err(|e| format!("获取 authlib-injector 版本失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("获取 authlib-injector 版本失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("authlib-injector 版本信息无法解析: {e}"))?;
    let download = meta
        .get("download_url")
        .and_then(|v| v.as_str())
        .ok_or("authlib-injector 版本信息没有下载地址")?;
    let download = mirror_url(download, source);
    let bytes = http
        .get(&download)
        .send()
        .await
        .map_err(|e| format!("下载 authlib-injector 失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("下载 authlib-injector 失败: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("下载 authlib-injector 失败: {e}"))?;
    if let Some(sum) = meta.pointer("/checksums/sha256").and_then(|v| v.as_str()) {
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let got = format!("{:x}", hasher.finalize());
        if !got.eq_ignore_ascii_case(sum) {
            return Err("authlib-injector 校验失败".into());
        }
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("保存 authlib-injector 失败: {e}"))?;
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        assert_eq!(offline_uuid("Steve"), "5627dd98-e6be-3c21-b8a8-e92344183641");
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn roots_for_littleskin_host() {
        let roots = candidate_roots("littleskin.cn").unwrap();
        assert_eq!(roots[0], "https://littleskin.cn/api/yggdrasil");
        let full = candidate_roots("https://littleskin.cn/api/yggdrasil/").unwrap();
        assert_eq!(full, vec!["https://littleskin.cn/api/yggdrasil".to_string()]);
        let pasted = candidate_roots("https://skin.example.com/api/yggdrasil/authserver/authenticate").unwrap();
        assert_eq!(pasted, vec!["https://skin.example.com/api/yggdrasil".to_string()]);
    }

    #[test]
    fn rejects_empty_name() {
        assert_eq!(validate_player_name("  ").unwrap_err(), "没有填写游戏名字");
    }
}
