use crate::{
    model::{Codec, Object},
    storage::{atomic_write, hash_file},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use url::Url;
/// A condition that stops a run without indicating damage. The UI maps each to one sentence
/// and one next action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    Stopped,
    GameRunning,
    UnknownVersion,
    DiskSpace,
    Network,
}
impl std::fmt::Display for Blocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stopped => "Run stopped at a recoverable boundary",
            Self::GameRunning => "Waiting for the game or its installer to close",
            Self::UnknownVersion => {
                "Game version is not one this release supports; no files were changed"
            }
            Self::DiskSpace => "Not enough free disk space",
            Self::Network => "Download server is unreachable; progress is kept",
        })
    }
}
#[derive(Clone, Default)]
pub struct Control(pub Arc<AtomicBool>);
impl Control {
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn check(&self) -> Result<()> {
        ensure!(!self.0.load(Ordering::Acquire), Blocker::Stopped);
        Ok(())
    }
}
pub trait ObjectSource {
    fn fetch(
        &self,
        object: &Object,
        cache: &Path,
        control: &Control,
        progress: &dyn Fn(u64, u64),
    ) -> Result<PathBuf>;
}
pub struct LocalSource {
    pub directory: PathBuf,
}
pub struct HttpSource {
    pub origin: Url,
    /// Zero means unlimited.
    pub max_bytes_per_second: u64,
    /// Delay before each retry; its length is the retry budget after the first attempt.
    pub retry_delays: Vec<Duration>,
}
fn cached(object: &Object, cache: &Path) -> Result<Option<PathBuf>> {
    fs::create_dir_all(cache)?;
    let p = cache.join(&object.hash);
    if p.exists() {
        if hash_file(&p)? == (object.hash.clone(), object.encoded_size) {
            return Ok(Some(p));
        }
        fs::remove_file(p)?;
    }
    Ok(None)
}
impl ObjectSource for LocalSource {
    fn fetch(
        &self,
        o: &Object,
        cache: &Path,
        c: &Control,
        p: &dyn Fn(u64, u64),
    ) -> Result<PathBuf> {
        o.validate()?;
        if let Some(path) = cached(o, cache)? {
            return Ok(path);
        }
        let part = cache.join(format!("{}.part", o.hash));
        let mut input = File::open(self.directory.join(&o.hash))?;
        let mut out = File::create(&part)?;
        copy_bounded(&mut input, &mut out, o.encoded_size, c, p)?;
        out.sync_all()?;
        ensure!(
            hash_file(&part)? == (o.hash.clone(), o.encoded_size),
            "Object integrity check failed"
        );
        let path = cache.join(&o.hash);
        fs::rename(part, &path)?;
        crate::storage::sync_dir(cache)?;
        Ok(path)
    }
}
#[derive(Serialize, Deserialize)]
struct Resume {
    etag: String,
}

/// Default delays before the second, third and fourth attempts.
pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(8),
    Duration::from_secs(30),
];
/// Longest server-requested delay honored before giving up on this run.
const MAX_SERVER_DELAY: Duration = Duration::from_secs(600);
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

enum Failure {
    /// Temporary: retry within the budget, waiting at least the server's delay if given.
    Retry(anyhow::Error, Option<Duration>),
    /// Permanent for this run.
    Stop(anyhow::Error),
}
type Attempt<T> = std::result::Result<T, Failure>;

fn stop<T, E: Into<anyhow::Error>>(r: std::result::Result<T, E>) -> Attempt<T> {
    r.map_err(|e| Failure::Stop(e.into()))
}
fn retry<T, E: Into<anyhow::Error>>(r: std::result::Result<T, E>) -> Attempt<T> {
    r.map_err(|e| Failure::Retry(e.into(), None))
}
fn transient(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}
/// Spreads retries by up to 20 percent either side so clients do not retry in lockstep.
fn jitter(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let spread = u64::from(nanos % 401);
    base * u32::try_from(800 + spread).unwrap_or(1000) / 1000
}
async fn wait(duration: Duration, c: &Control) -> Result<()> {
    let end = tokio::time::Instant::now() + duration;
    while tokio::time::Instant::now() < end {
        c.check()?;
        let left = end - tokio::time::Instant::now();
        tokio::time::sleep(left.min(Duration::from_millis(100))).await;
    }
    Ok(())
}
fn discard(paths: &[&Path]) -> Result<()> {
    for path in paths {
        match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}
fn complete(part: &Path, o: &Object) -> Result<bool> {
    Ok(fs::metadata(part).is_ok_and(|m| m.len() == o.encoded_size)
        && hash_file(part)? == (o.hash.clone(), o.encoded_size))
}

/// One object download: where it comes from and where its partial bytes live.
#[derive(Clone, Copy)]
struct Download<'a> {
    url: &'a Url,
    object: &'a Object,
    part: &'a Path,
    validator: &'a Path,
}

