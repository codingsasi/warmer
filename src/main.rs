mod client;
mod config;
mod crawler;
mod extract;
mod js_crawler;
mod sitemap;
mod stats;
mod user_agent;

use clap::CommandFactory;
use clap::Parser;
use client::FORCE_HTTP1;
use config::{Cli, FileConfig, load_config, parse_duration, resolve_config};
use crawler::{crawl_urls, follow_links_from_url, run_user};
use ctrlc;
use sitemap::load_sitemap;
use stats::{Stats, print_header, print_statistics};
use std::collections::HashMap;
use std::process::exit;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use user_agent::build_user_agent_mode;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // If no arguments were provided, show help/usage and exit
    if std::env::args().len() == 1 {
        let mut cmd = Cli::command();
        cmd.print_help().unwrap();
        println!();
        return Ok(());
    }

    let args = Cli::parse();

    let file_cfg = if let Some(ref config_path) = args.config {
        load_config(config_path)
    } else {
        FileConfig::default()
    };
    let url = args.url.clone();
    let resolved = resolve_config(args, &file_cfg);

    let worker_threads = resolved.concurrent * 2;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async_main(resolved, url))
}

async fn async_main(
    resolved: config::ResolvedConfig,
    url: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    FORCE_HTTP1.store(resolved.http1, Ordering::Relaxed);

    let stats = Arc::new(Mutex::new(Stats::new()));
    let stats_clone = stats.clone();

    let user_agent_mode = Arc::new(build_user_agent_mode(&resolved));

    ctrlc::set_handler(move || {
        let mut stats = stats_clone.lock().unwrap();
        stats.finish();
        print_statistics(&stats);
        exit(0);
    })?;

    let urls = if let Some(ref url) = url {
        if resolved.js_mode {
            match js_crawler::crawl_js_site(
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

    let duration = if let Some(ref time_str) = resolved.time {
        Some(parse_duration(time_str)?)
    } else {
        None
    };

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

    if resolved.crawl {
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

        for handle in handles {
            handle.await?;
        }
    }

    {
        let mut stats = stats.lock().unwrap();
        stats.finish();
        print_statistics(&stats);
    }

    Ok(())
}
