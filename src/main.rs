mod export;
mod indexer;
mod model;
mod parsers;
mod query;
mod saved;
mod store;
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
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Index {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    Query(QueryArgs),
    Repl {
        #[arg(long)]
        index: Option<PathBuf>,
    },
}
#[derive(Args, Debug, Clone)]
struct QueryArgs {
    #[arg(long)]
    index: Option<PathBuf>,
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
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Index { paths } => {
            let (base, s) = indexer::run(&paths)?;
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
    }
    Ok(())
}
fn run_query(a: QueryArgs) -> Result<()> {
    let base = store::find_base(a.index.as_deref())?;
    if a.list_saved {
        for n in saved::list(&base)? {
            println!("{n}")
        }
        return Ok(());
    }
    let index = store::load(&base)?;
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
