//! End-to-end checks of the stage-1 acceptance criteria, run against the built binary.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

struct Env {
    _tmp: tempfile::TempDir,
    base: PathBuf,
}
impl Env {
    fn new() -> Env {
        let tmp = match std::env::var_os("TPSEARCH_TEST_TMP") {
            Some(root) => tempfile::tempdir_in(root).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let base = dunce::canonicalize(tmp.path()).unwrap();
        Env { _tmp: tmp, base }
    }
    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let o = Command::new(env!("CARGO_BIN_EXE_tpsearch"))
            .args(args)
            .env("TPSEARCH_CONFIG", self.base.join("cfg/config.toml"))
            .env("TPSEARCH_DATA", self.base.join("data"))
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stdout).into(),
            String::from_utf8_lossy(&o.stderr).into(),
        )
    }
    fn ok(&self, args: &[&str]) -> String {
        let (ok, out, err) = self.run(args);
        assert!(ok, "{args:?} failed:\n{out}\n{err}");
        out
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
}

const DOC: &str = "---\ntitle: Grace Paper\nauthor: D. L.\n---\n# Intro\n\nOpening paragraph.\n\n## Field\nThe grace field couples to curvature.\nSecond line here.\n";

fn snapshot(dir: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let m = e.metadata().unwrap();
            (
                e.path().display().to_string(),
                (fs::read(e.path()).unwrap(), m.modified().unwrap()),
            )
        })
        .collect()
}

fn setup() -> (Env, PathBuf, PathBuf) {
    let env = Env::new();
    let src = env.base.join("src");
    let nas = env.base.join("nas");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::create_dir_all(src.join("node_modules")).unwrap();
    fs::create_dir_all(&nas).unwrap();
    fs::write(src.join("paper.md"), DOC).unwrap();
    fs::write(src.join("sub/paper-copy.md"), DOC).unwrap();
    fs::write(
        src.join("notes.txt"),
        "plain notes about entropy\n\nand judgment\n",
    )
    .unwrap();
    fs::write(
        src.join("talk.srt"),
        "1\n00:00:01,000 --> 00:00:04,000\nwelcome to the grace lecture\n",
    )
    .unwrap();
    fs::write(
        src.join("paper.analysis.json"),
        r#"{"source_path": "paper.md", "summary": "grace field analysis", "tags": ["a","b"]}"#,
    )
    .unwrap();
    fs::write(src.join("scan.pdf"), b"%PDF-1.4 fake").unwrap();
    fs::write(
        src.join("page.html"),
        "<html><title>Web</title><body><p>Logos &amp; information</p></body></html>",
    )
    .unwrap();
    fs::write(src.join("node_modules/skip.md"), "grace should be excluded").unwrap();
    fs::write(src.join(".hidden.md"), "grace hidden").unwrap();
    fs::write(
        nas.join("remote.md"),
        "remote coherence document on the NAS\n",
    )
    .unwrap();
    let (s, n) = (src.display().to_string(), nas.display().to_string());
    env.ok(&["collection", "new", "docs", "--root", &s, "--root", &n]);
    (env, src, nas)
}

