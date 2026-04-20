use clap::CommandFactory;
use clap::Parser;
use colored::*;
use ctrlc;
use isahc::HttpClient;
use isahc::config::SslOption;
use isahc::config::VersionNegotiation;
use isahc::{Request, config::RedirectPolicy, prelude::*};
use rand::Rng;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_xml_rs::from_str;
use std::collections::HashMap;
use std::fs;
use std::process::exit;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use url::Url;
mod js_crawler;

/// When true, requests force HTTP/1.1 instead of negotiating HTTP/2.
/// Set once at startup from the resolved config; read on every request.
static FORCE_HTTP1: AtomicBool = AtomicBool::new(false);

/// Shared HTTP client with unlimited connection pool per host.
/// isahc's defaults are browser-like (~6 connections per host), which caps real
/// concurrency well below the requested `--concurrent` level in a load test.
static HTTP_CLIENT: OnceLock<HttpClient> = OnceLock::new();

fn http_client() -> &'static HttpClient {
    HTTP_CLIENT.get_or_init(|| {
        HttpClient::builder()
            .max_connections(0)
            .max_connections_per_host(0)
            .connect_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .ssl_options(
                SslOption::DANGER_ACCEPT_INVALID_CERTS
                    | SslOption::DANGER_ACCEPT_REVOKED_CERTS
                    | SslOption::DANGER_ACCEPT_INVALID_HOSTS,
            )
            .redirect_policy(RedirectPolicy::Follow)
            .build()
            .expect("failed to build shared HttpClient")
    })
}

/// The struct to deserialize and hold the items in <url></url>
/// in the sitemap.xml
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Urlc {
    loc: String,
    #[serde(default = "default_lastmod")]
    lastmod: String,
    #[serde(default = "default_changefreq")]
    changefreq: String,
    #[serde(default = "default_priority")]
    priority: String,
}

/// The struct to hold the urlset items in the sitemap.xml.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct UrlSet {
    url: Vec<Urlc>,
}

