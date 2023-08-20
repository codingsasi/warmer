use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::{thread, time};
use std::process::exit;
use std::sync::{Arc, Mutex};
use isahc::{config::RedirectPolicy, prelude::*, Request};
use clap::Parser;
use ctrlc;
use std::time::Instant;
use isahc::config::SslOption;
use scraper::{Html, Selector};
use url::{Url};

/// The struct to deserialize and hold the items in <url></url>
/// in the sitemap.xml
/// <url>
///     <loc>https://abh.ai/</loc>
///     <lastmod>2022-06-25T20:46Z</lastmod>
///     <changefreq>daily</changefreq>
///     <priority>1.0</priority>
/// </url>
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
struct Cli {
    /// The sitemap url
    url: String,
    /// The time interval between requests
    #[arg(default_value_t = 5)]
    interval: u64,
}

/// Store static assets that have been cached once in memory.
struct Assets {
    urls: Vec<String>
}

impl Assets {
    fn add(&mut self, value: String) {
        self.urls.push(value)
    }

    fn contains(&mut self, value: &String) -> bool {
        self.urls.contains(&value.to_string())
    }
}

/// The summary of loading URLs from sitemap.
#[derive(Clone)]
struct Summary {
    count: usize,
    response_times: Vec<usize>,
    avg_response_time: usize
}

impl Summary {
    fn count(&mut self) {
        self.count = self.count + 1;
    }

    fn calc_response_time(&mut self, value: usize) {
        self.response_times.push(value)
    }

    fn calc_avg_response_time(&mut self, value: usize) {
        self.avg_response_time = value;
    }
}

fn main() {
    let summary = Arc::new(Mutex::new(Summary {
        count: 0,
        response_times: Vec::new(),
        avg_response_time: 0
    }));
    let static_assets = Arc::new(Mutex::new(Assets {
        urls: Vec::new()
    }));
    let s = summary.clone();
    ctrlc::set_handler(move || {
        let mut total = 0;
        for rt in s.lock().unwrap().response_times.iter() {
            total = total + rt;
        }
        let avg_res_time = total / s.lock().unwrap().response_times.len();
        println!("\n--------------------------------------------------------");
        println!("Warmer stopped abruptly!");
        println!("Total requests: {}", s.lock().unwrap().count);
        println!("Average Response time: {} ms", avg_res_time);
        println!("--------------------------------------------------------");
        exit(0);
    })
    .expect("Error setting Ctrl-C handler");
    let args = Cli::parse();
    let base_url = args.url.to_owned();
    let sitemap_url = base_url.clone() + "/sitemap.xml";
    let mut response = Request::get(sitemap_url)
        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
        .redirect_policy(RedirectPolicy::Follow)
        .body(()).unwrap()
        .send().unwrap();
    if response.status().as_str() != "200" {
        panic!("The Sitemap URL returned non 200 status.");
    }
    let src = response.text().unwrap();
    println!("The sitemap was loaded successfully!");
    let us: UrlSet = from_str(src.as_str()).unwrap();
    load_pages(&us, &args, summary.clone(), &base_url, static_assets.clone());
    show_summary(summary.clone());
}

/// Load pages in urlset.
fn load_pages(urlset: &UrlSet, args: &Cli, summary: Arc<Mutex<Summary>>, base_url: &String, static_assets: Arc<Mutex<Assets>>) {
    for url in urlset.url.iter() {
        println!("{:?}", url.loc);
        let eurl = Url::parse(&url.loc).unwrap().to_string();
        // Start measuring time.
        let now = Instant::now();
        let mut page = Request::get(eurl)
            .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
            .redirect_policy(RedirectPolicy::Follow)
            .body(()).unwrap()
            .send().unwrap();
        // End measuring and save elapsed.
        let elapsed = now.elapsed();
        match page.text() {
            Ok(body) => {
                println!("{}", page.status().as_str());
                summary.lock().unwrap().calc_response_time(elapsed.as_millis() as usize);
                summary.lock().unwrap().count();
                load_assets(body, summary.clone(), &base_url, static_assets.clone());
                if args.interval != 0 {
                    thread::sleep(time::Duration::from_secs(args.interval));
                }
            }
            Err(e) => {
                println!("Some error occurred: {}", e.to_string());
                println!("{}", page.status().as_str());
                summary.lock().unwrap().calc_response_time(elapsed.as_millis() as usize);
                summary.lock().unwrap().count();
            }
        }

    }
}

