use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type McResult<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Official,
    Bmclapi,
}

impl Source {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "official" | "mojang" | "官方" => Self::Official,
            _ => Self::Bmclapi,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Bmclapi => "bmclapi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Official => "官方",
            Self::Bmclapi => "BMCLAPI",
        }
    }

    pub fn manifest_url(self) -> &'static str {
        match self {
            Self::Official => "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
            Self::Bmclapi => "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    Offline,
    Yggdrasil,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String,
    /// Yggdrasil API root, without a trailing slash.
    pub server: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub game_dir: String,
    pub java_path: String,
    pub max_memory_mb: u32,
    pub memory_auto: bool,
    pub download_source: String,
    pub debug: bool,
    pub version_isolation: bool,
    pub selected_version: String,
    pub client_token: String,
    pub active_account: String,
    pub accounts: Vec<Account>,
    pub setup_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        let (_, game) = default_paths();
        Self {
            game_dir: game.to_string_lossy().to_string(),
            java_path: String::new(),
            max_memory_mb: 1024,
            memory_auto: true,
            download_source: "bmclapi".into(),
            debug: false,
            version_isolation: false,
            selected_version: String::new(),
            client_token: uuid::Uuid::new_v4().to_string(),
            active_account: String::new(),
            accounts: Vec::new(),
            setup_done: false,
        }
    }
}

impl Config {
    pub fn source(&self) -> Source {
        Source::parse(&self.download_source)
    }

    pub fn game_path(&self) -> PathBuf {
        PathBuf::from(&self.game_dir)
    }

    pub fn memory_mb(&self) -> u32 {
        if self.memory_auto {
            1300
        } else {
            self.max_memory_mb.clamp(256, 32768)
        }
    }

    pub fn active(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == self.active_account)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteVersion {
    pub id: String,
    pub type_name: String,
    pub type_label: String,
    pub release_time: String,
    pub url: String,
    pub installed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub file: String,
    pub completed: u32,
    pub total: u32,
    pub bytes: u64,
    pub bytes_total: u64,
    pub speed: u64,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct LaunchPlan {
    pub java: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub log_path: Option<PathBuf>,
    pub command_line: String,
}

pub fn default_paths() -> (PathBuf, PathBuf) {
    if let Some(proj) = directories::ProjectDirs::from("net", "MCLauncher", "mclauncher-rust") {
        let cfg = proj.config_dir().join("config.json");
        let game = proj.data_dir().join("minecraft");
        (cfg, game)
    } else {
        (PathBuf::from("mclauncher-config.json"), PathBuf::from(".minecraft"))
    }
}

pub fn load_config(path: &std::path::Path) -> McResult<Config> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取配置失败: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("配置损坏: {e}"))
}

pub fn save_config(path: &std::path::Path, cfg: &Config) -> McResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(())
}
