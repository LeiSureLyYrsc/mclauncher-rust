#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mclauncher_core::{
    default_paths, delete_version as delete_ver, discover_javas, ensure_authlib, http_client, install_version,
    installed_ids, list_versions, load_config, offline_account, prepare_launch, rename_version as rename_ver,
    save_config, spawn_launch, yggdrasil_login, yggdrasil_refresh, AccountKind, Config, Progress, Report, Source,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

struct Core {
    http: reqwest::Client,
    config_path: PathBuf,
    config: Config,
    cancel: Arc<AtomicBool>,
}

struct AppState {
    inner: Mutex<Core>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Patch {
    game_dir: Option<String>,
    java_path: Option<String>,
    max_memory_mb: Option<u32>,
    memory_auto: Option<bool>,
    download_source: Option<String>,
    debug: Option<bool>,
    version_isolation: Option<bool>,
    selected_version: Option<String>,
    setup_done: Option<bool>,
}

fn snapshot(core: &Core) -> Value {
    let installed = installed_ids(core.config.game_path().as_path());
    let java_label = if core.config.java_path.is_empty() {
        discover_javas()
            .first()
            .map(|j| format!("{} (Java {})", j.path.display(), j.major))
            .unwrap_or_default()
    } else {
        core.config.java_path.clone()
    };
    json!({
        "config": core.config,
        "installed": installed,
        "javaLabel": java_label
    })
}

fn authlib_path(config_path: &std::path::Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("authlib-injector.jar")
}

#[tauri::command]
async fn app_state(state: State<'_, AppState>) -> Result<Value, String> {
    let core = state.inner.lock().await;
    Ok(snapshot(&core))
}

#[tauri::command]
async fn save_settings(state: State<'_, AppState>, patch: Patch) -> Result<Value, String> {
    let mut core = state.inner.lock().await;
    if let Some(v) = patch.game_dir {
        if !v.trim().is_empty() {
            core.config.game_dir = v;
        }
    }
    if let Some(v) = patch.java_path {
        core.config.java_path = v;
    }
    if let Some(v) = patch.max_memory_mb {
        core.config.max_memory_mb = v.clamp(256, 32768);
    }
    if let Some(v) = patch.memory_auto {
        core.config.memory_auto = v;
    }
    if let Some(v) = patch.download_source {
        core.config.download_source = Source::parse(&v).as_str().to_string();
    }
    if let Some(v) = patch.debug {
        core.config.debug = v;
    }
    if let Some(v) = patch.version_isolation {
        core.config.version_isolation = v;
    }
    if let Some(v) = patch.selected_version {
        core.config.selected_version = v;
    }
    if let Some(v) = patch.setup_done {
        core.config.setup_done = v;
    }
    save_config(&core.config_path, &core.config)?;
    Ok(snapshot(&core))
}

#[tauri::command]
async fn offline_login(state: State<'_, AppState>, name: String) -> Result<Value, String> {
    let account = offline_account(&name)?;
    let mut core = state.inner.lock().await;
    core.config.accounts.retain(|a| !(a.kind == AccountKind::Offline && a.name == account.name));
    core.config.active_account = account.id.clone();
    core.config.accounts.push(account);
    save_config(&core.config_path, &core.config)?;
    Ok(snapshot(&core))
}

#[tauri::command(rename = "yggdrasil_login")]
async fn yggdrasil_login_cmd(
    state: State<'_, AppState>,
    server: String,
    username: String,
    password: String,
) -> Result<Value, String> {
    let (http, token) = {
        let core = state.inner.lock().await;
        (core.http.clone(), core.config.client_token.clone())
    };
    let account = yggdrasil_login(&http, &server, &username, &password, &token).await?;
    let mut core = state.inner.lock().await;
    core.config
        .accounts
        .retain(|a| !(a.kind == AccountKind::Yggdrasil && a.uuid == account.uuid && a.server == account.server));
    core.config.active_account = account.id.clone();
    core.config.accounts.push(account);
    save_config(&core.config_path, &core.config)?;
    Ok(snapshot(&core))
}

#[tauri::command]
async fn remove_account(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let mut core = state.inner.lock().await;
    core.config.accounts.retain(|a| a.id != id);
    if core.config.active_account == id {
        core.config.active_account = core.config.accounts.first().map(|a| a.id.clone()).unwrap_or_default();
    }
    save_config(&core.config_path, &core.config)?;
    Ok(snapshot(&core))
}

#[tauri::command]
async fn use_account(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let mut core = state.inner.lock().await;
    if !core.config.accounts.iter().any(|a| a.id == id) {
        return Err("没有这个账号".into());
    }
    core.config.active_account = id;
    save_config(&core.config_path, &core.config)?;
    Ok(snapshot(&core))
}

#[tauri::command(rename = "list_versions")]
async fn list_versions_cmd(state: State<'_, AppState>) -> Result<Vec<mclauncher_core::RemoteVersion>, String> {
    let core = state.inner.lock().await;
    list_versions(&core.http, core.config.source(), &core.config.game_path()).await
}

#[tauri::command(rename = "install_version")]
async fn install_version_cmd(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let (http, game, source, cancel) = {
        let core = state.inner.lock().await;
        core.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        (
            core.http.clone(),
            core.config.game_path(),
            core.config.source(),
            core.cancel.clone(),
        )
    };
    let versions = list_versions(&http, source, &game).await?;
    let url = versions
        .iter()
        .find(|v| v.id == id)
        .map(|v| v.url.clone())
        .ok_or_else(|| format!("版本列表里没有 {id}"))?;
    let report: Report = Arc::new(move |p: &Progress| {
        let _ = app.emit("download-progress", p);
    });
    install_version(&http, &game, &id, &url, source, &cancel, &report).await?;
    let mut core = state.inner.lock().await;
    if core.config.selected_version.is_empty() {
        core.config.selected_version = id.clone();
        save_config(&core.config_path, &core.config)?;
    }
    Ok(json!({ "preview": false, "id": id }))
}

#[tauri::command]
async fn cancel_install(state: State<'_, AppState>) -> Result<(), String> {
    let core = state.inner.lock().await;
    core.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
async fn launch_game(state: State<'_, AppState>) -> Result<u32, String> {
    let (http, token, mut account, source, jar_path) = {
        let core = state.inner.lock().await;
        let account = core.config.active().cloned().ok_or("没有填写游戏名字")?;
        (
            core.http.clone(),
            core.config.client_token.clone(),
            account,
            core.config.source(),
            authlib_path(&core.config_path),
        )
    };
    let authlib = if account.kind == AccountKind::Yggdrasil {
        account = yggdrasil_refresh(&http, &account, &token).await?;
        {
            let mut core = state.inner.lock().await;
            if let Some(slot) = core.config.accounts.iter_mut().find(|a| a.id == account.id) {
                *slot = account.clone();
            }
            save_config(&core.config_path, &core.config)?;
        }
        Some(ensure_authlib(&http, &jar_path, source).await?)
    } else {
        None
    };
    let core = state.inner.lock().await;
    let plan = prepare_launch(&core.config, &account, authlib.as_deref())?;
    spawn_launch(&plan)
}

#[tauri::command]
async fn find_java(state: State<'_, AppState>) -> Result<Value, String> {
    let core = state.inner.lock().await;
    if !core.config.java_path.is_empty() {
        let path = std::path::PathBuf::from(&core.config.java_path);
        if path.is_file() {
            let major = mclauncher_core::java::java_version(&path).unwrap_or(0);
            return Ok(json!({ "path": core.config.java_path, "major": major, "found": true }));
        }
    }
    if let Some(j) = discover_javas().into_iter().next() {
        Ok(json!({ "path": j.path, "major": j.major, "found": true }))
    } else {
        Ok(json!({ "path": "", "major": 0, "found": false }))
    }
}

#[tauri::command]
async fn open_game_dir(state: State<'_, AppState>) -> Result<(), String> {
    let core = state.inner.lock().await;
    let dir = core.config.game_path();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_folder(&dir)
}

#[tauri::command(rename = "delete_version")]
async fn delete_version_cmd(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let mut core = state.inner.lock().await;
    delete_ver(&core.config.game_path(), &id)?;
    if core.config.selected_version == id {
        core.config.selected_version = installed_ids(&core.config.game_path()).into_iter().next().unwrap_or_default();
        save_config(&core.config_path, &core.config)?;
    }
    Ok(snapshot(&core))
}

#[tauri::command(rename = "rename_version")]
async fn rename_version_cmd(state: State<'_, AppState>, id: String, name: String) -> Result<Value, String> {
    let mut core = state.inner.lock().await;
    rename_ver(&core.config.game_path(), &id, &name)?;
    if core.config.selected_version == id {
        core.config.selected_version = name;
        save_config(&core.config_path, &core.config)?;
    }
    Ok(snapshot(&core))
}

fn open_folder(path: &std::path::Path) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map_err(|e| format!("打不开目录: {e}"))?;
    Ok(())
}

fn main() {
    let (config_path, default_game) = default_paths();
    let mut config = load_config(&config_path).unwrap_or_default();
    if config.game_dir.trim().is_empty() {
        config.game_dir = default_game.to_string_lossy().to_string();
    }
    let http = http_client().expect("http client");
    tauri::Builder::default()
        .manage(AppState {
            inner: Mutex::new(Core {
                http,
                config_path,
                config,
                cancel: Arc::new(AtomicBool::new(false)),
            }),
        })
        .invoke_handler(tauri::generate_handler![
            app_state,
            save_settings,
            offline_login,
            yggdrasil_login_cmd,
            remove_account,
            use_account,
            list_versions_cmd,
            install_version_cmd,
            cancel_install,
            launch_game,
            find_java,
            open_game_dir,
            delete_version_cmd,
            rename_version_cmd
        ])
        .run(tauri::generate_context!())
        .expect("启动器窗口启动失败");
}
