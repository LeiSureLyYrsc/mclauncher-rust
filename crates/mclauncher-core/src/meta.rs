use crate::model::Source;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn mirror_url(url: &str, source: Source) -> String {
    if source == Source::Official {
        return url.to_string();
    }
    const MAP: &[(&str, &str)] = &[
        ("https://piston-meta.mojang.com/", "https://bmclapi2.bangbang93.com/"),
        ("https://piston-data.mojang.com/", "https://bmclapi2.bangbang93.com/"),
        ("https://launchermeta.mojang.com/", "https://bmclapi2.bangbang93.com/"),
        ("https://launcher.mojang.com/", "https://bmclapi2.bangbang93.com/"),
        ("https://libraries.minecraft.net/", "https://bmclapi2.bangbang93.com/maven/"),
        ("http://libraries.minecraft.net/", "https://bmclapi2.bangbang93.com/maven/"),
        ("https://resources.download.minecraft.net/", "https://bmclapi2.bangbang93.com/assets/"),
        ("http://resources.download.minecraft.net/", "https://bmclapi2.bangbang93.com/assets/"),
        ("https://files.minecraftforge.net/maven/", "https://bmclapi2.bangbang93.com/maven/"),
        ("https://maven.fabricmc.net/", "https://bmclapi2.bangbang93.com/maven/"),
        ("https://meta.fabricmc.net/", "https://bmclapi2.bangbang93.com/fabric-meta/"),
        ("https://authlib-injector.yushi.moe/", "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/"),
    ];
    for (from, to) in MAP {
        if let Some(rest) = url.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    url.to_string()
}

/// `group:artifact:version[:classifier][@ext]` → Maven relative path.
pub fn maven_path(name: &str) -> String {
    let (coords, ext) = match name.rsplit_once('@') {
        Some((n, e)) => (n, e),
        None => (name, "jar"),
    };
    let parts: Vec<&str> = coords.split(':').collect();
    if parts.len() < 3 {
        return format!("{coords}.{ext}");
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    if parts.len() >= 4 && !parts[3].is_empty() {
        format!("{group}/{artifact}/{version}/{artifact}-{version}-{}.{ext}", parts[3])
    } else {
        format!("{group}/{artifact}/{version}/{artifact}-{version}.{ext}")
    }
}

pub fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        "linux" => "linux",
        other => other,
    }
}

pub fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86",
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => other,
    }
}

pub fn arch_bits() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "32",
        _ => "64",
    }
}

fn os_obj_matches(os: &Value) -> bool {
    if let Some(name) = os.get("name").and_then(|v| v.as_str()) {
        if name != os_name() {
            return false;
        }
    }
    if let Some(arch) = os.get("arch").and_then(|v| v.as_str()) {
        let mine = os_arch();
        let ok = arch == mine || (arch == "aarch64" && mine == "arm64") || (arch == "arm64" && mine == "aarch64");
        if !ok {
            return false;
        }
    }
    true
}

/// Minecraft rule list: empty → allow. Otherwise start denied, last matching rule wins.
/// Rules that require launcher features (demo user, custom resolution) never match.
pub fn rules_allow(rules: Option<&Vec<Value>>) -> bool {
    let Some(rules) = rules else {
        return true;
    };
    if rules.is_empty() {
        return true;
    }
    let mut allow = false;
    for rule in rules {
        if rule.get("features").is_some() {
            continue;
        }
        if let Some(os) = rule.get("os") {
            if !os_obj_matches(os) {
                continue;
            }
        }
        allow = rule.get("action").and_then(|a| a.as_str()) == Some("allow");
    }
    allow
}

#[derive(Clone, Debug)]
pub struct Artifact {
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub native: bool,
    pub extract_exclude: Vec<String>,
}

