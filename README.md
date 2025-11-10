# warmer
A modern HTTP load testing and CDN cache warming tool in Rust. Inspired by tools like siege and wrk, warmer provides advanced features including sitemap-based cache warming, JavaScript site crawling, and comprehensive asset discovery with concurrent request handling.

## Quick Start

```bash
# Pull and run warmer with Docker (works on x86_64 and ARM64)
docker run abhaisasidharan/warmer warmer https://example.com -t5S -c10

# For JavaScript/WASM sites
docker run abhaisasidharan/warmer warmer https://example-spa.com -j

# For sitemap-based cache warming
docker run abhaisasidharan/warmer warmer https://example.com -s -t1M -c25
```

### For Developers

```bash
# Build and push latest version (both architectures)
./build-docker.sh

# Build specific version
./build-docker.sh v1.0.0

# Build locally for testing
./build-docker.sh v1.0.0 --local
```

## Features

- **Parallel URL Processing**: Each thread processes different URLs from the sitemap in parallel with dynamic thread pool scaling
- **Time-based Testing**: Run tests for specific durations (seconds, minutes, hours)
- **Advanced Sitemap Support**: Finds sitemap URLs from robots.txt and handles sitemap indexes
- **Single URL Testing**: Test individual URLs like siege
- **Asset Loading**: Automatically loads CSS, JS, and images from HTML pages
- **Internet Mode**: Random URL selection for realistic load testing
- **Crawl Mode**: Process each URL only once, perfect for cache warming
- **Follow Links Mode**: Automatically discover and test URLs by following links from the provided URL
- **Siege-like Output**: Colored status codes, actual HTTP version, and comprehensive statistics
- **Performance Metrics**: Transaction rate, throughput, response times, availability
- **Cloudflare Bypass**: Rotating user agents and realistic request patterns to avoid bot detection

## Usage

### Command Line Options

- `-c, --concurrent <NUM>`: Number of concurrent users (default: 25)
- `-t, --time <TIME>`: Time to run the test (e.g., 5S, 1M, 1H)
- `-r, --repetitions <NUM>`: Number of repetitions per user
- `-d, --delay <SECONDS>`: Delay between requests (default: 1)
- `-v, --verbose`: Verbose output
- `-s, --sitemap`: Use sitemap mode (default for all modes)
- `-i, --internet`: Internet mode - random URL selection from sitemap
- `-n, --no-assets`: Disable static asset loading (CSS, JS, images) from HTML pages
- `-w, --crawl`: Crawl mode - process each URL only once, then stop (uses concurrency 1, automatically uses sitemap)
- `-f, --follow-links`: Follow links mode - discover URLs by following links from the provided URL (bypasses sitemap processing)
- `-j, --js`: JavaScript mode - use headless Chrome browser to crawl JavaScript/WASM sites and discover dynamically generated links (automatically disables sitemap mode)
- `-T, --discovery-threads <NUM>`: Number of discovery threads for JavaScript mode (default: CPU cores / 2, min 2, max 8)

### Examples

**Single URL load testing:**
```bash
docker run abhaisasidharan/warmer warmer https://example.com -t5S -c10
```

**Sitemap-based cache warming:**
```bash
docker run abhaisasidharan/warmer warmer https://example.com -s -t1M -c25
```

**Internet mode with random URL selection:**
```bash
docker run abhaisasidharan/warmer warmer https://example.com -s -i -t30S -c50
```

**Crawl mode (cache warming - each URL once):**
```bash
# Crawl all URLs from sitemap once (automatically detects sitemap)
docker run abhaisasidharan/warmer warmer https://example.com -w

# Or explicitly specify sitemap URL
docker run abhaisasidharan/warmer warmer https://example.com/sitemap.xml -w
```

**Pure load testing without assets:**
```bash
docker run abhaisasidharan/warmer warmer https://abh.ai -t5S -n
```

**Verbose mode with asset loading:**
```bash
docker run abhaisasidharan/warmer warmer https://abh.ai -t30S -c10 -v
```

