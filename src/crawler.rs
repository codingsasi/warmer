use crate::client::{FORCE_HTTP1, http_client};
use crate::extract::{extract_assets, extract_links, normalize_url};
use crate::stats::{Stats, print_transaction};
use crate::user_agent::{UserAgentMode, get_user_agent};
use isahc::{
    Request,
    config::{RedirectPolicy, SslOption, VersionNegotiation},
    prelude::*,
};
use rand::Rng;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use url::Url;

/// Make a single HTTP request asynchronously.
///
/// `need_body = true`: reads the response body as text (for HTML → link/asset extraction).
/// `need_body = false`: returns as soon as response headers arrive. The body is drained
/// in a background task so the connection can be reused (keep-alive). This is what we
/// want for load testing and for asset fetches — we only care that the server served a
/// response, not about its contents.
pub async fn make_request(
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

/// Extract links from a URL and follow them to build a sitemap-like list.
/// Uses a BFS crawl with a link cache to avoid re-fetching pages already visited.
pub async fn follow_links_from_url(
    start_url: &str,
    concurrency: usize,
    stats: Arc<Mutex<Stats>>,
    user_agent_mode: Arc<UserAgentMode>,
    no_assets: bool,
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
    let link_cache: Arc<Mutex<HashMap<String, Vec<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));

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

                if !no_assets {
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

/// Crawl mode - process each URL only once
pub async fn crawl_urls(
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

/// Run a single user's requests
pub async fn run_user(
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
        if let Some(dur) = duration {
            if start_time.elapsed() >= dur {
                break;
            }
        }

        if let Some(reps) = repetitions {
            if request_count >= reps {
                break;
            }
        }

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

        if no_assets {
            let (status_code, response_time, data_size, _, _) =
                make_request(&url, verbose, true, user_agent_mode.clone(), false).await;

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
