# warmer
A siege-like HTTP load testing and CDN cache warming tool in Rust. Supports both sitemap-based cache warming and single URL load testing with concurrent request handling.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<?xml-stylesheet type="text/xsl" href="/sitemap.xsl"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
<url>
    <loc>https://abh.ai/</loc>
    <lastmod>2022-06-25T20:46Z</lastmod>
    <changefreq>daily</changefreq>
    <priority>1.0</priority>
</url>
<url>
    <loc>https://abh.ai/photos/nature</loc>
    <lastmod>2022-09-25T05:33Z</lastmod>
    <changefreq>monthly</changefreq>
    <priority>0.7</priority>
</url>
<url>
    <loc>https://abh.ai/portraits</loc>
    <lastmod>2022-09-24T18:42Z</lastmod>
    <changefreq>monthly</changefreq>
    <priority>0.7</priority>
</url>
</urlset>
```

## Other examples of sitemaps that work

- https://abh.ai/sitemap.xml
- https://qed42.com/sitemap.xml
- https://www.australia.gov.au/sitemap.xml
- https://www.alkhaleej.ae/sitemap.xml?page=1
- https://www.axelerant.com/sitemap.xml
- https://ffw.com/sitemap.xml

## Features

- **Parallel URL Processing**: Each thread processes different URLs from the sitemap in parallel
- **Time-based Testing**: Run tests for specific durations (seconds, minutes, hours)
- **Sitemap Support**: Load test all URLs from a sitemap.xml
- **Single URL Testing**: Test individual URLs like siege
- **Asset Loading**: Automatically loads CSS, JS, and images from HTML pages
- **Internet Mode**: Random URL selection for realistic load testing
- **Crawl Mode**: Process each URL only once, perfect for cache warming
- **Siege-like Output**: Colored status codes and comprehensive statistics
- **Performance Metrics**: Transaction rate, throughput, response times, availability
- **Cloudflare Bypass**: Rotating user agents and realistic request patterns to avoid bot detection

## Usage

### Command Line Options

- `-c, --concurrent <NUM>`: Number of concurrent users (default: 25)
- `-t, --time <TIME>`: Time to run the test (e.g., 5S, 1M, 1H)
- `-r, --repetitions <NUM>`: Number of repetitions per user
- `-d, --delay <SECONDS>`: Delay between requests (default: 1)
- `-v, --verbose`: Verbose output
- `--sitemap`: Use sitemap mode (default for all modes)
- `-i, --internet`: Internet mode - random URL selection from sitemap
- `--no-assets`: Disable static asset loading (CSS, JS, images) from HTML pages
- `--crawl`: Crawl mode - process each URL only once, then stop (uses concurrency 1, automatically uses sitemap)

### Examples

**Single URL load testing (like siege):**
```bash
./warmer https://abh.ai -t5S -c10
```

**Sitemap-based cache warming:**
```bash
./warmer https://example.com --sitemap -t1M -c25
```

**Internet mode with random URL selection:**
```bash
./warmer https://example.com --sitemap -i -t30S -c50
```

**Crawl mode (cache warming - each URL once):**
```bash
# Crawl all URLs from sitemap once (automatically detects sitemap)
./warmer https://example.com --crawl

# Or explicitly specify sitemap URL
./warmer https://example.com/sitemap.xml --crawl
```

**Pure load testing without assets:**
```bash
./warmer https://abh.ai -t5S --no-assets
```

**Verbose mode with asset loading:**
```bash
./warmer https://abh.ai -t30S -c10 -v
```

## Installation

### Build from source
1. Clone the repo
2. cd warmer
3. Install cargo https://doc.rust-lang.org/cargo/getting-started/installation.html
4. cargo build --release
5. The binary will be in the target/release folder. It will also be named `warmer`
6. You may need to install `libudev-dev`, `libssl-dev`, `openssl`, `pkg-config`, `build-essential`.

### Running using docker
1. docker pull abhaisasidharan/warmer
2. docker run abhaisasidharan/warmer -it warmer https://abh.ai -t5S -c10

## Output Example

```
** WARMER 0.1.2
** Preparing 25 concurrent users for battle.
The server is now under siege...
HTTP/1.1 200     0.03 secs: 8971 bytes ==> GET  /
HTTP/1.1 200     0.15 secs: 1585 bytes ==> GET  /menu/page.js
HTTP/1.1 200     0.20 secs: 8423 bytes ==> GET  /s3fs-public/styles/max_325x325/public/2023-10/ubuntu-canonical.png
...

Lifting the server siege...

Transactions:                475 hits
Availability:             100.00 %
Elapsed time:              19.59 secs
Data transferred:           0.00 MB
Response time:             38.21 ms
Transaction rate:          24.25 trans/sec
Throughput:                 0.00 MB/sec
Concurrency:                0.93
Successful transactions:      475
Failed transactions:           0
Longest transaction:      141.00 ms
Shortest transaction:      26.00 ms
```

## Notes
- Large sitemaps that include other zipped or gzipped sitemaps are not supported yet
- Currently supported on 64-bit Linux OS
- Asset loading is enabled by default for comprehensive cache warming
- Use `--no-assets` for pure load testing without asset crawling