#[test]
fn acceptance() {
    let (env, src, nas) = setup();
    let before = snapshot(&env.base.join("src"));

    // Dry run writes nothing.
    let dry = env.ok(&["index", "-c", "docs", "--dry-run"]);
    assert!(dry.contains("Dry run"), "{dry}");
    assert!(!env.base.join("data/collections/docs/index.sqlite").exists());

    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(st["files_seen"], 8, "{st}"); // excludes node_modules and hidden
    assert_eq!(st["failed"], 0);

    // Keyword search works with AI disabled (default settings).
    let r = env.json(&["search", "-c", "docs", "grace", "field", "--json"]);
    let hits = r["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    let h = hits
        .iter()
        .find(|h| h["location"]["kind"] == "line")
        .unwrap();
    // Duplicate copies collapse: paper.md and its copy count as one source.
    assert!(h["copies"].as_array().unwrap().len() == 1, "{h}");
    // The derived analysis JSON links to paper.md, so it does not add a source.
    assert_eq!(r["independent_sources"], 1, "{r}");
    // Location is correct and the quote is re-verified against the source.
    let line = h["location"]["start"].as_u64().unwrap() as usize;
    assert_eq!(
        DOC.lines().nth(line - 1).unwrap(),
        "The grace field couples to curvature."
    );
    assert_eq!(h["quote_check"], "verified against source lines");
    assert_eq!(h["location"]["section"], "Field");
    let pid = h["passage_id"].as_i64().unwrap().to_string();
    let loc = env.ok(&["open", "-c", "docs", &pid, "--print"]);
    assert!(
        loc.trim().ends_with(&format!("paper.md:{line}")) || loc.contains("paper-copy.md"),
        "{loc}"
    );

    // Exact phrase vs keywords.
    let p = env.json(&["search", "-c", "docs", "--phrase", "field grace", "--json"]);
    assert_eq!(p["matching_passages"], 0);
    // Transcript timestamp, excluded folders, hidden files.
    let t = env.json(&["search", "-c", "docs", "lecture", "--json"]);
    assert_eq!(t["hits"][0]["location"]["t_start"], "00:00:01.000");
    assert_eq!(t["hits"][0]["representation"], "transcript");
    let x = env.json(&["search", "-c", "docs", "excluded", "--json"]);
    assert_eq!(x["matching_passages"], 0);
    // Filters: type, metadata.
    let m = env.json(&[
        "search",
        "-c",
        "docs",
        "grace",
        "--meta",
        "author=D. L.",
        "--type",
        "md",
        "--json",
    ]);
    assert!(m["hits"]
        .as_array()
        .unwrap()
        .iter()
        .all(|h| h["ext"] == "md"));
    assert!(m["matching_files"].as_i64().unwrap() >= 1);
    let html = env.json(&["search", "-c", "docs", "logos", "--json"]);
    assert_eq!(html["hits"][0]["text"], "Logos & information");

    // Unchanged files are not re-extracted.
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(
        (st["extracted"].as_u64(), st["unchanged"].as_u64()),
        (Some(0), Some(8)),
        "{st}"
    );
    // Timestamp-only change: rehash, still no extraction.
    let f = fs::File::options()
        .write(true)
        .open(src.join("paper.md"))
        .unwrap();
    f.set_modified(SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    drop(f);
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(
        (st["extracted"].as_u64(), st["rehashed_unchanged"].as_u64()),
        (Some(0), Some(1)),
        "{st}"
    );

    // Unsupported/pending formats are visible.
    let s = env.json(&["status", "-c", "docs", "--json"]);
    assert_eq!(s["files_by_status"]["pending_adapter"], 1, "{s}");

    // Folder report export.
    let rep = env.json(&["export-folder", "-c", "docs", &src.display().to_string()]);
    assert_eq!(rep["schema"], "tpsearch.folder_report/v1");
    assert_eq!(rep["folder"]["direct_files"], 6, "{rep}");
    assert_eq!(rep["child_folders"][0]["files_recursive"], 1);
    assert!(rep["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["sha256"].as_str().is_some_and(|s| s.len() == 64)));
    let out = env.base.join("reports");
    env.ok(&[
        "export-folder",
        "-c",
        "docs",
        &src.display().to_string(),
        "--recursive",
        "--out",
        &out.display().to_string(),
    ]);
    assert!(out.join("manifest.json").exists());

    // Source files remain unchanged (bytes; and no index or report files inside roots).
    let mut after = snapshot(&env.base.join("src"));
    let mut b = before.clone();
    let key = src.join("paper.md").display().to_string();
    after.get_mut(&key).unwrap().1 = b[&key].1; // the test itself changed this mtime
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        b.keys().collect::<Vec<_>>()
    );
    for (k, v) in &mut b {
        assert_eq!(after[k], *v, "{k} changed");
    }

    // Offline NAS root keeps its records.
    let away = env.base.join("nas-unplugged");
    fs::rename(&nas, &away).unwrap();
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(
        (st["offline_roots"].as_u64(), st["marked_missing"].as_u64()),
        (Some(1), Some(0)),
        "{st}"
    );
    let r = env.json(&["search", "-c", "docs", "coherence", "--json"]);
    assert_eq!(r["hits"][0]["root_offline"], true, "{r}");
    fs::rename(&away, &nas).unwrap();

    // A deleted file in an online root becomes a visible stale record, excluded by default.
    fs::remove_file(src.join("notes.txt")).unwrap();
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(st["marked_missing"], 1);
    assert_eq!(
        env.json(&["search", "-c", "docs", "entropy", "--json"])["matching_passages"],
        0
    );
    assert_eq!(
        env.json(&[
            "search",
            "-c",
            "docs",
            "entropy",
            "--include-missing",
            "--json"
        ])["matching_passages"],
        1
    );
}

#[test]
fn ai_cannot_be_enabled_in_stage_one() {
    let (env, _, _) = setup();
    let (ok, _, err) = env.run(&["collection", "set", "docs", "depth.semantic", "true"]);
    assert!(!ok && err.contains("not available"), "{err}");
    let (ok, _, _) = env.run(&["collection", "set", "docs", "depth.paid_api", "true"]);
    assert!(!ok);
}

#[test]
fn overlapping_roots_ingest_once() {
    let (env, src, _) = setup();
    let sub = src.join("sub").display().to_string();
    env.ok(&["collection", "add-root", "docs", &sub]);
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(st["files_seen"], 8, "{st}");
}

#[cfg(unix)]
#[test]
fn symlink_loops_are_safe() {
    let (env, src, _) = setup();
    std::os::unix::fs::symlink(&src, src.join("sub/loop")).unwrap();
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(
        st["files_seen"], 8,
        "symlinks not followed by default: {st}"
    );
    env.ok(&["collection", "set", "docs", "follow_symlinks", "true"]);
    let st = env.json(&["index", "-c", "docs", "--json"]);
    assert_eq!(st["files_seen"], 8, "loop avoided when following: {st}");
}

#[test]
fn legacy_markdown_index_still_works() {
    let env = Env::new();
    let d = env.base.join("legacy");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("a.md"),
        "---\ntitle: Legacy\ntags: [resurrection]\nscore_total: 80\n---\nbody text\n",
    )
    .unwrap();
    env.ok(&["index", &d.display().to_string()]);
    let out = env.ok(&[
        "query",
        "--index",
        &d.join(".tpsearch").display().to_string(),
        "--tag",
        "resurrection",
        "--min-score",
        "70",
    ]);
    assert!(out.contains("Legacy"), "{out}");
}
