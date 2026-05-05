use crate::client::{FORCE_HTTP1, http_client};
use crate::user_agent::{UserAgentMode, get_user_agent};
use isahc::{Request, config::VersionNegotiation, prelude::*};
use serde::{Deserialize, Serialize};
use serde_xml_rs::from_str;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct UrlSet {
    url: Vec<Urlc>,
}

fn default_lastmod() -> String {
    "2021-12-28T08:37Z".to_string()
}

fn default_priority() -> String {
    "0.5".to_string()
}

fn default_changefreq() -> String {
    "daily".to_string()
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

async fn find_sitemap_url_from_robots(
    base_url: &str,
    user_agent: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let robots_url = format!("{}/robots.txt", base_url);
    println!("Checking robots.txt at {}", robots_url);

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

    println!("No sitemap directive found in robots.txt, will try common sitemap locations");
    Ok(common_sitemap_candidates(base_url))
}

async fn parse_sitemap_index(content: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapEntry {
        loc: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SitemapIndex {
        sitemap: Vec<SitemapEntry>,
    }

    match from_str::<SitemapIndex>(content) {
        Ok(index) => {
            println!("Found sitemap index with {} sitemaps", index.sitemap.len());
            Ok(index.sitemap.into_iter().map(|s| s.loc).collect())
        }
        Err(_) => Err("Not a sitemap index".into()),
    }
}

pub async fn load_sitemap(
    base_url: &str,
    user_agent_mode: Arc<UserAgentMode>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let user_agent = get_user_agent(&user_agent_mode);

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

        content = content.trim_start().to_string();
        if content.starts_with('\u{feff}') {
            content = content[3..].to_string(); // Remove BOM
        }

        if content.starts_with("<?xml") {
            if let Some(end) = content.find("?>") {
                content = content[end + 2..].trim_start().to_string();
            }
        }

        match parse_sitemap_index(&content).await {
            Ok(more_sitemap_urls) => {
                println!(
                    "Adding {} more sitemaps to process",
                    more_sitemap_urls.len()
                );
                sitemap_urls_to_process.extend(more_sitemap_urls);
            }
            Err(_) => match from_str::<UrlSet>(&content) {
                Ok(urlset) => {
                    let mut urls: Vec<String> = urlset.url.into_iter().map(|u| u.loc).collect();
                    println!("Found {} URLs in sitemap", urls.len());
                    all_page_urls.append(&mut urls);
                }
                Err(e) => {
                    println!("Error parsing sitemap: {}", e);
                    let preview = if content.len() > 100 {
                        &content[..100]
                    } else {
                        &content
                    };
                    println!("Content preview: {}", preview);
                }
            },
        }
    }

    if !any_sitemap_found {
        return Err("No valid sitemaps found".into());
    }

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
