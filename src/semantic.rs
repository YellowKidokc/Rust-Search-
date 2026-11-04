//! Local embedding API fallback. Lexical search remains independent.
use crate::{config, db};
use anyhow::{ensure, Context, Result};
use clap::Args;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

#[derive(Args, Debug)]
pub struct EmbedArgs {
    #[arg(short, long)]
    collection: String,
    /// Local service model. Defaults to the collection setting, then nomic-embed-text.
    #[arg(long)]
    model: Option<String>,
    /// Ollama base URL or a local OpenAI-compatible /v1/embeddings endpoint.
    #[arg(long)]
    api_url: Option<String>,
    #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u16).range(1..=1024))]
    batch_size: u16,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
pub struct SemanticArgs {
    #[arg(short, long)]
    collection: String,
    #[arg(required = true)]
    query: Vec<String>,
    #[arg(long, default_value_t = 20)]
    top: usize,
    #[arg(long, default_value_t = 0.0)]
    threshold: f32,
    /// Build/reuse a simple CPU IVF index (approximate candidate selection).
    #[arg(long)]
    index: bool,
    #[arg(long)]
    json: bool,
}

struct LocalModel {
    client: reqwest::blocking::Client,
    url: reqwest::Url,
    model: String,
    openai: bool,
}
impl LocalModel {
    fn new(url: &str, model: &str) -> Result<Self> {
        let mut url = reqwest::Url::parse(url).context("invalid local embedding URL")?;
        ensure!(
            url.scheme() == "http",
            "local embedding API must use http on loopback"
        );
        let local = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        ensure!(
            local && url.username().is_empty() && url.password().is_none(),
            "embedding API must be a local loopback service without credentials"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "embedding URL cannot contain a query or fragment"
        );
        ensure!(!model.trim().is_empty(), "model name cannot be empty");
        let openai = url.path().trim_end_matches('/').ends_with("/embeddings")
            || url.path().trim_end_matches('/') == "/v1";
        if openai {
            if url.path().trim_end_matches('/') == "/v1" {
                url.set_path("/v1/embeddings");
            }
        } else {
            ensure!(
                matches!(url.path(), "" | "/" | "/api/embed"),
                "use an Ollama base URL or /v1/embeddings endpoint"
            );
            url.set_path("/api/embed");
        }
        let client = reqwest::blocking::Client::builder()
            .no_proxy() // Local IPC only; never send source passages through an Internet proxy.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build()?;
        Ok(Self {
            client,
            url,
            model: model.into(),
            openai,
        })
    }

