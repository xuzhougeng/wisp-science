//! Long-term markdown memory, ported from mangopi-cli's `MemoryManager`.
//!
//! Notes append to a per-day markdown file under `<root>/.wisp/memory`.
//! Search is keyword-scored (count × 10 + chunk-length bonus + recency bonus).

use glob::glob;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub struct MemoryManager {
    dir: PathBuf,
}

impl MemoryManager {
    pub fn new(root: &Path) -> Self {
        let dir = root.join(".wisp").join("memory");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn today_path(&self) -> PathBuf {
        self.dir
            .join(format!("{}.md", chrono::Local::now().format("%Y-%m-%d")))
    }

    pub fn append(&self, content: &str) -> std::io::Result<()> {
        use std::io::Write;
        let path = self.today_path();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(content.trim().as_bytes())?;
        f.write_all(b"\n\n")?;
        Ok(())
    }

    fn tokenize(text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn split_chunks(text: &str) -> Vec<String> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"\n\s*\n").unwrap());
        re.split(text)
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect()
    }

    /// Scored keyword search over all memory files, newest first.
    pub fn search(&self, query: &str, top_k: usize) -> String {
        let keywords = Self::tokenize(query);
        if keywords.is_empty() {
            return "empty query".into();
        }
        let pattern = format!("{}/*.md", self.dir.display());
        let mut scored: Vec<(i64, String, String)> = vec![];
        let mut paths: Vec<PathBuf> = glob(&pattern)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();
        paths.sort_by(|a, b| b.cmp(a)); // newest filename first
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mtime = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            for chunk in Self::split_chunks(&text) {
                let lower = chunk.to_ascii_lowercase();
                let mut score = 0;
                for kw in &keywords {
                    if lower.contains(kw) {
                        score += lower.matches(kw).count() as i64 * 10;
                    }
                }
                if score <= 0 {
                    continue;
                }
                score += (chunk.len() / 200).min(5) as i64;
                let age_days = (chrono::Utc::now().timestamp() - mtime) / 86_400;
                score += (30 - age_days).max(0);
                let label = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                scored.push((score, label, chunk.chars().take(2000).collect()));
            }
        }
        if scored.is_empty() {
            return "No memory found. Tip: append important user preferences, decisions, and non-obvious fixes so future sessions can recall them.".into();
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .take(top_k)
            .map(|(s, f, c)| format!("# {f} (score={s})\n{c}"))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}
