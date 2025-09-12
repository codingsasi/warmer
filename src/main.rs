use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::sync::{Arc, Mutex};
// Removed unused imports
use std::time::{Duration, Instant};
use std::collections::HashMap;
use isahc::{config::RedirectPolicy, prelude::*, Request};
use clap::Parser;
use ctrlc;
use std::process::exit;
use isahc::config::SslOption;
use url::{Url};
use tokio::time::sleep;
use colored::*;
use scraper::{Html, Selector};
use rand::Rng;

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
    url: Vec<Urlc>
}

#[derive(Parser)]
#[command(name = "warmer")]
#[command(about = "A siege-like HTTP load testing and cache warming tool")]
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
    #[arg(short = 'd', long = "delay", default_value_t = 1)]
    delay: u64,

    /// Verbose output
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Use sitemap mode (default if no URL provided)
    #[arg(long = "sitemap")]
    sitemap: bool,

    /// Internet mode - random URL selection from sitemap
    #[arg(short = 'i', long = "internet")]
    internet: bool,

    /// Disable static asset loading (CSS, JS, images) from HTML pages
    #[arg(long = "no-assets")]
    no_assets: bool,

    /// Crawl mode - go through each URL only once, then stop
    #[arg(long = "crawl")]
    crawl: bool,

    /// Follow links mode - extract and follow links from pages when sitemap.xml is not found
    #[arg(long = "follow-links")]
    follow_links: bool,
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
        (&time_str[..time_str.len()-1], "S")
    } else if time_str.ends_with('M') {
        (&time_str[..time_str.len()-1], "M")
    } else if time_str.ends_with('H') {
        (&time_str[..time_str.len()-1], "H")
    } else {
        return Err("Invalid time format. Use format like 5S, 1M, 1H".to_string());
    };

    let num: u64 = num_str.parse().map_err(|_| "Invalid number in time format".to_string())?;

    match unit {
        "S" => Ok(Duration::from_secs(num)),
        "M" => Ok(Duration::from_secs(num * 60)),
        "H" => Ok(Duration::from_secs(num * 3600)),
        _ => Err("Invalid time unit. Use S, M, or H".to_string()),
    }
}

/// Color status codes like siege
fn color_status_code(status_code: u16) -> ColoredString {
    match status_code {
        200..=299 => status_code.to_string().green(),
        300..=399 => status_code.to_string().yellow(),
        400..=499 => status_code.to_string().red(),
        500..=599 => status_code.to_string().red().bold(),
        _ => status_code.to_string().white(),
    }
}

/// Format response time like siege
fn format_response_time(ms: f64) -> String {
    format!("{:.2} secs", ms / 1000.0)
}

/// Format data size like siege
fn format_data_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Print siege-style header
fn print_header(concurrent: usize, _url: &str) {
    println!("** WARMER 0.1.2");
    println!("** Preparing {} concurrent users for battle.", concurrent);
    println!("The server is now under siege...");
}

/// Print transaction details with optional highlighting for main URLs
fn print_transaction(status_code: u16, response_time: f64, data_size: u64, method: &str, path: &str, _verbose: bool, is_main_url: bool, http_version: &str) {
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
            http_version,
            status_colored,
            response_time_str,
            data_size_str,
            method,
            path
        );
    }
}

/// Print final statistics like siege
fn print_statistics(stats: &Stats) {
    println!("\nLifting the server siege...");
    println!();
    println!("Transactions:\t\t{:8} hits", stats.transactions);
    println!("Availability:\t\t{:8.2} %", stats.availability());
    println!("Elapsed time:\t\t{:8.2} secs", stats.elapsed_time());
    println!("Data transferred:\t{:8.2} MB", stats.data_transferred as f64 / (1024.0 * 1024.0));
    println!("Response time:\t\t{:8.2} ms", stats.avg_response_time());
    println!("Transaction rate:\t{:8.2} trans/sec", stats.transaction_rate());
    println!("Throughput:\t\t{:8.2} MB/sec", stats.throughput());
    println!("Concurrency:\t\t{:8.2}", stats.concurrency());
    println!("Successful transactions: {:8}", stats.successful_transactions);
    println!("Failed transactions:\t{:8}", stats.failed_transactions);

    if let Some(&max_time) = stats.response_times.iter().max_by(|a, b| a.partial_cmp(b).unwrap()) {
        println!("Longest transaction:\t{:8.2} ms", max_time);
    }

    if let Some(&min_time) = stats.response_times.iter().min_by(|a, b| a.partial_cmp(b).unwrap()) {
        println!("Shortest transaction:\t{:8.2} ms", min_time);
    }

    println!();
}

