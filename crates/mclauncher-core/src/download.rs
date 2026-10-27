use crate::meta::{client_artifact, library_files, mirror_url, version_dir};
use crate::model::{McResult, Progress, RemoteVersion, Source};
use futures_util::StreamExt;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

pub type Http = reqwest::Client;
pub type Report = Arc<dyn Fn(&Progress) + Send + Sync>;

pub fn http_client() -> McResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .user_agent("mclauncher-rust/0.1.0")
        .build()
        .map_err(|e| e.to_string())
}

pub async fn list_versions(
    http: &reqwest::Client,
    source: Source,
    game_dir: &Path,
) -> McResult<Vec<RemoteVersion>> {
    let manifest: Value = http
        .get(source.manifest_url())
        .send()
        .await
        .map_err(|e| format!("获取版本列表失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("获取版本列表失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("版本列表无法解析: {e}"))?;
    let installed = installed_ids(game_dir);
    let mut out = Vec::new();
    let Some(versions) = manifest.get("versions").and_then(|v| v.as_array()) else {
        return Err("获取列表失败".into());
    };
    for v in versions {
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let type_name = v.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if !matches!(type_name.as_str(), "release" | "snapshot") {
            continue;
        }
        let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        out.push(RemoteVersion {
            installed: installed.iter().any(|i| i == &id),
            type_label: type_label(&type_name),
            release_time: v.get("releaseTime").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            type_name,
            url: mirror_url(&url, source),
            id,
        });
    }
    Ok(out)
}

fn type_label(t: &str) -> String {
    match t {
        "release" => "正式版".into(),
        "snapshot" => "快照预览版".into(),
        "old_beta" => "早期测试".into(),
        "old_alpha" => "远古版本".into(),
        other => other.into(),
    }
}

pub fn installed_ids(game_dir: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    let dir = game_dir.join("versions");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return ids;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_string();
        if path.join(format!("{name}.json")).is_file() {
            ids.push(name);
        }
    }
    ids.sort_by(|a, b| cmp_version(b, a));
    ids
}

fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = version_parts(a);
    let pb = version_parts(b);
    pa.cmp(&pb)
}

fn version_parts(s: &str) -> Vec<(bool, String, u64)> {
    s.split(|c| c == '.' || c == '-' || c == '+')
        .map(|p| {
            if let Ok(n) = p.parse::<u64>() {
                (true, String::new(), n)
            } else {
                (false, p.to_string(), 0)
            }
        })
        .collect()
}

struct Task {
    url: String,
    dest: PathBuf,
    sha1: Option<String>,
    size: Option<u64>,
    verify: bool,
    label: String,
}