    fn vectors(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        let body = if self.openai {
            json!({"model":self.model,"input":input,"encoding_format":"float"})
        } else {
            json!({"model":self.model,"input":input,"truncate":false,"options":{"num_gpu":0}})
        };
        let response = self.client.post(self.url.clone()).json(&body).send()
            .context("local embedding service unavailable; start Ollama and pull the configured model (see README)")?;
        ensure!(
            response.status().is_success(),
            "local embedding service returned HTTP {}",
            response.status()
        );
        let value: Value = response.json().context("invalid embedding response JSON")?;
        let vectors: Vec<Vec<f32>> = if self.openai {
            let rows = value["data"]
                .as_array()
                .context("embedding response has no data array")?;
            let mut ordered = vec![None; input.len()];
            for row in rows {
                let index = row["index"].as_u64().context("embedding has no index")? as usize;
                ensure!(
                    index < ordered.len() && ordered[index].is_none(),
                    "invalid or duplicate embedding index"
                );
                ordered[index] = Some(serde_json::from_value(row["embedding"].clone())?);
            }
            ordered
                .into_iter()
                .collect::<Option<_>>()
                .context("embedding response omitted an input")?
        } else {
            serde_json::from_value(value["embeddings"].clone())
                .context("embedding response has no embeddings array")?
        };
        ensure!(
            vectors.len() == input.len(),
            "embedding response count does not match batch"
        );
        let dims = vectors.first().context("empty embedding response")?.len();
        for vector in &vectors {
            validate(vector, dims)?;
        }
        Ok(vectors)
    }
}

fn validate(vector: &[f32], dims: usize) -> Result<()> {
    ensure!(
        dims > 0 && dims <= 65_536 && vector.len() == dims,
        "inconsistent embedding dimensions"
    );
    ensure!(
        vector.iter().all(|x| x.is_finite()),
        "embedding contains non-finite values"
    );
    ensure!(
        vector.iter().any(|x| *x != 0.0),
        "zero embedding cannot be compared by cosine similarity"
    );
    Ok(())
}

fn encode(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn decode(bytes: &[u8], dims: usize) -> Result<Vec<f32>> {
    ensure!(
        dims > 0 && dims <= 65_536 && bytes.len() == dims * 4,
        "invalid stored embedding length"
    );
    let vector = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect::<Vec<_>>();
    validate(&vector, dims)?;
    Ok(vector)
}
fn cosine(a: &[f32], b: &[f32]) -> Result<f32> {
    validate(a, b.len())?;
    validate(b, a.len())?;
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    Ok((dot / (norm(a) * norm(b))).clamp(-1.0, 1.0) as f32)
}

#[derive(Debug)]
struct Passage {
    id: i64,
    file: i64,
    ordinal: i64,
    text: String,
}
#[derive(Default, Serialize)]
struct EmbedReport {
    model: String,
    dry_run: bool,
    total: usize,
    embedded: usize,
    skipped: usize,
    pending: usize,
    dims: Option<usize>,
    errors: Vec<String>,
}

pub fn embed(path: &Path, args: EmbedArgs) -> Result<()> {
    let cfg = config::load(path)?;
    let collection = cfg.get(&args.collection)?;
    let previous = collection.embedding.clone();
    let model = args
        .model
        .or_else(|| previous.as_ref().map(|s| s.model.clone()))
        .unwrap_or_else(|| "nomic-embed-text".into());
    let url = args
        .api_url
        .or_else(|| previous.as_ref().map(|s| s.api_url.clone()))
        .unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let service = LocalModel::new(&url, &model)?;
    let url = service.url.to_string();
    let dir = config::index_dir(&args.collection, collection)?;
    ensure!(
        db::exists(&dir),
        "index the collection before generating embeddings"
    );
    let mut conn = db::open(&dir)?;
    let mut report = EmbedReport {
        model: model.clone(),
        dry_run: args.dry_run,
        ..Default::default()
    };
    let mut dims = previous
        .as_ref()
        .filter(|s| s.model == model)
        .map(|s| s.dims);
    let identity: Option<(String, usize)> = conn
        .query_row(
            "SELECT api_url,dims FROM passage_embedding_models WHERE model=?1",
            [&model],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((endpoint, size)) = identity {
        ensure!(endpoint == url, "this model belongs to another endpoint; use a distinct model alias to avoid mixing vector spaces");
        ensure!(
            dims.is_none_or(|d| d == size),
            "model dimensions disagree with settings"
        );
        dims = Some(size);
    }
    let mut q = conn.prepare("SELECT DISTINCT dims FROM passage_embeddings WHERE model=?1")?;
    let existing = q
        .query_map([&model], |r| r.get::<_, usize>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        existing.len() <= 1,
        "stored model has inconsistent dimensions"
    );
    if let Some(d) = existing.first() {
        ensure!(
            dims.is_none_or(|n| n == *d),
            "model dimensions disagree with settings"
        );
        dims = Some(*d);
    }
    drop(q);
    let mut q = conn.prepare("SELECT p.id,p.file_id,p.ord,t.text,e.embedding,e.dims FROM passages p JOIN files f ON f.id=p.file_id JOIN passages_fts t ON t.rowid=p.id LEFT JOIN passage_embeddings e ON e.file_id=p.file_id AND e.passage_idx=p.ord AND e.model=?1 WHERE p.ord>=0 AND f.status='indexed' ORDER BY p.file_id,p.ord")?;
    let rows = q.query_map([&model], |r| {
        Ok((
            Passage {
                id: r.get(0)?,
                file: r.get(1)?,
                ordinal: r.get(2)?,
                text: r.get(3)?,
            },
            r.get::<_, Option<Vec<u8>>>(4)?,
            r.get::<_, Option<usize>>(5)?,
        ))
    })?;
    let mut pending = vec![];
    for row in rows {
        let (passage, bytes, count) = row?;
        report.total += 1;
        if let (Some(bytes), Some(count)) = (bytes, count) {
            decode(&bytes, count)?;
            report.skipped += 1;
        } else {
            pending.push(passage);
        }
    }
    drop(q);
    report.pending = pending.len();
    if !args.dry_run {
        for batch in pending.chunks(usize::from(args.batch_size)) {
            let input = batch.iter().map(|p| p.text.clone()).collect::<Vec<_>>();
            let result = (|| -> Result<usize> {
                let vectors = service.vectors(&input)?;
                let count = vectors[0].len();
                ensure!(
                    dims.is_none_or(|d| d == count),
                    "model changed vector dimensions; use a new model alias and re-embed"
                );
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute("INSERT INTO passage_embedding_models(model,api_url,dims) VALUES(?1,?2,?3) ON CONFLICT(model) DO NOTHING", params![model,url,count])?;
                let identity: (String, usize) = tx.query_row(
                    "SELECT api_url,dims FROM passage_embedding_models WHERE model=?1",
                    [&model],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                ensure!(
                    identity == (url.clone(), count),
                    "another embedding job registered a different model space"
                );
                for (p, vector) in batch.iter().zip(&vectors) {
                    let unchanged: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM passages p JOIN files f ON f.id=p.file_id JOIN passages_fts t ON t.rowid=p.id WHERE p.id=?1 AND p.file_id=?2 AND p.ord=?3 AND t.text=?4 AND f.status='indexed')", params![p.id,p.file,p.ordinal,p.text], |r| r.get(0))?;
                    ensure!(
                        unchanged,
                        "passages changed during embedding; rerun embed after indexing finishes"
                    );
                    tx.execute("INSERT INTO passage_embeddings(file_id,passage_idx,model,embedding,dims) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(file_id,passage_idx,model) DO UPDATE SET embedding=excluded.embedding,dims=excluded.dims,created_at=datetime('now')",params![p.file,p.ordinal,model,encode(vector),count])?;
                }
                tx.commit()?;
                dims = Some(count);
                Ok(batch.len())
            })();
            match result {
                Ok(n) => report.embedded += n,
                Err(e) => report.errors.push(format!(
                    "file {} paragraph {}: {e:#}",
                    batch[0].file, batch[0].ordinal
                )),
            }
        }
        if report.errors.is_empty() {
            if let Some(dims) = dims {
                let mut current = config::load(path)?;
                ensure!(config::index_dir(&args.collection,current.get(&args.collection)?)? == dir,
                    "collection index directory changed during embedding; settings were not overwritten");
                let settings = config::EmbeddingSettings {
                    model: model.clone(),
                    api_url: url,
                    dims,
                };
                if current.get(&args.collection)?.embedding.as_ref() != Some(&settings) {
                    current.get_mut(&args.collection)?.embedding = Some(settings);
                    config::save(path, &current)?;
                }
            }
        }
    }
    report.pending = report.pending.saturating_sub(report.embedded);
    report.dims = dims;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Model {}: {} embedded, {} skipped, {} pending; {} batch errors{}",
            model,
            report.embedded,
            report.skipped,
            report.pending,
            report.errors.len(),
            if args.dry_run { " (dry run)" } else { "" }
        );
        for error in &report.errors {
            eprintln!("{error}");
        }
    }
    ensure!(
        report.errors.is_empty(),
        "embedding failed for {} batches; successful batches were saved",
        report.errors.len()
    );
    Ok(())
}

#[derive(Serialize)]
struct Hit {
    collection: String,
    passage_id: i64,
    path: String,
    text: String,
    location: crate::search::Location,
    similarity: f32,
    model: String,
}

pub fn search(path: &Path, args: SemanticArgs) -> Result<()> {
    ensure!(
        args.top > 0 && args.top <= 100_000,
        "--top must be between 1 and 100000"
    );
    ensure!(
        args.threshold.is_finite() && (-1.0..=1.0).contains(&args.threshold),
        "threshold must be between -1 and 1"
    );
    let query = args.query.join(" ");
    ensure!(!query.trim().is_empty(), "query cannot be empty");
    let cfg = config::load(path)?;
    let collection = cfg.get(&args.collection)?;
    let settings = collection
        .embedding
        .as_ref()
        .context("run tpsearch embed for this collection first")?;
    let dir = config::index_dir(&args.collection, collection)?;
    ensure!(db::exists(&dir), "collection has not been indexed");
    let mut conn = db::open(&dir)?;
    let mut vectors =
        LocalModel::new(&settings.api_url, &settings.model)?.vectors(&[query.clone()])?;
    let vector = vectors.remove(0);
    validate(&vector, settings.dims)?;
    let candidates = if args.index {
        ivf_candidates(&mut conn, &settings.model, settings.dims, &vector, args.top)?
    } else {
        String::new()
    };
    let sql = format!("SELECT p.id,f.path,t.text,p.loc_kind,p.loc_start,p.loc_end,p.section,p.t_start,p.t_end,p.page,e.embedding,e.dims FROM passage_embeddings e JOIN passages p ON p.file_id=e.file_id AND p.ord=e.passage_idx JOIN files f ON f.id=p.file_id JOIN passages_fts t ON t.rowid=p.id WHERE e.model=?1 AND f.status='indexed' {candidates}");
    let mut q = conn.prepare(&sql)?;
    let mut rows = q.query([&settings.model])?;
    let mut hits = vec![];
    while let Some(r) = rows.next()? {
        let stored = decode(&r.get::<_, Vec<u8>>(10)?, r.get(11)?)?;
        let similarity = cosine(&vector, &stored)?;
        if similarity < args.threshold {
            continue;
        }
        hits.push(Hit {
            collection: args.collection.clone(),
            passage_id: r.get(0)?,
            path: r.get(1)?,
            text: r.get(2)?,
            location: crate::search::Location {
                kind: r.get(3)?,
                start: r.get(4)?,
                end: r.get(5)?,
                section: r.get(6)?,
                t_start: r.get(7)?,
                t_end: r.get(8)?,
                page: r.get(9)?,
            },
            similarity,
            model: settings.model.clone(),
        });
    }
    hits.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a.passage_id.cmp(&b.passage_id))
    });
    hits.truncate(args.top);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"query":query,"model":settings.model,"hits":hits,"indexed":args.index,"score_note":"cosine similarity; not a truth or evidence-quality score"})
            )?
        );
    } else {
        for h in hits {
            let loc = h
                .location
                .page
                .map(|p| format!("page {p}, "))
                .unwrap_or_default();
            println!(
                "[{:.4}] {} — {}{} {}–{}{}\n{}\n",
                h.similarity,
                h.path,
                loc,
                h.location.kind,
                h.location.start,
                h.location.end,
                h.location
                    .section
                    .map(|s| format!(" — {s}"))
                    .unwrap_or_default(),
                h.text
            );
        }
    }
    Ok(())
}

