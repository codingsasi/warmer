#!/bin/bash

# warmer Docker Build Script
# Builds and pushes Docker images for multiple architectures

set -e  # Exit on any error

# Configuration
IMAGE_NAME="abhaisasidharan/warmer"
VERSION=${1:-"latest"}
BUILDER_NAME="warmer-builder"

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
    echo "warmer Docker Build Script"
    echo ""
    echo "Usage: $0 [VERSION] [OPTIONS]"
    echo ""
    echo "Arguments:"
    echo "  VERSION    Version tag for the image (default: latest)"
    echo ""
    echo "Options:"
    echo "  --help, -h     Show this help message"
    echo "  --local        Build locally only (no push)"
    echo "  --x86-only     Build x86_64 only"
    echo "  --arm-only     Build ARM64 only"
    echo "  --clean        Clean up builder and exit"
    echo ""
    echo "Examples:"
    echo "  $0                    # Build latest version"
    echo "  $0 v1.0.0            # Build version v1.0.0"
    echo "  $0 v1.0.0 --local    # Build locally without pushing"
    echo "  $0 --x86-only        # Build x86_64 only"
    echo "  $0 --arm-only        # Build ARM64 only"
    echo "  $0 --clean           # Clean up builder"
}

# Parse arguments
LOCAL_ONLY=false
X86_ONLY=false
ARM_ONLY=false
CLEAN_ONLY=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --help|-h)
            show_help
            exit 0
            ;;
        --local)
            LOCAL_ONLY=true
            shift
            ;;
        --x86-only)
            X86_ONLY=true
            shift
            ;;
        --arm-only)
            ARM_ONLY=true
            shift
            ;;
        --clean)
            CLEAN_ONLY=true
            shift
            ;;
        -*)
            log_error "Unknown option $1"
            show_help
            exit 1
            ;;
        *)
            VERSION="$1"
            shift
            ;;
    esac
done

# Clean up function
cleanup() {
    log_info "Cleaning up Docker builder..."
    docker buildx rm $BUILDER_NAME 2>/dev/null || true
    log_success "Cleanup completed"
}

# If clean only, do cleanup and exit
if [ "$CLEAN_ONLY" = true ]; then
    cleanup
    exit 0
fi

# Check if Docker is running
if ! docker info >/dev/null 2>&1; then
    log_error "Docker is not running. Please start Docker and try again."
    exit 1
fi

# Check if buildx is available
if ! docker buildx version >/dev/null 2>&1; then
    log_error "Docker buildx is not available. Please update Docker or install buildx."
    exit 1
fi

log_info "Building warmer Docker images"
log_info "Image: $IMAGE_NAME"
log_info "Version: $VERSION"

# Create or use existing builder
log_info "Setting up Docker builder..."
if docker buildx inspect $BUILDER_NAME >/dev/null 2>&1; then
    log_info "Using existing builder: $BUILDER_NAME"
    docker buildx use $BUILDER_NAME
else
    log_info "Creating new builder: $BUILDER_NAME"
    docker buildx create --name $BUILDER_NAME --use
fi

# Determine platforms to build
PLATFORMS=""
if [ "$X86_ONLY" = true ]; then
    PLATFORMS="linux/amd64"
elif [ "$ARM_ONLY" = true ]; then
    PLATFORMS="linux/arm64"
else
    PLATFORMS="linux/amd64,linux/arm64"
fi

log_info "Building for platforms: $PLATFORMS"

# Build command
BUILD_CMD="docker buildx build --platform $PLATFORMS -t $IMAGE_NAME:$VERSION"

# Add push flag if not local only
if [ "$LOCAL_ONLY" = false ]; then
    BUILD_CMD="$BUILD_CMD --push"
    log_info "Images will be pushed to registry"
else
    BUILD_CMD="$BUILD_CMD --load"
    log_info "Images will be built locally only"
fi

# Add build context
BUILD_CMD="$BUILD_CMD ."

# Execute build
log_info "Starting build process..."
log_info "Command: $BUILD_CMD"

if eval $BUILD_CMD; then
    log_success "Build completed successfully!"

    if [ "$LOCAL_ONLY" = false ]; then
        log_success "Images pushed to registry:"
        log_info "  - $IMAGE_NAME:$VERSION (multi-arch)"
    else
        log_success "Images built locally:"
        log_info "  - $IMAGE_NAME:$VERSION"

        # Show local images
        log_info "Local images:"
        docker images $IMAGE_NAME:$VERSION
    fi

    # Show usage examples
    echo ""
    log_info "Usage examples:"
    echo "  # Run the image"
    echo "  docker run $IMAGE_NAME:$VERSION warmer https://abh.ai -t5S -c10"
    echo ""
    echo "  # Run with specific platform"
    echo "  docker run --platform linux/arm64 $IMAGE_NAME:$VERSION warmer https://abh.ai -t5S -c10"
    echo ""
    echo "  # Run with JavaScript mode"
    echo "  docker run $IMAGE_NAME:$VERSION warmer https://example.com -j -T4"

else
    log_error "Build failed!"
    exit 1
fi

# Optional cleanup
if [ "$LOCAL_ONLY" = false ]; then
    echo ""
    read -p "Do you want to clean up the builder? (y/N): " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cleanup
    fi
fi

log_success "Build script completed!"