/// Load, css, js and other static assets.
fn load_assets(body: String, summary: Arc<Mutex<Summary>>, base_url: &String, static_assets: Arc<Mutex<Assets>>) {
    let html = Html::parse_fragment(&body);
    let links_selector = Selector::parse("link").unwrap();
    let img_selector = Selector::parse("img").unwrap();
    let script_selector = Selector::parse("script").unwrap();
    let mut url: String;
    for link in html.select(&links_selector) {
        match link.value().attr("href") {
            Some(href) => {
                if !static_assets.lock().unwrap().contains(&href.to_string()) {
                    if href.contains("http://") || href.contains("https://") {
                        url = href.to_string();
                    }
                    else if href.starts_with("//") {
                        url = "https:".to_string() + &href.to_string();
                    }
                    else if href.starts_with("/") {
                        url = base_url.to_string() + &href.to_string();
                    }
                    else {
                        url = "https://".to_string() + &href.to_string();
                    }
                    println!("{}", url);
                    // Start measuring time.
                    let now = Instant::now();
                    let eurl = Url::parse(&url).unwrap().to_string();
                    match Request::get(eurl)
                        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
                        .redirect_policy(RedirectPolicy::Follow)
                        .body(()).unwrap()
                        .send() {
                        Ok(link_asset) => {
                            // End measuring and save elapsed.
                            let elapsed = now.elapsed();
                            println!("{}", link_asset.status().as_str());
                            summary.lock().unwrap().calc_response_time(elapsed.as_millis() as usize);
                            summary.lock().unwrap().count();
                            static_assets.lock().unwrap().add(href.to_string());
                        }
                        Err(e) => {
                            println!("Request get error: {}", e.to_string());
                        }
                    };
                }
            }
            _ => {
                // Do nothing if href is not found.
            }
        }
    }
    for img in html.select(&img_selector) {
        match img.value().attr("src") {
            Some(src) => {
                if !static_assets.lock().unwrap().contains(&src.to_string()) && !src.starts_with("data:image/") {
                    if src.starts_with("http://") || src.starts_with("https://") {
                        url = src.to_string();
                    }
                    else if src.contains("//") {
                        url = "https:".to_string() + &src.to_string();
                    }
                    else if src.contains("/") {
                        url = base_url.to_string() + &src.to_string();
                    }
                    else {
                        url = "https://".to_string() + &src.to_string();
                    }
                    println!("{}", url);
                    // Start measuring time.
                    let now = Instant::now();
                    let eurl = Url::parse(&url).unwrap().to_string();
                    match Request::get(eurl)
                        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
                        .redirect_policy(RedirectPolicy::Follow)
                        .body(()).unwrap()
                        .send() {
                        Ok(img_asset) => {
                            // End measuring and save elapsed.
                            let elapsed = now.elapsed();
                            println!("{}", img_asset.status().as_str());
                            summary.lock().unwrap().calc_response_time(elapsed.as_millis() as usize);
                            summary.lock().unwrap().count();
                            static_assets.lock().unwrap().add(src.to_string());
                        }
                        Err(e) => {
                            println!("Request get error2: {}", e.to_string());
                        }
                    };

                }
            }
            _ => {
                // Do nothing if src is not found. Unlikely scenario.
            }
        }
    }
    for script in html.select(&script_selector) {
        match script.value().attr("src") {
            Some(src) => {
                if !static_assets.lock().unwrap().contains(&src.to_string()) {
                    if src.starts_with("http://") || src.starts_with("https://") {
                        url = src.to_string();
                    }
                    else if src.contains("//") {
                        url = "https:".to_string() + &src.to_string();
                    }
                    else if src.contains("/") {
                        url = base_url.to_string() + &src.to_string();
                    }
                    else {
                        url = "https://".to_string() + &src.to_string();
                    }
                    println!("{}", url);
                    // Start measuring time.
                    let now = Instant::now();
                    let eurl = Url::parse(&url).unwrap().to_string();
                    match Request::get(eurl)
                        .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
                        .redirect_policy(RedirectPolicy::Follow)
                        .body(()).unwrap()
                        .send() {
                        Ok(script_asset) => {
                            // End measuring and save elapsed.
                            let elapsed = now.elapsed();
                            println!("{}", script_asset.status().as_str());
                            summary.lock().unwrap().calc_response_time(elapsed.as_millis() as usize);
                            summary.lock().unwrap().count();
                            static_assets.lock().unwrap().add(src.to_string());
                        }
                        Err(e) => {
                            println!("Request get error3: {}", e.to_string());
                        }
                    };
                }
            }
            _ => {
                // If src is not found,  do nothing. Some <script> tags don't have src.
            }
        }
    }
}

/// Print summary.
/// Total URLs loaded and average response time.
fn show_summary(summary: Arc<Mutex<Summary>>) {
    let mut total = 0;
    for rt in summary.lock().unwrap().response_times.iter() {
        total = total + rt;
    }
    let avg_res_time = total / summary.lock().unwrap().response_times.len();
    summary.lock().unwrap().calc_avg_response_time(avg_res_time);
    println!("\n--------------------------------------------------------");
    println!("Total requests: {}", summary.lock().unwrap().count);
    println!("Average Response time: {} ms", summary.lock().unwrap().avg_response_time);
    println!("--------------------------------------------------------");
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