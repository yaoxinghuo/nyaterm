use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const API_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub publisher: String,
    pub engine: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub contributions: Contributions,
    pub backend: Option<Backend>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub probes: Vec<Probe>,
    #[serde(default)]
    pub monitors: Vec<Monitor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Probe {
    pub id: String,
    pub title: String,
    pub entry: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Monitor {
    pub id: String,
    pub title: String,
    pub schema: String,
    pub method: String,
    pub panel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub title: String,
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub panel: Option<String>,
    pub method: Option<String>,
    #[serde(default)]
    pub menus: Vec<MenuLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MenuLocation {
    Terminal,
    Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Backend {
    pub transport: Transport,
    /// Keys such as windows-x86_64, linux-aarch64 and macos-aarch64.
    pub executables: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    StdioJsonl,
    StdioFramed,
}

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-_".contains(&b))
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && !value.contains("..")
}

/// Portable archive paths; rejects Windows aliases even when installed on Unix.
pub fn validate_path(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 240 || value.contains(['\\', ':', '\0']) {
        return Err(invalid("Invalid plugin asset path"));
    }
    for component in value.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.chars().any(char::is_control)
            || component.contains(['<', '>', '"', '|', '?', '*'])
        {
            return Err(invalid("Invalid plugin asset path"));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
        {
            return Err(invalid("Reserved plugin asset path"));
        }
    }
    Ok(())
}

pub fn network_origin(permission: &str) -> Option<&str> {
    permission.strip_prefix("network:")
}

pub fn validate_permission(permission: &str) -> Result<()> {
    if [
        "session.read",
        "terminal.read",
        "terminal.execute",
        "filesystem.read",
        "storage",
        "native",
        "remote.probe",
    ]
    .contains(&permission)
    {
        return Ok(());
    }
    if let Some(origin) = network_origin(permission) {
        let url = url::Url::parse(origin).map_err(|_| invalid("Invalid network permission"))?;
        if url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.origin().ascii_serialization() == origin
        {
            return Ok(());
        }
    }
    Err(invalid(format!(
        "Unknown or invalid permission: {permission}"
    )))
}

impl Manifest {
    pub fn parse(bytes: &[u8], app_version: &str) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(invalid("Plugin manifest is too large"));
        }
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate(app_version)?;
        Ok(manifest)
    }

    pub fn validate(&self, app_version: &str) -> Result<()> {
        if self.manifest_version != API_VERSION || !valid_id(&self.id) || !self.id.contains('.') {
            return Err(invalid("Unsupported manifest version or invalid plugin ID"));
        }
        for (name, value, limit) in [
            ("name", &self.name, 120),
            ("description", &self.description, 2000),
            ("publisher", &self.publisher, 120),
        ] {
            if value.trim().is_empty() || value.len() > limit || value.chars().any(|c| c == '\0') {
                return Err(invalid(format!("Invalid plugin {name}")));
            }
        }
        let version =
            semver::Version::parse(&self.version).map_err(|_| invalid("Invalid plugin version"))?;
        if version.to_string() != self.version || self.version.len() > 80 {
            return Err(invalid("Invalid plugin version"));
        }
        let requirement = semver::VersionReq::parse(&self.engine)
            .map_err(|_| invalid("Invalid engine requirement"))?;
        let app =
            semver::Version::parse(app_version).map_err(|_| invalid("Invalid app version"))?;
        if !requirement.matches(&app) {
            return Err(invalid("Plugin is incompatible with this NyaTerm version"));
        }
        if self.permissions.len() > 24
            || self.contributions.panels.len() > 16
            || self.contributions.commands.len() > 64
            || self.contributions.probes.len() > 16
            || self.contributions.monitors.len() > 16
        {
            return Err(invalid("Too many plugin permissions or contributions"));
        }
        let mut permissions = HashSet::new();
        for permission in &self.permissions {
            validate_permission(permission)?;
            if !permissions.insert(permission) {
                return Err(invalid("Duplicate permission"));
            }
        }
        if self
            .permissions
            .iter()
            .filter(|p| network_origin(p).is_some())
            .count()
            > 8
        {
            return Err(invalid("Too many network origins"));
        }
        let mut ids = HashSet::new();
        for probe in &self.contributions.probes {
            validate_path(&probe.entry)?;
            if !valid_id(&probe.id)
                || !ids.insert(&probe.id)
                || probe.title.trim().is_empty()
                || probe.title.len() > 120
                || !probe.entry.starts_with("assets/probes/")
                || !probe.entry.ends_with(".sh")
                || !(1000..=30_000).contains(&probe.timeout_ms)
                || !permissions.contains(&"remote.probe".to_string())
            {
                return Err(invalid(
                    "Invalid probe declaration or missing remote.probe permission",
                ));
            }
        }
        for panel in &self.contributions.panels {
            if !valid_id(&panel.id)
                || panel.title.trim().is_empty()
                || panel.title.len() > 120
                || !ids.insert(&panel.id)
            {
                return Err(invalid("Invalid or duplicate panel"));
            }
            validate_path(&panel.entry)?;
            if !panel.entry.starts_with("ui/") || !panel.entry.ends_with(".html") {
                return Err(invalid("Panel entry must be HTML"));
            }
        }
        for command in &self.contributions.commands {
            if !valid_id(&command.id)
                || command.title.trim().is_empty()
                || command.title.len() > 120
                || !ids.insert(&command.id)
            {
                return Err(invalid("Invalid or duplicate command"));
            }
            match (&command.panel, &command.method) {
                (Some(panel), None) if self.contributions.panels.iter().any(|p| &p.id == panel) => {
                }
                (None, Some(method))
                    if self.backend.is_some()
                        && method.starts_with("command/")
                        && method.len() <= 128 => {}
                _ => return Err(invalid("Command must reference a panel or backend method")),
            }
            if command.menus.len() > 2 {
                return Err(invalid("Too many command menu locations"));
            }
        }
        let mut schemas = HashSet::new();
        for monitor in &self.contributions.monitors {
            if !valid_id(&monitor.id)
                || !ids.insert(&monitor.id)
                || monitor.title.trim().is_empty()
                || monitor.title.len() > 120
                || monitor.schema != "gpu.v1"
                || !schemas.insert(&monitor.schema)
                || !monitor.method.starts_with("monitor/")
                || monitor.method.len() > 128
                || monitor.method.len() <= 8
                || self.backend.is_none()
                || self.contributions.probes.is_empty()
                || !self
                    .contributions
                    .panels
                    .iter()
                    .any(|p| p.id == monitor.panel)
            {
                return Err(invalid("Invalid monitor declaration"));
            }
        }
        if let Some(backend) = &self.backend {
            if !self.permissions.iter().any(|p| p == "native")
                || backend.executables.is_empty()
                || backend.executables.len() > 8
            {
                return Err(invalid(
                    "Native backend requires the native permission and executables",
                ));
            }
            for (platform, path) in &backend.executables {
                if ![
                    "windows-x86_64",
                    "windows-aarch64",
                    "linux-x86_64",
                    "linux-aarch64",
                    "macos-x86_64",
                    "macos-aarch64",
                ]
                .contains(&platform.as_str())
                {
                    return Err(invalid("Unsupported backend platform"));
                }
                validate_path(path)?;
                if !path.starts_with("bin/") {
                    return Err(invalid("Backend executable must be under bin/"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_nonportable_and_traversing_paths() {
        for path in [
            "../secret",
            "/absolute",
            "C:/x",
            "ui\\x",
            "ui/a:stream",
            "ui/CON.html",
            "ui/a.",
            "ui/a ",
            "ui//a",
            "ui/./a",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        assert!(validate_path("ui/index.html").is_ok());
    }
    #[test]
    fn network_permissions_are_exact_https_origins() {
        for value in [
            "network:*",
            "network:http://example.com",
            "network:https://a.com/path",
            "network:https://u@a.com",
            "network:https://a.com/",
            "network:https://a.com?x",
        ] {
            assert!(validate_permission(value).is_err(), "{value}");
        }
        assert!(validate_permission("network:https://example.com:8443").is_ok());
    }
}