**Follow links mode (for sites without sitemap.xml):**
```bash
docker run abhaisasidharan/warmer warmer https://www.tdtreedays.com -f
```

**JavaScript mode (for JS/WASM sites):**
```bash
docker run abhaisasidharan/warmer warmer https://example-spa.com -j
```

**JavaScript mode with custom thread count:**
```bash
docker run abhaisasidharan/warmer warmer https://example-spa.com -j -T4
```

**Sitemap mode (XML parsing only):**
```bash
docker run abhaisasidharan/warmer warmer https://example.com -s
```

## Installation

### Docker (Recommended)
The easiest way to run warmer is using Docker:

```bash
# Pull the latest image
docker pull abhaisasidharan/warmer

# Run warmer with any URL
docker run abhaisasidharan/warmer warmer https://abh.ai -t5S -c10

# Run with JavaScript mode
docker run abhaisasidharan/warmer warmer https://example.com -j -T4

# Run with sitemap mode
docker run abhaisasidharan/warmer warmer https://example.com -s -t1M -c25
```

### Install from Package (.deb or .rpm)

#### Debian/Ubuntu (.deb)

1. Download the `.deb` package from the [releases page](https://github.com/codingsasi/warmer/releases)
2. Install the package:
   ```bash
   sudo dpkg -i warmer_*.deb
   ```
3. If there are missing dependencies, install them:
   ```bash
   sudo apt-get install -f
   ```

#### Fedora/RHEL (.rpm)

1. Download the `.rpm` package from the [releases page](https://github.com/codingsasi/warmer/releases)
2. Install the package:
   ```bash
   sudo dnf install warmer-*.rpm
   ```

#### Google Chrome for JavaScript Mode

The `--js` flag requires Google Chrome to be installed. Install it as follows:

**For Debian/Ubuntu:**
```bash
wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
sudo apt install ./google-chrome-stable_current_amd64.deb
```

**For Fedora/RHEL:**
```bash
wget https://dl.google.com/linux/direct/google-chrome-stable_current_x86_64.rpm
sudo dnf install ./google-chrome-stable_current_x86_64.rpm
```

**Note**: Without Chrome installed, the `--js` flag will not work. Regular sitemap and crawling modes do not require Chrome.

### Build from source
If you prefer to build from source:

1. Clone the repo: `git clone https://github.com/codingsasi/warmer.git && cd warmer`
2. Install Rust: https://doc.rust-lang.org/cargo/getting-started/installation.html
3. Build: `cargo build --release`
4. The binary will be in `target/release/warmer`
5. You may need to install: `libudev-dev`, `libssl-dev`, `openssl`, `pkg-config`, `build-essential`

## Output Example

```
** WARMER 0.1.8
** Preparing 25 concurrent users for battle.
The server is now under load...
HTTP/2.0 200     0.03 secs: 8971 bytes ==> GET  /
HTTP/1.1 200     0.15 secs: 1585 bytes ==> GET  /menu/page.js
HTTP/2.0 200     0.20 secs: 8423 bytes ==> GET  /s3fs-public/styles/max_325x325/public/2023-10/ubuntu-canonical.png
...

Load testing completed...

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
- **Docker Recommended**: The easiest way to run warmer is using Docker. No local installation needed!
- **Package Installation**: Native `.deb` and `.rpm` packages are available for direct installation on Linux systems
- **Multi-architecture Support**: Works on x86_64 (Intel/AMD) and ARM64 (Apple Silicon, ARM servers)
- **Chrome Required for JS Mode**: The `--js` flag requires Google Chrome to be installed separately (see Installation section)
- Large sitemaps that include other zipped or gzipped sitemaps are not supported yet
- Asset loading is enabled by default for comprehensive cache warming
- Use `-n, --no-assets` for pure load testing without asset crawling
- The tool automatically checks robots.txt to find the correct sitemap URL
- Sitemap indexes (XML files containing links to other sitemaps) are fully supported and recursively processed
- JavaScript mode uses multiple headless Chrome instances for parallel discovery - use `-T, --discovery-threads` to control memory usage
- Cross-platform: Works on Linux, macOS, and Windows with Docker
