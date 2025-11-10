# Packaging Guide for warmer

This guide explains how to build `.deb` (Debian/Ubuntu) and `.rpm` (Fedora/Red Hat) packages for warmer.

## Automated Building with GitHub Actions

Packages are automatically built and published when you create a new version tag (e.g., `v1.0.0`).

The workflow:
- Builds both `.deb` and `.rpm` packages in a Docker container (Ubuntu 22.04) for better glibc compatibility
- Automatically creates a GitHub release with the packages attached
- Includes installation instructions in the release notes

To create a new release:
1. Create and push a version tag: `git tag v1.0.0 && git push origin v1.0.0`
2. The workflow will automatically build packages and create a published release with the packages attached

## Prerequisites

### For .deb packages
- `cargo-deb`: Install with `cargo install cargo-deb`
- `dpkg`: Usually pre-installed on Debian/Ubuntu systems

### For .rpm packages
- `cargo-generate-rpm`: Install with `cargo install cargo-generate-rpm`
- `rpm`: Usually pre-installed on Fedora/RHEL systems

## Quick Start

### Build Both Packages (Recommended: Use Docker for Better Compatibility)

For better compatibility across different Linux distributions, use the Docker build:

```bash
./build-packages.sh --docker
```

This builds packages in a Docker container with Ubuntu 22.04 (glibc 2.35), ensuring compatibility with older systems.

### Build Locally

```bash
./build-packages.sh
```

This will:
1. Build the release binary (`cargo build --release`)
2. Generate both `.deb` and `.rpm` packages
3. Show the location of the generated packages

**Note**: Building locally may result in binaries that require newer glibc versions. Use `--docker` for better compatibility.

### Build Only .deb Package

```bash
./build-packages.sh --deb-only
```

### Build Only .rpm Package

```bash
./build-packages.sh --rpm-only
```

Or with Docker for better compatibility:
```bash
./build-packages.sh --rpm-only --docker
```

### Build Packages Without Rebuilding Binary

If you've already built the binary and just want to repackage:

```bash
./build-packages.sh --no-build
```

### Build Packages in Docker (Recommended)

For maximum compatibility, build packages in a Docker container with an older glibc:

```bash
./build-packages.sh --docker
```

This uses Ubuntu 22.04 which has glibc 2.35, making the binary compatible with systems that have glibc 2.35 or newer (instead of requiring 2.38+).

## Manual Build Instructions

### Building .deb Package

1. Ensure the binary is built:
   ```bash
   cargo build --release
   ```

2. Build the package:
   ```bash
   cargo deb
   ```

3. The `.deb` file will be in `target/debian/warmer_<version>_<arch>.deb`

4. Install the package:
   ```bash
   sudo dpkg -i target/debian/warmer_*.deb
   ```

### Building .rpm Package

1. Ensure the binary is built:
   ```bash
   cargo build --release
   ```

2. Build the package:
   ```bash
   cargo generate-rpm
   ```

3. The `.rpm` file will be in `target/generate-rpm/warmer-<version>-1.x86_64.rpm`

4. Install the package:
   ```bash
   sudo dnf install target/generate-rpm/warmer-*.rpm
   ```

## Package Configuration

Package metadata is configured in `Cargo.toml` under:
- `[package.metadata.deb]` for Debian packages
- `[package.metadata.generate-rpm]` for RPM packages

### Current Configuration

- **Binary**: Installed to `/usr/bin/warmer`
- **Documentation**: README.md and LICENSE installed to `/usr/share/doc/warmer/`
- **License**: GPL-2.0-only
- **Section**: `net` (for .deb)
- **Dependencies**:
  - `.deb`: Automatically detected dependencies (via `$auto` - includes libc6)
  - `.rpm`: `glibc >= 2.38`

**Important**: The binary requires glibc 2.38 or newer if built locally. If you need compatibility with older systems, build packages using `--docker` flag which uses Ubuntu 22.04 (glibc 2.35) for building, resulting in binaries compatible with glibc 2.35+.

## Testing Packages

### Test .deb Package

```bash
# Install
sudo dpkg -i target/debian/warmer_*.deb

# Verify installation
warmer --help

# Check package info
dpkg -l warmer
dpkg -L warmer

# Uninstall
sudo dpkg -r warmer
```

### Test .rpm Package

```bash
# Install
sudo dnf install target/generate-rpm/warmer-*.rpm

# Verify installation
warmer --help

# Check package info
rpm -qi warmer
rpm -ql warmer

# Uninstall
sudo dnf remove warmer
```

## Cross-Compilation

### For .deb packages

`cargo-deb` supports cross-compilation. You'll need to:
1. Install the target architecture: `rustup target add <target>`
2. Build for that target: `cargo build --release --target <target>`
3. Generate package: `cargo deb --target <target>`

Example for ARM64:
```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
cargo deb --target aarch64-unknown-linux-gnu
```

### For .rpm packages

`cargo-generate-rpm` also supports cross-compilation similarly.

## Troubleshooting

### cargo-deb not found
```bash
cargo install cargo-deb
```

### cargo-generate-rpm not found
```bash
cargo install cargo-generate-rpm
```

### Missing dependencies

If the package installation fails due to missing dependencies:
- For .deb: Use `sudo apt-get install -f` to install missing dependencies
- For .rpm: The package manager will list required dependencies

### Binary not found

Make sure you've built the release binary:
```bash
cargo build --release
```

### GLIBC version errors

If you see errors like:
```
warmer: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found
```

This means the binary was built on a system with a newer glibc than your target system. Solutions:

1. **Build in Docker** (recommended): Use `./build-packages.sh --docker` to build with an older glibc
2. **Upgrade glibc**: Update your system's glibc (may require system upgrade)
3. **Build on older system**: Build the binary on a system with an older glibc version

The Docker build uses Ubuntu 22.04 (glibc 2.35), making binaries compatible with systems that have glibc 2.35 or newer.

## Additional Requirements

### Google Chrome for JavaScript Mode (`--js` flag)

The `--js` flag requires Google Chrome to be installed on the system. Install it as follows:

**For Debian/Ubuntu (.deb systems):**
```bash
wget https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb
sudo apt install ./google-chrome-stable_current_amd64.deb
```

**For Fedora/RHEL (.rpm systems):**
```bash
# Download Chrome RPM
wget https://dl.google.com/linux/direct/google-chrome-stable_current_x86_64.rpm

# Install Chrome
sudo dnf install ./google-chrome-stable_current_x86_64.rpm
```

**Note**: Without Chrome installed, the `--js` flag will not work. The regular sitemap and crawling modes do not require Chrome.

## Package Contents

The packages include:
- Binary: `/usr/bin/warmer`
- Documentation: `/usr/share/doc/warmer/README.md`
- License: `/usr/share/doc/warmer/LICENSE`

## References

- [cargo-deb Documentation](https://github.com/mmstick/cargo-deb)
- [cargo-generate-rpm Documentation](https://github.com/cat-in-136/cargo-generate-rpm)
- [Debian Packaging Guide](https://www.debian.org/doc/manuals/packaging-tutorial/packaging-tutorial.en.pdf)
- [RPM Packaging Guide](https://rpm-packaging-guide.github.io/)

