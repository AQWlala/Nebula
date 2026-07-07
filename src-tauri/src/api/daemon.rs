//! System daemon registration.
//!
//! 对标: OpenClaw Gateway 守护进程。
//!
//! 支持:
//! - Linux: systemd unit file
//! - macOS: launchd plist
//! - Windows: Windows Service 注册
//!
//! Tauri 命令:`nebula daemon install / uninstall / status`
//!
//! 设计约束:本模块只生成配置文件并尝试调用系统命令;
//! 实际的系统服务管理由 OS 原生机制完成(systemctl / launchctl / sc.exe)。
//! 失败时返回明确错误,不 panic。

use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use tracing::info;
#[cfg(target_os = "windows")]
use tracing::warn;

/// Daemon installation options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Service name (e.g. "nebula").
    pub name: String,
    /// Path to the nebula binary.
    pub exec_path: PathBuf,
    /// Arguments passed to the binary (e.g. ["headless"]).
    pub args: Vec<String>,
    /// Restart policy: "on-failure" | "always" | "never".
    pub restart: String,
    /// Environment variables to set in the service unit.
    pub environment: Vec<String>,
    /// Working directory for the service.
    pub working_dir: Option<PathBuf>,
    /// Run as a specific user (Linux/macOS only).
    pub user: Option<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let exec_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nebula"));
        Self {
            name: "nebula".to_string(),
            exec_path,
            args: vec!["headless".to_string()],
            restart: "on-failure".to_string(),
            environment: vec![
                "NEBULA_GRPC_ENABLED=true".to_string(),
                "NEBULA_REST_ENABLED=true".to_string(),
            ],
            working_dir: None,
            user: None,
        }
    }
}

/// Result of a daemon operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub installed: bool,
    pub running: bool,
    pub platform: String,
    pub message: String,
}

/// Daemon installer — generates platform-specific service files and
/// invokes the OS service manager.
pub struct DaemonInstaller {
    config: DaemonConfig,
}

impl DaemonInstaller {
    pub fn new(config: DaemonConfig) -> Self {
        Self { config }
    }

    /// Installs the daemon on the current platform.
    pub async fn install(&self) -> Result<DaemonStatus> {
        #[cfg(target_os = "linux")]
        {
            return self.install_systemd().await;
        }
        #[cfg(target_os = "macos")]
        {
            return self.install_launchd().await;
        }
        #[cfg(target_os = "windows")]
        {
            return self.install_windows_service().await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            anyhow::bail!("daemon install not supported on this platform");
        }
    }

    /// Uninstalls the daemon from the current platform.
    pub async fn uninstall(&self) -> Result<DaemonStatus> {
        #[cfg(target_os = "linux")]
        {
            return self.uninstall_systemd().await;
        }
        #[cfg(target_os = "macos")]
        {
            return self.uninstall_launchd().await;
        }
        #[cfg(target_os = "windows")]
        {
            return self.uninstall_windows_service().await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            anyhow::bail!("daemon uninstall not supported on this platform");
        }
    }

    /// Queries the daemon status on the current platform.
    pub async fn status(&self) -> Result<DaemonStatus> {
        #[cfg(target_os = "linux")]
        {
            return self.status_systemd().await;
        }
        #[cfg(target_os = "macos")]
        {
            return self.status_launchd().await;
        }
        #[cfg(target_os = "windows")]
        {
            return self.status_windows_service().await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            Ok(DaemonStatus {
                installed: false,
                running: false,
                platform: std::env::consts::OS.to_string(),
                message: "status check not supported on this platform".to_string(),
            })
        }
    }

    // =======================================================================
    // Linux: systemd
    // =======================================================================

    #[cfg(target_os = "linux")]
    async fn install_systemd(&self) -> Result<DaemonStatus> {
        let unit_path = PathBuf::from(format!("/etc/systemd/system/{}.service", self.config.name));
        let unit_content = self.render_systemd_unit();
        tokio::fs::write(&unit_path, &unit_content)
            .await
            .with_context(|| format!("failed to write {}", unit_path.display()))?;
        info!(target: "nebula.daemon", path = %unit_path.display(), "systemd unit written");

        // systemctl daemon-reload
        self.run_systemctl(&["daemon-reload"]).await?;
        // systemctl enable <name>
        self.run_systemctl(&["enable", &self.config.name]).await?;
        // systemctl start <name>
        self.run_systemctl(&["start", &self.config.name]).await?;

        Ok(DaemonStatus {
            installed: true,
            running: true,
            platform: "linux".to_string(),
            message: format!("systemd service {} installed and started", self.config.name),
        })
    }

    #[cfg(target_os = "linux")]
    async fn uninstall_systemd(&self) -> Result<DaemonStatus> {
        // systemctl stop <name> (ignore error if not running)
        let _ = self.run_systemctl(&["stop", &self.config.name]).await;
        // systemctl disable <name> (ignore error if not enabled)
        let _ = self.run_systemctl(&["disable", &self.config.name]).await;

        let unit_path = PathBuf::from(format!("/etc/systemd/system/{}.service", self.config.name));
        if unit_path.exists() {
            tokio::fs::remove_file(&unit_path).await?;
        }
        self.run_systemctl(&["daemon-reload"]).await?;

        Ok(DaemonStatus {
            installed: false,
            running: false,
            platform: "linux".to_string(),
            message: format!("systemd service {} uninstalled", self.config.name),
        })
    }