impl HttpSource {
    pub fn new(origin: Url, max_bytes_per_second: u64) -> Self {
        Self {
            origin,
            max_bytes_per_second,
            retry_delays: RETRY_DELAYS.to_vec(),
        }
    }

    fn check_origin(&self) -> Result<()> {
        let loopback = matches!(
            self.origin.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]")
        );
        ensure!(
            self.origin.scheme() == "https" || (self.origin.scheme() == "http" && loopback),
            "Object origin requires HTTPS"
        );
        ensure!(
            self.origin.username().is_empty()
                && self.origin.password().is_none()
                && self.origin.query().is_none(),
            "Origin must not contain credentials or query parameters"
        );
        Ok(())
    }

    async fn attempt(
        &self,
        client: &reqwest::Client,
        d: &Download<'_>,
        c: &Control,
        p: &dyn Fn(u64, u64),
    ) -> Attempt<()> {
        let Download {
            url,
            object: o,
            part,
            validator,
        } = *d;
        if stop(complete(part, o))? {
            return Ok(());
        }
        let saved = fs::read(validator)
            .ok()
            .and_then(|b| serde_json::from_slice::<Resume>(&b).ok());
        let mut offset = fs::metadata(part).map_or(0, |m| m.len());
        if offset >= o.encoded_size || saved.is_none() {
            offset = 0;
        }
        let mut request = client
            .get(url.clone())
            .header("Accept-Encoding", "identity");
        if let (true, Some(v)) = (offset > 0, &saved) {
            request = request
                .header("Range", format!("bytes={offset}-"))
                .header("If-Range", &v.etag);
        }
        let mut response = retry(retry(
            tokio::time::timeout(IDLE_TIMEOUT, request.send()).await,
        )?)?;
        let status = response.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            if stop(complete(part, o))? {
                return Ok(());
            }
            stop(discard(&[part, validator]))?;
            return Err(Failure::Retry(
                anyhow::anyhow!("Server rejected the resume range"),
                None,
            ));
        }
        if transient(status) {
            return Err(Failure::Retry(
                anyhow::anyhow!("Download server returned HTTP {}", status.as_u16()),
                retry_after(response.headers()),
            ));
        }
        if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(Failure::Stop(anyhow::anyhow!(
                "Download server returned HTTP {}",
                status.as_u16()
            )));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        if status == reqwest::StatusCode::OK {
            offset = 0;
        } else {
            let expected = format!("bytes {}-{}/{}", offset, o.encoded_size - 1, o.encoded_size);
            let range = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok());
            let same = offset == 0 || etag.as_deref() == saved.as_ref().map(|v| v.etag.as_str());
            if range != Some(expected.as_str()) || !same {
                stop(discard(&[part, validator]))?;
                return Err(Failure::Retry(
                    anyhow::anyhow!("Partial response does not continue the saved download"),
                    None,
                ));
            }
        }
        if response
            .content_length()
            .is_some_and(|len| len != o.encoded_size - offset)
        {
            return Err(Failure::Retry(
                anyhow::anyhow!("Object response length differs"),
                None,
            ));
        }
        match etag.filter(|v| v.starts_with('"') && v.ends_with('"')) {
            Some(etag) => stop(atomic_write(
                validator,
                &stop(serde_json::to_vec(&Resume { etag }))?,
            ))?,
            None => stop(discard(&[validator]))?,
        }
        let mut out = stop(
            fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(offset == 0)
                .append(offset > 0)
                .open(part),
        )?;
        let mut done = offset;
        let started = tokio::time::Instant::now();
        p(done, o.encoded_size);
        while let Some(chunk) = retry(retry(
            tokio::time::timeout(IDLE_TIMEOUT, response.chunk()).await,
        )?)? {
            stop(c.check())?;
            if done + chunk.len() as u64 > o.encoded_size {
                drop(out);
                stop(discard(&[part, validator]))?;
                return Err(Failure::Retry(
                    anyhow::anyhow!("Object exceeds declared size"),
                    None,
                ));
            }
            stop(out.write_all(&chunk))?;
            done += chunk.len() as u64;
            p(done, o.encoded_size);
            if self.max_bytes_per_second > 0 {
                let target = Duration::from_secs_f64(
                    (done - offset) as f64 / self.max_bytes_per_second as f64,
                );
                let elapsed = started.elapsed();
                if target > elapsed {
                    stop(wait(target - elapsed, c).await)?;
                }
            }
        }
        stop(out.sync_all())?;
        drop(out);
        if done != o.encoded_size {
            return Err(Failure::Retry(
                anyhow::anyhow!("Object download incomplete"),
                None,
            ));
        }
        if !stop(complete(part, o))? {
            stop(discard(&[part, validator]))?;
            return Err(Failure::Retry(
                anyhow::anyhow!("Object integrity check failed"),
                None,
            ));
        }
        Ok(())
    }
}

