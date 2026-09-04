//! XDG-style paths, overridable for tests.

use std::path::{Path, PathBuf};

/// Locations of package defaults, user config, Omarchy, and state.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `$HOME`.
    pub home: PathBuf,
    /// Package defaults (`/usr/share/omacell/default` in production).
    pub default_dir: PathBuf,
    /// `$XDG_CONFIG_HOME/omacell`, falling back to `~/.config/omacell`.
    pub user_config: PathBuf,
    /// `$XDG_STATE_HOME/omacell`, falling back to `~/.local/state/omacell`.
    pub state_dir: PathBuf,
    /// Omarchy's state root, then its config root, for current-theme lookup.
    pub omarchy_state: PathBuf,
    /// `$XDG_CONFIG_HOME/omarchy`, falling back to `~/.config/omarchy`.
    pub omarchy_config: PathBuf,
}

impl Paths {
    /// Resolve from the process environment (`HOME`, XDG roots,
    /// `OMACELL_DEFAULT_DIR`). Relative XDG roots are ignored.
    pub fn from_env() -> Result<Self, omacell_core::error::CoreError> {
        Self::from_env_parts(
            std::env::var_os("HOME").map(PathBuf::from),
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            std::env::var_os("OMACELL_DEFAULT_DIR").map(PathBuf::from),
        )
    }

    /// Build paths under `home` (tests pass a temp dir).
    #[must_use]
    pub fn from_home(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let default_dir = std::env::var_os("OMACELL_DEFAULT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(shipped_default_dir);
        Self::from_roots(home, None, None, default_dir)
    }

    fn from_env_parts(
        home: Option<PathBuf>,
        xdg_config: Option<PathBuf>,
        xdg_state: Option<PathBuf>,
        default_dir: Option<PathBuf>,
    ) -> Result<Self, omacell_core::error::CoreError> {
        let home = home
            .ok_or_else(|| crate::error::schema("HOME is not set; refusing to use / as a home"))?;
        Ok(Self::from_roots(
            home,
            xdg_config.filter(|path| path.is_absolute()),
            xdg_state.filter(|path| path.is_absolute()),
            default_dir.unwrap_or_else(shipped_default_dir),
        ))
    }

    fn from_roots(
        home: PathBuf,
        xdg_config: Option<PathBuf>,
        xdg_state: Option<PathBuf>,
        default_dir: PathBuf,
    ) -> Self {
        let config = xdg_config.unwrap_or_else(|| home.join(".config"));
        let state = xdg_state.unwrap_or_else(|| home.join(".local/state"));
        Self {
            user_config: config.join("omacell"),
            state_dir: state.join("omacell"),
            omarchy_state: state.join("omarchy"),
            omarchy_config: config.join("omarchy"),
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

#[cfg(test)]
mod tests {
    use super::Paths;
    use std::path::PathBuf;

    #[test]
    fn absolute_xdg_roots_are_honored() {
        let paths = Paths::from_env_parts(
            Some(PathBuf::from("/home/tester")),
            Some(PathBuf::from("/var/config")),
            Some(PathBuf::from("/var/state")),
            Some(PathBuf::from("/opt/omacell/default")),
        )
        .unwrap();
        assert_eq!(paths.user_config, PathBuf::from("/var/config/omacell"));
        assert_eq!(paths.state_dir, PathBuf::from("/var/state/omacell"));
        assert_eq!(paths.omarchy_config, PathBuf::from("/var/config/omarchy"));
        assert_eq!(paths.omarchy_state, PathBuf::from("/var/state/omarchy"));
    }

    #[test]
    fn relative_xdg_roots_fall_back_under_home() {
        let paths = Paths::from_env_parts(
            Some(PathBuf::from("/home/tester")),
            Some(PathBuf::from("relative-config")),
            Some(PathBuf::from("relative-state")),
            None,
        )
        .unwrap();
        assert_eq!(
            paths.user_config,
            PathBuf::from("/home/tester/.config/omacell")
        );
        assert_eq!(
            paths.state_dir,
            PathBuf::from("/home/tester/.local/state/omacell")
        );
    }
}
