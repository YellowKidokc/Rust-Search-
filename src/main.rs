mod cli;
mod config;
mod db;
mod export;
mod extract;
mod indexer;
mod model;
mod parsers;
mod query;
mod report;
mod saved;
mod scan;
mod search;
mod semantic;
mod serve;
mod server;
mod store;
mod watch;
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use model::SearchResult;
use query::Query;
use std::{
    io::{self, Write},
    path::PathBuf,
};
#[derive(Parser)]
#[command(
    name = "tpsearch",
    version,
    about = "Local structured-document search engine"
)]
struct Cli {
    /// Collection settings file (default: per-user config dir, or $TPSEARCH_CONFIG).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Index a collection (`-c NAME`), or legacy: index Markdown paths into <root>/.tpsearch.
    Index {
        #[arg(long)]
        name: Option<String>,
        #[arg(required_unless_present = "collection")]
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        collection: Option<String>,
        /// Inventory only: show what would be indexed; writes nothing.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Manage named collections and their settings.
    #[command(subcommand, alias = "col")]
    Collection(cli::CollectionCmd),
    /// Full-text passage search over one or more collections.
    Search(cli::SearchArgs),
    /// Generate passage vectors with a local CPU embedding service.
    Embed(semantic::EmbedArgs),
    /// Rank collection passages by cosine similarity to a query.
    Semantic(semantic::SemanticArgs),
    /// Serve the search UI on http://127.0.0.1:PORT (loopback only).
    Serve {
        #[arg(short, long)]
        collection: Option<String>,
        #[arg(long)]
        index_name: Option<String>,
        #[arg(long)]
        index: Option<PathBuf>,
        #[arg(long, default_value_t = 9800)]
        port: u16,
        /// Open the UI in the default browser.
        #[arg(long)]
        open: bool,
    },
    /// Roots (online/offline), file states, failures, recent runs.
    Status {
        #[arg(short, long)]
        collection: String,
        #[arg(long)]
        json: bool,
    },
    /// Pause a running index of a collection (workers wait between files).
    Pause {
        #[arg(short, long)]
        collection: String,
    },
    Resume {
        #[arg(short, long)]
        collection: String,
    },
    /// Re-index incrementally every interval (scheduled/watched modes).
    Watch {
        #[arg(short, long)]
        collection: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<PathBuf>,
        /// Minutes; defaults to the collection's interval_minutes.
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Open the source of a search hit at its location.
    Open {
        #[arg(short, long)]
        collection: String,
        passage_id: i64,
        /// Only print the location.
        #[arg(long)]
        print: bool,
    },
    /// Export the documented folder-report JSON for a folder (or tree).
    ExportFolder {
        #[arg(short, long)]
        collection: String,
        folder: String,
        #[arg(long)]
        recursive: bool,
        /// File (single folder) or directory (--recursive). Default: stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Query(QueryArgs),
    Repl {
        #[arg(long)]
        index: Option<PathBuf>,
    },
    /// List centrally stored named indexes.
    Indexes,
}
#[derive(Args, Debug, Clone)]
struct QueryArgs {
    #[arg(long)]
    index: Option<PathBuf>,
    #[arg(long)]
    index_name: Option<String>,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    topic: Option<String>,
    #[arg(long)]
    domain: Option<String>,
    #[arg(long)]
    series: Option<String>,
    #[arg(long)]
    claim: Option<String>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    min_score: Option<i32>,
    #[arg(long)]
    min_s01: Option<i32>,
    #[arg(long)]
    min_s02: Option<i32>,
    #[arg(long)]
    min_s03: Option<i32>,
    #[arg(long)]
    min_s04: Option<i32>,
    #[arg(long)]
    min_s05: Option<i32>,
    #[arg(long)]
    min_s06: Option<i32>,
    #[arg(long)]
    min_s07: Option<i32>,
    #[arg(long)]
    min_s08: Option<i32>,
    #[arg(long)]
    min_s09: Option<i32>,
    #[arg(long)]
    min_s10: Option<i32>,
    #[arg(long)]
    min_evd_support: Option<i32>,
    #[arg(long,num_args=0..=1,default_missing_value="true")]
    has_counter: Option<bool>,
    #[arg(long, default_value = "score")]
    sort: String,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    verbose: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    save: Option<String>,
    #[arg(long)]
    load: Option<String>,
    #[arg(long)]
    frozen: bool,
    #[arg(long)]
    list_saved: bool,
    #[arg(long)]
    select: bool,
    #[arg(long)]
    select_all: bool,
    #[arg(long)]
    export_to: Option<PathBuf>,
    #[arg(long)]
    flatten: bool,
    #[arg(long)]
    export_manifest: Option<PathBuf>,
    #[arg(long)]
    export_csv: Option<PathBuf>,
    #[arg(long)]
    export_snippets: Option<PathBuf>,
}
impl QueryArgs {
    fn query(&self) -> Query {
        Query {
            tag: self.tag.clone(),
            topic: self.topic.clone(),
            domain: self.domain.clone(),
            series: self.series.clone(),
            claim: self.claim.clone(),
            search: self.search.clone(),
            min_score: self.min_score,
            min_sections: [
                self.min_s01,
                self.min_s02,
                self.min_s03,
                self.min_s04,
                self.min_s05,
                self.min_s06,
                self.min_s07,
                self.min_s08,
                self.min_s09,
                self.min_s10,
            ],
            min_evd_support: self.min_evd_support,
            has_counter: self.has_counter,
            sort: self.sort.clone(),
            limit: self.limit,
        }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let cfg_path = config::config_path(args.config.as_deref())?;
    match args.command {
        Command::Collection(cmd) => cli::collection(&cfg_path, cmd)?,
        Command::Search(args) => cli::search(&cfg_path, args)?,
        Command::Embed(args) => {
            tokio::task::spawn_blocking(move || semantic::embed(&cfg_path, args)).await??
        }
        Command::Semantic(args) => {
            tokio::task::spawn_blocking(move || semantic::search(&cfg_path, args)).await??
        }
        Command::Status { collection, json } => cli::status(&cfg_path, &collection, json)?,
        Command::Pause { collection } => cli::pause(&cfg_path, &collection, true)?,
        Command::Resume { collection } => cli::pause(&cfg_path, &collection, false)?,
        Command::Open {
            collection,
            passage_id,
            print,
        } => cli::open(&cfg_path, &collection, passage_id, print)?,
        Command::ExportFolder {
            collection,
            folder,
            recursive,
            out,
        } => cli::export_folder(&cfg_path, &collection, &folder, recursive, out)?,
        Command::Index {
            paths,
            name,
            collection,
            dry_run,
            json,
        } => {
            if let Some(collection) = collection {
                cli::index(&cfg_path, &collection, dry_run, json)?;
                return Ok(());
            }
            anyhow::ensure!(!dry_run, "--dry-run requires --collection");
            let (base, s) = indexer::run(&paths, name.as_deref())?;
            println!(
                "Index: {}\nAdded: {}, updated: {}, unchanged: {}, removed: {}, failed: {}",
                base.join("index.json").display(),
                s.added,
                s.updated,
                s.unchanged,
                s.removed,
                s.failed
            )
        }
        Command::Query(a) => run_query(a)?,
        Command::Repl { index } => repl(index)?,
        Command::Indexes => {
            for i in store::list_named()? {
                println!(
                    "{}\t{} records\t{}\t{}",
                    i.name,
                    i.record_count,
                    i.indexed_at.as_deref().unwrap_or("unknown"),
                    i.roots.join(", ")
                );
            }
        }
        Command::Watch {
            collection,
            interval,
            name,
            index,
        } => {
            if let Some(collection) = collection {
                cli::watch(&cfg_path, &collection, interval)?;
            } else {
                watch::run(name, index)?;
            }
        }
        Command::Serve {
            port,
            index_name,
            index,
            collection,
            open,
        } => {
            if index_name.is_some() || index.is_some() {
                anyhow::ensure!(
                    collection.is_none(),
                    "legacy index and collection cannot be combined"
                );
                server::serve(port, index_name, index).await?;
            } else {
                serve::run(&cfg_path, port, open, collection).await?;
            }
        }
    }
    Ok(())
}
fn run_query(a: QueryArgs) -> Result<()> {
    let base = match a.index_name.as_deref() {
        Some(n) if n != "all" => store::named_base(n)?,
        Some(_) => store::home()?,
        None => store::find_base(a.index.as_deref())?,
    };
    if a.list_saved {
        for n in saved::list(&base)? {
            println!("{n}")
        }
        return Ok(());
    }
    let index = match a.index_name.as_deref() {
        Some(n) => store::load_named(n)?,
        None => store::load(&base)?,
    };
    let (mut q, results) = if let Some(name) = &a.load {
        let s = saved::load(&base, name)?;
        let r = if a.frozen {
            s.results
        } else {
            query::execute(&index, &s.query)
        };
        (s.query, r)
    } else {
        let q = a.query();
        let r = query::execute(&index, &q);
        (q, r)
    };
    q.limit = a.limit;
    if let Some(name) = &a.save {
        saved::save(&base, name, &q, &results)?
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&results)?)
    } else {
        print_results(&results, a.verbose)?
    }
    let selected = if a.select {
        print!("Select: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        export::parse_selection(&line, results.len())?
            .into_iter()
            .map(|i| results[i].clone())
            .collect()
    } else if a.select_all
        || a.export_to.is_some()
        || a.export_manifest.is_some()
        || a.export_csv.is_some()
        || a.export_snippets.is_some()
    {
        results.clone()
    } else {
        vec![]
    };
    if let Some(p) = a.export_to {
        export::copy(&selected, &p, a.flatten, &index.roots)?
    }
    if let Some(p) = a.export_manifest {
        export::manifest(&selected, &p)?
    }
    if let Some(p) = a.export_csv {
        export::csv(&selected, &p)?
    }
    if let Some(p) = a.export_snippets {
        export::snippets(&selected, &q, &p)?
    }
    Ok(())
}
fn print_results(results: &[SearchResult], verbose: bool) -> Result<()> {
    for (i, x) in results.iter().enumerate() {
        let r = &x.record;
        println!(
            "{}. [{}] {} — {}",
            i + 1,
            r.score_total
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into()),
            r.title
                .as_deref()
                .or(r.clean_title.as_deref())
                .unwrap_or("Untitled"),
            r.file_path
        );
        if !x.matched_fields.is_empty() || !x.snippet.is_empty() {
            println!("   {}: {}", x.matched_fields.join(", "), x.snippet)
        }
        if verbose {
            println!("{}", serde_yaml::to_string(&r.frontmatter)?);
        }
    }
    Ok(())
}
fn repl(index: Option<PathBuf>) -> Result<()> {
    println!("tpsearch REPL. Enter query options, or 'quit'.");
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut s = String::new();
        if io::stdin().read_line(&mut s)? == 0 || matches!(s.trim(), "quit" | "exit") {
            break;
        }
        let mut argv = vec!["tpsearch".into(), "query".into()];
        argv.extend(shell_words::split(&s)?);
        match Cli::try_parse_from(argv) {
            Ok(Cli {
                command: Command::Query(mut q),
                ..
            }) => {
                if q.index.is_none() {
                    q.index = index.clone()
                }
                if let Err(e) = run_query(q) {
                    eprintln!("error: {e:#}")
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("{e}"),
        }
    }
    Ok(())
}