pub fn library_files(version: &Value, source: Source) -> Vec<Artifact> {
    let mut out = Vec::new();
    let Some(libs) = version.get("libraries").and_then(|v| v.as_array()) else {
        return out;
    };
    for lib in libs {
        let rules = lib.get("rules").and_then(|v| v.as_array());
        if !rules_allow(rules) {
            continue;
        }
        let name = lib.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let downloads = lib.get("downloads");
        if let Some(artifact) = downloads.and_then(|d| d.get("artifact")) {
            if let Some(art) = artifact_from(artifact, name, source, false) {
                out.push(art);
            }
        } else if lib.get("natives").is_none() && !name.is_empty() {
            let path = maven_path(name);
            let url = mirror_url(&format!("https://libraries.minecraft.net/{path}"), source);
            out.push(Artifact {
                path,
                url,
                sha1: None,
                size: None,
                native: false,
                extract_exclude: Vec::new(),
            });
        }
        if let Some(key) = native_classifier(lib) {
            let exclude = lib
                .get("extract")
                .and_then(|e| e.get("exclude"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            if let Some(classifiers) = downloads.and_then(|d| d.get("classifiers")) {
                if let Some(artifact) = classifiers.get(&key) {
                    if let Some(mut art) = artifact_from(artifact, name, source, true) {
                        art.extract_exclude = exclude;
                        out.push(art);
                    }
                }
            }
        }
    }
    out
}

fn artifact_from(artifact: &Value, name: &str, source: Source, native: bool) -> Option<Artifact> {
    let path = artifact
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| maven_path(name));
    let url = artifact
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://libraries.minecraft.net/{path}"));
    Some(Artifact {
        path,
        url: mirror_url(&url, source),
        sha1: artifact.get("sha1").and_then(|v| v.as_str()).map(|s| s.to_string()),
        size: artifact.get("size").and_then(|v| v.as_u64()),
        native,
        extract_exclude: Vec::new(),
    })
}

fn native_classifier(lib: &Value) -> Option<String> {
    let natives = lib.get("natives")?;
    let key = natives.get(os_name()).and_then(|v| v.as_str())?;
    Some(key.replace("${arch}", arch_bits()))
}

pub fn subst(input: &str, vars: &HashMap<String, String>) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        let needle = format!("${{{k}}}");
        if out.contains(&needle) {
            out = out.replace(&needle, v);
        }
    }
    out
}

fn push_arg(out: &mut Vec<String>, value: &Value, vars: &HashMap<String, String>) {
    match value {
        Value::String(s) => out.push(subst(s, vars)),
        Value::Array(arr) => {
            for item in arr {
                push_arg(out, item, vars);
            }
        }
        Value::Object(map) => {
            let rules = map.get("rules").and_then(|v| v.as_array());
            if map.contains_key("rules") && !rules_allow(rules) {
                return;
            }
            if let Some(v) = map.get("value") {
                push_arg(out, v, vars);
            }
        }
        _ => {}
    }
}

pub fn argument_list(value: Option<&Value>, vars: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(v) = value {
        push_arg(&mut out, v, vars);
    }
    out
}

pub fn client_artifact(version: &Value, source: Source) -> Option<Artifact> {
    let client = version.get("downloads")?.get("client")?;
    Some(Artifact {
        path: String::new(),
        url: mirror_url(client.get("url")?.as_str()?, source),
        sha1: client.get("sha1").and_then(|v| v.as_str()).map(|s| s.to_string()),
        size: client.get("size").and_then(|v| v.as_u64()),
        native: false,
        extract_exclude: Vec::new(),
    })
}

pub fn classpath_separator() -> &'static str {
    if cfg!(windows) { ";" } else { ":" }
}

pub fn version_dir(game: &Path, id: &str) -> PathBuf {
    game.join("versions").join(id)
}

pub fn dashed_uuid(id: &str) -> String {
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return id.to_string();
    }
    let hex = hex.to_ascii_lowercase();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maven_and_mirror() {
        assert_eq!(
            maven_path("com.mojang:brigadier:1.0.17"),
            "com/mojang/brigadier/1.0.17/brigadier-1.0.17.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.1:natives-windows"),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );
        let url = "https://libraries.minecraft.net/com/mojang/brigadier/1.0.17/brigadier-1.0.17.jar";
        assert_eq!(mirror_url(url, Source::Official), url);
        assert_eq!(
            mirror_url(url, Source::Bmclapi),
            "https://bmclapi2.bangbang93.com/maven/com/mojang/brigadier/1.0.17/brigadier-1.0.17.jar"
        );
        assert_eq!(
            mirror_url("https://resources.download.minecraft.net/ab/cd", Source::Bmclapi),
            "https://bmclapi2.bangbang93.com/assets/ab/cd"
        );
    }

    #[test]
    fn rules_skip_other_os_and_features() {
        let rules = vec![json!({"action": "allow", "os": {"name": "definitely-not-an-os"}})];
        assert!(!rules_allow(Some(&rules)));
        let allow_all = vec![
            json!({"action": "allow"}),
            json!({"action": "disallow", "os": {"name": "definitely-not-an-os"}}),
        ];
        assert!(rules_allow(Some(&allow_all)));
        let demo = vec![json!({"action": "allow", "features": {"is_demo_user": true}})];
        assert!(!rules_allow(Some(&demo)));
        assert!(rules_allow(None));
    }

    #[test]
    fn substitutes_and_skips_feature_args() {
        let mut vars = HashMap::new();
        vars.insert("auth_player_name".into(), "Steve".into());
        let args = json!([
            "--username",
            "${auth_player_name}",
            {"rules": [{"action": "allow", "features": {"is_demo_user": true}}], "value": "--demo"}
        ]);
        let list = argument_list(Some(&args), &vars);
        assert_eq!(list, vec!["--username", "Steve"]);
    }

    #[test]
    fn dashes_uuid() {
        assert_eq!(
            dashed_uuid("5627dd98e6be3c21b8a8e92344183641"),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
    }
}
