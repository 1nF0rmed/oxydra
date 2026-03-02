use std::collections::BTreeSet;
use std::path::PathBuf;

use types::RunnerGlobalConfig;

/// Shared state for the web configurator server.
#[derive(Debug, Clone)]
pub struct WebState {
    /// The parsed global config (used for user registry, workspace root, etc.).
    pub global_config: RunnerGlobalConfig,
    /// Absolute path to the runner global config file.
    pub config_path: PathBuf,
    /// Resolved workspace root directory (absolute).
    pub workspace_root: PathBuf,
    /// Effective bind address used by the running web server.
    pub bind_address: String,
    /// Host header values accepted by the DNS rebinding guard.
    allowed_hosts: BTreeSet<String>,
    /// Resolved bearer token when token auth mode is enabled.
    auth_token: Option<String>,
}

impl WebState {
    pub fn new(
        global_config: RunnerGlobalConfig,
        config_path: PathBuf,
        bind_address: String,
    ) -> Self {
        let config_dir = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let configured_root = PathBuf::from(&global_config.workspace_root);
        let workspace_root = if configured_root.is_absolute() {
            configured_root
        } else {
            config_dir.join(configured_root)
        };
        let allowed_hosts = allowed_host_values(&bind_address);
        let auth_token = global_config.web.resolve_auth_token();
        Self {
            global_config,
            config_path,
            workspace_root,
            bind_address,
            allowed_hosts,
            auth_token,
        }
    }

    /// Directory containing the runner config file (used to resolve relative paths).
    pub fn config_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Loads the latest global config from disk. Callers should use this for
    /// read-after-write consistency instead of relying on startup-time state.
    pub fn load_latest_global_config(&self) -> Result<RunnerGlobalConfig, crate::RunnerError> {
        crate::load_runner_global_config(&self.config_path)
    }

    /// Loads the latest global config, falling back to the startup snapshot if
    /// the file cannot be reloaded.
    pub fn latest_global_config_or_cached(&self) -> RunnerGlobalConfig {
        self.load_latest_global_config()
            .unwrap_or_else(|_| self.global_config.clone())
    }

    /// Resolves a user config path against the runner config directory.
    pub fn resolve_user_config_path(&self, configured_path: &str) -> PathBuf {
        let configured = PathBuf::from(configured_path);
        if configured.is_absolute() {
            configured
        } else {
            self.config_dir().join(configured)
        }
    }

    /// Returns true when the request Host header matches the configured bind
    /// address allow-list.
    pub fn allows_host_header(&self, host_header: &str) -> bool {
        self.allowed_hosts
            .contains(&host_header.trim().to_ascii_lowercase())
    }

    /// Returns the resolved web bearer token (if token auth is enabled).
    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Compute the control socket path for a user daemon.
    pub fn control_socket_path(&self, user_id: &str) -> PathBuf {
        self.workspace_root
            .join(user_id)
            .join("ipc")
            .join(crate::RUNNER_CONTROL_SOCKET_NAME)
    }
}

fn allowed_host_values(bind_address: &str) -> BTreeSet<String> {
    let mut allowed = BTreeSet::new();
    if let Ok(addr) = bind_address.parse::<std::net::SocketAddr>() {
        match addr {
            std::net::SocketAddr::V4(v4) => {
                let host = format!("{}:{}", v4.ip(), v4.port()).to_ascii_lowercase();
                allowed.insert(host);
                if v4.ip().is_loopback() {
                    allowed.insert(format!("localhost:{}", v4.port()).to_ascii_lowercase());
                }
            }
            std::net::SocketAddr::V6(v6) => {
                let host = format!("[{}]:{}", v6.ip(), v6.port()).to_ascii_lowercase();
                allowed.insert(host);
                if v6.ip().is_loopback() {
                    allowed.insert(format!("localhost:{}", v6.port()).to_ascii_lowercase());
                }
            }
        }
    } else {
        allowed.insert(bind_address.to_ascii_lowercase());
    }

    allowed
}
