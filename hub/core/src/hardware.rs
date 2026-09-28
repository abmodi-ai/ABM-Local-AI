//! What this computer has, as far as running a model is concerned.

use std::path::Path;

use serde::Serialize;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};

#[derive(Clone, Debug, Serialize)]
pub struct Hardware {
    pub os: String,
    pub arch: String,
    pub cpu_name: String,
    /// Physical cores (performance and efficiency cores both count on Apple Silicon).
    pub cpu_cores: usize,
    pub total_ram_gb: f64,
    /// Free space on the volume that holds the packs folder.
    pub free_disk_gb: f64,
    /// A GPU the bundled llama.cpp build can use. v1 bundles Metal on Apple Silicon only;
    /// Windows and Linux builds are CPU-only (Vulkan/CUDA come later).
    pub gpu: Option<String>,
}

const GIB: f64 = (1u64 << 30) as f64;

impl Hardware {
    pub fn detect(data_dir: &Path) -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::nothing().with_ram())
                .with_cpu(CpuRefreshKind::nothing()),
        );
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_owned())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| std::env::consts::ARCH.to_owned());
        let cpu_cores = System::physical_core_count().unwrap_or_else(|| sys.cpus().len().max(1));
        let gpu = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some("Apple Silicon GPU (Metal)".to_owned())
        } else {
            None
        };
        Hardware {
            os: System::long_os_version().unwrap_or_else(|| std::env::consts::OS.to_owned()),
            arch: std::env::consts::ARCH.to_owned(),
            cpu_name,
            cpu_cores,
            total_ram_gb: sys.total_memory() as f64 / GIB,
            free_disk_gb: free_disk_gb(data_dir),
            gpu,
        }
    }

    /// Developer and demo aid: `ABM_SIMULATE_PC="ram=8,cores=4,gpu=0,disk=50"` pretends to be a
    /// different computer, so the library can be checked without the hardware. Unknown keys are ignored.
    pub fn with_simulation(mut self) -> Self {
        let Ok(spec) = std::env::var("ABM_SIMULATE_PC") else { return self };
        for (k, v) in spec.split(',').filter_map(|kv| kv.split_once('=')) {
            match (k.trim(), v.trim()) {
                ("ram", v) => self.total_ram_gb = v.parse().unwrap_or(self.total_ram_gb),
                ("cores", v) => self.cpu_cores = v.parse().unwrap_or(self.cpu_cores),
                ("disk", v) => self.free_disk_gb = v.parse().unwrap_or(self.free_disk_gb),
                ("gpu", "0") => self.gpu = None,
                _ => {}
            }
        }
        self.cpu_name = format!("{} (simulated)", self.cpu_name);
        self
    }

    /// Refresh only the value that changes while the app runs.
    pub fn refresh_disk(&mut self, data_dir: &Path) {
        self.free_disk_gb = free_disk_gb(data_dir);
    }
}

fn free_disk_gb(path: &Path) -> f64 {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space() as f64 / GIB)
        .unwrap_or(0.0)
}
