#!/bin/bash

# warmer Package Build Script
# Builds .deb and .rpm packages for Linux distributions

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Help function
show_help() {
    echo "warmer Package Build Script"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --help, -h        Show this help message"
    echo "  --deb-only        Build .deb package only"
    echo "  --rpm-only        Build .rpm package only"
    echo "  --no-build        Skip building the binary (assumes it's already built)"
    echo "  --clean           Clean build artifacts after packaging"
    echo "  --docker          Build packages in Docker container (better glibc compatibility)"
    echo ""
    echo "Examples:"
    echo "  $0                # Build both .deb and .rpm packages"
    echo "  $0 --deb-only     # Build .deb package only"
    echo "  $0 --rpm-only     # Build .rpm package only"
    echo "  $0 --no-build     # Package existing binary"
    echo "  $0 --docker       # Build packages in Docker (recommended for compatibility)"
}

# Parse arguments
DEB_ONLY=false
RPM_ONLY=false
NO_BUILD=false
CLEAN=false
DOCKER_BUILD=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            show_help
            exit 0
            ;;
        --deb-only)
            DEB_ONLY=true
            shift
            ;;
        --rpm-only)
            RPM_ONLY=true
            shift
            ;;
        --no-build)
            NO_BUILD=true
            shift
            ;;
        --clean)
            CLEAN=true
            shift
            ;;
        --docker)
            DOCKER_BUILD=true
            shift
            ;;
        *)
            log_error "Unknown option $1"
            show_help
            exit 1
            ;;
    esac
done

# Docker build function
build_in_docker() {
    log_info "Building packages in Docker container for better glibc compatibility..."

    # Check if Docker is available
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed. Please install Docker or build without --docker flag."
        exit 1
    fi

    # Build Docker image if needed
    if ! docker images | grep -q "warmer-package-builder"; then
        log_info "Building Docker image for package building..."
        docker build -f Dockerfile.package -t warmer-package-builder .
    fi

    # Determine what to build
    BUILD_CMD=""
    if [ "$RPM_ONLY" = true ]; then
        BUILD_CMD="cargo generate-rpm"
    elif [ "$DEB_ONLY" = true ]; then
        BUILD_CMD="cargo deb"
    else
        BUILD_CMD="cargo deb && cargo generate-rpm"
    fi

    # Run build in container
    log_info "Running package build in Docker container..."
    docker run --rm -v "$(pwd):/build" -w /build warmer-package-builder bash -c "$BUILD_CMD"

    log_success "Docker build completed!"
    return 0
}

# If Docker build requested, do that and exit
if [ "$DOCKER_BUILD" = true ]; then
    build_in_docker
    exit 0
fi

# Check if cargo is available
if ! command -v cargo &> /dev/null; then
    log_error "cargo is not installed. Please install Rust: https://rustup.rs/"
    exit 1
fi

# Build the release binary if needed
if [ "$NO_BUILD" = false ]; then
    log_info "Building release binary..."
    cargo build --release
    log_success "Binary built successfully"
else
    log_info "Skipping build (using existing binary)"
    if [ ! -f "target/release/warmer" ]; then
        log_error "Binary not found at target/release/warmer. Run without --no-build first."
        exit 1
    fi
fi

# Build .deb package
build_deb() {
    log_info "Building .deb package..."

    # Check if cargo-deb is installed
    if ! command -v cargo-deb &> /dev/null; then
        log_warning "cargo-deb not found. Installing..."
        cargo install cargo-deb
    fi

    # Build the package
    if cargo deb; then
        DEB_FILE=$(find target/debian -name "*.deb" -type f | head -n 1)
        if [ -n "$DEB_FILE" ]; then
            log_success ".deb package created: $DEB_FILE"
            echo "  Install with: sudo dpkg -i $DEB_FILE"
        else
            log_error ".deb file not found in target/debian/"
            return 1
        fi
    else
        log_error "Failed to build .deb package"
        return 1
    fi
}

# Build .rpm package
build_rpm() {
    log_info "Building .rpm package..."

    # Check if cargo-generate-rpm is installed
    if ! command -v cargo-generate-rpm &> /dev/null; then
        log_warning "cargo-generate-rpm not found. Installing..."
        cargo install cargo-generate-rpm
    fi

    # Build the package
    if cargo generate-rpm; then
        RPM_FILE=$(find target/generate-rpm -name "*.rpm" -type f 2>/dev/null | head -n 1)
        if [ -n "$RPM_FILE" ]; then
            log_success ".rpm package created: $RPM_FILE"
            echo "  Install with: sudo dnf install $RPM_FILE"
        else
            log_error ".rpm file not found in target/generate-rpm/"
            return 1
        fi
    else
        log_error "Failed to build .rpm package"
        return 1
    fi
}

# Main build process
log_info "Starting package build process..."

BUILD_SUCCESS=true

if [ "$RPM_ONLY" = true ]; then
    build_rpm || BUILD_SUCCESS=false
elif [ "$DEB_ONLY" = true ]; then
    build_deb || BUILD_SUCCESS=false
else
    # Build both
    build_deb || BUILD_SUCCESS=false
    build_rpm || BUILD_SUCCESS=false
fi

if [ "$BUILD_SUCCESS" = true ]; then
    log_success "Package build completed successfully!"

    # Show package locations
    echo ""
    log_info "Package locations:"
    if [ "$RPM_ONLY" != true ]; then
        DEB_FILE=$(find target/debian -name "*.deb" -type f 2>/dev/null | head -n 1)
        if [ -n "$DEB_FILE" ]; then
            echo "  .deb: $DEB_FILE"
        fi
    fi
    if [ "$DEB_ONLY" != true ]; then
        RPM_FILE=$(find target/generate-rpm -name "*.rpm" -type f 2>/dev/null | head -n 1)
        if [ -n "$RPM_FILE" ]; then
            echo "  .rpm: $RPM_FILE"
        fi
    fi

    # Clean up if requested
    if [ "$CLEAN" = true ]; then
        log_info "Cleaning up build artifacts..."
        cargo clean
        log_success "Cleanup completed"
    fi
else
    log_error "Package build failed!"
    exit 1
fi

log_success "Build script completed!"