impl ObjectSource for HttpSource {
    fn fetch(
        &self,
        o: &Object,
        cache: &Path,
        c: &Control,
        p: &dyn Fn(u64, u64),
    ) -> Result<PathBuf> {
        o.validate()?;
        if let Some(path) = cached(o, cache)? {
            return Ok(path);
        }
        self.check_origin()?;
        let part = cache.join(format!("{}.part", o.hash));
        let validator = cache.join(format!("{}.resume", o.hash));
        let url = self.origin.join(&format!("objects/{}", o.hash))?;
        tokio::runtime::Runtime::new()?.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            for attempt in 0..=self.retry_delays.len() {
                c.check()?;
                match self
                    .attempt(
                        &client,
                        &Download {
                            url: &url,
                            object: o,
                            part: &part,
                            validator: &validator,
                        },
                        c,
                        p,
                    )
                    .await
                {
                    Ok(()) => {
                        let dst = cache.join(&o.hash);
                        fs::rename(&part, &dst)?;
                        crate::storage::sync_dir(cache)?;
                        discard(&[&validator])?;
                        return Ok(dst);
                    }
                    Err(Failure::Stop(e)) => return Err(e),
                    Err(Failure::Retry(e, server)) => {
                        c.check()?;
                        let Some(base) = self.retry_delays.get(attempt) else {
                            return Err(e.context(Blocker::Network));
                        };
                        let requested = server.unwrap_or_default();
                        if requested > MAX_SERVER_DELAY {
                            return Err(e.context(Blocker::Network));
                        }
                        wait(jitter(*base).max(requested), c).await?;
                    }
                }
            }
            bail!("Download retry budget exhausted")
        })
    }
}
pub fn copy_bounded(
    input: &mut dyn Read,
    out: &mut dyn Write,
    size: u64,
    c: &Control,
    progress: &dyn Fn(u64, u64),
) -> Result<()> {
    let mut buf = [0u8; 128 * 1024];
    let mut done = 0;
    loop {
        c.check()?;
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ensure!(
            done + n as u64 <= size,
            "Decoded output exceeds declared size"
        );
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, size);
    }
    ensure!(done == size, "Decoded output is shorter than declared size");
    Ok(())
}
pub fn decode(path: &Path, o: &Object, out: &mut dyn Write, c: &Control) -> Result<()> {
    let file = File::open(path)?;
    match o.codec {
        Codec::Raw => copy_bounded(
            &mut std::io::BufReader::new(file),
            out,
            o.decoded_size,
            c,
            &|_, _| {},
        ),
        Codec::Zstd => {
            let mut decoder =
                zstd::stream::read::Decoder::new(file).context("Invalid zstd object")?;
            decoder.window_log_max(26)?;
            copy_bounded(&mut decoder, out, o.decoded_size, c, &|_, _| {})
        }
    }
}
