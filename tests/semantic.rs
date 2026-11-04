//! Deterministic local HTTP service tests; these do not claim real-model quality.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

struct Service {
    url: String,
    calls: Arc<AtomicUsize>,
    bad: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Service {
    fn start() -> Self {
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let bad = Arc::new(AtomicBool::new(false));
        let (c, s, b) = (calls.clone(), stop.clone(), bad.clone());
        let task = thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match socket.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut head = vec![];
                        let mut byte = [0];
                        while !head.ends_with(b"\r\n\r\n") {
                            if stream.read(&mut byte).unwrap() == 0 {
                                break;
                            }
                            head.push(byte[0]);
                        }
                        if head.is_empty() {
                            continue;
                        }
                        let head = String::from_utf8(head).unwrap();
                        let len = head
                            .lines()
                            .find_map(|s| {
                                s.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        let mut body = vec![0; len];
                        stream.read_exact(&mut body).unwrap();
                        let body: Value = serde_json::from_slice(&body).unwrap();
                        c.fetch_add(1, Ordering::SeqCst);
                        let vectors = body["input"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|t| {
                                let text = t.as_str().unwrap();
                                let mut v = vec![0.0; 384];
                                if text.contains("quantum") || text.contains("qubits") {
                                    v[0] = 1.0;
                                } else {
                                    v[1] = 1.0;
                                }
                                v
                            })
                            .collect::<Vec<_>>();
                        let result = if b.load(Ordering::SeqCst) {
                            json!({"embeddings":[]})
                        } else if head.starts_with("POST /v1/embeddings ") {
                            json!({"data":vectors.iter().enumerate().rev().map(|(i,v)|json!({"index":i,"embedding":v})).collect::<Vec<_>>()})
                        } else {
                            assert!(head.starts_with("POST /api/embed "));
                            assert_eq!(body["options"]["num_gpu"], 0);
                            json!({"embeddings":vectors})
                        };
                        let result = result.to_string();
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}",result.len()).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        Self {
            url,
            calls,
            bad,
            stop,
            task: Some(task),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        self.task.take().unwrap().join().unwrap();
    }
}
struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}
impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        fs::write(
            root.join("docs/quantum.txt"),
            "Recover damaged qubits with a quantum recovery code.",
        )
        .unwrap();
        fs::write(
            root.join("docs/kitchen.txt"),
            "Cook a delicious soup with carrots and potatoes.",
        )
        .unwrap();
        let e = Self { _tmp: tmp, root };
        e.ok(&[
            "collection",
            "new",
            "research",
            "--root",
            e.root.join("docs").to_str().unwrap(),
        ]);
        e.ok(&["index", "-c", "research"]);
        e
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tpsearch"))
            .args(args)
            .env("TPSEARCH_CONFIG", self.root.join("config.toml"))
            .env("TPSEARCH_DATA", self.root.join("data"))
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.root.join("data/collections/research/index.sqlite"))
            .unwrap()
    }
}

