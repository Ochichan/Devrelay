//! Development-mode user service templates for the local agent.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MACOS_LAUNCH_AGENT_LABEL: &str = "com.devrelay.agent";
pub const LINUX_SYSTEMD_UNIT: &str = "devrelay-agent.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceTemplateKind {
    MacosLaunchAgent,
    LinuxSystemdUser,
}

impl ServiceTemplateKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MacosLaunchAgent => "macos-launch-agent",
            Self::LinuxSystemdUser => "linux-systemd-user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTemplateInput {
    pub agent_bin: PathBuf,
    pub devrelay_home: PathBuf,
    pub socket_path: PathBuf,
    pub log_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTemplate {
    pub kind: ServiceTemplateKind,
    pub service_path: PathBuf,
    pub content: String,
}

pub fn macos_launch_agent_template(
    input: &ServiceTemplateInput,
    service_dir: &Path,
) -> ServiceTemplate {
    let service_path = service_dir.join(format!("{MACOS_LAUNCH_AGENT_LABEL}.plist"));
    let stdout = input
        .devrelay_home
        .join("logs")
        .join("agent.launchd.out.log");
    let stderr = input
        .devrelay_home
        .join("logs")
        .join("agent.launchd.err.log");
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{agent_bin}</string>
    <string>--foreground</string>
    <string>--socket-path</string>
    <string>{socket_path}</string>
    <string>--log-level</string>
    <string>{log_level}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>DEVRELAY_HOME</key>
    <string>{devrelay_home}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = MACOS_LAUNCH_AGENT_LABEL,
        agent_bin = escape_xml_path(&input.agent_bin),
        socket_path = escape_xml_path(&input.socket_path),
        log_level = escape_xml(&input.log_level),
        devrelay_home = escape_xml_path(&input.devrelay_home),
        stdout = escape_xml_path(&stdout),
        stderr = escape_xml_path(&stderr),
    );
    ServiceTemplate {
        kind: ServiceTemplateKind::MacosLaunchAgent,
        service_path,
        content,
    }
}

pub fn linux_systemd_user_template(
    input: &ServiceTemplateInput,
    service_dir: &Path,
) -> ServiceTemplate {
    let service_path = service_dir.join(LINUX_SYSTEMD_UNIT);
    let content = format!(
        r#"[Unit]
Description=DevRelay local agent
After=default.target

[Service]
Type=simple
ExecStart="{agent_bin}" --foreground --socket-path "{socket_path}" --log-level {log_level}
Environment="DEVRELAY_HOME={devrelay_home}"
Restart=on-failure
RestartSec=2

[Install]
WantedBy=default.target
"#,
        agent_bin = escape_systemd(&input.agent_bin.to_string_lossy()),
        socket_path = escape_systemd(&input.socket_path.to_string_lossy()),
        log_level = escape_systemd(&input.log_level),
        devrelay_home = escape_systemd(&input.devrelay_home.to_string_lossy()),
    );
    ServiceTemplate {
        kind: ServiceTemplateKind::LinuxSystemdUser,
        service_path,
        content,
    }
}

fn escape_xml_path(path: &Path) -> String {
    escape_xml(&path.to_string_lossy())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_systemd(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ServiceTemplateInput {
        ServiceTemplateInput {
            agent_bin: PathBuf::from("/opt/devrelay/devrelay-agent"),
            devrelay_home: PathBuf::from("/tmp/devrelay home"),
            socket_path: PathBuf::from("/tmp/devrelay home/agent.sock"),
            log_level: "info".to_string(),
        }
    }

    #[test]
    fn renders_macos_launch_agent_template() {
        let template =
            macos_launch_agent_template(&input(), Path::new("/Users/me/Library/LaunchAgents"));

        assert_eq!(template.kind, ServiceTemplateKind::MacosLaunchAgent);
        assert_eq!(
            template.service_path,
            PathBuf::from("/Users/me/Library/LaunchAgents/com.devrelay.agent.plist")
        );
        assert!(
            template
                .content
                .contains("<string>com.devrelay.agent</string>")
        );
        assert!(template.content.contains("<string>--foreground</string>"));
        assert!(
            template
                .content
                .contains("<key>DEVRELAY_HOME</key>\n    <string>/tmp/devrelay home</string>")
        );
        assert!(
            template
                .content
                .contains("<string>/tmp/devrelay home/agent.sock</string>")
        );
    }

    #[test]
    fn renders_linux_systemd_user_template() {
        let template =
            linux_systemd_user_template(&input(), Path::new("/home/me/.config/systemd/user"));

        assert_eq!(template.kind, ServiceTemplateKind::LinuxSystemdUser);
        assert_eq!(
            template.service_path,
            PathBuf::from("/home/me/.config/systemd/user/devrelay-agent.service")
        );
        assert!(
            template
                .content
                .contains("ExecStart=\"/opt/devrelay/devrelay-agent\" --foreground")
        );
        assert!(
            template
                .content
                .contains("Environment=\"DEVRELAY_HOME=/tmp/devrelay home\"")
        );
        assert!(template.content.contains("WantedBy=default.target"));
    }
}