pub async fn install_version(
    http: &reqwest::Client,
    game_dir: &Path,
    id: &str,
    version_url: &str,
    source: Source,
    cancel: &AtomicBool,
    report: &Report,
) -> McResult<()> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("版本号不合法".into());
    }
    let raw: Value = http
        .get(version_url)
        .send()
        .await
        .map_err(|e| format!("下载版本信息失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("下载版本信息失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("版本信息无法解析: {e}"))?;
    if raw.get("inheritsFrom").is_some() {
        return Err("暂不支持继承版本（Forge / Fabric 安装会在后续版本加上）".into());
    }
    let vdir = version_dir(game_dir, id);
    std::fs::create_dir_all(&vdir).map_err(|e| e.to_string())?;
    let json_path = vdir.join(format!("{id}.json"));
    std::fs::write(&json_path, serde_json::to_string_pretty(&raw).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut tasks = Vec::new();
    if let Some(client) = client_artifact(&raw, source) {
        tasks.push(Task {
            url: client.url,
            dest: vdir.join(format!("{id}.jar")),
            sha1: client.sha1,
            size: client.size,
            verify: true,
            label: format!("{id}.jar"),
        });
    } else {
        return Err("版本信息里没有客户端".into());
    }
    for art in library_files(&raw, source) {
        let dest = game_dir.join("libraries").join(&art.path);
        tasks.push(Task {
            label: art.path.clone(),
            url: art.url,
            dest,
            sha1: art.sha1,
            size: art.size,
            verify: !art.native,
        });
    }
    if let Some(index) = raw.get("assetIndex") {
        if let Some(url) = index.get("url").and_then(|v| v.as_str()) {
            let name = index.get("id").and_then(|v| v.as_str()).unwrap_or("legacy");
            tasks.push(Task {
                url: mirror_url(url, source),
                dest: game_dir.join("assets").join("indexes").join(format!("{name}.json")),
                sha1: index.get("sha1").and_then(|v| v.as_str()).map(|s| s.to_string()),
                size: index.get("size").and_then(|v| v.as_u64()),
                verify: true,
                label: format!("assets/indexes/{name}.json"),
            });
        }
    }
    if let Some(file) = raw.pointer("/logging/client/file") {
        if let Some(url) = file.get("url").and_then(|v| v.as_str()) {
            let name = file.get("id").and_then(|v| v.as_str()).unwrap_or("client.xml");
            tasks.push(Task {
                url: mirror_url(url, source),
                dest: game_dir.join("assets").join("log_configs").join(name),
                sha1: file.get("sha1").and_then(|v| v.as_str()).map(|s| s.to_string()),
                size: file.get("size").and_then(|v| v.as_u64()),
                verify: true,
                label: format!("log_configs/{name}"),
            });
        }
    }

    // Asset objects are added after the index is local.
    let index_task_pos = tasks.iter().position(|t| t.label.starts_with("assets/indexes/"));
    let pre_total = tasks.len() as u32;
    download_all(http, &tasks, cancel, report, 0, pre_total.max(1) + 1).await?;

    if let Some(pos) = index_task_pos {
        let index_path = &tasks[pos].dest;
        let text = std::fs::read_to_string(index_path).map_err(|e| format!("读取资源索引失败: {e}"))?;
        let index: Value = serde_json::from_str(&text).map_err(|e| format!("资源索引损坏: {e}"))?;
        let mut asset_tasks = Vec::new();
        if let Some(objects) = index.get("objects").and_then(|v| v.as_object()) {
            for (name, obj) in objects {
                let hash = obj.get("hash").and_then(|v| v.as_str()).unwrap_or("");
                if hash.len() < 2 {
                    continue;
                }
                let rel = format!("{}/{}", &hash[..2], hash);
                let url = mirror_url(&format!("https://resources.download.minecraft.net/{rel}"), source);
                asset_tasks.push(Task {
                    url,
                    dest: game_dir.join("assets").join("objects").join(&hash[..2]).join(hash),
                    sha1: Some(hash.to_string()),
                    size: obj.get("size").and_then(|v| v.as_u64()),
                    verify: false,
                    label: name.clone(),
                });
            }
        }
        let base = pre_total;
        let total = base + asset_tasks.len() as u32;
        download_all(http, &asset_tasks, cancel, report, base, total.max(1)).await?;
        link_legacy_assets(game_dir, &index)?;
    }

    // Extract old-style natives.
    extract_natives(game_dir, &raw, source)?;
    let _ = source;
    Ok(())
}

fn link_legacy_assets(game_dir: &Path, index: &Value) -> McResult<()> {
    let virtual_flag = index.get("virtual").and_then(|v| v.as_bool()).unwrap_or(false);
    let map_res = index.get("map_to_resources").and_then(|v| v.as_bool()).unwrap_or(false);
    if !virtual_flag && !map_res {
        return Ok(());
    }
    let Some(objects) = index.get("objects").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    for (name, obj) in objects {
        let hash = obj.get("hash").and_then(|v| v.as_str()).unwrap_or("");
        if hash.len() < 2 {
            continue;
        }
        let src = game_dir.join("assets").join("objects").join(&hash[..2]).join(hash);
        if !src.is_file() {
            continue;
        }
        if virtual_flag {
            let dest = game_dir.join("assets").join("virtual").join("legacy").join(name);
            link_or_copy(&src, &dest)?;
        }
        if map_res {
            let dest = game_dir.join("resources").join(name);
            link_or_copy(&src, &dest)?;
        }
    }
    Ok(())
}

fn link_or_copy(src: &Path, dest: &Path) -> McResult<()> {
    if dest.is_file() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if std::fs::hard_link(src, dest).is_err() {
        std::fs::copy(src, dest).map_err(|e| format!("整理资源失败: {e}"))?;
    }
    Ok(())
}

fn extract_natives(game_dir: &Path, version: &Value, _source: Source) -> McResult<()> {
    let id = version.get("id").and_then(|v| v.as_str()).unwrap_or("game");
    let natives = version_dir(game_dir, id).join("natives");
    std::fs::create_dir_all(&natives).map_err(|e| e.to_string())?;
    for art in library_files(version, Source::Official) {
        if !art.native {
            continue;
        }
        let jar = game_dir.join("libraries").join(&art.path);
        if !jar.is_file() {
            continue;
        }
        extract_zip(&jar, &natives, &art.extract_exclude)?;
    }
    Ok(())
}

fn extract_zip(jar: &Path, dest: &Path, exclude: &[String]) -> McResult<()> {
    let file = std::fs::File::open(jar).map_err(|e| format!("打开 natives 失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("natives 压缩包损坏: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if name.starts_with("META-INF") || exclude.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let Some(rel) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut f).map_err(|e| format!("解压 natives 失败: {e}"))?;
    }
    Ok(())
}

async fn download_all(
    http: &reqwest::Client,
    tasks: &[Task],
    cancel: &AtomicBool,
    report: &Report,
    completed_base: u32,
    total: u32,
) -> McResult<()> {
    let mut failed = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let completed = completed_base + i as u32;
        if let Err(e) = download_one(http, task, cancel, report, completed, total).await {
            if e == "已取消" {
                return Err(e);
            }
            // Client jar and libraries must succeed. Assets may be retried later.
            if task.label.ends_with(".jar") || task.label.ends_with(".json") || task.label.contains("log_configs") {
                return Err(e);
            }
            failed.push(format!("{} ({e})", task.label));
        }
    }
    if !failed.is_empty() {
        let shown = failed.iter().take(12).cloned().collect::<Vec<_>>().join("\n");
        report(&Progress {
            phase: "warn".into(),
            file: String::new(),
            completed: total,
            total,
            bytes: 0,
            bytes_total: 0,
            speed: 0,
            message: format!("以下文件更新失败，可尝试再次启动游戏重试更新\n{shown}"),
        });
    }
    Ok(())
}