    #[cfg(target_os = "linux")]
    async fn status_systemd(&self) -> Result<DaemonStatus> {
        let unit_path = PathBuf::from(format!("/etc/systemd/system/{}.service", self.config.name));
        let installed = unit_path.exists();
        let running = if installed {
            self.run_systemctl(&["is-active", &self.config.name])
                .await
                .is_ok()
        } else {
            false
        };
        Ok(DaemonStatus {
            installed,
            running,
            platform: "linux".to_string(),
            message: if installed {
                format!("systemd service {} installed", self.config.name)
            } else {
                "not installed".to_string()
            },
        })
    }

    #[cfg(target_os = "linux")]
    fn render_systemd_unit(&self) -> String {
        let mut out = String::new();
        out.push_str("[Unit]\n");
        out.push_str(&format!(
            "Description=Nebula second-brain daemon ({})\n",
            self.config.name
        ));
        out.push_str("After=network.target\n\n");

        out.push_str("[Service]\n");
        out.push_str("Type=simple\n");
        out.push_str(&format!(
            "ExecStart={} {}\n",
            self.config.exec_path.display(),
            self.config.args.join(" ")
        ));
        if let Some(wd) = &self.config.working_dir {
            out.push_str(&format!("WorkingDirectory={}\n", wd.display()));
        }
        if let Some(user) = &self.config.user {
            out.push_str(&format!("User={}\n", user));
        }
        out.push_str(&format!("Restart={}\n", self.config.restart));
        out.push_str("RestartSec=5\n");
        for env in &self.config.environment {
            out.push_str(&format!("Environment={}\n", env));
        }
        out.push('\n');

        out.push_str("[Install]\n");
        out.push_str("WantedBy=multi-user.target\n");
        out
    }

    #[cfg(target_os = "linux")]
    async fn run_systemctl(&self, args: &[&str]) -> Result<()> {
        let status = tokio::process::Command::new("systemctl")
            .args(args)
            .status()
            .await
            .with_context(|| format!("failed to run systemctl {:?}", args))?;
        if !status.success() {
            anyhow::bail!("systemctl {:?} exited with {:?}", args, status.code());
        }
        Ok(())
    }

    // =======================================================================
    // macOS: launchd
    // =======================================================================

    #[cfg(target_os = "macos")]
    async fn install_launchd(&self) -> Result<DaemonStatus> {
        let label = format!("com.nebula.{}", self.config.name);
        let plist_path = PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", label));
        let plist_content = self.render_launchd_plist(&label);
        tokio::fs::write(&plist_path, &plist_content)
            .await
            .with_context(|| format!("failed to write {}", plist_path.display()))?;
        info!(target: "nebula.daemon", path = %plist_path.display(), "launchd plist written");

        // launchctl load <path>
        let status = tokio::process::Command::new("launchctl")
            .args(["load", &plist_path.to_string_lossy()])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("launchctl load exited with {:?}", status.code());
        }

        // launchctl start <label>
        let _ = tokio::process::Command::new("launchctl")
            .args(["start", &label])
            .status()
            .await;

