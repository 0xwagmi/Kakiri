//! Play a link without downloading it first.
//!
//! `yt-dlp -g` hands us a direct CDN URL for the audio stream; this module turns
//! that URL into something symphonia can read, by answering its reads with HTTP
//! range requests. No temp file, nothing added to the library.
//!
//! Two things about these CDN URLs shape the code:
//!
//! - they are **seek-heavy**: the container's index (`moov`) sits at the end of
//!   the file, so probing jumps to EOF, then back. Seeking backwards must refill
//!   the chunk, or the reader silently answers from the wrong offset.
//! - they **expire** (hours, not days) and are rate-limited per IP, so the caller
//!   resolves once per playback and re-resolves on failure rather than caching a
//!   URL forever.
//!
//! Reads happen on the per-voice decoder thread, never on the audio callback.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use symphonia::core::io::MediaSource;

/// Range requests start small: probing asks for 16 bytes at the file head and a
/// few hundred at the tail, and fetching 512 KB to answer that doubled the time
/// to first sound. The window then grows as playback becomes sequential, where a
/// large chunk is what you want (one request every few seconds).
const CHUNK_MIN: u64 = 32 * 1024;
const CHUNK_MAX: u64 = 1024 * 1024;

pub struct HttpSource {
    client: reqwest::blocking::Client,
    url: String,
    len: u64,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
    /// bytes actually pulled over the network, for tests and logging
    pub fetched: u64,
    /// current range window; grows on sequential reads, shrinks back on seeks
    chunk: u64,
    /// hint for the container, derived from the response's content type
    ext: Option<&'static str>,
    /// optional track cache: bytes that pass through are kept on disk
    sink: Option<CacheHandle>,
}

/// Marks a finished cache entry. `<path>.done` exists only once every block has
/// been written, so a replay can open the file instead of the network.
pub fn done_marker(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".done");
    PathBuf::from(p)
}

/// True when `path` was streamed through completely at least once.
pub fn is_complete(path: &Path) -> bool {
    done_marker(path).exists()
}

const BLOCK: u64 = 64 * 1024;

/// Per-track budget: a 4-hour stream must not fill the disk.
const TRACK_CAP: u64 = 100 * 1024 * 1024;

/// Bytes covered so far / total, for the UI to show what is cached.
pub fn cache_progress(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let len = meta.len();
    if is_complete(path) {
        return (len, len);
    }
    let cov = cov_path(path);
    let Ok(bytes) = std::fs::read(&cov) else {
        return (0, len);
    };
    let total_blocks = len.div_ceil(BLOCK).max(1);
    let mut covered = 0u64;
    for b in 0..total_blocks {
        let idx = (b / 8) as usize;
        let bit = 1u8 << (b % 8);
        if bytes.get(idx).copied().unwrap_or(0) & bit != 0 {
            covered += (BLOCK).min(len.saturating_sub(b * BLOCK));
        }
    }
    (covered, len)
}

fn cov_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".cov");
    PathBuf::from(p)
}

/// Writes streamed bytes into the cache file and remembers which blocks arrived.
///
/// Coverage is a bitmap rather than a byte count because reads are not strictly
/// sequential: the container index (`moov`) sits at the end of the file, so the
/// tail arrives during probing and leaves a hole in the middle. "Complete" must
/// mean every block, not "we fetched N bytes".
struct CacheSink {
    file: File,
    path: PathBuf,
    blocks: Vec<u64>,
    total_blocks: u64,
    covered_bytes: u64,
    written: u64,
    cap: u64,
    dirty: bool,
}