async fn download_one(
    http: &reqwest::Client,
    task: &Task,
    cancel: &AtomicBool,
    report: &Report,
    completed: u32,
    total: u32,
) -> McResult<()> {
    if file_ok(&task.dest, task.size, task.sha1.as_deref(), task.verify)? {
        report(&Progress {
            phase: "skip".into(),
            file: task.label.clone(),
            completed,
            total,
            bytes: task.size.unwrap_or(0),
            bytes_total: task.size.unwrap_or(0),
            speed: 0,
            message: format!("正在下载: {}", task.label),
        });
        return Ok(());
    }
    if let Some(parent) = task.dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    let part = task.dest.with_extension(format!(
        "{}.part",
        task.dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    let mut last_err = String::new();
    for _try in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        match stream_to(http, &task.url, &part, &task.label, task.size, cancel, report, completed, total).await {
            Ok(()) => {
                if let Some(sum) = task.sha1.as_deref() {
                    if task.verify && sha1_file(&part)? != sum {
                        let _ = std::fs::remove_file(&part);
                        last_err = format!("{} 校验失败", task.label);
                        continue;
                    }
                }
                if let Some(parent) = task.dest.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::rename(&part, &task.dest).map_err(|e| format!("保存文件失败: {e}"))?;
                return Ok(());
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                if e == "已取消" {
                    return Err(e);
                }
                last_err = e;
            }
        }
    }
    Err(if last_err.is_empty() { format!("下载失败: {}", task.label) } else { last_err })
}

async fn stream_to(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
    label: &str,
    expected: Option<u64>,
    cancel: &AtomicBool,
    report: &Report,
    completed: u32,
    total: u32,
) -> McResult<()> {
    let resp = http.get(url).send().await.map_err(|e| format!("{label}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{label}: HTTP {}", resp.status()));
    }
    let bytes_total = resp.content_length().or(expected).unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut got = 0u64;
    let started = Instant::now();
    let mut last_emit = Instant::now();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let chunk = chunk.map_err(|e| format!("{label}: {e}"))?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if last_emit.elapsed() > Duration::from_millis(200) {
            let secs = started.elapsed().as_secs_f64().max(0.001);
            report(&Progress {
                phase: "download".into(),
                file: label.to_string(),
                completed,
                total,
                bytes: got,
                bytes_total,
                speed: (got as f64 / secs) as u64,
                message: format!("正在下载: {label}"),
            });
            last_emit = Instant::now();
        }
    }
    file.flush().await.ok();
    Ok(())
}

fn file_ok(path: &Path, size: Option<u64>, sha1: Option<&str>, verify: bool) -> McResult<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let len = path.metadata().map_err(|e| e.to_string())?.len();
    if let Some(size) = size {
        if len != size {
            return Ok(false);
        }
    }
    if verify {
        if let Some(sum) = sha1 {
            return Ok(sha1_file(path)? == sum);
        }
    }
    Ok(true)
}

fn sha1_file(path: &Path) -> McResult<String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha1::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn delete_version(game_dir: &Path, id: &str) -> McResult<()> {
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.is_empty() {
        return Err("版本号不合法".into());
    }
    let dir = version_dir(game_dir, id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除失败: {e}"))?;
    }
    Ok(())
}

pub fn rename_version(game_dir: &Path, id: &str, new_id: &str) -> McResult<()> {
    if new_id.trim().is_empty() {
        return Err("没有填写游戏名字".into());
    }
    let new_id = new_id.trim();
    if new_id.contains('/') || new_id.contains('\\') || new_id.contains("..") {
        return Err("版本号不合法".into());
    }
    let from = version_dir(game_dir, id);
    let to = version_dir(game_dir, new_id);
    if to.exists() {
        return Err("已经有这个版本名称".into());
    }
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    let json_from = from.join(format!("{id}.json"));
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(&json_from).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("id".into(), Value::String(new_id.to_string()));
    }
    std::fs::write(to.join(format!("{new_id}.json")), serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let jar_from = from.join(format!("{id}.jar"));
    if jar_from.is_file() {
        std::fs::rename(&jar_from, to.join(format!("{new_id}.jar"))).map_err(|e| e.to_string())?;
    }
    let natives = from.join("natives");
    if natives.is_dir() {
        let _ = std::fs::rename(&natives, to.join("natives"));
    }
    std::fs::remove_dir_all(&from).ok();
    Ok(())
}

/// Used by the UI when it already has the manifest entry.
pub fn version_url_from_list(list: &[RemoteVersion], id: &str) -> Option<String> {
    list.iter().find(|v| v.id == id).map(|v| v.url.clone())
}
