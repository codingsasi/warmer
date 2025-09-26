use headless_chrome::Browser;
use isahc::{prelude::*, config::{SslOption, RedirectPolicy}, Request};
use serde_json;
use std::sync::{Arc, Mutex};
use url::Url;
use crate::Stats;

/// Crawl JavaScript/WASM sites using headless Chrome browser with recursive discovery and load testing
pub async fn crawl_js_site(start_url: &str, concurrency: usize, stats: Arc<Mutex<Stats>>, discovery_threads: Option<usize>) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    println!("JavaScript mode: Starting headless Chrome browser to crawl from {}", start_url);

    // Extract base host for filtering
    let base_host = if let Ok(parsed_url) = Url::parse(start_url) {
        parsed_url.host_str().unwrap_or("localhost").to_string()
    } else {
        "localhost".to_string()
    };

    // Global collections to track everything
    let all_discovered_urls = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let all_discovered_assets = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let visited_urls = Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Function to discover URLs and assets from a single page
    fn discover_page(url: &str, base_host: &str, browser: &Browser) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
        // JavaScript for extracting links
        let links_js = r#"
            (() => {
                const links = Array.from(document.querySelectorAll('a[href]'));
                console.log('Found links:', links.length);
                const result = links.map(link => {
                    try {
                        const fullUrl = new URL(link.href, window.location.href).href;
                        console.log('Link:', link.href, '->', fullUrl);
                        return fullUrl;
                    } catch (e) {
                        console.log('Invalid link:', link.href, e);
                        return null;
                    }
                }).filter(link => link !== null);
                console.log('Valid links:', result.length);
                return result;
            })()
        "#;

        // JavaScript for extracting assets
        let assets_js = r#"
            (() => {
                const assets = [];

                // CSS files
                document.querySelectorAll('link[rel="stylesheet"]').forEach(link => {
                    if (link.href) assets.push(link.href);
                });

                // JavaScript files
                document.querySelectorAll('script[src]').forEach(script => {
                    if (script.src) assets.push(script.src);
                });

                // Images
                document.querySelectorAll('img[src]').forEach(img => {
                    if (img.src) assets.push(img.src);
                });

                // Favicons
                document.querySelectorAll('link[rel*="icon"]').forEach(link => {
                    if (link.href) assets.push(link.href);
                });

                return assets.map(asset => {
                    try {
                        return new URL(asset, window.location.href).href;
                    } catch (e) {
                        return null;
                    }
                }).filter(asset => asset !== null);
            })()
        "#;

        let tab = browser.new_tab()?;
        tab.navigate_to(url)?;
        tab.wait_until_navigated()?;

        // Wait for dynamic content to load
        std::thread::sleep(std::time::Duration::from_millis(3000));

        // Extract links
        let links_result = tab.evaluate(links_js, true)?;
        // println!("Links evaluation result: {:?}", links_result);

        let page_links: Vec<String> = match links_result.value {
            Some(value) => {
                println!("Links value: {:?}", value);
                serde_json::from_value(value).unwrap_or_else(|e| {
                    println!("Failed to parse links JSON: {}", e);
                    Vec::new()
                })
            },
            None => {
                println!("No links value, checking preview...");
                if let Some(preview) = &links_result.preview {
                    // println!("Links preview: {:?}", preview);
                    let mut links = Vec::new();
                    for prop in &preview.properties {
                        if let Some(value) = &prop.value {
                            if value.starts_with("http") {
                                links.push(value.clone());
                            }
                        }
                    }
                    links
                } else {
                    Vec::new()
                }
            },
        };

        // Extract assets
        let assets_result = tab.evaluate(assets_js, true)?;
        let page_assets: Vec<String> = match assets_result.value {
            Some(value) => serde_json::from_value(value).unwrap_or_else(|_| Vec::new()),
            None => {
                // Extract from preview when value is None (common with large arrays)
                if let Some(preview) = &assets_result.preview {
                    let mut assets = Vec::new();
                    for prop in &preview.properties {
                        if let Some(value) = &prop.value {
                            if value.starts_with("http") {
                                assets.push(value.clone());
                            }
                        }
                    }
                    assets
                } else {
                    Vec::new()
                }
            },
        };

        // Filter same-host links
        let same_host_links: Vec<String> = page_links.into_iter()
            .filter(|link| {
                if let Ok(parsed_url) = Url::parse(link) {
                    if let Some(link_host) = parsed_url.host_str() {
                        return link_host == base_host;
                    }
                }
                false
            })
            .collect();

        println!("Discovered {} same-host links and {} assets from {}", same_host_links.len(), page_assets.len(), url);
        Ok((same_host_links, page_assets))
    }

    // Function to load test assets using HTTP requests
    fn load_test_assets(assets: Vec<String>, stats: Arc<Mutex<Stats>>, concurrency: usize) {
        if assets.is_empty() {
            return;
        }

        println!("Load testing {} assets with {} threads", assets.len(), concurrency);

        let assets = Arc::new(Mutex::new(assets));
        let mut handles = Vec::new();

        for _i in 0..concurrency {
            let assets = assets.clone();
            let stats = stats.clone();

            let handle = std::thread::spawn(move || {
                loop {
                    let asset = {
                        let mut assets = assets.lock().unwrap();
                        assets.pop()
                    };

                    if let Some(url) = asset {
                        // Perform the HTTP request
                        let start_time = std::time::Instant::now();
                        let response = Request::get(&url)
                            .ssl_options(SslOption::DANGER_ACCEPT_INVALID_CERTS | SslOption::DANGER_ACCEPT_REVOKED_CERTS | SslOption::DANGER_ACCEPT_INVALID_HOSTS)
                            .redirect_policy(RedirectPolicy::Follow)
                            .body(())
                            .map_err(|e| format!("Request creation failed: {}", e))
                            .and_then(|req| req.send().map_err(|e| format!("Request send failed: {}", e)));

                        let elapsed = start_time.elapsed();
                        let mut stats = stats.lock().unwrap();

                        match response {
                            Ok(response) => {
                                let status = response.status();
                                let content_length = response.headers().get("content-length")
                                    .and_then(|h| h.to_str().ok())
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or(0);

                                stats.add_transaction(elapsed.as_millis() as f64, content_length as u64, status.as_u16());
                                println!("HTTP/{} {}     {:.2} secs: {} KB ==> GET  {}",
                                    status.as_str().chars().next().unwrap_or('?'),
                                    status.as_str(),
                                    elapsed.as_secs_f64(),
                                    content_length / 1024,
                                    url
                                );
                            }
                            Err(e) => {
                                stats.add_transaction(elapsed.as_millis() as f64, 0, 0);
                                println!("HTTP/1.1 0     {:.2} secs: 0 bytes ==> GET  {} (Error: {})",
                                    elapsed.as_secs_f64(),
                                    url,
                                    e
                                );
                            }
                        }
                    } else {
                        break;
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all load testing to complete
        for handle in handles {
            handle.join().unwrap();
        }
    }

    // Calculate discovery threads (use provided value or default to CPU cores / 2, min 2, max 8)
    let discovery_threads = discovery_threads.unwrap_or_else(|| std::cmp::min(8, std::cmp::max(2, num_cpus::get() / 2)));
    println!("Using {} discovery threads for parallel page crawling (CPU cores: {})", discovery_threads, num_cpus::get());

    // Shared queue for URLs to process
    let urls_to_process = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let active_threads = Arc::new(Mutex::new(discovery_threads));
    {
        let mut queue = urls_to_process.lock().unwrap();
        queue.push_back(start_url.to_string());
    }

    // Start discovery threads
    let mut discovery_handles = Vec::new();
    for i in 0..discovery_threads {
        let urls_to_process = urls_to_process.clone();
        let visited_urls = visited_urls.clone();
        let all_discovered_urls = all_discovered_urls.clone();
        let all_discovered_assets = all_discovered_assets.clone();
        let active_threads = active_threads.clone();
        let stats = stats.clone();
        let base_host = base_host.clone();

        let handle = std::thread::spawn(move || {
            // Each thread gets its own browser instance
            let browser = match Browser::default() {
                Ok(browser) => {
                    browser
                },
                Err(e) => {
                    eprintln!("Discovery thread {} failed to create browser: {}", i, e);
                    return;
                }
            };

            loop {
                let current_url = {
                    let mut queue = urls_to_process.lock().unwrap();
                    queue.pop_front()
                };

                let current_url = match current_url {
                    Some(url) => url,
                    None => {
                        // No more URLs available right now
                        // Check if there are other active threads that might add URLs
                        let active_count = {
                            let mut active = active_threads.lock().unwrap();
                            *active -= 1; // This thread is going idle
                            *active
                        };

                        if active_count == 0 {
                            // No other threads working, we're done
                            break;
                        } else {
                            // Other threads might add URLs, wait and check again
                            std::thread::sleep(std::time::Duration::from_millis(100));

                            // Reactivate this thread
                            {
                                let mut active = active_threads.lock().unwrap();
                                *active += 1;
                            }
                            continue;
                        }
                    }
                };

                // Skip if already visited
                {
                    let mut visited = visited_urls.lock().unwrap();
                    if visited.contains(&current_url) {
                        continue;
                    }
                    visited.insert(current_url.clone());
                }

                // 1. Discover URLs and assets from this page
                match discover_page(&current_url, &base_host, &browser) {
                    Ok((page_urls, page_assets)) => {
                        // 2. Load test the discovered assets immediately
                        load_test_assets(page_assets.clone(), stats.clone(), concurrency);

                        // 3. Add new URLs to global collection and processing queue
                        {
                            let mut all_urls = all_discovered_urls.lock().unwrap();
                            let mut all_assets = all_discovered_assets.lock().unwrap();
                            let mut queue = urls_to_process.lock().unwrap();

                            for url in &page_urls {
                                if all_urls.insert(url.clone()) {
                                    queue.push_back(url.clone());
                                }
                            }

                            for asset in &page_assets {
                                all_assets.insert(asset.clone());
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Discovery thread {} failed to process {}: {}", i, current_url, e);
                    }
                }
            }

            // Thread is exiting, decrement active count
            {
                let mut active = active_threads.lock().unwrap();
                *active -= 1;
                println!("Discovery thread {} exiting, {} threads remaining", i, *active);
            }
        });
        discovery_handles.push(handle);
    }

    // Wait for all discovery threads to complete
    for handle in discovery_handles {
        handle.join().unwrap();
    }
    println!("All discovery threads completed");

    // Return all discovered URLs and assets
    let all_urls = all_discovered_urls.lock().unwrap();
    let all_assets = all_discovered_assets.lock().unwrap();

    let mut combined: Vec<String> = all_urls.iter().cloned().collect();
    combined.extend(all_assets.iter().cloned());
    combined.sort();
    combined.dedup();

    println!("Total discovered {} URLs and {} assets via JavaScript crawling", all_urls.len(), all_assets.len());

    Ok(combined)
}