//! XDG-style paths, overridable for tests.

use std::path::{Path, PathBuf};

/// Locations of package defaults, user config, Omarchy, and state.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `$HOME`.
    pub home: PathBuf,
    /// Package defaults (`/usr/share/omacell/default` in production).
    pub default_dir: PathBuf,
    /// `~/.config/omacell`.
    pub user_config: PathBuf,
    /// `~/.local/state/omacell`.
    pub state_dir: PathBuf,
    /// `~/.local/state/omarchy` then `~/.config/omarchy` for current theme.
    pub omarchy_state: PathBuf,
    /// `~/.config/omarchy`.
    pub omarchy_config: PathBuf,
}

impl Paths {
    /// Resolve from the process environment (`HOME`, `OMACELL_DEFAULT_DIR`).
    pub fn from_env() -> Result<Self, omacell_core::error::CoreError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| crate::error::schema("HOME is not set; refusing to use / as a home"))?;
        Ok(Self::from_home(home))
    }

    /// Build paths under `home` (tests pass a temp dir).
    #[must_use]
    pub fn from_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let default_dir = std::env::var_os("OMACELL_DEFAULT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(shipped_default_dir);
        Self {
            user_config: home.join(".config/omacell"),
            state_dir: home.join(".local/state/omacell"),
            omarchy_state: home.join(".local/state/omarchy"),
            omarchy_config: home.join(".config/omarchy"),
            home,
            default_dir,
        }
    }

    /// User `config.toml`.
    #[must_use]
    pub fn user_config_toml(&self) -> PathBuf {
        self.user_config.join("config.toml")
    }

    /// User `theme.toml` (sparse role overrides).
    #[must_use]
    pub fn user_theme_toml(&self) -> PathBuf {
        self.user_config.join("theme.toml")
    }

    /// Timestamped backup directory.
    #[must_use]
    pub fn backup_dir(&self, stamp: &str) -> PathBuf {
        self.state_dir.join("backups").join(stamp)
    }
}

fn shipped_default_dir() -> PathBuf {
    let exe = std::env::current_exe().ok();
    if let Some(exe) = exe {
        let sibling = exe
            .parent()
            .map(|p| p.join("../share/omacell/default"))
            .unwrap_or_default();
        if sibling.join("config.toml").is_file() {
            return sibling;
        }
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../default");
    if repo.join("config.toml").is_file() {
        return repo;
    }
    PathBuf::from("/usr/share/omacell/default")
}
