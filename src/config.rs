use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "warmer")]
#[command(about = "A modern HTTP load testing and cache warming tool")]
pub struct Cli {
    /// URL to test (single URL mode) or base URL for sitemap mode
    pub url: Option<String>,

    /// Number of concurrent users (default: 25)
    #[arg(short = 'c', long = "concurrent", default_value_t = 25)]
    concurrent: usize,

    /// Time to run the test (e.g., 5S, 1M, 1H)
    #[arg(short = 't', long = "time")]
    time: Option<String>,

    /// Number of repetitions per user
    #[arg(short = 'r', long = "repetitions")]
    repetitions: Option<usize>,

    /// Delay between requests (seconds)
    #[arg(short = 'd', long = "delay", default_value_t = 0)]
    delay: u64,

    /// Verbose output
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Use sitemap mode (default if no URL provided)
    #[arg(short = 's', long = "sitemap")]
    sitemap: bool,

    /// Internet mode - random URL selection from sitemap
    #[arg(short = 'i', long = "internet")]
    internet: bool,

    /// Disable static asset loading (CSS, JS, images) from HTML pages
    #[arg(short = 'n', long = "no-assets")]
    no_assets: bool,

    /// Crawl mode - go through each URL only once, then stop
    #[arg(short = 'w', long = "crawl")]
    crawl: bool,

    /// Follow links mode - extract and follow links from pages when sitemap.xml is not found
    #[arg(short = 'f', long = "follow-links")]
    follow_links: bool,

    /// JavaScript mode - use headless Chrome browser to crawl JS/WASM sites
    #[arg(short = 'j', long = "js")]
    js_mode: bool,

    /// Number of discovery threads for JavaScript mode (default: CPU cores / 2, min 2, max 8)
    #[arg(short = 'T', long = "discovery-threads")]
    discovery_threads: Option<usize>,

    /// Path to TOML config file (e.g., warmer-config.toml)
    #[arg(short = 'C', long = "config")]
    pub config: Option<String>,

    /// Rotate through the built-in browser-like User-Agent list (anonymize requests)
    #[arg(short = 'a', long = "anonymize")]
    anonymize: bool,

    /// Force HTTP/1.1 instead of negotiating HTTP/2. HTTP/1.1 responses reliably
    /// include Content-Length, so inline per-request byte counts match siege.
    #[arg(short = 'H', long = "http1")]
    http1: bool,
}

/// Configuration loaded from a TOML file (everything except URL).
/// Kept separate from Cli because clap needs concrete types and default_value_t
/// for good CLI UX, while serde needs Option<T> for optional TOML keys.
/// Fields that CLI overrides are still deserialized for schema compatibility but not read.
#[derive(Default, Deserialize)]
#[allow(dead_code)]
pub struct FileConfig {
    #[serde(default)]
    concurrent: Option<usize>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    repetitions: Option<usize>,
    #[serde(default)]
    delay: Option<u64>,
    #[serde(default)]
    verbose: Option<bool>,
    #[serde(default)]
    sitemap: Option<bool>,
    #[serde(default)]
    internet: Option<bool>,
    #[serde(default, rename = "no_assets", alias = "no-assets")]
    no_assets: Option<bool>,
    #[serde(default)]
    crawl: Option<bool>,
    #[serde(default, rename = "follow_links", alias = "follow-links")]
    follow_links: Option<bool>,
    #[serde(default, rename = "js_mode", alias = "js", alias = "js-mode")]
    js_mode: Option<bool>,
    #[serde(default, rename = "discovery_threads", alias = "discovery-threads")]
    discovery_threads: Option<usize>,
    #[serde(default, rename = "user_agent", alias = "user-agent")]
    user_agent: Option<String>,
    #[serde(
        default,
        rename = "user_agent_list",
        alias = "user-agents",
        alias = "user_agents",
        alias = "user-agent-list",
        alias = "user_agents_list"
    )]
    user_agent_list: Vec<String>,
    #[serde(default)]
    http1: Option<bool>,
}

/// Effective configuration after merging CLI and file. Single source of truth for runtime.
#[derive(Clone)]
pub struct ResolvedConfig {
    pub concurrent: usize,
    pub time: Option<String>,
    pub repetitions: Option<usize>,
    pub delay: u64,
    pub verbose: bool,
    #[allow(dead_code)] // reserved for future sitemap-mode branching
    pub sitemap: bool,
    pub internet: bool,
    pub no_assets: bool,
    pub crawl: bool,
    pub follow_links: bool,
    pub js_mode: bool,
    pub discovery_threads: Option<usize>,
    pub user_agent: Option<String>,
    pub user_agent_list: Vec<String>,
    pub anonymize: bool,
    pub http1: bool,
}

/// Merges CLI and file config. **CLI takes precedence for all options** except user-agent
/// (user_agent and user_agent_list), which are long and stay config-only / config wins.
pub fn resolve_config(cli: Cli, file: &FileConfig) -> ResolvedConfig {
    ResolvedConfig {
        concurrent: cli.concurrent,
        time: cli.time.or_else(|| file.time.clone()),
        repetitions: cli.repetitions.or(file.repetitions),
        delay: cli.delay,
        verbose: cli.verbose,
        sitemap: cli.sitemap,
        internet: cli.internet,
        no_assets: cli.no_assets,
        crawl: cli.crawl,
        follow_links: cli.follow_links,
        js_mode: cli.js_mode,
        discovery_threads: cli.discovery_threads.or(file.discovery_threads),
        user_agent: file.user_agent.clone(),
        user_agent_list: if file.user_agent_list.is_empty() {
            vec![]
        } else {
            file.user_agent_list.clone()
        },
        anonymize: cli.anonymize,
        http1: cli.http1 || file.http1.unwrap_or(false),
    }
}

pub fn load_config(path: &str) -> FileConfig {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Failed to read config file {}: {}. Using CLI options only.",
                path, e
            );
            return FileConfig::default();
        }
    };

    match toml::from_str::<FileConfig>(&contents) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!(
                "Failed to parse config file {} as TOML: {}. Using CLI options only.",
                path, e
            );
            FileConfig::default()
        }
    }
}

pub fn parse_duration(time_str: &str) -> Result<Duration, String> {
    let time_str = time_str.to_uppercase();
    let (num_str, unit) = if time_str.ends_with('S') {
        (&time_str[..time_str.len() - 1], "S")
    } else if time_str.ends_with('M') {
        (&time_str[..time_str.len() - 1], "M")
    } else if time_str.ends_with('H') {
        (&time_str[..time_str.len() - 1], "H")
    } else {
        return Err("Invalid time format. Use format like 5S, 1M, 1H".to_string());
    };

    let num: u64 = num_str
        .parse()
        .map_err(|_| "Invalid number in time format".to_string())?;

    match unit {
        "S" => Ok(Duration::from_secs(num)),
        "M" => Ok(Duration::from_secs(num * 60)),
        "H" => Ok(Duration::from_secs(num * 3600)),
        _ => Err("Invalid time unit. Use S, M, or H".to_string()),
    }
}