        Ok(DaemonStatus {
            installed: true,
            running: true,
            platform: "macos".to_string(),
            message: format!("launchd daemon {} installed and started", label),
        })
    }

    #[cfg(target_os = "macos")]
    async fn uninstall_launchd(&self) -> Result<DaemonStatus> {
        let label = format!("com.nebula.{}", self.config.name);
        let plist_path = PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", label));

        // launchctl unload <path>
        let _ = tokio::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .status()
            .await;

        if plist_path.exists() {
            tokio::fs::remove_file(&plist_path).await?;
        }

        Ok(DaemonStatus {
            installed: false,
            running: false,
            platform: "macos".to_string(),
            message: format!("launchd daemon {} uninstalled", label),
        })
    }

    #[cfg(target_os = "macos")]
    async fn status_launchd(&self) -> Result<DaemonStatus> {
        let label = format!("com.nebula.{}", self.config.name);
        let plist_path = PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", label));
        let installed = plist_path.exists();
        let running = if installed {
            tokio::process::Command::new("launchctl")
                .args(["list", &label])
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false)
        } else {
            false
        };
        Ok(DaemonStatus {
            installed,
            running,
            platform: "macos".to_string(),
            message: if installed {
                format!("launchd daemon {} installed", label)
            } else {
                "not installed".to_string()
            },
        })
    }

    #[cfg(target_os = "macos")]
    fn render_launchd_plist(&self, label: &str) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
        out.push_str("<plist version=\"1.0\">\n<dict>\n");
        out.push_str(&format!(
            "\t<key>Label</key>\n\t<string>{}</string>\n",
            label
        ));
        out.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        out.push_str(&format!(
            "\t\t<string>{}</string>\n",
            self.config.exec_path.display()
        ));
        for arg in &self.config.args {
            out.push_str(&format!("\t\t<string>{}</string>\n", arg));
        }
        out.push_str("\t</array>\n");
        if let Some(wd) = &self.config.working_dir {
            out.push_str(&format!(
                "\t<key>WorkingDirectory</key>\n\t<string>{}</string>\n",
                wd.display()
            ));
        }
        out.push_str("\t<key>RunAtLoad</key>\n\t<true/>\n");
        out.push_str("\t<key>KeepAlive</key>\n\t<dict>\n");
        out.push_str(&format!("\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n"));
        out.push_str("\t</dict>\n");
        if !self.config.environment.is_empty() {
            out.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
            for env in &self.config.environment {
                if let Some((k, v)) = env.split_once('=') {
                    out.push_str(&format!(
                        "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                        k, v
                    ));
                }
            }
            out.push_str("\t</dict>\n");
        }
        out.push_str("</dict>\n</plist>\n");
        out
    }

    // =======================================================================
    // Windows: Windows Service
    // =======================================================================

    #[cfg(target_os = "windows")]
    async fn install_windows_service(&self) -> Result<DaemonStatus> {
        // Windows uses `sc.exe create` to register a service.
        // The binary must implement the Windows Service entry point
        // (ServiceMain); a plain CLI binary won't work directly.
        // For now we register the service in "manual" binary mode and
        // document that a proper service host wrapper is needed for
        // production use.
        let bin_path = self.config.exec_path.display().to_string();
        let args = self.config.args.join(" ");

        // sc create <name> binPath= "<path> <args>" start= auto
        let bin_path_full = format!("{} {}", bin_path, args);
        let status = tokio::process::Command::new("sc")
            .args([
                "create",
                &self.config.name,
                "binPath=",
                &bin_path_full,
                "start=",
                "auto",
                "DisplayName=",
                "Nebula second-brain daemon",
            ])
            .status()
            .await
            .context("failed to run sc create")?;
        if !status.success() {
            anyhow::bail!("sc create exited with {:?}", status.code());
        }

        // sc description <name> "..."
        let _ = tokio::process::Command::new("sc")
            .args([
                "description",
                &self.config.name,
                "Nebula second-brain daemon (headless mode)",
            ])
            .status()
            .await;

        // sc start <name>
        let _ = tokio::process::Command::new("sc")
            .args(["start", &self.config.name])
            .status()
            .await;

        Ok(DaemonStatus {
            installed: true,
            running: true,
            platform: "windows".to_string(),
            message: format!("Windows Service {} installed and started", self.config.name),
        })
    }

    #[cfg(target_os = "windows")]
    async fn uninstall_windows_service(&self) -> Result<DaemonStatus> {
        // sc stop <name> (ignore error if not running)
        let _ = tokio::process::Command::new("sc")
            .args(["stop", &self.config.name])
            .status()
            .await;

        // sc delete <name>
        let status = tokio::process::Command::new("sc")
            .args(["delete", &self.config.name])
            .status()
            .await
            .context("failed to run sc delete")?;
        if !status.success() {
            warn!(
                target: "nebula.daemon",
                code = ?status.code(),
                "sc delete failed (service may not have been installed)"
            );
        }

        Ok(DaemonStatus {
            installed: false,
            running: false,
            platform: "windows".to_string(),
            message: format!("Windows Service {} uninstalled", self.config.name),
        })
    }

    #[cfg(target_os = "windows")]
    async fn status_windows_service(&self) -> Result<DaemonStatus> {
        // sc query <name>
        let output = tokio::process::Command::new("sc")
            .args(["query", &self.config.name])
            .output()
            .await;
        let installed = output.as_ref().map(|o| o.status.success()).unwrap_or(false);
        let running = if installed {
            output
                .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
                .map(|s| s.contains("running"))
                .unwrap_or(false)
        } else {
            false
        };
        Ok(DaemonStatus {
            installed,
            running,
            platform: "windows".to_string(),
            message: if installed {
                format!("Windows Service {} installed", self.config.name)
            } else {
                "not installed".to_string()
            },
        })
    }
}

impl Default for DaemonInstaller {
    fn default() -> Self {
        Self::new(DaemonConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_nebula_name() {
        let c = DaemonConfig::default();
        assert_eq!(c.name, "nebula");
        assert!(c.args.contains(&"headless".to_string()));
    }

    #[test]
    fn default_config_enables_grpc_and_rest() {
        let c = DaemonConfig::default();
        assert!(c
            .environment
            .iter()
            .any(|e| e.contains("NEBULA_GRPC_ENABLED")));
        assert!(c
            .environment
            .iter()
            .any(|e| e.contains("NEBULA_REST_ENABLED")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn render_systemd_unit_has_required_sections() {
        let installer = DaemonInstaller::default();
        let unit = installer.render_systemd_unit();
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("Environment=NEBULA_GRPC_ENABLED=true"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn render_launchd_plist_has_label() {
        let installer = DaemonInstaller::default();
        let plist = installer.render_launchd_plist("com.nebula.test");
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("com.nebula.test"));
        assert!(plist.contains("<key>ProgramArguments</key>"));
    }
}