fn unit(vector: &[f32]) -> Vec<f32> {
    let norm = vector
        .iter()
        .map(|x| f64::from(*x).powi(2))
        .sum::<f64>()
        .sqrt();
    vector
        .iter()
        .map(|x| (f64::from(*x) / norm) as f32)
        .collect()
}
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn nearest(vector: &[f32], centers: &[Vec<f32>]) -> usize {
    (0..centers.len())
        .max_by(|a, b| {
            dot(vector, &centers[*a])
                .total_cmp(&dot(vector, &centers[*b]))
                .then_with(|| b.cmp(a))
        })
        .unwrap()
}

/// Small deterministic spherical k-means training; assignments use cosine distance.
fn kmeans(vectors: &[Vec<f32>], k: usize) -> Vec<Vec<f32>> {
    let mut centers = vec![vectors[0].clone()];
    while centers.len() < k {
        let farthest = vectors
            .iter()
            .min_by(|a, b| {
                dot(a, &centers[nearest(a, &centers)])
                    .total_cmp(&dot(b, &centers[nearest(b, &centers)]))
            })
            .unwrap();
        centers.push(farthest.clone());
    }
    for _ in 0..6 {
        let mut sums = vec![vec![0.0; vectors[0].len()]; k];
        let mut counts = vec![0; k];
        for vector in vectors {
            let cluster = nearest(vector, &centers);
            counts[cluster] += 1;
            for (sum, value) in sums[cluster].iter_mut().zip(vector) {
                *sum += value;
            }
        }
        for i in 0..k {
            if counts[i] > 0 && sums[i].iter().any(|x| x.abs() > 1e-12) {
                centers[i] = unit(&sums[i]);
            }
        }
    }
    centers
}

