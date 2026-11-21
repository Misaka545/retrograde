use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::error_log;
use crate::scanner::Scanner;

const DEBOUNCE_DELAY: Duration = Duration::from_secs(4);

/// Watches the library root folders recursively and emits batches of changed/new audio paths
/// after a 4-second quiet window.
pub struct FolderWatcher {
    watcher: RecommendedWatcher,
    roots: Vec<PathBuf>,
}

impl FolderWatcher {
    pub fn start(
        roots: &[PathBuf],
    ) -> Result<(Self, mpsc::UnboundedReceiver<Vec<PathBuf>>), notify::Error> {
        let top_roots = top_level_roots(roots);
        let pending = Arc::new(Mutex::new((HashSet::<PathBuf>::new(), Instant::now())));
        let (tx, rx) = mpsc::unbounded_channel();

        let pending_cb = Arc::clone(&pending);
        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            let Ok(ev) = res else {
                return;
            };
            // Only new or changed files are picked up. A removal or a rename away is ignored: a
            // moved folder and an offline drive look the same, so deletions are left to the manual
            // rescan, which asks first.
            if !matches!(
                ev.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any
            ) {
                return;
            }
            let mut changed = false;
            if let Ok(mut guard) = pending_cb.lock() {
                for path in ev.paths {
                    if path.is_dir() || (Scanner::is_audio_file(&path) && path.exists()) {
                        guard.0.insert(path);
                        changed = true;
                    }
                }
                if changed {
                    guard.1 = Instant::now();
                }
            }
        })?;

        for root in &top_roots {
            if let Err(err) = watcher.watch(root, RecursiveMode::Recursive) {
                error_log::write("watch", &format!("{}: {err}", root.display()));
            }
        }

        let pending_flush = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                if tx.is_closed() {
                    break;
                }
                let ready = {
                    let Ok(mut guard) = pending_flush.lock() else {
                        continue;
                    };
                    if guard.0.is_empty() || guard.1.elapsed() < DEBOUNCE_DELAY {
                        continue;
                    }
                    let all: Vec<PathBuf> = guard.0.drain().collect();
                    let mut ready = Vec::new();
                    for p in all {
                        if p.is_dir() {
                            ready.push(p);
                        } else if p.exists() {
                            if File::open(&p).is_ok() {
                                ready.push(p);
                            } else {
                                guard.0.insert(p);
                                guard.1 = Instant::now();
                            }
                        }
                    }
                    ready
                };
                if !ready.is_empty() && tx.send(ready).is_err() {
                    break;
                }
            }
        });

        Ok((
            Self {
                watcher,
                roots: top_roots,
            },
            rx,
        ))
    }

    pub fn update_roots(&mut self, next_roots: &[PathBuf]) {
        let next = top_level_roots(next_roots);
        for old in &self.roots {
            if !next.contains(old) {
                let _ = self.watcher.unwatch(old);
            }
        }
        for r in &next {
            if !self.roots.contains(r) {
                let _ = self.watcher.watch(r, RecursiveMode::Recursive);
            }
        }
        self.roots = next;
    }
}

fn top_level_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut cleaned: Vec<PathBuf> = roots
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| {
            let s = p.to_string_lossy();
            let trimmed = s.trim_end_matches('/');
            if trimmed.is_empty() {
                PathBuf::from("/")
            } else {
                PathBuf::from(trimmed)
            }
        })
        .collect();
    cleaned.sort_by_key(|p| p.as_os_str().len());

    let mut kept: Vec<PathBuf> = Vec::new();
    for r in cleaned {
        let is_child = kept.iter().any(|k| Path::new(&r).starts_with(k));
        if !is_child {
            kept.push(r);
        }
    }
    kept
}