impl CacheSink {
    fn new(path: PathBuf, len: u64, cap: u64) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = OpenOptions::new().read(true).write(true).create(true).open(&path)?;
        file.set_len(len)?;
        let total_blocks = len.div_ceil(BLOCK).max(1);
        Ok(Self {
            file,
            path,
            blocks: vec![0u64; (total_blocks as usize).div_ceil(64)],
            total_blocks,
            covered_bytes: 0,
            written: 0,
            cap,
            dirty: false,
        })
    }

    fn len_total(&self) -> u64 {
        self.total_blocks * BLOCK
    }

    fn store(&mut self, offset: u64, data: &[u8]) {
        // over the per-track budget: keep playing, stop caching
        if self.written + data.len() as u64 > self.cap && self.written > 0 {
            return;
        }
        if self.file.seek(SeekFrom::Start(offset)).is_err() {
            return;
        }
        if self.file.write_all(data).is_err() {
            return;
        }
        self.written += data.len() as u64;

        let first = offset / BLOCK;
        let last = ((offset + data.len() as u64).saturating_sub(1) / BLOCK)
            .min(self.total_blocks.saturating_sub(1));
        for b in first..=last {
            let word = (b / 64) as usize;
            let bit = 1u64 << (b % 64);
            if self.blocks[word] & bit == 0 {
                self.blocks[word] |= bit;
                self.covered_bytes += BLOCK.min(self.len_total() - b * BLOCK);
                self.dirty = true;
            }
        }
        self.persist();
    }

    /// Coverage lives next to the track so a *later* process (or the UI, from
    /// another thread) can say how much is cached without asking this one.
    fn persist(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let _ = self.file.flush();
        let total = self.len_total();
        if self.covered_bytes >= total {
            let _ = File::create(done_marker(&self.path));
            let _ = std::fs::remove_file(cov_path(&self.path));
            return;
        }
        let mut bytes = vec![0u8; (self.total_blocks as usize).div_ceil(8)];
        for b in 0..self.total_blocks {
            if self.blocks[(b / 64) as usize] & (1u64 << (b % 64)) != 0 {
                bytes[(b / 8) as usize] |= 1u8 << (b % 8);
            }
        }
        let _ = std::fs::write(cov_path(&self.path), bytes);
    }

    fn next_uncovered(&self, max_blocks: u64) -> Option<(u64, u64)> {
        let first = (0..self.total_blocks)
            .find(|b| self.blocks[(b / 64) as usize] & (1u64 << (b % 64)) == 0)?;
        let mut last = first;
        while last + 1 < self.total_blocks
            && last + 1 - first < max_blocks
            && self.blocks[((last + 1) / 64) as usize] & (1u64 << ((last + 1) % 64)) == 0
        {
            last += 1;
        }
        Some((first * BLOCK, (last + 1) * BLOCK))
    }

    fn at_cap(&self) -> bool {
        self.written >= self.cap
    }
}

/// Shared handle so the player and the prefetcher write into one cache entry.
#[derive(Clone)]
pub struct CacheHandle(std::sync::Arc<std::sync::Mutex<CacheSink>>);

impl CacheHandle {
    pub fn new(path: PathBuf, len: u64) -> Option<Self> {
        CacheSink::new(path, len, TRACK_CAP).ok().map(|s| Self(std::sync::Arc::new(std::sync::Mutex::new(s))))
    }

    fn store(&self, offset: u64, data: &[u8]) {
        if let Ok(mut sink) = self.0.lock() {
            sink.store(offset, data);
        }
    }

    pub fn progress(&self) -> (u64, u64) {
        let sink = self.0.lock().unwrap();
        (sink.covered_bytes.min(sink.len_total()), sink.len_total())
    }

    /// Keep pulling the parts of the track that are not cached yet, honouring the
    /// per-track budget. Runs until complete, stopped, or the budget is reached —
    /// so pausing a small track still gets it cached, and a four-hour stream still
    /// only ever costs `TRACK_CAP`.
    pub fn prefetch(self, url: String, stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        use std::sync::atomic::Ordering;
        let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Wagmify")
            .build()
        else {
            return;
        };
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let (range, at_cap) = {
                let sink = self.0.lock().unwrap();
                let at_cap = sink.at_cap();
                (sink.next_uncovered(16), at_cap)
            };
            if at_cap {
                return;
            }
            let Some((start, end)) = range else {
                return; // nothing left to fetch
            };
            let want = format!("bytes={}-{}", start, end.saturating_sub(1));
            match client
                .get(&url)
                .header(reqwest::header::RANGE, want)
                .send()
                .and_then(|r| r.error_for_status())
            {
                Ok(resp) => match resp.bytes() {
                    Ok(body) if !body.is_empty() => self.store(start, &body),
                    _ => std::thread::sleep(Duration::from_millis(500)),
                },
                Err(_) => std::thread::sleep(Duration::from_millis(750)),
            }
        }
    }
}

impl HttpSource {
    /// HEAD-ish probe: one 2-byte range request tells us the size, whether the
    /// server honours ranges, and the container type.
    pub fn open(url: &str) -> Result<Self> {
        Ok(Self::open_cached(url, None)?.0)
    }

    /// Like [`open`], but every fetched byte is also written to `cache`, so a
    /// later replay of the same track can start from disk.
    pub fn open_cached(
        url: &str,
        cache: Option<PathBuf>,
    ) -> Result<(Self, Option<CacheHandle>)> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            // these CDNs dislike a missing UA
            .user_agent("Wagmify")
            .build()
            .context("http client")?;

        let resp = client
            .get(url)
            .header(reqwest::header::RANGE, "bytes=0-1")
            .send()?
            .error_for_status()?;

        let ext = match resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
        {
            t if t.contains("mp4") || t.contains("m4a") => Some("m4a"),
            t if t.contains("webm") => Some("webm"),
            t if t.contains("mpeg") || t.contains("mp3") => Some("mp3"),
            t if t.contains("ogg") || t.contains("opus") => Some("ogg"),
            _ => None,
        };

