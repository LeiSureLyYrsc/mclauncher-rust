use crate::java::pick_java;
use crate::meta::{argument_list, classpath_separator, library_files, subst, version_dir};
use crate::model::{Account, AccountKind, Config, LaunchPlan, McResult};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn prepare_launch(cfg: &Config, account: &Account, authlib_jar: Option<&Path>) -> McResult<LaunchPlan> {
    let game = cfg.game_path();
    let id = cfg.selected_version.trim();
    if id.is_empty() {
        return Err("没有选择游戏,请选择或点击更新按钮更新游戏".into());
    }
    let vdir = version_dir(&game, id);
    let json_path = vdir.join(format!("{id}.json"));
    if !json_path.is_file() {
        return Err("没有选择游戏,请选择或点击更新按钮更新游戏".into());
    }
    let version: Value = serde_json::from_str(&std::fs::read_to_string(&json_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("版本配置损坏: {e}"))?;
    let need = version.pointer("/javaVersion/majorVersion").and_then(|v| v.as_u64()).map(|n| n as u32);
    let java = pick_java(&cfg.java_path, need)?;
    let main = version
        .get("mainClass")
        .and_then(|v| v.as_str())
        .ok_or("版本配置里没有主类")?
        .to_string();

    let natives = vdir.join("natives");
    let _ = std::fs::create_dir_all(&natives);
    let assets = game.join("assets");
    let libraries_dir = game.join("libraries");
    let game_directory = if cfg.version_isolation {
        let dir = vdir.join("game");
        std::fs::create_dir_all(&dir).ok();
        let marker = dir.join("mclauncher.txt");
        if !marker.exists() {
            let _ = std::fs::write(&marker, "不要删除此文件,此文件用于mclauncher的游戏目录设置判断\n");
        }
        dir
    } else {
        game.clone()
    };

    let mut cp = Vec::new();
    for art in library_files(&version, cfg.source()) {
        if art.native {
            continue;
        }
        let path = libraries_dir.join(&art.path);
        if path.is_file() {
            cp.push(path.to_string_lossy().to_string());
        }
    }
    let client = vdir.join(format!("{id}.jar"));
    if !client.is_file() {
        return Err(format!("缺少 {id}.jar，请先在更新游戏里下载"));
    }
    cp.push(client.to_string_lossy().to_string());

    let asset_index = version
        .pointer("/assetIndex/id")
        .and_then(|v| v.as_str())
        .or_else(|| version.get("assets").and_then(|v| v.as_str()))
        .unwrap_or("legacy");
    let mut vars = HashMap::new();
    vars.insert("auth_player_name".into(), account.name.clone());
    vars.insert("version_name".into(), id.to_string());
    vars.insert("game_directory".into(), game_directory.to_string_lossy().to_string());
    vars.insert("assets_root".into(), assets.to_string_lossy().to_string());
    vars.insert("game_assets".into(), assets.join("virtual").join("legacy").to_string_lossy().to_string());
    vars.insert("assets_index_name".into(), asset_index.to_string());
    vars.insert("auth_uuid".into(), account.uuid.clone());
    vars.insert("auth_access_token".into(), account.access_token.clone());
    vars.insert("auth_session".into(), format!("{}:{}", account.access_token, account.uuid));
    vars.insert("user_type".into(), account.user_type.clone());
    vars.insert("user_properties".into(), "{}".into());
    vars.insert("version_type".into(), version.get("type").and_then(|v| v.as_str()).unwrap_or("release").to_string());
    vars.insert("natives_directory".into(), natives.to_string_lossy().to_string());
    vars.insert("library_directory".into(), libraries_dir.to_string_lossy().to_string());
    vars.insert("classpath_separator".into(), classpath_separator().to_string());
    vars.insert("launcher_name".into(), "mclauncher-rust".into());
    vars.insert("launcher_version".into(), "0.1.0".into());
    vars.insert("classpath".into(), cp.join(classpath_separator()));
    vars.insert("resolution_width".into(), "854".into());
    vars.insert("resolution_height".into(), "480".into());

    let mut jvm = Vec::new();
    if let Some(jar) = authlib_jar {
        if account.kind == AccountKind::Yggdrasil {
            let server = account.server.clone().unwrap_or_default();
            jvm.push(format!("-javaagent:{}={server}", jar.display()));
            jvm.push("-Dauthlibinjector.side=client".into());
        }
    }
    jvm.push(format!("-Xmx{}M", cfg.memory_mb()));
    jvm.push("-Dfml.ignoreInvalidMinecraftCertificates=true".into());
    jvm.push("-Dfml.ignorePatchDiscrepancies=true".into());

    if version.get("arguments").is_some() {
        let extra = argument_list(version.pointer("/arguments/jvm"), &vars);
        if !extra.iter().any(|a| a.starts_with("-Xmx")) {
            // keep the one we already pushed
        }
        for arg in extra {
            if arg.starts_with("-Xmx") {
                continue;
            }
            jvm.push(arg);
        }
    } else {
        jvm.push(format!("-Djava.library.path={}", natives.display()));
        jvm.push("-cp".into());
        jvm.push(vars["classpath"].clone());
    }

    if let Some(arg) = version.pointer("/logging/client/argument").and_then(|v| v.as_str()) {
        if let Some(idlog) = version.pointer("/logging/client/file/id").and_then(|v| v.as_str()) {
            let path = game.join("assets").join("log_configs").join(idlog);
            if path.is_file() {
                jvm.push(arg.replace("${path}", &path.to_string_lossy()));
            }
        }
    }

    let game_args = if let Some(args) = version.pointer("/arguments/game") {
        argument_list(Some(args), &vars)
    } else if let Some(legacy) = version.get("minecraftArguments").and_then(|v| v.as_str()) {
        legacy.split_whitespace().map(|t| subst(t, &vars)).collect()
    } else {
        Vec::new()
    };

    let mut args = jvm;
    args.push(main);
    args.extend(game_args);

    let log_path = if cfg.debug {
        Some(game.join("输出日志.txt"))
    } else {
        None
    };
    let command_line = format!("{} {}", java.path.display(), quote_join(&args));
    if let Some(log) = &log_path {
        let _ = std::fs::write(game.join("mclauncher-last-command.txt"), &command_line);
        let _ = log;
    } else {
        let _ = std::fs::write(game.join("mclauncher-last-command.txt"), &command_line);
    }

    Ok(LaunchPlan {
        java: java.path,
        args,
        working_dir: game_directory,
        log_path,
        command_line,
    })
}

fn quote_join(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn spawn_launch(plan: &LaunchPlan) -> McResult<u32> {
    let mut cmd = Command::new(&plan.java);
    cmd.args(&plan.args).current_dir(&plan.working_dir);
    if let Some(log) = &plan.log_path {
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let file = OpenOptions::new().create(true).append(true).open(log).map_err(|e| format!("无法写日志: {e}"))?;
        let err = file.try_clone().map_err(|e| e.to_string())?;
        cmd.stdout(Stdio::from(file)).stderr(Stdio::from(err));
    } else {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;
    Ok(child.id())
}