fn ivf_candidates(
    conn: &mut rusqlite::Connection,
    model: &str,
    dims: usize,
    query: &[f32],
    top: usize,
) -> Result<String> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM passage_embedding_ivf_centroids WHERE model=?1",
        [model],
        |r| r.get(0),
    )?;
    if count == 0 {
        let mut statement=tx.prepare("SELECT e.file_id,e.passage_idx,e.embedding,e.dims FROM passage_embeddings e JOIN files f ON f.id=e.file_id WHERE e.model=?1 AND f.status='indexed' ORDER BY e.file_id,e.passage_idx")?;
        let entries = statement
            .query_map([model], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, usize>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let vectors = entries
            .iter()
            .map(|(_, _, bytes, n)| {
                ensure!(*n == dims, "stored vector dimensions do not match model");
                Ok(unit(&decode(bytes, *n)?))
            })
            .collect::<Result<Vec<_>>>()?;
        tx.execute(
            "DELETE FROM passage_embedding_ivf_members WHERE model=?1",
            [model],
        )?;
        if !vectors.is_empty() {
            // Bound training cost; every vector still receives a cluster assignment.
            let step = (vectors.len() / 2048).max(1);
            let training = vectors
                .iter()
                .step_by(step)
                .take(2048)
                .cloned()
                .collect::<Vec<_>>();
            let k = ((vectors.len() as f64).sqrt() as usize)
                .clamp(1, 64)
                .min(training.len());
            let centers = kmeans(&training, k);
            for (cluster, center) in centers.iter().enumerate() {
                tx.execute("INSERT INTO passage_embedding_ivf_centroids(model,cluster,centroid,dims) VALUES(?1,?2,?3,?4)",params![model,cluster,encode(center),dims])?;
            }
            for ((file, ordinal, _, _), vector) in entries.iter().zip(&vectors) {
                tx.execute("INSERT INTO passage_embedding_ivf_members(model,file_id,passage_idx,cluster) VALUES(?1,?2,?3,?4)",params![model,file,ordinal,nearest(vector,&centers)])?;
            }
        }
    }
    let mut statement=tx.prepare("SELECT cluster,centroid,dims FROM passage_embedding_ivf_centroids WHERE model=?1 ORDER BY cluster")?;
    let rows = statement
        .query_map([model], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, usize>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    let mut centers = rows
        .into_iter()
        .map(|(id, bytes, n)| Ok((id, cosine(query, &decode(&bytes, n)?)?)))
        .collect::<Result<Vec<_>>>()?;
    centers.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let probes = ((centers.len() as f64).sqrt().ceil() as usize).max(1);
    let mut selected = vec![];
    let mut candidates = 0;
    for (id, _) in centers {
        selected.push(id.to_string());
        candidates+=tx.query_row("SELECT count(*) FROM passage_embedding_ivf_members m JOIN files f ON f.id=m.file_id WHERE m.model=?1 AND m.cluster=?2 AND f.status='indexed'",params![model,id],|r|r.get::<_,usize>(0))?;
        if selected.len() >= probes && candidates >= top {
            break;
        }
    }
    tx.commit()?;
    if selected.is_empty() {
        return Ok("AND 0".into());
    }
    // IDs originate from SQLite integers, not user input.
    Ok(format!("AND EXISTS(SELECT 1 FROM passage_embedding_ivf_members m WHERE m.model=e.model AND m.file_id=e.file_id AND m.passage_idx=e.passage_idx AND m.cluster IN ({}))",selected.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_roundtrip_and_cosine_order() {
        let query = vec![1.0, 0.0, 0.0];
        let vectors = [
            vec![1.0, 0.0, 0.0],
            vec![0.8, 0.2, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![-1.0, 0.0, 0.0],
        ];
        let scores = vectors
            .iter()
            .map(|v| cosine(&query, v).unwrap())
            .collect::<Vec<_>>();
        assert!(scores.windows(2).all(|w| w[0] > w[1]));
        for vector in vectors {
            assert_eq!(decode(&encode(&vector), 3).unwrap(), vector);
        }
        assert!(decode(&[0; 3], 1).is_err());
        assert!(cosine(&[0.0], &[1.0]).is_err());
        assert!(cosine(&[f32::NAN], &[1.0]).is_err());
        assert!(cosine(&[1.0, 2.0], &[1.0]).is_err());
    }
    #[test]
    fn spherical_clusters_separate_topics() {
        let vectors = vec![
            unit(&[1.0, 0.0]),
            unit(&[0.98, 0.02]),
            unit(&[-1.0, 0.0]),
            unit(&[-0.98, -0.02]),
        ];
        let centers = kmeans(&vectors, 2);
        assert_eq!(
            nearest(&vectors[0], &centers),
            nearest(&vectors[1], &centers)
        );
        assert_eq!(
            nearest(&vectors[2], &centers),
            nearest(&vectors[3], &centers)
        );
        assert_ne!(
            nearest(&vectors[0], &centers),
            nearest(&vectors[2], &centers)
        );
    }
    #[test]
    fn rejects_remote_embedding_services() {
        for url in [
            "http://api.openai.com/v1/embeddings",
            "http://192.168.1.1:11434",
            "https://localhost:11434",
            "http://user:password@localhost:11434",
        ] {
            assert!(LocalModel::new(url, "model").is_err());
        }
    }
}
