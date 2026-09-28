//! The model library (packs/catalog.json, embedded at build time) and the rules that decide
//! whether a model runs comfortably on this computer.

use serde::{Deserialize, Serialize};

use crate::hardware::Hardware;

const CATALOG_JSON: &str = include_str!("../../../packs/catalog.json");

/// Installed RAM is reported a little under the nominal size (firmware and GPU reservations),
/// so an "8 GB" computer can show 7.6 GB. Accept anything within 10% of the requirement.
const RAM_TOLERANCE: f64 = 0.9;
/// Free disk space to keep beyond the model file itself.
const DISK_MARGIN_GB: f64 = 1.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Catalog {
    pub version: u32,
    pub allowed_hosts: Vec<String>,
    pub models: Vec<Model>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub publisher: String,
    /// "chat" or "embeddings".
    pub kind: String,
    pub tags: Vec<String>,
    pub summary: String,
    pub license: String,
    pub url: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    pub context: u32,
    pub server_args: Vec<String>,
    pub requirements: Requirements,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Requirements {
    pub ram_gb: u32,
    /// Cores needed when a supported GPU does most of the work.
    pub cpu_cores: usize,
    /// Cores needed when the CPU does all the work (Windows and Linux in v1).
    pub cpu_cores_without_gpu: usize,
    /// Too slow on a CPU alone to be usable.
    pub needs_gpu: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub label: &'static str,
    pub required: String,
    pub actual: String,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Compatibility {
    pub ok: bool,
    pub checks: Vec<Check>,
    /// One line with the minimum specs, e.g. "16 GB RAM · 12 CPU cores (6 with a supported GPU) · 6 GB free disk".
    pub minimum: String,
}

impl Catalog {
    pub fn builtin() -> Self {
        serde_json::from_str(CATALOG_JSON).expect("packs/catalog.json is invalid")
    }

    pub fn get(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Whether a download URL (or redirect target) is on a host the catalog allows.
    pub fn host_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.allowed_hosts.iter().any(|pattern| match pattern.strip_prefix("*.") {
            Some(suffix) => host.ends_with(&format!(".{suffix}")),
            None => host == *pattern,
        })
    }
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / (1u64 << 30) as f64
}

fn fmt_gb(v: f64) -> String {
    if v >= 10.0 {
        format!("{v:.0} GB")
    } else {
        format!("{v:.1} GB")
    }
}

impl Model {
    pub fn download_gb(&self) -> f64 {
        gb(self.size)
    }

    fn disk_needed_gb(&self) -> f64 {
        (self.download_gb() + DISK_MARGIN_GB).ceil()
    }

    /// Evaluate the model against this computer. `installed` skips the disk-space check.
    pub fn compatibility(&self, hw: &Hardware, installed: bool) -> Compatibility {
        let r = &self.requirements;
        let has_gpu = hw.gpu.is_some();
        let mut checks = vec![Check {
            label: "Memory (RAM)",
            required: format!("{} GB", r.ram_gb),
            actual: format!("{:.0} GB", hw.total_ram_gb),
            ok: hw.total_ram_gb >= f64::from(r.ram_gb) * RAM_TOLERANCE,
        }];
        if r.needs_gpu {
            checks.push(Check {
                label: "Graphics",
                required: "Apple Silicon GPU (Windows and Linux GPUs coming later)".into(),
                actual: hw.gpu.clone().unwrap_or_else(|| "none supported".into()),
                ok: has_gpu,
            });
        }
        let cores = if has_gpu { r.cpu_cores } else { r.cpu_cores_without_gpu };
        checks.push(Check {
            label: "Processor",
            required: format!("{cores} cores"),
            actual: format!("{} cores", hw.cpu_cores),
            ok: hw.cpu_cores >= cores,
        });
        if !installed {
            checks.push(Check {
                label: "Free disk space",
                required: fmt_gb(self.disk_needed_gb()),
                actual: fmt_gb(hw.free_disk_gb),
                ok: hw.free_disk_gb >= self.disk_needed_gb(),
            });
        }
        let mut minimum = vec![format!("{} GB RAM", r.ram_gb)];
        if r.needs_gpu {
            minimum.push("Apple Silicon Mac".into());
            minimum.push(format!("{} CPU cores", r.cpu_cores));
        } else if r.cpu_cores_without_gpu != r.cpu_cores {
            minimum.push(format!("{} CPU cores ({} on an Apple Silicon Mac)", r.cpu_cores_without_gpu, r.cpu_cores));
        } else {
            minimum.push(format!("{} CPU cores", r.cpu_cores));
        }
        minimum.push(format!("{} free disk space", fmt_gb(self.disk_needed_gb())));
        Compatibility { ok: checks.iter().all(|c| c.ok), checks, minimum: minimum.join(" · ") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(ram: f64, cores: usize, gpu: bool, disk: f64) -> Hardware {
        Hardware {
            os: "test".into(),
            arch: "x86_64".into(),
            cpu_name: "test".into(),
            cpu_cores: cores,
            total_ram_gb: ram,
            free_disk_gb: disk,
            gpu: gpu.then(|| "Apple Silicon GPU (Metal)".into()),
        }
    }

    #[test]
    fn catalog_parses_and_is_permissively_licensed() {
        let c = Catalog::builtin();
        assert!(!c.models.is_empty());
        for m in &c.models {
            assert!(matches!(m.license.as_str(), "Apache-2.0" | "MIT"), "{} has licence {}", m.id, m.license);
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.url.starts_with("https://"), "{}", m.id);
            let host = m.url.trim_start_matches("https://").split('/').next().unwrap();
            assert!(c.host_allowed(host), "{} is hosted on a host the catalog doesn't allow", m.id);
        }
    }

    #[test]
    fn host_allowlist() {
        let c = Catalog::builtin();
        assert!(c.host_allowed("huggingface.co"));
        assert!(c.host_allowed("us.aws.cdn.hf.co"));
        assert!(!c.host_allowed("hf.co.evil.example"));
        assert!(!c.host_allowed("evilhuggingface.co"));
    }

    #[test]
    fn typical_8gb_pc_gets_small_but_not_large_models() {
        let c = Catalog::builtin();
        let hw = pc(7.6, 4, false, 200.0); // an "8 GB" Windows laptop reports a bit less
        assert!(c.get("qwen3-1.7b").unwrap().compatibility(&hw, false).ok);
        assert!(!c.get("qwen3-8b").unwrap().compatibility(&hw, false).ok);
        let big = c.get("qwen3-14b").unwrap().compatibility(&hw, false);
        assert!(!big.ok);
        assert!(big.checks.iter().any(|ch| ch.label == "Graphics" && !ch.ok));
    }

    #[test]
    fn disk_space_is_checked_only_before_install() {
        let c = Catalog::builtin();
        let m = c.get("qwen3-1.7b").unwrap();
        let hw = pc(16.0, 8, false, 0.5);
        assert!(!m.compatibility(&hw, false).ok);
        assert!(m.compatibility(&hw, true).ok);
    }

    #[test]
    fn gpu_lowers_the_core_requirement() {
        let c = Catalog::builtin();
        let m = c.get("qwen3-8b").unwrap();
        assert!(!m.compatibility(&pc(16.0, 8, false, 100.0), false).ok);
        assert!(m.compatibility(&pc(16.0, 8, true, 100.0), false).ok);
    }
}
