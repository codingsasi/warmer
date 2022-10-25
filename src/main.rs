use serde::{Deserialize, Serialize};
use serde_xml_rs::{from_str};
use std::fs;
use curl::easy::Easy;
use std::{thread, time};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Url {
    loc: String,
    #[serde(default = "default_lastmod")]
    lastmod: String,
    changefreq: String,
    #[serde(default = "default_priority")]
    priority: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct UrlSet {
    url: Vec<Url>
}

fn main() {
    let src = fs::read_to_string("sitemap.xml").unwrap();
    let us: UrlSet = from_str(src.as_str()).unwrap();
    for url in us.url.iter() {
        println!("{:?}", url.loc);
        let mut easy = Easy::new();
        easy.url(&url.loc).unwrap();
        easy.write_function(|data| {
            Ok(data.len())
        }).unwrap();
        easy.perform().unwrap();

        println!("{}", easy.response_code().unwrap());
        thread::sleep(time::Duration::from_secs(5));
    }
}

fn default_lastmod() -> String {
    "2021-12-28T08:37Z".to_string()
}

fn default_priority() -> String {
    "0.5".to_string()
}