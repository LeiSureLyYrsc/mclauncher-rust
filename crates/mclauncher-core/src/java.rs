use crate::model::McResult;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct JavaInst {
    pub path: PathBuf,
    pub major: u32,
}

pub fn parse_java_major(stderr: &str) -> Option<u32> {
    // openjdk version "17.0.9"  /  java version "1.8.0_333"
    let line = stderr.lines().next().unwrap_or(stderr);
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    let ver = &line[start..end];
    let mut parts = ver.split(|c| c == '.' || c == '_');
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}

pub fn java_version(path: &Path) -> Option<u32> {
    let out = Command::new(path).arg("-version").output().ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_java_major(&stderr).or_else(|| parse_java_major(&stdout))
}

pub fn discover_javas() -> Vec<JavaInst> {
    let mut found = Vec::new();
    let mut push = |path: PathBuf| {
        if !path.is_file() {
            return;
        }
        if found.iter().any(|j: &JavaInst| j.path == path) {
            return;
        }
        if let Some(major) = java_version(&path) {
            found.push(JavaInst { path, major });
        }
    };

    if let Ok(home) = std::env::var("JAVA_HOME") {
        push(PathBuf::from(&home).join("bin").join(java_bin()));
    }
    if let Some(path) = which_java() {
        push(path);
    }
    for dir in candidate_dirs() {
        if !dir.exists() {
            continue;
        }
        let entries = std::fs::read_dir(&dir);
        let Ok(entries) = entries else { continue };
        for ent in entries.flatten() {
            let p = ent.path();
            push(p.join("bin").join(java_bin()));
            push(p.join(java_bin()));
        }
    }
    found.sort_by(|a, b| b.major.cmp(&a.major));
    found
}

fn java_bin() -> &'static str {
    if cfg!(windows) { "javaw.exe" } else { "java" }
}

fn which_java() -> Option<PathBuf> {
    let (cmd, arg) = if cfg!(windows) { ("where", "javaw") } else { ("which", "java") };
    let out = Command::new(cmd).arg(arg).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().map(str::trim).find(|l| !l.is_empty()).map(PathBuf::from)
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        for base in ["C:\\Program Files\\Java", "C:\\Program Files\\Eclipse Adoptium", "C:\\Program Files\\Microsoft", "C:\\Program Files\\Zulu"] {
            dirs.push(PathBuf::from(base));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        dirs.push(PathBuf::from("/opt/homebrew/opt"));
    } else {
        dirs.push(PathBuf::from("/usr/lib/jvm"));
        dirs.push(PathBuf::from("/usr/lib/jvm"));
    }
    dirs
}

pub fn pick_java(preferred: &str, required_major: Option<u32>) -> McResult<JavaInst> {
    if !preferred.trim().is_empty() {
        let path = PathBuf::from(preferred);
        if !path.is_file() {
            return Err("java路径有误,是否已经安装java?".into());
        }
        let major = java_version(&path).unwrap_or(0);
        if let Some(need) = required_major {
            if major > 0 && major < need {
                return Err(format!("当前 Java 是 {major}，这个版本需要 Java {need} 或更高"));
            }
        }
        return Ok(JavaInst { path, major });
    }
    let all = discover_javas();
    if all.is_empty() {
        return Err("没有找到java路径,请手动指定javaw.exe".into());
    }
    if let Some(need) = required_major {
        if let Some(ok) = all.iter().find(|j| j.major >= need) {
            return Ok(ok.clone());
        }
        return Err(format!(
            "已找到 Java {}，但这个版本需要 Java {need}。请在设置里指定更高版本的 java",
            all[0].major
        ));
    }
    Ok(all[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_and_modern() {
        assert_eq!(parse_java_major("java version \"1.8.0_333\"\n"), Some(8));
        assert_eq!(parse_java_major("openjdk version \"21.0.2\" 2024-01-16\n"), Some(21));
        assert_eq!(parse_java_major("openjdk version \"17.0.9\""), Some(17));
    }
}
