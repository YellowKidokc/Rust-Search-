use crate::{indexer, store};
use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

pub fn run(name: Option<String>, explicit: Option<PathBuf>) -> Result<()> {
    let base = match name.as_deref() {
        Some(n) => store::named_base(n)?,
        None => store::find_base(explicit.as_deref())?,
    };
    let index = store::load(&base)?;
    let roots: Vec<PathBuf> = index.roots.iter().map(PathBuf::from).collect();
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |e| {
            let _ = tx.send(e);
        },
        Config::default(),
    )?;
    for root in &roots {
        watcher.watch(root, RecursiveMode::Recursive)?;
    }
    println!("Watching {} root(s). Press Ctrl+C to stop.", roots.len());
    loop {
        let first: Event = rx.recv()?.map_err(anyhow::Error::from)?;
        let mut paths: HashSet<PathBuf> = first.paths.into_iter().collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(wait) {
                Ok(Ok(e)) => paths.extend(e.paths),
                Ok(Err(e)) => eprintln!("warning: {e}"),
                Err(_) => break,
            }
        }
        for p in paths.iter().filter(|p| indexer::is_supported(p)) {
            if p.exists() {
                println!("[+] indexed: {}", p.display())
            } else {
                println!("[-] removed: {}", p.display())
            }
        }
        indexer::run(&roots, name.as_deref())?;
    }
}
