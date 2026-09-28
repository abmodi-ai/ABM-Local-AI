//! Installed model packs and the downloader.
//!
//! Layout: `<data dir>/packs/<id>/<file>` plus `<id>/pack.json`, written only after the sha256
//! matches the catalog. Downloads go to `<file>.part`, resume after a cancel or restart, follow
//! redirects only to hosts the catalog allows, and are verified before they're renamed into
//! place. At load time a pack is re-hashed if its size or modification time changed since install.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::catalog::{Catalog, Model};
use crate::Error;

const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PackRecord {
    id: String,
    file: String,
    sha256: String,
    size: u64,
    mtime: u64,
    installed_at: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Download {
    Downloading { downloaded: u64, total: u64, bytes_per_s: f64 },
    Verifying,
    Paused { downloaded: u64, total: u64 },
    Failed { error: String, downloaded: u64, total: u64 },
}

struct Job {
    status: Download,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct Packs {
    dir: PathBuf,
    catalog: Arc<Catalog>,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn sha256_file(path: &Path) -> Result<String, Error> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl Packs {
    pub fn new(data_dir: &Path, catalog: Arc<Catalog>) -> Self {
        let dir = data_dir.join("packs");
        let _ = fs::create_dir_all(&dir);
        let packs = Packs { dir, catalog, jobs: Arc::new(Mutex::new(HashMap::new())) };
        packs.restore_paused();
        packs
    }

    /// A partial download left by a previous session shows as Paused so the user can resume it.
    fn restore_paused(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        for m in &self.catalog.models {
            let part = self.part_path(m);
            if let Ok(meta) = fs::metadata(&part) {
                jobs.insert(
                    m.id.clone(),
                    Job {
                        status: Download::Paused { downloaded: meta.len(), total: m.size },
                        cancel: Arc::new(AtomicBool::new(false)),
                    },
                );
            }
        }
    }

    fn pack_dir(&self, m: &Model) -> PathBuf {
        self.dir.join(&m.id)
    }

    pub fn model_path(&self, m: &Model) -> PathBuf {
        self.pack_dir(m).join(&m.file)
    }

    fn part_path(&self, m: &Model) -> PathBuf {
        self.pack_dir(m).join(format!("{}.part", m.file))
    }

    fn record_path(&self, m: &Model) -> PathBuf {
        self.pack_dir(m).join("pack.json")
    }

    fn record(&self, m: &Model) -> Option<PackRecord> {
        let r: PackRecord = serde_json::from_slice(&fs::read(self.record_path(m)).ok()?).ok()?;
        (r.sha256 == m.sha256 && r.file == m.file).then_some(r)
    }

    pub fn is_installed(&self, m: &Model) -> bool {
        self.record(m).is_some() && self.model_path(m).is_file()
    }

    /// Check the file before loading it: cheap if unchanged since install, a full re-hash if not.
    pub fn verify_for_load(&self, m: &Model) -> Result<PathBuf, Error> {
        let path = self.model_path(m);
        let rec = self.record(m).ok_or_else(|| Error::new(format!("{} is not installed", m.name)))?;
        let size = fs::metadata(&path)?.len();
        if size == rec.size && mtime_secs(&path) == rec.mtime {
            return Ok(path);
        }
        if sha256_file(&path)? != m.sha256 {
            return Err(Error::new(format!("{} failed its integrity check. Remove it and download it again.", m.name)));
        }
        self.write_record(m, &path)?;
        Ok(path)
    }

    fn write_record(&self, m: &Model, path: &Path) -> Result<(), Error> {
        let rec = PackRecord {
            id: m.id.clone(),
            file: m.file.clone(),
            sha256: m.sha256.clone(),
            size: fs::metadata(path)?.len(),
            mtime: mtime_secs(path),
            installed_at: now_secs(),
        };
        fs::write(self.record_path(m), serde_json::to_vec_pretty(&rec).unwrap())?;
        Ok(())
    }

    pub fn download_status(&self, id: &str) -> Option<Download> {
        self.jobs.lock().unwrap().get(id).map(|j| j.status.clone())
    }

    /// Start (or resume) a download in the background. No-op if one is already running.
    pub fn start_download(&self, id: &str) -> Result<(), Error> {
        let m = self.catalog.get(id).ok_or_else(|| Error::new(format!("unknown model {id}")))?.clone();
        if self.is_installed(&m) {
            return Ok(());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = self.jobs.lock().unwrap();
            if let Some(j) = jobs.get(id) {
                if matches!(j.status, Download::Downloading { .. } | Download::Verifying) {
                    return Ok(());
                }
            }
            let have = fs::metadata(self.part_path(&m)).map(|x| x.len()).unwrap_or(0);
            jobs.insert(
                id.to_owned(),
                Job {
                    status: Download::Downloading { downloaded: have, total: m.size, bytes_per_s: 0.0 },
                    cancel: Arc::clone(&cancel),
                },
            );
        }
        let this = self.clone();
        thread::Builder::new()
            .name(format!("download-{id}"))
            .spawn(move || {
                let result = this.run_download(&m, &cancel);
                let mut jobs = this.jobs.lock().unwrap();
                let have = fs::metadata(this.part_path(&m)).map(|x| x.len()).unwrap_or(0);
                match result {
                    Ok(()) => {
                        jobs.remove(&m.id);
                    }
                    Err(_) if cancel.load(Ordering::Relaxed) => {
                        if let Some(j) = jobs.get_mut(&m.id) {
                            j.status = Download::Paused { downloaded: have, total: m.size };
                        }
                    }
                    Err(e) => {
                        if let Some(j) = jobs.get_mut(&m.id) {
                            j.status = Download::Failed { error: e.to_string(), downloaded: have, total: m.size };
                        }
                    }
                }
            })
            .map_err(|e| Error::new(e.to_string()))?;
        Ok(())
    }

    /// Pause a running download; the partial file is kept so it can resume.
    pub fn pause_download(&self, id: &str) {
        if let Some(j) = self.jobs.lock().unwrap().get(id) {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Delete an installed pack or a partial download.
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let m = self.catalog.get(id).ok_or_else(|| Error::new(format!("unknown model {id}")))?;
        if let Some(j) = self.jobs.lock().unwrap().remove(id) {
            j.cancel.store(true, Ordering::Relaxed);
            // Give the download thread a moment to release the file (Windows can't delete open files).
            thread::sleep(Duration::from_millis(300));
        }
        let dir = self.pack_dir(m);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    fn set_status(&self, id: &str, status: Download) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(id) {
            j.status = status;
        }
    }

    fn run_download(&self, m: &Model, cancel: &AtomicBool) -> Result<(), Error> {
        fs::create_dir_all(self.pack_dir(m))?;
        let part = self.part_path(m);
        let mut have = fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
        if have > m.size {
            fs::remove_file(&part)?;
            have = 0;
        }
        if have < m.size {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .max_redirects(0)
                .max_redirects_will_error(false)
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(30)))
                .timeout_recv_response(Some(Duration::from_secs(60)))
                .user_agent(concat!("abm-local-ai/", env!("CARGO_PKG_VERSION")))
                .build()
                .into();
            let mut url = m.url.clone();
            let mut response = None;
            for _ in 0..=MAX_REDIRECTS {
                let uri: ureq::http::Uri = url.parse().map_err(|_| Error::new(format!("bad download URL {url}")))?;
                let host = uri.host().unwrap_or_default();
                if uri.scheme_str() != Some("https") || !self.catalog.host_allowed(host) {
                    return Err(Error::new(format!("refusing to download from {host}: not an allowed host")));
                }
                let mut req = agent.get(&url);
                if have > 0 {
                    req = req.header("Range", &format!("bytes={have}-"));
                }
                let r = req.call().map_err(|e| Error::new(format!("download failed: {e}")))?;
                let status = r.status().as_u16();
                if (300..400).contains(&status) {
                    url = r
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .ok_or_else(|| Error::new("redirect without a location"))?
                        .to_owned();
                    continue;
                }
                response = Some((status, r));
                break;
            }
            let (status, r) = response.ok_or_else(|| Error::new("too many redirects"))?;
            let append = match status {
                206 => true,
                200 => false, // server ignored the range: start over
                416 if have > 0 => {
                    // Range not satisfiable: the partial file is stale. Start over next time.
                    fs::remove_file(&part)?;
                    return Err(Error::new("the partial download was out of date; please try again"));
                }
                s => return Err(Error::new(format!("download failed: the server returned HTTP {s}"))),
            };
            if !append {
                have = 0;
            }
            let mut out = OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part)?;
            let mut body = r.into_body().into_reader();
            let mut buf = vec![0u8; 1 << 20];
            let (mut window_start, mut window_bytes, mut rate) = (Instant::now(), 0u64, 0.0);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::new("paused"));
                }
                let n = body.read(&mut buf).map_err(|e| Error::new(format!("download interrupted: {e}")))?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                have += n as u64;
                window_bytes += n as u64;
                let elapsed = window_start.elapsed().as_secs_f64();
                if elapsed >= 1.0 {
                    rate = window_bytes as f64 / elapsed;
                    (window_start, window_bytes) = (Instant::now(), 0);
                }
                self.set_status(&m.id, Download::Downloading { downloaded: have, total: m.size, bytes_per_s: rate });
            }
            out.sync_all()?;
        }
        if have != m.size {
            return Err(Error::new(format!("download incomplete ({have} of {} bytes); resume to continue", m.size)));
        }
        self.set_status(&m.id, Download::Verifying);
        if sha256_file(&part)? != m.sha256 {
            fs::remove_file(&part)?;
            return Err(Error::new("the downloaded file failed its integrity check and was deleted; please try again"));
        }
        let path = self.model_path(m);
        fs::rename(&part, &path)?;
        self.write_record(m, &path)?;
        Ok(())
    }
}

/// Small persistent settings (which pack is active).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Settings {
    pub active: Option<String>,
}

impl Settings {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("settings.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        fs::read(Self::path(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), Error> {
        fs::create_dir_all(data_dir)?;
        fs::write(Self::path(data_dir), serde_json::to_vec_pretty(self).unwrap())?;
        Ok(())
    }
}
