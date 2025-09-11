use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::sync::{Arc, Mutex};
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

/// Print siege-style transaction line
fn print_transaction(status_code: u16, response_time: f64, data_size: u64, method: &str, path: &str, verbose: bool) {
    let status_colored = color_status_code(status_code);
    let response_time_str = format_response_time(response_time);
    let data_size_str = format_data_size(data_size);

    if verbose {
        println!(
            "HTTP/1.1 {}     {}: {} ==> {}  {}",
            status_colored,
            response_time_str,
            data_size_str,
            method,
            path
        );
    } else {
        println!(
            "HTTP/1.1 {}     {}: {} ==> {}  {}",
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

/// Load URLs from sitemap
async fn load_sitemap(base_url: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let sitemap_url = format!("{}/sitemap.xml", base_url);
    let mut response = Request::get(&sitemap_url)
        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
        .redirect_policy(RedirectPolicy::Follow)
        .body(())?
        .send()?;

    if response.status().as_str() != "200" {
        return Err(format!("Sitemap URL returned status: {}", response.status()).into());
    }

    let src = response.text()?;
    let us: UrlSet = from_str(&src)?;

    Ok(us.url.into_iter().map(|u| u.loc).collect())
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

/// Make a single HTTP request with browser-like headers
async fn make_request(url: &str, _verbose: bool) -> (u16, f64, u64, Option<String>) {
    let start = Instant::now();

    let result = Request::get(url)
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Accept-Encoding", "gzip, deflate, br")
        .header("Connection", "keep-alive")
        .header("Upgrade-Insecure-Requests", "1")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Cache-Control", "max-age=0")
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
            let data_size = resp.headers()
                .get("content-length")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            let parsed_url = Url::parse(url).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
            let path = parsed_url.path();
            if path.is_empty() {
                print_transaction(status_code, response_time, data_size, "GET", "/", _verbose);
            } else {
                print_transaction(status_code, response_time, data_size, "GET", path, _verbose);
            }

            // Try to get HTML content for asset extraction
            let html_content = if status_code == 200 {
                resp.text().ok()
            } else {
                None
            };

            (status_code, response_time, data_size, html_content)
        }
        Err(_) => {
            print_transaction(0, response_time, 0, "GET", url, _verbose);
            (0, response_time, 0, None)
        }
    }
}

/// Load static assets from a URL
async fn load_assets_from_url(url: &str, base_url: &str, stats: Arc<Mutex<Stats>>, verbose: bool) {
    let (status_code, response_time, data_size, html_content) = make_request(url, verbose).await;

    // Update stats for the main request
    {
        let mut stats = stats.lock().unwrap();
        stats.add_transaction(response_time, data_size, status_code);
    }

    // If we got HTML content, extract and load assets
    if let Some(html) = html_content {
        let assets = extract_assets(&html, base_url);

        // Load each asset
        for asset_url in assets {
            let (asset_status, asset_response_time, asset_data_size, _) = make_request(&asset_url, verbose).await;

            // Update stats for the asset request
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(asset_response_time, asset_data_size, asset_status);
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
) {
    let mut rng = std::collections::hash_map::DefaultHasher::new();
    let start_time = Instant::now();
    let mut request_count = 0;

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
        let url = if internet_mode && urls.len() > 1 {
            // Random selection for internet mode
            use std::hash::{Hash, Hasher};
            request_count.hash(&mut rng);
            let idx = (rng.finish() as usize) % urls.len();
            urls[idx].clone()
        } else {
            // Sequential selection
            urls[request_count % urls.len()].clone()
        };

        // Extract base URL for asset loading
        let base_url = if let Ok(parsed_url) = Url::parse(&url) {
            format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or("localhost"))
        } else {
            "http://localhost".to_string()
        };

        // Make request and load assets unless disabled
        if no_assets {
            let (status_code, response_time, data_size, _) = make_request(&url, verbose).await;

            // Update stats
            {
                let mut stats = stats.lock().unwrap();
                stats.add_transaction(response_time, data_size, status_code);
            }
        } else {
            load_assets_from_url(&url, &base_url, stats.clone(), verbose).await;
        }

        request_count += 1;

        // Delay between requests
        if delay > 0 {
            sleep(Duration::from_secs(delay)).await;
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

    // Determine URLs to test
    let urls = if let Some(url) = args.url {
        if args.sitemap {
            // Sitemap mode with provided base URL
            load_sitemap(&url).await?
        } else {
            // Single URL mode
            vec![url]
        }
    } else {
        // Default to sitemap mode with localhost
        load_sitemap("http://localhost").await?
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
    print_header(args.concurrent, &display_url);

    // Spawn concurrent users
    let mut handles = vec![];

    for _ in 0..args.concurrent {
        let urls = urls.clone();
        let stats = stats.clone();
        let repetitions = args.repetitions;
        let duration = duration;
        let delay = args.delay;
        let verbose = args.verbose;
        let internet_mode = args.internet;
        let no_assets = args.no_assets;

        let handle = tokio::spawn(async move {
            run_user(urls, stats, repetitions, duration, delay, verbose, internet_mode, no_assets).await;
        });

        handles.push(handle);
    }

    // Wait for all users to complete
    for handle in handles {
        handle.await?;
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