/// Find sitemap URL from robots.txt
async fn find_sitemap_url_from_robots(base_url: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Construct robots.txt URL
    let robots_url = format!("{}/robots.txt", base_url);
    println!("Checking robots.txt at {}", robots_url);

    // Request robots.txt
    let response = Request::get(&robots_url)
        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
        .redirect_policy(RedirectPolicy::Follow)
        .body(());

    if response.is_err() {
        println!("Error creating request for robots.txt");
        return Ok(format!("{}/sitemap.xml", base_url));
    }

    let response = response?.send();

    if response.is_err() {
        println!("Error fetching robots.txt");
        return Ok(format!("{}/sitemap.xml", base_url));
    }

    let mut response = response?;

    if response.status().as_str() != "200" {
        println!("No robots.txt found (status: {}), defaulting to /sitemap.xml", response.status());
        return Ok(format!("{}/sitemap.xml", base_url));
    }

    // Parse robots.txt to find Sitemap: directive
    let robots_content = response.text()?;
    for line in robots_content.lines() {
        let line = line.trim();
        if line.to_lowercase().starts_with("sitemap:") {
            let sitemap_url = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            if !sitemap_url.is_empty() {
                println!("Found sitemap URL in robots.txt: {}", sitemap_url);
                return Ok(sitemap_url);
            }
        }
    }

    // If no sitemap found in robots.txt, default to standard location
    println!("No sitemap directive found in robots.txt, defaulting to /sitemap.xml");
    Ok(format!("{}/sitemap.xml", base_url))
}

