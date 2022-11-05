extern crate core;

use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::{thread, time};
use std::cell::RefCell;
use std::iter::Sum;
use std::process::exit;
use isahc::{config::RedirectPolicy, prelude::*, Request};
use clap::Parser;
use ctrlc;
use std::time::Instant;

/// The struct to deserialize and hold the items in <url></url>
/// in the sitemap.xml
/// <url>
///     <loc>https://abh.ai/</loc>
///     <lastmod>2022-06-25T20:46Z</lastmod>
///     <changefreq>daily</changefreq>
///     <priority>1.0</priority>
/// </url>
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Url {
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
    url: Vec<Url>
}

#[derive(Parser)]
struct Cli {
    /// The sitemap url
    url: String,
    /// The time interval between requests
    #[arg(default_value_t = 5)]
    interval: u64,
}

/// The summary of loading URLs from sitemap.
#[derive(Clone)]
struct Summary {
    count: usize,
    response_times: Vec<usize>,
    avg_response_time: usize
}

fn main() {
    let mut summary = Summary {
        count: 0,
        response_times: Vec::new(),
        avg_response_time: 0
    };
    ctrlc::set_handler(move || {
        println!("received Ctrl+C!");
        // show_summary_after_ctrlc(summary.clone());
        let mut total = 0;
        for rt in summary.response_times.iter() {
            total = total + rt;
        }
        summary.avg_response_time = total / summary.response_times.len();
        println!("Warmer stopped abruptly!");
        println!("Total URLs loaded: {}", summary.count);
        println!("Average Response time: {} ms", summary.avg_response_time);
        exit(0);
    })
    .expect("Error setting Ctrl-C handler");
    let args = Cli::parse();
    let mut response = Request::get(&args.url)
        .redirect_policy(RedirectPolicy::Follow)
        .body(()).unwrap()
        .send().unwrap();
    if response.status().as_str() != "200" {
        panic!("The Sitemap URL returned non 200 status.");
    }
    let src = response.text().unwrap();
    println!("The sitemap was loaded successfully!");
    let us: UrlSet = from_str(src.as_str()).unwrap();
    load_pages(&us, &args, &mut summary);
    show_summary(&mut summary);
}

/// Load pages in urlset.
fn load_pages(urlset: &UrlSet, args: &Cli, summary: &mut Summary) {
    for url in urlset.url.iter() {
        println!("{:?}", url.loc);
        summary.count = summary.count + 1;
        // Start measuring time.
        let now = Instant::now();
        let page = Request::get(&url.loc)
            .redirect_policy(RedirectPolicy::Follow)
            .body(()).unwrap()
            .send().unwrap();
        // End measuring and save elapsed.
        let elapsed = now.elapsed();
        summary.response_times.push(elapsed.as_millis() as usize);
        println!("{}", page.status().as_str());
        println!("{:?}", args.interval);
        if args.interval != 0 {
            thread::sleep(time::Duration::from_secs(args.interval));
        }
    }
}

/// Print summary.
/// Total URLs loaded and average response time.
fn show_summary(summary: &mut Summary) {
    let mut total = 0;
    for rt in summary.response_times.iter() {
        total = total + rt;
    }
    summary.avg_response_time = total / summary.response_times.len();
    println!("Total URLs loaded: {}", summary.count);
    println!("Average Response time: {} ms", summary.avg_response_time);
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