#[test]
fn incremental_embedding_ranking_and_invalidation() {
    let service = Service::start();
    let e = Env::new();
    let before = fs::read(e.root.join("config.toml")).unwrap();
    let args = [
        "embed",
        "-c",
        "research",
        "--api-url",
        &service.url,
        "--model",
        "fixture-minilm",
        "--batch-size",
        "1",
        "--json",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    assert_eq!(e.json(&dry)["pending"], 2);
    assert_eq!(service.calls(), 0);
    assert_eq!(fs::read(e.root.join("config.toml")).unwrap(), before);
    let report = e.json(&args);
    assert_eq!(report["embedded"], 2);
    assert_eq!(report["dims"], 384);
    assert_eq!(service.calls(), 2);
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embeddings WHERE dims=384 AND length(embedding)=1536",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let report = e.json(&["embed", "-c", "research", "--json"]);
    assert_eq!(report["embedded"], 0);
    assert_eq!(report["skipped"], 2);
    assert_eq!(service.calls(), 2);
    let hits = e.json(&[
        "semantic",
        "-c",
        "research",
        "quantum error correction",
        "--json",
    ]);
    assert_eq!(hits["model"], "fixture-minilm");
    assert!(hits["hits"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("quantum.txt"));
    assert_eq!(hits["hits"][0]["similarity"], 1.0);
    let hits = e.json(&[
        "semantic",
        "-c",
        "research",
        "quantum error correction",
        "--threshold",
        "0.5",
        "--top",
        "1",
        "--json",
    ]);
    assert_eq!(hits["hits"].as_array().unwrap().len(), 1);
    let indexed = e.json(&[
        "semantic", "-c", "research", "quantum", "--index", "--top", "1", "--json",
    ]);
    assert_eq!(indexed["indexed"], true);
    assert!(indexed["hits"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("quantum.txt"));
    assert!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_centroids",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            > 0
    );
    // The second query reuses the same stored clusters.
    e.ok(&[
        "semantic", "-c", "research", "quantum", "--index", "--top", "1",
    ]);
    let count = service.calls();
    fs::write(
        e.root.join("docs/quantum.txt"),
        "Updated quantum qubits can recover information with redundancy.",
    )
    .unwrap();
    e.ok(&["index", "-c", "research"]);
    assert_eq!(
        e.db()
            .query_row("SELECT count(*) FROM passage_embeddings", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_centroids",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let r = e.json(&["embed", "-c", "research", "--json"]);
    assert_eq!(r["embedded"], 1);
    assert_eq!(r["skipped"], 1);
    assert_eq!(service.calls(), count + 1);
    let r = e.json(&[
        "embed",
        "-c",
        "research",
        "--model",
        "different-model",
        "--json",
    ]);
    assert_eq!(r["embedded"], 2);
    assert_eq!(r["skipped"], 0);
    fs::remove_file(e.root.join("docs/quantum.txt")).unwrap();
    e.ok(&["index", "-c", "research"]);
    let hits = e.json(&[
        "semantic",
        "-c",
        "research",
        "quantum",
        "--threshold",
        "0.5",
        "--json",
    ]);
    assert!(hits["hits"].as_array().unwrap().is_empty());
    // A fully cached embed run must also succeed with the inference service stopped.
    drop(service);
    let report = e.json(&["embed", "-c", "research", "--json"]);
    assert_eq!(report["embedded"], 0);
    assert_eq!(report["skipped"], 1);
}

#[test]
fn local_openai_ordering_and_failed_batches() {
    let service = Service::start();
    let e = Env::new();
    let endpoint = format!("{}/v1", service.url);
    e.ok(&[
        "embed",
        "-c",
        "research",
        "--api-url",
        &endpoint,
        "--model",
        "local-model",
    ]);
    let hits = e.json(&["semantic", "-c", "research", "quantum", "--json"]);
    assert!(hits["hits"][0]["path"]
        .as_str()
        .unwrap()
        .ends_with("quantum.txt"));
    // The provider omitted vectors; do not store a partial batch or activate the new model.
    service.bad.store(true, Ordering::SeqCst);
    let o = e.run(&[
        "embed",
        "-c",
        "research",
        "--api-url",
        &service.url,
        "--model",
        "broken-model",
        "--json",
    ]);
    assert!(!o.status.success());
    let report: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(report["embedded"], 0);
    assert!(!report["errors"].as_array().unwrap().is_empty());
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embeddings WHERE model='broken-model'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(fs::read_to_string(e.root.join("config.toml"))
        .unwrap()
        .contains("model = \"local-model\""));
}

#[test]
fn migration_is_additive_and_preserves_lexical_tables() {
    let e = Env::new();
    let c = e.db();
    let before: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='files'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch("DROP TRIGGER invalidate_passage_embedding; DROP TRIGGER delete_file_embeddings; DROP TABLE passage_embeddings; DROP TABLE passage_embedding_models;").unwrap();
    drop(c);
    e.ok(&["status", "-c", "research", "--json"]);
    let c = e.db();
    let after: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='files'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        c.query_row("SELECT count(*) FROM passage_embeddings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM passages_fts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let hits = e.json(&["search", "-c", "research", "qubits", "--json"]);
    assert!(!hits["hits"].as_array().unwrap().is_empty());
}

#[test]
fn ivf_partitions_multiple_topics_and_rebuilds_after_changes() {
    let service = Service::start();
    let e = Env::new();
    fs::write(
        e.root.join("docs/physics.txt"),
        "Quantum qubits use redundancy to recover information.",
    )
    .unwrap();
    fs::write(
        e.root.join("docs/food.txt"),
        "Roast potatoes and carrots for dinner.",
    )
    .unwrap();
    e.ok(&["index", "-c", "research"]);
    e.ok(&[
        "embed",
        "-c",
        "research",
        "--api-url",
        &service.url,
        "--model",
        "cluster-fixture",
    ]);
    let results = e.json(&[
        "semantic",
        "-c",
        "research",
        "quantum recovery",
        "--index",
        "--top",
        "1",
        "--json",
    ]);
    assert_eq!(results["hits"][0]["similarity"], 1.0);
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_centroids",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(DISTINCT cluster) FROM passage_embedding_ivf_members",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_members",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
    fs::write(
        e.root.join("docs/physics.txt"),
        "Quantum qubits need a revised recovery protocol.",
    )
    .unwrap();
    e.ok(&["index", "-c", "research"]);
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_centroids",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let report = e.json(&["embed", "-c", "research", "--json"]);
    assert_eq!(report["embedded"], 1);
    let results = e.json(&[
        "semantic",
        "-c",
        "research",
        "quantum recovery",
        "--index",
        "--top",
        "1",
        "--json",
    ]);
    assert_eq!(results["hits"][0]["similarity"], 1.0);
    assert_eq!(
        e.db()
            .query_row(
                "SELECT count(*) FROM passage_embedding_ivf_centroids",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}