#[derive(Parser)]
#[command(name = "warmer")]
#[command(about = "A modern HTTP load testing and cache warming tool")]
struct Cli {
    /// URL to test (single URL mode) or base URL for sitemap mode
    url: Option<String>,

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
    config: Option<String>,

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
struct FileConfig {
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
struct ResolvedConfig {
    concurrent: usize,
    time: Option<String>,
    repetitions: Option<usize>,
    delay: u64,
    verbose: bool,
    #[allow(dead_code)] // reserved for future sitemap-mode branching
    sitemap: bool,
    internet: bool,
    no_assets: bool,
    crawl: bool,
    follow_links: bool,
    js_mode: bool,
    discovery_threads: Option<usize>,
    user_agent: Option<String>,
    user_agent_list: Vec<String>,
    anonymize: bool,
    http1: bool,
}

/// Merges CLI and file config. **CLI takes precedence for all options** except user-agent
/// (user_agent and user_agent_list), which are long and stay config-only / config wins.
fn resolve_config(cli: Cli, file: &FileConfig) -> ResolvedConfig {
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

/// Performance statistics tracking
#[derive(Clone, Default)]
struct Stats {
    transactions: usize,
    successful_transactions: usize,
    failed_transactions: usize,
    response_times: Vec<f64>,
    data_transferred: u64,
    start_time: Option<Instant>,
    end_time: Option<Instant>,
    status_codes: HashMap<u16, usize>,
}

impl Stats {
    fn new() -> Self {
        Self {
            start_time: Some(Instant::now()),
            ..Default::default()
        }
    }

    fn add_transaction(&mut self, response_time: f64, data_size: u64, status_code: u16) {
        self.transactions += 1;
        self.response_times.push(response_time);
        self.data_transferred += data_size;

        if status_code < 400 {
            self.successful_transactions += 1;
        } else {
            self.failed_transactions += 1;
        }

        *self.status_codes.entry(status_code).or_insert(0) += 1;
    }

    fn finish(&mut self) {
        self.end_time = Some(Instant::now());
    }

    fn elapsed_time(&self) -> f64 {
        if let (Some(start), Some(end)) = (self.start_time, self.end_time) {
            end.duration_since(start).as_secs_f64()
        } else if let Some(start) = self.start_time {
            start.elapsed().as_secs_f64()
        } else {
            0.0
        }
    }

    fn avg_response_time(&self) -> f64 {
        if self.response_times.is_empty() {
            0.0
        } else {
            self.response_times.iter().sum::<f64>() / self.response_times.len() as f64
        }
    }

    fn transaction_rate(&self) -> f64 {
        let elapsed = self.elapsed_time();
        if elapsed > 0.0 {
            self.transactions as f64 / elapsed
        } else {
            0.0
        }
    }

    fn throughput(&self) -> f64 {
        let elapsed = self.elapsed_time();
        if elapsed > 0.0 {
            self.data_transferred as f64 / elapsed / 1024.0 / 1024.0 // MB/sec
        } else {
            0.0
        }
    }

    fn concurrency(&self) -> f64 {
        if self.response_times.is_empty() {
            0.0
        } else {
            self.avg_response_time() * self.transaction_rate() / 1000.0
        }
    }

    fn availability(&self) -> f64 {
        if self.transactions == 0 {
            0.0
        } else {
            (self.successful_transactions as f64 / self.transactions as f64) * 100.0
        }
    }
}

/// Parse time duration string (e.g., "5S", "1M", "1H")
fn parse_duration(time_str: &str) -> Result<Duration, String> {
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

/// Color status codes for better readability
fn color_status_code(status_code: u16) -> ColoredString {
    match status_code {
        200..=299 => status_code.to_string().green(),
        300..=399 => status_code.to_string().yellow(),
        400..=499 => status_code.to_string().red(),
        500..=599 => status_code.to_string().red().bold(),
        _ => status_code.to_string().white(),
    }
}

/// Format response time for display
fn format_response_time(ms: f64) -> String {
    format!("{:.2} secs", ms / 1000.0)
}

/// Format data size for display
fn format_data_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Print load testing header
fn print_header(concurrent: usize, _url: &str) {
    println!("** WARMER 0.1.2");
    println!("** Preparing {} concurrent users for battle.", concurrent);
    println!("The server is now under load...");
}

/// Print transaction details with optional highlighting for main URLs
fn print_transaction(
    status_code: u16,
    response_time: f64,
    data_size: u64,
    method: &str,
    path: &str,
    _verbose: bool,
    is_main_url: bool,
    http_version: &str,
) {
    let status_colored = color_status_code(status_code);
    let response_time_str = format_response_time(response_time);
    let data_size_str = format_data_size(data_size);

    if is_main_url {
        // Highlight main URLs with bold and bright colors
        println!(
            "{} {}     {}: {} ==> {}  {}",
            http_version,
            status_colored.bold(),
            response_time_str.bold(),
            data_size_str.bold(),
            method.bold(),
            path.bold().bright_blue()
        );
    } else {
        println!(
            "{} {}     {}: {} ==> {}  {}",
            http_version, status_colored, response_time_str, data_size_str, method, path
        );
    }
}

/// Print final statistics
fn print_statistics(stats: &Stats) {
    println!("\nLoad testing completed...");
    println!();
    println!("Transactions:\t\t{:8} hits", stats.transactions);
    println!("Availability:\t\t{:8.2} %", stats.availability());
    println!("Elapsed time:\t\t{:8.2} secs", stats.elapsed_time());
    println!(
        "Data transferred:\t{:8.2} MB",
        stats.data_transferred as f64 / (1024.0 * 1024.0)
    );
    println!("Response time:\t\t{:8.2} ms", stats.avg_response_time());
    println!(
        "Transaction rate:\t{:8.2} trans/sec",
        stats.transaction_rate()
    );
    println!("Throughput:\t\t{:8.2} MB/sec", stats.throughput());
    println!("Concurrency:\t\t{:8.2}", stats.concurrency());
    println!(
        "Successful transactions: {:8}",
        stats.successful_transactions
    );
    println!("Failed transactions:\t{:8}", stats.failed_transactions);

    if let Some(&max_time) = stats
        .response_times
        .iter()
        .max_by(|a, b| a.partial_cmp(b).unwrap())
    {
        println!("Longest transaction:\t{:8.2} ms", max_time);
    }

    if let Some(&min_time) = stats
        .response_times
        .iter()
        .min_by(|a, b| a.partial_cmp(b).unwrap())
    {
        println!("Shortest transaction:\t{:8.2} ms", min_time);
    }

    println!();
}

fn common_sitemap_candidates(base_url: &str) -> Vec<String> {
    vec![
        format!("{}/sitemap.xml", base_url),
        format!("{}/sitemap_index.xml", base_url),
        format!("{}/sitemap-index.xml", base_url),
        format!("{}/sitemaps.xml", base_url),
        format!("{}/sitemap-0.xml", base_url),
        format!("{}/news-sitemap.xml", base_url),
    ]
}

/// Find sitemap URL from robots.txt
async fn find_sitemap_url_from_robots(
    base_url: &str,
    user_agent: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Construct robots.txt URL
    let robots_url = format!("{}/robots.txt", base_url);
    println!("Checking robots.txt at {}", robots_url);

    // Request robots.txt
    let mut builder = Request::get(&robots_url).header("User-Agent", user_agent);
    if FORCE_HTTP1.load(Ordering::Relaxed) {
        builder = builder.version_negotiation(VersionNegotiation::http11());
    }
    let request = builder.body(());

    if request.is_err() {
        println!("Error creating request for robots.txt");
        return Ok(common_sitemap_candidates(base_url));
    }

    let response = http_client().send_async(request?).await;

    if response.is_err() {
        println!("Error fetching robots.txt");
        return Ok(common_sitemap_candidates(base_url));
    }

    let mut response = response?;

    if response.status().as_str() != "200" {
        println!(
            "No robots.txt found (status: {}), will try common sitemap locations",
            response.status()
        );
        return Ok(common_sitemap_candidates(base_url));
    }

    // Parse robots.txt to find Sitemap: directives (there may be multiple)
    let robots_content = response.text().await?;
    let mut sitemap_urls: Vec<String> = Vec::new();
    for line in robots_content.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with("sitemap:") {
            let sitemap_url = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            if !sitemap_url.is_empty() {
                sitemap_urls.push(sitemap_url);
            }
        }
    }

    if !sitemap_urls.is_empty() {
        println!("Found {} sitemap URL(s) in robots.txt", sitemap_urls.len());
        return Ok(sitemap_urls);
    }

    // If no sitemap found in robots.txt, try common locations
    println!("No sitemap directive found in robots.txt, will try common sitemap locations");
    Ok(common_sitemap_candidates(base_url))
}

/// Parse a sitemap index file and return all sitemap URLs
async fn parse_sitemap_index(content: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapEntry {
        loc: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapIndex {
        sitemap: Vec<SitemapEntry>,
    }

    // Try to parse as sitemap index
    match from_str::<SitemapIndex>(content) {
        Ok(index) => {
            println!("Found sitemap index with {} sitemaps", index.sitemap.len());
            Ok(index.sitemap.into_iter().map(|s| s.loc).collect())
        }
        Err(_) => {
            // Not a sitemap index, might be a regular sitemap
            Err("Not a sitemap index".into())
        }
    }
}

/// Load URLs from all sitemaps
async fn load_sitemap(
    base_url: &str,
    user_agent_mode: Arc<UserAgentMode>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let user_agent = get_user_agent(&user_agent_mode);

    // Find candidate sitemap URLs from robots.txt (or common locations)
    let initial_candidates = find_sitemap_url_from_robots(base_url, &user_agent).await?;

    let mut sitemap_urls_to_process = initial_candidates;
    let mut tried_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_page_urls = Vec::new();
    let mut any_sitemap_found = false;

    while !sitemap_urls_to_process.is_empty() {
        let current_sitemap_url = sitemap_urls_to_process.remove(0);

        if tried_urls.contains(&current_sitemap_url) {
            continue;
        }
        tried_urls.insert(current_sitemap_url.clone());

        println!("Processing sitemap: {}", current_sitemap_url);

        // Fetch the sitemap
        let mut builder = Request::get(&current_sitemap_url).header("User-Agent", &user_agent);
        if FORCE_HTTP1.load(Ordering::Relaxed) {
            builder = builder.version_negotiation(VersionNegotiation::http11());
        }
        let request = builder.body(());

        if request.is_err() {
            println!(
                "Error creating request for sitemap: {}",
                current_sitemap_url
            );
            continue;
        }

        let response = http_client().send_async(request?).await;

        if response.is_err() {
            println!("Error fetching sitemap: {}", current_sitemap_url);
            continue;
        }

        let mut response = response?;

        if response.status().as_str() != "200" {
            println!("Sitemap URL returned status: {}", response.status());
            // If we haven't found any working sitemap yet, queue remaining common locations
            if !any_sitemap_found {
                for candidate in common_sitemap_candidates(base_url) {
                    if !tried_urls.contains(&candidate) {
                        sitemap_urls_to_process.push(candidate);
                    }
                }
            }
            continue;
        }

        any_sitemap_found = true;
        let mut content = response.text().await?;

        // Check if we got HTML instead of XML
        if content.trim_start().to_lowercase().starts_with("<!doctype")
            || content.trim_start().to_lowercase().starts_with("<html")
        {
            println!("Sitemap URL returned HTML instead of XML, trying alternative approaches...");

            for candidate in common_sitemap_candidates(base_url) {
                if !tried_urls.contains(&candidate) {
                    sitemap_urls_to_process.push(candidate);
                }
            }
            continue;
        }

        // Clean up the content to handle XML declarations and BOMs
        content = content.trim_start().to_string();
        if content.starts_with('\u{feff}') {
            content = content[3..].to_string(); // Remove BOM
        }

        // Remove XML declaration if present
        if content.starts_with("<?xml") {
            if let Some(end) = content.find("?>") {
                content = content[end + 2..].trim_start().to_string();
            }
        }

        // Try to parse as sitemap index first
        match parse_sitemap_index(&content).await {
            Ok(more_sitemap_urls) => {
                // This is a sitemap index, add all the sitemaps to our processing queue
                println!(
                    "Adding {} more sitemaps to process",
                    more_sitemap_urls.len()
                );
                sitemap_urls_to_process.extend(more_sitemap_urls);
            }
            Err(_) => {
                // Not a sitemap index, try to parse as a regular sitemap
                match from_str::<UrlSet>(&content) {
                    Ok(urlset) => {
                        // Extract URLs from this sitemap
                        let mut urls: Vec<String> = urlset.url.into_iter().map(|u| u.loc).collect();
                        println!("Found {} URLs in sitemap", urls.len());
                        all_page_urls.append(&mut urls);
                    }
                    Err(e) => {
                        println!("Error parsing sitemap: {}", e);
                        // Try to debug by showing first few characters
                        let preview = if content.len() > 100 {
                            &content[..100]
                        } else {
                            &content
                        };
                        println!("Content preview: {}", preview);
                    }
                }
            }
        }
    }

    // If no sitemap was successfully processed, return an error
    if !any_sitemap_found {
        return Err("No valid sitemaps found".into());
    }

    // Deduplicate URLs from all sitemaps
    all_page_urls.sort();
    all_page_urls.dedup();

    println!(
        "Total unique URLs found across all sitemaps: {}",
        all_page_urls.len()
    );

    if all_page_urls.is_empty() {
        return Err("No URLs found in any sitemap".into());
    }

    Ok(all_page_urls)
}

/// Extract links from a URL and follow them to build a sitemap-like list.
/// Uses a BFS crawl with a link cache to avoid re-fetching pages already visited.
async fn follow_links_from_url(
    start_url: &str,
    concurrency: usize,
    stats: Arc<Mutex<Stats>>,
    user_agent_mode: Arc<UserAgentMode>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    println!("Follow-links mode: Starting to crawl from {}", start_url);

    let base_url = Arc::new(if let Ok(parsed) = Url::parse(start_url) {
        let host = parsed.host_str().unwrap_or_default();
        if host.is_empty() {
            start_url.to_string()
        } else {
            format!("{}://{}", parsed.scheme(), host)
        }
    } else {
        start_url.to_string()
    });

    // Cache: URL -> outgoing same-domain links found on that page.
    // Populated on first fetch; avoids re-fetching a page just to discover its links.
    let link_cache: Arc<Mutex<HashMap<String, Vec<String>>>> = Arc::new(Mutex::new(HashMap::new()));

    // All URLs we have seen (or queued). Prevents duplicate frontier entries.
    let visited: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    visited.lock().unwrap().insert(start_url.to_string());

    let sem = Arc::new(Semaphore::new(concurrency));
    let max_urls = 500;

    let mut frontier = vec![start_url.to_string()];

    while !frontier.is_empty() {
        if visited.lock().unwrap().len() >= max_urls {
            break;
        }

        let batch = std::mem::take(&mut frontier);
        println!(
            "Processing {} URLs (total discovered: {})",
            batch.len(),
            visited.lock().unwrap().len()
        );

        let mut handles = vec![];

        for url in batch {
            let sem = sem.clone();
            let link_cache = link_cache.clone();
            let base_url = base_url.clone();
            let stats = stats.clone();
            let ua = user_agent_mode.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.unwrap();

                // Return cached links without re-fetching the page
                if let Some(cached) = link_cache.lock().unwrap().get(&url).cloned() {
                    return cached;
                }

                // Fetch page
                let (status, time, size, html, _) =
                    make_request(&url, false, true, ua.clone(), true).await;
                stats.lock().unwrap().add_transaction(time, size, status);

                let Some(html_content) = html else {
                    link_cache.lock().unwrap().insert(url.clone(), vec![]);
                    return vec![];
                };

                // Load assets concurrently (cache warming)
                let protocol = Url::parse(&url)
                    .map(|p| p.scheme().to_string())
                    .unwrap_or_else(|_| "https".to_string());

                let mut asset_handles = vec![];
                for mut asset_url in extract_assets(&html_content, &base_url) {
                    if normalize_url(&asset_url) == normalize_url(&url) {
                        continue;
                    }
                    if asset_url.starts_with("http://") && protocol == "https" {
                        asset_url = asset_url.replace("http://", "https://");
                    } else if asset_url.starts_with("https://") && protocol == "http" {
                        asset_url = asset_url.replace("https://", "http://");
                    }
                    let stats = stats.clone();
                    let ua = ua.clone();
                    asset_handles.push(tokio::spawn(async move {
                        let (s, t, sz, _, _) =
                            make_request(&asset_url, false, false, ua, false).await;
                        stats.lock().unwrap().add_transaction(t, sz, s);
                    }));
                }
                for h in asset_handles {
                    let _ = h.await;
                }

                // Extract and cache same-domain links
                let links = extract_links(&html_content, &base_url);
                link_cache
                    .lock()
                    .unwrap()
                    .insert(url.clone(), links.clone());
                links
            }));
        }

        // Collect all links returned by this wave
        let mut new_links: Vec<String> = Vec::new();
        for h in handles {
            if let Ok(links) = h.await {
                new_links.extend(links);
            }
        }

        // Enqueue only URLs we haven't seen, up to the cap
        let mut vis = visited.lock().unwrap();
        for link in new_links {
            if vis.len() >= max_urls {
                break;
            }
            if vis.insert(link.clone()) {
                frontier.push(link);
            }
        }
    }