        // Content-Range: "bytes 0-1/10271496"
        let len = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);

        if len == 0 {
            return Err(anyhow!(
                "server did not report a length; cannot stream this link"
            ));
        }

        let sink = cache.and_then(|p| CacheHandle::new(p, len));

        Ok((Self {
            client,
            url: url.to_string(),
            sink: sink.clone(),
            len,
            pos: 0,
            buf: Vec::new(),
            buf_start: 0,
            fetched: 0,
            chunk: CHUNK_MIN,
            ext,
        }, sink))
    }

    /// Size of the stream in bytes, as reported by the server.
    pub fn byte_size(&self) -> u64 {
        self.len
    }

    /// Container hint for the probe, if the server told us.
    pub fn extension(&self) -> Option<&'static str> {
        self.ext
    }

    fn fill(&mut self) -> std::io::Result<()> {
        let end = (self.pos + self.chunk - 1).min(self.len - 1);
        let resp = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, format!("bytes={}-{}", self.pos, end))
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(std::io::Error::other)?;
        let body = resp.bytes().map_err(std::io::Error::other)?;
        if body.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty range response",
            ));
        }
        self.fetched += body.len() as u64;
        if let Some(sink) = self.sink.as_ref() {
            sink.store(self.pos, &body);
        }
        self.buf_start = self.pos;
        self.buf = body.to_vec();
        // read the window all the way through? then widen it for the next one
        self.chunk = (self.chunk * 2).min(CHUNK_MAX);
        Ok(())
    }
}

impl Read for HttpSource {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        // refill when the position left the buffered chunk in either direction
        let off = self.pos.saturating_sub(self.buf_start);
        if self.pos < self.buf_start || off >= self.buf.len() as u64 {
            self.fill()?;
        }
        let off = (self.pos - self.buf_start) as usize;
        let n = (self.buf.len() - off).min(out.len());
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let target = match from {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.len as i64 + d,
        };
        self.pos = target.clamp(0, self.len as i64) as u64;
        // a seek means we are not streaming sequentially any more: go small again,
        // so the next read (usually a few bytes of container index) is cheap
        self.chunk = CHUNK_MIN;
        Ok(self.pos)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wagmify-cache-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("track.m4a")
    }

    #[test]
    fn complete_coverage_publishes_the_entry() {
        let path = tmp("complete");
        let len = BLOCK * 4;
        let mut sink = CacheSink::new(path.clone(), len, u64::MAX).unwrap();
        // out of order, exactly like probing the tail before the middle
        sink.store(0, &vec![1u8; BLOCK as usize]);
        sink.store(BLOCK * 3, &vec![1u8; BLOCK as usize]);
        assert!(!is_complete(&path), "half a file must not count as complete");
        sink.store(BLOCK, &vec![1u8; BLOCK as usize * 2]);
        assert!(is_complete(&path));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), len);
    }

    #[test]
    fn per_track_cap_stops_writing() {
        let path = tmp("cap");
        let mut sink = CacheSink::new(path.clone(), BLOCK * 100, BLOCK).unwrap();
        sink.store(0, &vec![1u8; BLOCK as usize]);
        sink.store(BLOCK, &vec![1u8; BLOCK as usize]); // over the cap
        assert!(!is_complete(&path));
    }

    #[test]
    fn progress_is_readable_from_another_thread() {
        // what the UI does: ask the cache how full it is without holding the sink
        let path = tmp("progress");
        let len = BLOCK * 4;
        let handle = CacheHandle::new(path.clone(), len).unwrap();
        assert_eq!(cache_progress(&path).0, 0);
        handle.store(0, &vec![1u8; BLOCK as usize]);
        assert_eq!(cache_progress(&path), (BLOCK, len));
        handle.store(BLOCK, &vec![1u8; (BLOCK * 3) as usize]);
        assert_eq!(cache_progress(&path), (len, len));
        assert!(is_complete(&path));
    }

    #[test]
    fn prefetch_asks_only_for_the_blocks_it_lacks() {
        let path = tmp("gaps");
        let len = BLOCK * 8;
        let handle = CacheHandle::new(path.clone(), len).unwrap();
        handle.store(0, &vec![1u8; BLOCK as usize]);
        handle.store(BLOCK * 6, &vec![1u8; BLOCK as usize * 2]);
        let sink = handle.0.lock().unwrap();
        // first gap starts right after the first block
        assert_eq!(sink.next_uncovered(16), Some((BLOCK, BLOCK * 6)));
    }

    #[test]
    fn a_done_entry_is_reused_and_a_partial_one_is_not() {
        let path = tmp("reuse");
        assert!(!is_complete(&path));
        let mut sink = CacheSink::new(path.clone(), BLOCK, u64::MAX).unwrap();
        sink.store(0, &vec![0u8; BLOCK as usize]);
        assert!(is_complete(&path));
    }
}