/// Parse a sitemap index file and return all sitemap URLs
async fn parse_sitemap_index(content: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapEntry {
        loc: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapIndex {
        sitemap: Vec<SitemapEntry>
    }

    // Try to parse as sitemap index
    match from_str::<SitemapIndex>(content) {
        Ok(index) => {
            println!("Found sitemap index with {} sitemaps", index.sitemap.len());
            Ok(index.sitemap.into_iter().map(|s| s.loc).collect())
        },
        Err(_) => {
            // Not a sitemap index, might be a regular sitemap
            Err("Not a sitemap index".into())
        }
    }
}

/// Load URLs from all sitemaps
async fn load_sitemap(base_url: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    // Find sitemap URL from robots.txt
    let initial_sitemap_url = find_sitemap_url_from_robots(base_url).await?;

    // Process sitemaps, starting with the initial one
    let mut sitemap_urls_to_process = vec![initial_sitemap_url];
    let mut all_page_urls = Vec::new();
    let mut any_sitemap_found = false;

    while !sitemap_urls_to_process.is_empty() {
        let current_sitemap_url = sitemap_urls_to_process.remove(0);
        println!("Processing sitemap: {}", current_sitemap_url);

        // Fetch the sitemap
        let response = Request::get(&current_sitemap_url)
            .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
            .redirect_policy(RedirectPolicy::Follow)
            .body(());

        if response.is_err() {
            println!("Error creating request for sitemap: {}", current_sitemap_url);
            continue;
        }

        let response = response?.send();

        if response.is_err() {
            println!("Error fetching sitemap: {}", current_sitemap_url);
            continue;
        }

        let mut response = response?;

        if response.status().as_str() != "200" {
            println!("Sitemap URL returned status: {}", response.status());
            continue;
        }

        any_sitemap_found = true;
        let content = response.text()?;

        // Try to parse as sitemap index first
        match parse_sitemap_index(&content).await {
            Ok(more_sitemap_urls) => {
                // This is a sitemap index, add all the sitemaps to our processing queue
                println!("Adding {} more sitemaps to process", more_sitemap_urls.len());
                sitemap_urls_to_process.extend(more_sitemap_urls);
            },
            Err(_) => {
                // Not a sitemap index, try to parse as a regular sitemap
                match from_str::<UrlSet>(&content) {
                    Ok(urlset) => {
                        // Extract URLs from this sitemap
                        let mut urls: Vec<String> = urlset.url.into_iter().map(|u| u.loc).collect();
                        println!("Found {} URLs in sitemap", urls.len());
                        all_page_urls.append(&mut urls);
                    },
                    Err(e) => {
                        println!("Error parsing sitemap: {}", e);
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

    println!("Total unique URLs found across all sitemaps: {}", all_page_urls.len());

    if all_page_urls.is_empty() {
        return Err("No URLs found in any sitemap".into());
    }

    Ok(all_page_urls)
}

/// Extract links from a URL and follow them to build a sitemap-like list
async fn follow_links_from_url(start_url: &str, concurrency: usize, stats: Arc<Mutex<Stats>>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    println!("Follow-links mode: Starting to crawl from {} with {} threads", start_url, concurrency);

    // First, get the initial page and extract links
    let base_url = if let Ok(parsed_url) = Url::parse(start_url) {
        let scheme = parsed_url.scheme();
        let host = parsed_url.host_str().unwrap_or("localhost");
        format!("{}://{}", scheme, host)
    } else {
        start_url.to_string()
    };

    // Make the request to get HTML content
    let (status_code, response_time, data_size, html_content, _) = make_request(start_url, false, true).await;

    // Update stats for the main request
    {
        let mut stats_guard = stats.lock().unwrap();
        stats_guard.add_transaction(response_time, data_size, status_code);
    }

    // Extract links from the homepage
    let mut all_links = vec![start_url.to_string()];
    if let Some(html) = html_content {
        let links = extract_links(&html, &base_url);
        all_links.extend(links);
    }

    // Deduplicate links
    all_links.sort();
    all_links.dedup();

    println!("Found {} initial links to process", all_links.len());

    // Create shared data structures with proper synchronization
    let processed_urls = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let discovered_urls = Arc::new(Mutex::new(all_links.clone()));

    // Process up to 100 URLs to avoid excessive crawling
    let max_limit = std::cmp::min(100, all_links.len());

    // Divide work among threads
    let mut thread_work = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        thread_work.push(Vec::new());
    }

    // Distribute URLs to threads in a round-robin fashion
    for (i, url) in all_links.iter().enumerate().take(max_limit) {
        let thread_idx = i % concurrency;
        thread_work[thread_idx].push(url.clone());
    }

    // Create worker threads
    let mut handles = vec![];
    let base_url = Arc::new(base_url);

    // Spawn worker threads
    for (_, work) in thread_work.into_iter().enumerate() {
        if work.is_empty() {
            continue; // Skip threads with no work
        }

        let processed_urls = processed_urls.clone();
        let discovered_urls = discovered_urls.clone();
        let base_url = base_url.clone();
        let stats = stats.clone();

        let handle = tokio::spawn(async move {
            for current_url in work {
                // Skip if already processed
                {
                    let mut processed = processed_urls.lock().unwrap();
                    if processed.contains(&current_url) {
                        continue;
                    }
                    processed.insert(current_url.clone());
                }

                // Extract protocol for asset loading
                let protocol = if let Ok(parsed_url) = Url::parse(&current_url) {
                    parsed_url.scheme().to_string()
                } else {
                    "https".to_string()
                };

                // Make the request to get HTML content
                let (status_code, response_time, data_size, html_content, _) = make_request(&current_url, false, true).await;

                // Update stats for the main request
                {
                    let mut stats_guard = stats.lock().unwrap();
                    stats_guard.add_transaction(response_time, data_size, status_code);
                }

                // If we got HTML content, load assets
                if let Some(html_clone) = html_content.clone() {
                    let assets = extract_assets(&html_clone, &base_url);

                    // Process assets in parallel using a local task group
                    let mut asset_handles = vec![];

                    // Load each asset, but skip the main URL and respect protocol
                    for mut asset_url in assets {
                        // Normalize URLs for comparison (ignore http/https difference)
                        let is_same_url = normalize_url(&asset_url) == normalize_url(&current_url);

                        // Skip if it's the main URL or if it's using a different protocol than requested
                        if !is_same_url {
                            // Enforce the same protocol as the main URL
                            if asset_url.starts_with("http://") && protocol == "https" {
                                asset_url = asset_url.replace("http://", "https://");
                            } else if asset_url.starts_with("https://") && protocol == "http" {
                                asset_url = asset_url.replace("https://", "http://");
                            }

                            let stats = stats.clone();
                            let asset_url_clone = asset_url.clone();

                            // Spawn a task for each asset
                            let handle = tokio::spawn(async move {
                                let (asset_status, asset_time, asset_size, _, _) = make_request(&asset_url_clone, false, false).await;

                                // Update stats for asset
                                {
                                    let mut stats_guard = stats.lock().unwrap();
                                    stats_guard.add_transaction(asset_time, asset_size, asset_status);
                                }
                            });

                            asset_handles.push(handle);
                        }
                    }

                    // Wait for all asset requests to complete
                    for handle in asset_handles {
                        let _ = handle.await;
                    }
                }

                // Extract additional links from HTML content
                if let Some(html) = html_content {
                    let links = extract_links(&html, &base_url);

                    for link in links {
                        let should_add = {
                            let processed = processed_urls.lock().unwrap();
                            !processed.contains(&link)
                        };

                        if should_add {
                            {
                                let mut discovered = discovered_urls.lock().unwrap();
                                discovered.push(link);
                            }
                        }
                    }
                }
            }
        });

        handles.push(handle);
    }

    // Wait for all workers to complete
    for handle in handles {
        handle.await?;
    }

    // Get the final list of discovered URLs
    let mut final_urls = {
        let discovered = discovered_urls.lock().unwrap();
        discovered.clone()
    };

    // Deduplicate URLs
    final_urls.sort();
    final_urls.dedup();

    println!("Discovered {} URLs by following links", final_urls.len());
    Ok(final_urls)
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
                   || href.starts_with("tel:") {
                    continue;
                }

                if let Ok(link_url) = build_asset_url(href, base_url) {
                    // Only include links from the same domain if we have a base domain
                    match (&base_domain, extract_domain(&link_url)) {
                        (Some(base), Some(link)) if base == &link => {
                            links.push(link_url);
                        },
                        (None, _) => {
                            // If we couldn't extract base domain, include all links
                            links.push(link_url);
                        },
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

/// Generate a random realistic user agent string
fn get_random_user_agent() -> &'static str {
    let user_agents = [
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/121.0 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/121.0 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/121.0 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0 +warmer (https://abh.ai/warmer)",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 OPR/129.0.0.0 +warmer (https://abh.ai/warmer)",
        // Fallback simple user agents
        "curl/7.68.0 +warmer (https://abh.ai/warmer)",
        "wget/1.20.3 +warmer (https://abh.ai/warmer)",
        "Python-urllib/3.8 +warmer (https://abh.ai/warmer)",
    ];

    let mut rng = rand::rng();
    user_agents[rng.random_range(0..user_agents.len())]
}

/// Normalize URL for comparison (ignore http/https difference and trailing slashes)
fn normalize_url(url: &str) -> String {
    // Remove protocol (http:// or https://)
    let without_protocol = url.replace("http://", "").replace("https://", "");

    // Remove trailing slash if present
    let mut normalized = without_protocol.trim_end_matches('/').to_string();

    // Add domain if it's just a path
    if !normalized.contains('.') && !normalized.is_empty() {
        normalized = format!("abh.ai{}", if normalized.starts_with('/') { normalized } else { format!("/{}", normalized) });
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

/// Make a single HTTP request with optional highlighting
async fn make_request(url: &str, _verbose: bool, is_main_url: bool) -> (u16, f64, u64, Option<String>, String) {
    let start = Instant::now();

    let result = Request::get(url)
        .header("User-Agent", get_random_user_agent())
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Accept-Encoding", "gzip, deflate")
        .header("Connection", "keep-alive")
        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
        .redirect_policy(RedirectPolicy::Follow)
        .body(())
        .map_err(|_| ())
        .and_then(|req| req.send().map_err(|_| ()));

    let elapsed = start.elapsed();
    let response_time = elapsed.as_millis() as f64;

    match result {
        Ok(mut resp) => {
            let status_code = resp.status().as_u16();
            let parsed_url = Url::parse(url).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
            let path = parsed_url.path();

            // Get the HTTP version
            let http_version = match resp.version() {
                isahc::http::Version::HTTP_09 => "HTTP/0.9",
                isahc::http::Version::HTTP_10 => "HTTP/1.0",
                isahc::http::Version::HTTP_11 => "HTTP/1.1",
                isahc::http::Version::HTTP_2 => "HTTP/2.0",
                isahc::http::Version::HTTP_3 => "HTTP/3.0",
                _ => "HTTP/1.1", // Default fallback
            }.to_string();

            // Try to get HTML content for asset extraction and calculate actual data size
            let (html_content, data_size) = if status_code == 200 {
                match resp.text() {
                    Ok(content) => {
                        let actual_size = content.len() as u64;
                        (Some(content), actual_size)
                    }
                    Err(_) => (None, 0)
                }
            } else {
                (None, 0)
            };

            if path.is_empty() {
                print_transaction(status_code, response_time, data_size, "GET", "/", _verbose, is_main_url, &http_version);
            } else {
                print_transaction(status_code, response_time, data_size, "GET", path, _verbose, is_main_url, &http_version);
            }

            (status_code, response_time, data_size, html_content, http_version)
        }
        Err(_) => {
            // For errors, we don't have HTTP version information, so use a default
            let default_version = "HTTP/1.1".to_string();
            print_transaction(0, response_time, 0, "GET", url, _verbose, is_main_url, &default_version);
            (0, response_time, 0, None, default_version)
        }
    }
}

/// Crawl mode - process each URL only once
async fn crawl_urls(
    urls: Vec<String>,
    stats: Arc<Mutex<Stats>>,
    verbose: bool,
    no_assets: bool,
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
            let host = parsed_url.host_str().unwrap_or("localhost");
            (format!("{}://{}", scheme, host), scheme.to_string())
        } else {
            ("https://localhost".to_string(), "https".to_string())
        };

        if no_assets {
            let (status_code, response_time, data_size, _, _) = make_request(&current_url, verbose, true).await;

            // Update stats
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(response_time, data_size, status_code);
            }
        } else {
            load_assets_from_url(&current_url, &base_url, stats.clone(), verbose, true, &current_url, &protocol).await;
        }
    }
}

/// Load static assets from a URL with optional highlighting
async fn load_assets_from_url(url: &str, base_url: &str, stats: Arc<Mutex<Stats>>, verbose: bool, is_main_url: bool, main_url: &str, protocol: &str) {
    let (status_code, response_time, data_size, html_content, _) = make_request(url, verbose, is_main_url).await;

    // Update stats for the main request
    {
        let mut stats = stats.lock().unwrap();
        stats.add_transaction(response_time, data_size, status_code);
    }

    // If we got HTML content, extract and load assets
    if let Some(html) = html_content {
        let assets = extract_assets(&html, base_url);

        // Load each asset, but skip the main URL and respect protocol
        for mut asset_url in assets {
            // Normalize URLs for comparison (ignore http/https difference)
            let is_same_url = normalize_url(&asset_url) == normalize_url(main_url);

            // Skip if it's the main URL or if it's using a different protocol than requested
            if !is_same_url {
                // Enforce the same protocol as the main URL
                if asset_url.starts_with("http://") && protocol == "https" {
                    asset_url = asset_url.replace("http://", "https://");
                } else if asset_url.starts_with("https://") && protocol == "http" {
                    asset_url = asset_url.replace("https://", "http://");
                }

                let (asset_status, asset_time, asset_size, _, _) = make_request(&asset_url, verbose, false).await;

                // Update stats for asset
                {
                    let mut stats = stats.lock().unwrap();
                    stats.add_transaction(asset_time, asset_size, asset_status);
                }
            }
        }
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
) {
    let mut rng = std::collections::hash_map::DefaultHasher::new();
    let start_time = Instant::now();
    let mut request_count = 0;

    // Calculate which URLs this thread should process
    let urls_per_thread = if urls.len() < total_threads {
        1 // If we have fewer URLs than threads, each thread gets at least one URL
    } else {
        urls.len() / total_threads + if urls.len() % total_threads > 0 { 1 } else { 0 }
    };

    // Calculate start and end indices for this thread's URL chunk
    let start_idx = thread_id * urls_per_thread;
    let end_idx = std::cmp::min(start_idx + urls_per_thread, urls.len());

    // If this thread has no URLs to process (can happen if we have more threads than URLs)
    if start_idx >= urls.len() {
        return;
    }

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
            let host = parsed_url.host_str().unwrap_or("localhost");
            (format!("{}://{}", scheme, host), scheme.to_string())
        } else {
            ("https://localhost".to_string(), "https".to_string())
        };

        // Make request and load assets unless disabled
        if no_assets {
            let (status_code, response_time, data_size, _, _) = make_request(&url, verbose, true).await;

            // Update stats
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(response_time, data_size, status_code);
            }
        } else {
            load_assets_from_url(&url, &base_url, stats.clone(), verbose, true, &url, &protocol).await;
        }

        request_count += 1;

        // Delay between requests with some randomness
        if delay > 0 {
            let random_delay = delay + rand::rng().random_range(0..=delay/2);
            sleep(Duration::from_secs(random_delay)).await;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();

    // Setup stats and signal handler
    let stats = Arc::new(Mutex::new(Stats::new()));
    let stats_clone = stats.clone();

    ctrlc::set_handler(move || {
        let mut stats = stats_clone.lock().unwrap();
        stats.finish();
        print_statistics(&stats);
        exit(0);
    })?;

    // Determine URLs to test - use follow-links directly if enabled, otherwise use sitemap
    let urls = if let Some(url) = args.url.clone() {
        if args.follow_links {
            // If follow-links is enabled, bypass sitemap processing entirely
            // Note: We don't need to load URLs in follow-links mode since we do the loading during discovery
            match follow_links_from_url(&url, args.concurrent, stats.clone()).await {
                Ok(discovered_urls) => discovered_urls,
                Err(follow_err) => {
                    eprintln!("Failed to follow links: {}", follow_err);
                    return Ok(());
                }
            }
        } else {
            // Try to load sitemap
            match load_sitemap(&url).await {
                Ok(sitemap_urls) => sitemap_urls,
                Err(e) => {
                    eprintln!("Failed to load sitemap: {}. Try using --follow-links option.", e);
                    return Ok(());
                }
            }
        }
    } else {
        // Default to sitemap mode with localhost, or follow-links if enabled
        if args.follow_links {
            match follow_links_from_url("http://localhost", args.concurrent, stats.clone()).await {
                Ok(discovered_urls) => discovered_urls,
                Err(_) => {
                    eprintln!("Failed to follow links from localhost");
                    return Ok(());
                }
            }
        } else {
            match load_sitemap("http://localhost").await {
                Ok(sitemap_urls) => sitemap_urls,
                Err(_) => {
                    eprintln!("Failed to load sitemap from localhost. Try using --follow-links option.");
                    return Ok(());
                }
            }
        }
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
    let duration = if let Some(time_str) = args.time {
        Some(parse_duration(&time_str)?)
    } else {
        None
    };

    // Print header
    if args.crawl {
        println!("** WARMER 0.1.2");
        println!("** Crawling mode - processing each URL only once");
        println!("** The server is now under siege...");
    } else {
        print_header(args.concurrent, &display_url);
    }

    // Handle special modes differently
    if args.crawl {
        // Crawl mode - process each URL only once, directly
        crawl_urls((*urls).clone(), stats.clone(), args.verbose, args.no_assets).await;
    } else if args.follow_links {
        // Follow-links mode already loaded the URLs during discovery, so we're done
        // Just wait a moment to ensure all stats are properly recorded
        sleep(Duration::from_millis(100)).await;
    } else {
        // Normal load testing mode - spawn concurrent users
        let mut handles = vec![];
        let total_threads = args.concurrent;

        for thread_id in 0..total_threads {
            let urls = urls.clone();
            let stats = stats.clone();
            let repetitions = args.repetitions;
            let duration = duration;
            let delay = args.delay;
            let verbose = args.verbose;
            let internet_mode = args.internet;
            let no_assets = args.no_assets;

            let handle = tokio::spawn(async move {
                run_user(urls, stats, repetitions, duration, delay, verbose, internet_mode, no_assets, thread_id, total_threads).await;
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