    let mut result: Vec<String> = visited.lock().unwrap().iter().cloned().collect();
    result.sort();
    println!("Discovered {} unique URLs by following links", result.len());
    Ok(result)
}

/// Crawl JavaScript/WASM sites using headless Chrome browser
async fn crawl_js_site(
    start_url: &str,
    concurrency: usize,
    stats: Arc<Mutex<Stats>>,
    discovery_threads: Option<usize>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    js_crawler::crawl_js_site(start_url, concurrency, stats, discovery_threads).await
}

/// Extract static assets from HTML content
fn extract_assets(html_content: &str, base_url: &str) -> Vec<String> {
    let mut assets = Vec::new();
    let html = Html::parse_fragment(html_content);

    // Extract CSS links
    if let Ok(links_selector) = Selector::parse("link[href]") {
        for link in html.select(&links_selector) {
            if let Some(href) = link.value().attr("href") {
                if let Ok(asset_url) = build_asset_url(href, base_url) {
                    assets.push(asset_url);
                }
            }
        }
    }

    // Extract JavaScript files
    if let Ok(script_selector) = Selector::parse("script[src]") {
        for script in html.select(&script_selector) {
            if let Some(src) = script.value().attr("src") {
                if let Ok(asset_url) = build_asset_url(src, base_url) {
                    assets.push(asset_url);
                }
            }
        }
    }

    // Extract images
    if let Ok(img_selector) = Selector::parse("img[src]") {
        for img in html.select(&img_selector) {
            if let Some(src) = img.value().attr("src") {
                if !src.starts_with("data:image/") {
                    if let Ok(asset_url) = build_asset_url(src, base_url) {
                        assets.push(asset_url);
                    }
                }
            }
        }
    }

    assets
}

/// Extract links from HTML content
fn extract_links(html_content: &str, base_url: &str) -> Vec<String> {
    let mut links = Vec::new();
    let html = Html::parse_fragment(html_content);

    // Get the base domain - if we can't extract it, we'll accept all links
    let base_domain = extract_domain(base_url);

    // Extract anchor links
    if let Ok(a_selector) = Selector::parse("a[href]") {
        for a in html.select(&a_selector) {
            if let Some(href) = a.value().attr("href") {
                // Skip empty links, anchors, javascript, mailto, tel links
                if href.is_empty()
                    || href.starts_with('#')
                    || href.starts_with("javascript:")
                    || href.starts_with("mailto:")
                    || href.starts_with("tel:")
                {
                    continue;
                }

                if let Ok(link_url) = build_asset_url(href, base_url) {
                    // Only include links from the same domain if we have a base domain
                    match (&base_domain, extract_domain(&link_url)) {
                        (Some(base), Some(link)) if base == &link => {
                            links.push(link_url);
                        }
                        (None, _) => {
                            // If we couldn't extract base domain, include all links
                            links.push(link_url);
                        }
                        _ => {} // Different domains or couldn't extract link domain
                    }
                }
            }
        }
    }

    links
}

/// Extract domain from URL
fn extract_domain(url: &str) -> Option<String> {
    if let Ok(parsed_url) = Url::parse(url) {
        if let Some(host) = parsed_url.host_str() {
            return Some(host.to_string());
        }
    }
    None
}

/// User-Agent selection strategy
#[derive(Clone)]
enum UserAgentMode {
    /// Single fixed User-Agent string
    Single(String),
    /// Rotate through the built-in browser-like User-Agent list
    RotateBuiltIn,
    /// Rotate through a list of User-Agents loaded from config
    RotateList(Vec<String>),
}

fn default_product_user_agent() -> String {
    "warmer/0.1.2 (+https://abh.ai/warmer)".to_string()
}

fn load_config(path: &str) -> FileConfig {
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

fn build_user_agent_mode(resolved: &ResolvedConfig) -> UserAgentMode {
    // 1. Single User-Agent from config
    if let Some(ref ua) = resolved.user_agent {
        return UserAgentMode::Single(ua.clone());
    }

    // 2. User-Agent list from config (custom list in .toml)
    if !resolved.user_agent_list.is_empty() {
        if resolved.user_agent_list.len() == 1 {
            return UserAgentMode::Single(resolved.user_agent_list[0].clone());
        } else {
            return UserAgentMode::RotateList(resolved.user_agent_list.clone());
        }
    }

    // 3. -a/--anonymize: rotate through built-in list in code
    if resolved.anonymize {
        UserAgentMode::RotateBuiltIn
    } else {
        // 4. Default: single product User-Agent
        UserAgentMode::Single(default_product_user_agent())
    }
}

fn get_user_agent(mode: &UserAgentMode) -> String {
    match mode {
        UserAgentMode::Single(ua) => ua.clone(),
        UserAgentMode::RotateBuiltIn => {
            let user_agents = [
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/121.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 OPR/129.0.0.0",
            ];

            let mut rng = rand::rng();
            user_agents[rng.random_range(0..user_agents.len())].to_string()
        }
        UserAgentMode::RotateList(list) => {
            if list.is_empty() {
                return default_product_user_agent();
            }
            let mut rng = rand::rng();
            list[rng.random_range(0..list.len())].clone()
        }
    }
}

/// Normalize URL for comparison (ignore http/https difference and trailing slashes)
fn normalize_url(url: &str) -> String {
    // Remove protocol (http:// or https://)
    let without_protocol = url.replace("http://", "").replace("https://", "");

    // Remove trailing slash if present
    let mut normalized = without_protocol.trim_end_matches('/').to_string();

    // Add domain if it's just a path
    if !normalized.contains('.') && !normalized.is_empty() {
        normalized = format!(
            "abh.ai{}",
            if normalized.starts_with('/') {
                normalized
            } else {
                format!("/{}", normalized)
            }
        );
    }

    normalized
}

/// Build full URL for asset
fn build_asset_url(asset_path: &str, base_url: &str) -> Result<String, url::ParseError> {
    if asset_path.starts_with("http://") || asset_path.starts_with("https://") {
        Ok(asset_path.to_string())
    } else if asset_path.starts_with("//") {
        Ok(format!("https:{}", asset_path))
    } else if asset_path.starts_with("/") {
        Ok(format!("{}{}", base_url, asset_path))
    } else {
        Ok(format!("{}/{}", base_url, asset_path))
    }
}

/// Make a single HTTP request asynchronously.
///
/// `need_body = true`: reads the response body as text (for HTML → link/asset extraction).
/// `need_body = false`: returns as soon as response headers arrive. The body is drained
/// in a background task so the connection can be reused (keep-alive). This is what we
/// want for load testing and for asset fetches — we only care that the server served a
/// response, not about its contents.
async fn make_request(
    url: &str,
    _verbose: bool,
    is_main_url: bool,
    user_agent_mode: Arc<UserAgentMode>,
    need_body: bool,
) -> (u16, f64, u64, Option<String>, String) {
    let start = Instant::now();
    let user_agent = get_user_agent(&user_agent_mode);

    let mut builder = Request::get(url)
        .header("User-Agent", user_agent)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Accept-Encoding", "gzip, deflate")
        .header("Connection", "keep-alive")
        .ssl_options(
            SslOption::DANGER_ACCEPT_INVALID_CERTS
                | SslOption::DANGER_ACCEPT_REVOKED_CERTS
                | SslOption::DANGER_ACCEPT_INVALID_HOSTS,
        )
        .redirect_policy(RedirectPolicy::Follow);
    if FORCE_HTTP1.load(Ordering::Relaxed) {
        builder = builder.version_negotiation(VersionNegotiation::http11());
    }
    let req_result = builder.body(());

    let req = match req_result {
        Ok(r) => r,
        Err(_) => return request_error(start, url, _verbose, is_main_url),
    };

    let mut resp = match http_client().send_async(req).await {
        Ok(r) => r,
        Err(_) => return request_error(start, url, _verbose, is_main_url),
    };

    // Response headers are in — stop the TTFB clock before we touch the body.
    let response_time = start.elapsed().as_millis() as f64;
    let status_code = resp.status().as_u16();

    let http_version = match resp.version() {
        isahc::http::Version::HTTP_09 => "HTTP/0.9",
        isahc::http::Version::HTTP_10 => "HTTP/1.0",
        isahc::http::Version::HTTP_11 => "HTTP/1.1",
        isahc::http::Version::HTTP_2 => "HTTP/2.0",
        isahc::http::Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/1.1",
    }
    .to_string();

    // Content-Length for reporting bytes even when we skip the body.
    let content_length: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let (html_content, data_size) = if status_code == 200 && need_body {
        match resp.text().await {
            Ok(content) => {
                let size = content.len() as u64;
                (Some(content), size)
            }
            Err(_) => (None, 0),
        }
    } else {
        // Drain the body in the background so libcurl can reuse the connection
        // instead of tearing it down. We've already stopped the timing clock.
        tokio::spawn(async move {
            let mut resp = resp;
            let _ = resp.consume().await;
        });
        (None, content_length)
    };

    let parsed_url = Url::parse(url);
    let path = parsed_url.as_ref().map(|u| u.path()).unwrap_or("/");
    let display_path = if path.is_empty() { "/" } else { path };
    print_transaction(
        status_code,
        response_time,
        data_size,
        "GET",
        display_path,
        _verbose,
        is_main_url,
        &http_version,
    );

    (
        status_code,
        response_time,
        data_size,
        html_content,
        http_version,
    )
}

fn request_error(
    start: Instant,
    url: &str,
    verbose: bool,
    is_main_url: bool,
) -> (u16, f64, u64, Option<String>, String) {
    let response_time = start.elapsed().as_millis() as f64;
    let default_version = "HTTP/1.1".to_string();
    print_transaction(
        0,
        response_time,
        0,
        "GET",
        url,
        verbose,
        is_main_url,
        &default_version,
    );
    (0, response_time, 0, None, default_version)
}

/// Crawl mode - process each URL only once
async fn crawl_urls(
    urls: Vec<String>,
    stats: Arc<Mutex<Stats>>,
    verbose: bool,
    no_assets: bool,
    user_agent_mode: Arc<UserAgentMode>,
    asset_cache: Arc<Mutex<HashMap<String, Vec<String>>>>,
) {
    let mut processed_urls = std::collections::HashSet::new();
    let mut urls_to_process = urls;

    while !urls_to_process.is_empty() {
        let current_url = urls_to_process.remove(0);

        // Skip if already processed
        if processed_urls.contains(&current_url) {
            continue;
        }

        processed_urls.insert(current_url.clone());

        // Extract base URL for asset/link loading and preserve the protocol
        let (base_url, protocol) = if let Ok(parsed_url) = Url::parse(&current_url) {
            let scheme = parsed_url.scheme();
            let host = parsed_url.host_str().unwrap_or_default();
            if host.is_empty() {
                (current_url.clone(), scheme.to_string())
            } else {
                (format!("{}://{}", scheme, host), scheme.to_string())
            }
        } else {
            (current_url.clone(), "https".to_string())
        };

        if no_assets {
            let (status_code, response_time, data_size, _, _) =
                make_request(&current_url, verbose, true, user_agent_mode.clone(), false).await;

            // Update stats
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(response_time, data_size, status_code);
            }
        } else {
            load_assets_from_url(
                &current_url,
                &base_url,
                stats.clone(),
                verbose,
                true,
                &current_url,
                &protocol,
                user_agent_mode.clone(),
                asset_cache.clone(),
            )
            .await;
        }
    }
}

/// Load static assets from a URL. Fetches the page, then fetches all assets in parallel.
/// Uses `asset_cache` so the HTML is parsed for assets only on the first visit per URL;
/// subsequent visits reuse the cached, deduped asset list.
async fn load_assets_from_url(
    url: &str,
    base_url: &str,
    stats: Arc<Mutex<Stats>>,
    verbose: bool,
    is_main_url: bool,
    main_url: &str,
    protocol: &str,
    user_agent_mode: Arc<UserAgentMode>,
    asset_cache: Arc<Mutex<HashMap<String, Vec<String>>>>,
) {
    let cached = asset_cache.lock().unwrap().get(url).cloned();

    // Only read the HTML body on the first visit to this URL (to extract assets).
    // Once assets are cached, we just need headers to know the server responded.
    let need_body = cached.is_none();
    let (status_code, response_time, data_size, html_content, _) = make_request(
        url,
        verbose,
        is_main_url,
        user_agent_mode.clone(),
        need_body,
    )
    .await;

    {
        let mut stats = stats.lock().unwrap();
        stats.add_transaction(response_time, data_size, status_code);
    }

    // Use cached asset list if we have one; otherwise parse HTML once and cache it.
    let assets: Vec<String> = if let Some(list) = cached {
        list
    } else if let Some(ref html) = html_content {
        let mut extracted = extract_assets(html, base_url);
        extracted.sort();
        extracted.dedup();
        asset_cache
            .lock()
            .unwrap()
            .insert(url.to_string(), extracted.clone());
        extracted
    } else {
        return;
    };

    let main_normalized = normalize_url(main_url);
    let mut handles = vec![];
    for mut asset_url in assets {
        if normalize_url(&asset_url) == main_normalized {
            continue;
        }
        if asset_url.starts_with("http://") && protocol == "https" {
            asset_url = asset_url.replace("http://", "https://");
        } else if asset_url.starts_with("https://") && protocol == "http" {
            asset_url = asset_url.replace("https://", "http://");
        }

        let stats = stats.clone();
        let ua = user_agent_mode.clone();
        handles.push(tokio::spawn(async move {
            let (s, t, sz, _, _) = make_request(&asset_url, verbose, false, ua, false).await;
            let mut stats = stats.lock().unwrap();
            stats.add_transaction(t, sz, s);
        }));
    }
    for h in handles {
        let _ = h.await;
    }
}

/// Run a single user's requests
async fn run_user(
    urls: Arc<Vec<String>>,
    stats: Arc<Mutex<Stats>>,
    repetitions: Option<usize>,
    duration: Option<Duration>,
    delay: u64,
    verbose: bool,
    internet_mode: bool,
    no_assets: bool,
    thread_id: usize,
    total_threads: usize,
    user_agent_mode: Arc<UserAgentMode>,
    asset_cache: Arc<Mutex<HashMap<String, Vec<String>>>>,
) {
    let mut rng = std::collections::hash_map::DefaultHasher::new();
    let start_time = Instant::now();
    let mut request_count = 0;

    // Assign each thread a contiguous slice of the URL list.
    // When there are fewer URLs than threads (common in -f mode with small sites),
    // wrap around so every thread gets at least one URL and stays active.
    let (start_idx, end_idx) = if urls.len() <= total_threads {
        // Each thread owns exactly one URL (round-robin wrapping).
        let idx = thread_id % urls.len();
        (idx, idx + 1)
    } else {
        let urls_per_thread = (urls.len() + total_threads - 1) / total_threads;
        let s = thread_id * urls_per_thread;
        let e = std::cmp::min(s + urls_per_thread, urls.len());
        (s, e)
    };

    loop {
        // Check if we should stop based on duration
        if let Some(dur) = duration {
            if start_time.elapsed() >= dur {
                break;
            }
        }

        // Check if we should stop based on repetitions
        if let Some(reps) = repetitions {
            if request_count >= reps {
                break;
            }
        }

        // Select URL
        let url = if internet_mode && (end_idx - start_idx) > 1 {
            // Random selection for internet mode within this thread's chunk
            use std::hash::{Hash, Hasher};
            request_count.hash(&mut rng);
            let offset = (rng.finish() as usize) % (end_idx - start_idx);
            urls[start_idx + offset].clone()
        } else {
            // Sequential selection within this thread's chunk
            let idx = start_idx + (request_count % (end_idx - start_idx));
            urls[idx].clone()
        };

        // Extract base URL for asset loading and preserve the protocol
        let (base_url, protocol) = if let Ok(parsed_url) = Url::parse(&url) {
            let scheme = parsed_url.scheme();
            let host = parsed_url.host_str().unwrap_or_default();
            if host.is_empty() {
                (url.clone(), scheme.to_string())
            } else {
                (format!("{}://{}", scheme, host), scheme.to_string())
            }
        } else {
            (url.clone(), "https".to_string())
        };

        // Make request and load assets unless disabled
        if no_assets {
            let (status_code, response_time, data_size, _, _) =
                make_request(&url, verbose, true, user_agent_mode.clone(), false).await;

            // Update stats
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(response_time, data_size, status_code);
            }
        } else {
            load_assets_from_url(
                &url,
                &base_url,
                stats.clone(),
                verbose,
                true,
                &url,
                &protocol,
                user_agent_mode.clone(),
                asset_cache.clone(),
            )
            .await;
        }

        request_count += 1;

        // Delay between requests with some randomness
        if delay > 0 {
            let random_delay = delay + rand::rng().random_range(0..=delay / 2);
            sleep(Duration::from_secs(random_delay)).await;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // If no arguments were provided, show help/usage and exit
    if std::env::args().len() == 1 {
        let mut cmd = Cli::command();
        cmd.print_help().unwrap();
        println!();
        return Ok(());
    }

    // Parse command line arguments first
    let args = Cli::parse();

    // Load config file (if provided) and merge with CLI into a single resolved config
    let file_cfg = if let Some(ref config_path) = args.config {
        load_config(config_path)
    } else {
        FileConfig::default()
    };
    let url = args.url.clone();
    let resolved = resolve_config(args, &file_cfg);

    // Configure Tokio runtime with thread count based on concurrency
    let worker_threads = resolved.concurrent * 2;

    // Create and run the Tokio runtime with our custom configuration
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .unwrap();

    // Run our async main function in the runtime
    runtime.block_on(async_main(resolved, url))
}

async fn async_main(
    resolved: ResolvedConfig,
    url: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    FORCE_HTTP1.store(resolved.http1, Ordering::Relaxed);

    // Setup stats and signal handler
    let stats = Arc::new(Mutex::new(Stats::new()));
    let stats_clone = stats.clone();

    // Configure User-Agent strategy from resolved config
    let user_agent_mode = Arc::new(build_user_agent_mode(&resolved));

    ctrlc::set_handler(move || {
        let mut stats = stats_clone.lock().unwrap();
        stats.finish();
        print_statistics(&stats);
        exit(0);
    })?;

    // Determine URLs to test - use JS mode, follow-links, or sitemap
    let urls = if let Some(ref url) = url {
        if resolved.js_mode {
            // If JS mode is enabled, use headless Chrome to crawl JavaScript/WASM sites
            match crawl_js_site(
                url,
                resolved.concurrent,
                stats.clone(),
                resolved.discovery_threads,
            )
            .await
            {
                Ok(discovered_urls) => discovered_urls,
                Err(js_err) => {
                    eprintln!("Failed to crawl JavaScript site: {}", js_err);
                    return Ok(());
                }
            }
        } else if resolved.follow_links {
            // If follow-links is enabled, bypass sitemap processing entirely
            match follow_links_from_url(
                url,
                resolved.concurrent,
                stats.clone(),
                user_agent_mode.clone(),
            )
            .await
            {
                Ok(discovered_urls) => discovered_urls,
                Err(follow_err) => {
                    eprintln!("Failed to follow links: {}", follow_err);
                    return Ok(());
                }
            }
        } else {
            // Try to load sitemap
            match load_sitemap(url, user_agent_mode.clone()).await {
                Ok(sitemap_urls) => sitemap_urls,
                Err(e) => {
                    eprintln!(
                        "Failed to load sitemap: {}. Try using --follow-links or --js option.",
                        e
                    );
                    return Ok(());
                }
            }
        }
    } else {
        // No URL provided: URL is required, so bail out with a clear error
        eprintln!("Error: URL argument is required. See --help for usage.");
        return Ok(());
    };

    if urls.is_empty() {
        eprintln!("No URLs found to test");
        return Ok(());
    }

    let urls = Arc::new(urls);
    let display_url = if urls.len() == 1 {
        urls[0].clone()
    } else {
        format!("{} URLs from sitemap", urls.len())
    };

    // Parse duration
    let duration = if let Some(ref time_str) = resolved.time {
        Some(parse_duration(time_str)?)
    } else {
        None
    };

    // Print header
    if resolved.crawl {
        println!("** WARMER 0.1.2");
        println!("** Crawling mode - processing each URL only once");
        println!("** The server is now under load...");
    } else if resolved.js_mode {
        println!("** WARMER 0.1.2");
        println!("** JavaScript mode - using headless Chrome browser to crawl JS/WASM sites");
        println!(
            "** Preparing {} concurrent users for battle.",
            resolved.concurrent
        );
        println!("The server is now under load...");
    } else {
        print_header(resolved.concurrent, &display_url);
    }

    // Shared asset cache: URL -> deduped list of asset URLs.
    // Avoids re-parsing HTML and re-discovering the same assets for every iteration.
    let asset_cache: Arc<Mutex<HashMap<String, Vec<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Handle execution modes
    if resolved.crawl {
        // Crawl mode - process each URL only once, directly
        crawl_urls(
            (*urls).clone(),
            stats.clone(),
            resolved.verbose,
            resolved.no_assets,
            user_agent_mode.clone(),
            asset_cache.clone(),
        )
        .await;
    } else {
        // Load testing mode (including JS and follow-links) - spawn concurrent users
        let mut handles = vec![];
        let total_threads = resolved.concurrent;

        for thread_id in 0..total_threads {
            let urls = urls.clone();
            let stats = stats.clone();
            let repetitions = resolved.repetitions;
            let duration = duration;
            let delay = resolved.delay;
            let verbose = resolved.verbose;
            let internet_mode = resolved.internet;
            let no_assets = resolved.no_assets;
            let user_agent_mode = user_agent_mode.clone();
            let asset_cache = asset_cache.clone();

            let handle = tokio::spawn(async move {
                run_user(
                    urls,
                    stats,
                    repetitions,
                    duration,
                    delay,
                    verbose,
                    internet_mode,
                    no_assets,
                    thread_id,
                    total_threads,
                    user_agent_mode,
                    asset_cache,
                )
                .await;
            });

            handles.push(handle);
        }

        // Wait for all users to complete
        for handle in handles {
            handle.await?;
        }
    }

    // Finish and print statistics
    {
        let mut stats = stats.lock().unwrap();
        stats.finish();
        print_statistics(&stats);
    }

    Ok(())
}

/// Returns a static string to use in place of missing
/// lastmod tag in the xml structure.
fn default_lastmod() -> String {
    "2021-12-28T08:37Z".to_string()
}

/// Returns a static string to use in place of missing
/// priority tag in the xml structure.
fn default_priority() -> String {
    "0.5".to_string()
}

/// Returns a static string to use in place of missing
/// changefreq tag in the xml structure.
fn default_changefreq() -> String {
    "daily".to_string()
}
