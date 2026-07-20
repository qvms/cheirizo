//! Best-effort host diagnostics that do not infer desktop capabilities.

use sysinfo::System;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub cpu_count: usize,
    pub total_memory_mb: u64,
    pub hostname: String,
}
impl SystemInfo {
    pub fn gather() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        Self {
            os_name: System::name().unwrap_or_else(|| "Unknown".into()),
            os_version: System::os_version().unwrap_or_default(),
            kernel_version: System::kernel_version().unwrap_or_default(),
            cpu_count: system.cpus().len(),
            total_memory_mb: system.total_memory() / 1024 / 1024,
            hostname: System::host_name().unwrap_or_default(),
        }
    }
    pub fn log(&self) {
        info!(os=%self.os_name,version=%self.os_version,kernel=%self.kernel_version,hostname=%self.hostname,cpus=self.cpu_count,memory_mb=self.total_memory_mb,"host diagnostics");
    }
}

pub fn log_startup_diagnostics() {
    SystemInfo::gather().log();
    info!(version = env!("CARGO_PKG_VERSION"), "WRDP startup");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gather_has_host_resources() {
        let info = SystemInfo::gather();
        assert!(info.cpu_count > 0);
        assert!(info.total_memory_mb > 0);
    }
}
