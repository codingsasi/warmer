use scraper::{Html, Selector};
use url::Url;

pub fn extract_assets(html_content: &str, base_url: &str) -> Vec<String> {
    let mut assets = Vec::new();
    let html = Html::parse_fragment(html_content);

    // CSS links
    if let Ok(links_selector) = Selector::parse("link[href]") {
        for link in html.select(&links_selector) {
            if let Some(href) = link.value().attr("href") {
                if let Ok(asset_url) = build_asset_url(href, base_url) {
                    assets.push(asset_url);
                }
            }
        }
    }

    // JavaScript files
    if let Ok(script_selector) = Selector::parse("script[src]") {
        for script in html.select(&script_selector) {
            if let Some(src) = script.value().attr("src") {
                if let Ok(asset_url) = build_asset_url(src, base_url) {
                    assets.push(asset_url);
                }
            }
        }
    }

    // Images
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

pub fn extract_links(html_content: &str, base_url: &str) -> Vec<String> {
    let mut links = Vec::new();
    let html = Html::parse_fragment(html_content);

    // Get the base domain - if we can't extract it, we'll accept all links
    let base_domain = extract_domain(base_url);

    if let Ok(a_selector) = Selector::parse("a[href]") {
        for a in html.select(&a_selector) {
            if let Some(href) = a.value().attr("href") {
                // Skip empty links, anchors, javascript, mailto, tel links
                if href.is_empty()
                    || href.starts_with('#')
                    || href.starts_with("javascript:")
                    || href.starts_with("mailto:")
                    || href.starts_with("tel:")
                {
                    continue;
                }

                if let Ok(link_url) = build_asset_url(href, base_url) {
                    match (&base_domain, extract_domain(&link_url)) {
                        (Some(base), Some(link)) if base == &link => {
                            links.push(link_url);
                        }
                        (None, _) => {
                            // If we couldn't extract base domain, include all links
                            links.push(link_url);
                        }
                        _ => {} // Different domains or couldn't extract link domain
                    }
                }
            }
        }
    }

    links
}

pub fn extract_domain(url: &str) -> Option<String> {
    if let Ok(parsed_url) = Url::parse(url) {
        if let Some(host) = parsed_url.host_str() {
            return Some(host.to_string());
        }
    }
    None
}

/// Normalize URL for comparison (ignore http/https difference and trailing slashes)
pub fn normalize_url(url: &str) -> String {
    let without_protocol = url.replace("http://", "").replace("https://", "");
    let mut normalized = without_protocol.trim_end_matches('/').to_string();

    // Add domain if it's just a path
    if !normalized.contains('.') && !normalized.is_empty() {
        normalized = format!(
            "abh.ai{}",
            if normalized.starts_with('/') {
                normalized
            } else {
                format!("/{}", normalized)
            }
        );
    }

    normalized
}

fn build_asset_url(asset_path: &str, base_url: &str) -> Result<String, url::ParseError> {
    if asset_path.starts_with("http://") || asset_path.starts_with("https://") {
        Ok(asset_path.to_string())
    } else if asset_path.starts_with("//") {
        Ok(format!("https:{}", asset_path))
    } else if asset_path.starts_with('/') {
        Ok(format!("{}{}", base_url, asset_path))
    } else {
        Ok(format!("{}/{}", base_url, asset_path))
    }
}
