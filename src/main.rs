extern crate core;

use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::{thread, time};
use isahc::{config::RedirectPolicy, prelude::*, Request};
use std::env;

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

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut interval = "5";
    if args.len() > 1 {
        interval = &args[2];
    }
    let mut response = Request::get(&args[1])
        .redirect_policy(RedirectPolicy::Follow)
        .body(()).unwrap()
        .send().unwrap();
    if response.status().as_str() != "200" {
        panic!("The Sitemap URL returned non 200 status.");
    }
    let src = response.text().unwrap();
    println!("The sitemap was loaded successfully!");
    let us: UrlSet = from_str(src.as_str()).unwrap();
    for url in us.url.iter() {
        println!("{:?}", url.loc);
        let page = Request::get(&url.loc)
            .redirect_policy(RedirectPolicy::Follow)
            .body(()).unwrap()
            .send().unwrap();
        println!("{}", page.status().as_str());
        thread::sleep(time::Duration::from_secs(interval.parse::<u64>().unwrap()));
    }
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