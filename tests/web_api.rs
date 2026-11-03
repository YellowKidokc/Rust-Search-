//! Real HTTP checks against the shipped binary and temporary collection indexes.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    port: u16,
    _tmp: tempfile::TempDir,
    root: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn start() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let command = |args: &[&str]| {
            let out = Command::new(env!("CARGO_BIN_EXE_tpsearch"))
                .args(args)
                .env("TPSEARCH_CONFIG", root.join("config.toml"))
                .env("TPSEARCH_DATA", root.join("data"))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{:?}: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        for name in ["alpha", "beta"] {
            let docs = root.join(name);
            fs::create_dir(&docs).unwrap();
            fs::write(
                docs.join("notes.md"),
                format!("# {name}\n\nThe shared constellation shines in {name}.\n"),
            )
            .unwrap();
            if name == "alpha" {
                fs::write(
                    docs.join("sample.pdf"),
                    include_bytes!("fixtures/sample.pdf"),
                )
                .unwrap();
                fs::write(docs.join("image.pdf"), include_bytes!("fixtures/image.pdf")).unwrap();
            }
            command(&["collection", "new", name, "--root", docs.to_str().unwrap()]);
            command(&["index", "-c", name, "--json"]);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let launcher = root.join("viewer");
            fs::write(
                &launcher,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                    root.join("opened").display()
                ),
            )
            .unwrap();
            fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
            command(&[
                "collection",
                "set",
                "alpha",
                "open_with",
                &format!("{} {{path}} {{line}}", launcher.display()),
            ]);
        }
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let child = Command::new(env!("CARGO_BIN_EXE_tpsearch"))
            .args(["serve", "-c", "alpha", "--port", &port.to_string()])
            .env("TPSEARCH_CONFIG", root.join("config.toml"))
            .env("TPSEARCH_DATA", root.join("data"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut server = Self {
            child,
            port,
            _tmp: tmp,
            root,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(server.child.try_wait().unwrap().is_none(), "server exited");
            assert!(Instant::now() < deadline, "server startup timed out");
            std::thread::sleep(Duration::from_millis(25));
        }
        server
    }
    fn request(&self, method: &str, path: &str, body: Option<Value>) -> (u16, String) {
        let body = body.map(|v| v.to_string()).unwrap_or_default();
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        write!(stream,"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",self.port,body.len()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        let (headers, body) = reply.split_once("\r\n\r\n").unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, body.into())
    }
    fn json(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, text) = self.request(method, path, body);
        assert_eq!(status, 200, "{text}");
        serde_json::from_str(&text).unwrap()
    }
}

#[test]
fn collections_search_pdf_status_and_index_jobs() {
    let server = Server::start();
    for (host, origin) in [
        ("malicious.example", "http://malicious.example"),
        ("127.0.0.1", "http://malicious.example"),
    ] {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "GET /api/collections HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nConnection: close\r\n\r\n").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    }

    let (status, html) = server.request("GET", "/", None);
    assert_eq!(status, 200);
    assert!(html.contains("David-OS"));
    assert!(html.contains("statusUI"));
    let collections = server.json("GET", "/api/collections", None);
    assert_eq!(collections.as_array().unwrap().len(), 2);
    assert!(collections[0]["status"].is_object());
    let settings = server.json("GET", "/api/config", None);
    assert_eq!(settings["default_collection"], "alpha");
    let results = server.json(
        "GET",
        "/api/search?q=constellation&collection=alpha,beta",
        None,
    );
    assert_eq!(results.as_array().unwrap().len(), 2);
    for (set, name) in results.as_array().unwrap().iter().zip(["alpha", "beta"]) {
        assert_eq!(set["hits"][0]["collection"], name);
        assert!(!set["hits"].as_array().unwrap().is_empty());
    }
    let default = server.json("GET", "/api/search?q=constellation", None);
    assert_eq!(default.as_array().unwrap().len(), 1);
    assert_eq!(default[0]["hits"][0]["collection"], "alpha");
    let results = server.json(
        "POST",
        "/api/search",
        Some(json!({"collections":["alpha","beta"],"query":"constell* -absent"})),
    );
    assert_eq!(results.as_array().unwrap().len(), 2);
    let all = server.json(
        "POST",
        "/api/search",
        Some(json!({"collections":[],"query":"constellation"})),
    );
    assert_eq!(all.as_array().unwrap().len(), 2);
    let pdf=server.json("GET","/api/search?q=quality%20of%20mercy&collection=alpha&type=pdf&phrase=true&meta=author%3DShakespeare",None);
    assert_eq!(pdf[0]["hits"][0]["location"]["page"], 2);
    assert_eq!(pdf[0]["hits"][0]["ext"], "pdf");
    #[cfg(unix)]
    {
        let hit = &pdf[0]["hits"][0];
        let opened = server.json(
            "POST",
            "/api/open",
            Some(json!({"collection":"alpha", "passage_id":hit["passage_id"]})),
        );
        assert_eq!(opened["opened"], true);
        let capture = server.root.join("opened");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(args) = fs::read_to_string(&capture) {
                if !args.is_empty() {
                    assert!(args.contains("sample.pdf"));
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "viewer did not receive the source"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let health = server.json("GET", "/api/status?collection=alpha", None);
    assert_eq!(health["pdf_needs_ocr"], 1);
    assert_eq!(health["files_by_type"]["pdf"], 2);
    assert!(health["failures"].as_array().unwrap().is_empty());
    assert_eq!(
        server.request("GET", "/api/search?q=x&after=wrong", None).0,
        400
    );
    assert_eq!(
        server
            .request("GET", "/api/search?q=x&collection=missing", None)
            .0,
        400
    );
    assert_eq!(
        server
            .request(
                "POST",
                "/api/open",
                Some(json!({"collection":"alpha","passage_id":-1}))
            )
            .0,
        400
    );
    // A dry run must not ingest a new document; an actual job must ingest it.
    fs::write(
        server.root.join("alpha/new.txt"),
        "uniquejobtoken appears after indexing",
    )
    .unwrap();
    for dry_run in [true, false] {
        let job = server.json(
            "POST",
            &format!("/api/index?collection=alpha&dry_run={dry_run}"),
            None,
        );
        assert_eq!(job["status"], "running");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let job = server.json("GET", "/api/index?collection=alpha", None);
            if job["status"] != "running" {
                assert_eq!(job["status"], "completed", "{job}");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(25));
        }
        let result = server.json("GET", "/api/search?q=uniquejobtoken&collection=alpha", None);
        assert_eq!(result[0]["hits"].as_array().unwrap().is_empty(), dry_run);
    }
}
