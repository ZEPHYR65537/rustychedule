use crate::domain::State;
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct Store {
    pub dir: PathBuf,
    _lock: File,
}
impl Store {
    pub fn open(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("无法创建数据目录 {}", dir.display()))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join(".lock"))?;
        FileExt::try_lock_exclusive(&lock)
            .context("数据正在被另一个 schedule/tc 进程使用，请稍后重试")?;
        Ok(Self { dir, _lock: lock })
    }
    pub fn load(&self) -> Result<State> {
        let path = self.dir.join("state.json");
        if !path.exists() {
            return Ok(State::default());
        }
        let data = fs::read(&path).context("读取数据失败")?;
        decode_state(&data).context("数据文件损坏或校验失败；未覆盖原文件，可使用 import 恢复")
    }
    pub fn save(&self, state: &State) -> Result<()> {
        state.validate()?;
        let bytes = serde_json::to_vec_pretty(state)?;
        let path = self.dir.join("state.json");
        if path.exists() {
            let previous = fs::read(&path)?;
            // Recovery must not overwrite the last good backup with a corrupt live file.
            let valid = decode_state(&previous).is_ok();
            let backup = if valid {
                "state.json.bak"
            } else {
                "state.json.corrupt"
            };
            atomic_write(&self.dir.join(backup), &previous)?;
        }
        atomic_write(&path, &bytes)
    }
    pub fn import(&self, path: &Path) -> Result<State> {
        let bytes = fs::read(path).context("读取备份失败")?;
        ensure!(bytes.len() <= 128 * 1024 * 1024, "备份文件超过 128 MiB");
        decode_state(&bytes)
    }
}
pub fn decode_state(bytes: &[u8]) -> Result<State> {
    let mut state: State = serde_json::from_slice(bytes).context("数据 JSON 格式无效")?;
    if state.version == 1 {
        state.version = 2;
    }
    state.validate()?;
    Ok(state)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("原子保存失败：{}", path.display()))?;
    // On Unix, also persist the directory entry. Windows does not allow this open mode.
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let current = base.join("schedule");
    let legacy = base.join("tongchou");
    if !current.join("state.json").exists() && legacy.join("state.json").exists() {
        legacy
    } else {
        current
    }
}
