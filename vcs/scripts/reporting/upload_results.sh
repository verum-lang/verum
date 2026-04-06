#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Results Uploader
# =============================================================================
#
# Uploads VCS test results to various destinations:
# - S3/GCS/Azure Blob storage
# - GitHub Artifacts
# - Custom HTTP endpoints
# - Local archive directories
#
# Usage:
#   ./upload_results.sh [options]
#
# Options:
#   --results DIR        Results directory to upload
#   --destination TYPE   Destination type: s3, gcs, azure, http, local
#   --bucket NAME        Bucket/container name (for cloud storage)
#   --path PATH          Path prefix within destination
#   --endpoint URL       HTTP endpoint (for http destination)
#   --archive DIR        Archive directory (for local destination)
#   --retain DAYS        Retention period in days (default: 30)
#   --compress           Compress results before upload
#   --dry-run            Print commands without executing
#   --verbose            Enable verbose output
#   -h, --help           Show help message
#
# Environment Variables:
#   AWS_ACCESS_KEY_ID     AWS access key (for S3)
#   AWS_SECRET_ACCESS_KEY AWS secret key (for S3)
#   GOOGLE_APPLICATION_CREDENTIALS GCP credentials (for GCS)
#   AZURE_STORAGE_CONNECTION_STRING Azure connection (for Azure)
#   VCS_UPLOAD_ENDPOINT   Default HTTP endpoint
#   VCS_UPLOAD_TOKEN      Authentication token
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
RESULTS_DIR="${VCS_ROOT}/reports"
DESTINATION="local"
BUCKET=""
PATH_PREFIX="vcs-results"
ENDPOINT="${VCS_UPLOAD_ENDPOINT:-}"
ARCHIVE_DIR="${VCS_ROOT}/archives"
RETAIN_DAYS=30
COMPRESS=0
DRY_RUN=0
VERBOSE=0

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Usage
usage() {
    head -n 45 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --results)
                RESULTS_DIR="$2"
                shift 2
                ;;
            --destination)
                DESTINATION="$2"
                shift 2
                ;;
            --bucket)
                BUCKET="$2"
                shift 2
                ;;
            --path)
                PATH_PREFIX="$2"
                shift 2
                ;;
            --endpoint)
                ENDPOINT="$2"
                shift 2
                ;;
            --archive)
                ARCHIVE_DIR="$2"
                shift 2
                ;;
            --retain)
                RETAIN_DAYS="$2"
                shift 2
                ;;
            --compress)
                COMPRESS=1
                shift
                ;;
            --dry-run)
                DRY_RUN=1
                shift
                ;;
            --verbose)
                VERBOSE=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done
}

# Run command (respects dry-run)
run_cmd() {
    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] $*"
        return 0
    fi

    if [ "$VERBOSE" -eq 1 ]; then
        echo "[CMD] $*"
    fi

    "$@"
}

# Generate timestamp path
get_timestamp_path() {
    echo "$PATH_PREFIX/$(date +%Y/%m/%d)/$(date +%H%M%S)"
}

# Compress results if requested
prepare_results() {
    local src="$1"
    local dst="$2"

    if [ "$COMPRESS" -eq 1 ]; then
        log_info "Compressing results..."
        local archive="$dst.tar.gz"
        run_cmd tar -czf "$archive" -C "$(dirname "$src")" "$(basename "$src")"
        echo "$archive"
    else
        echo "$src"
    fi
}

# Upload to S3
upload_s3() {
    local source="$1"
    local bucket="$2"
    local path="$3"

    if ! command -v aws &>/dev/null; then
        log_error "AWS CLI not found. Install with: pip install awscli"
        return 1
    fi

    log_info "Uploading to S3: s3://$bucket/$path"

    if [ -d "$source" ]; then
        run_cmd aws s3 sync "$source" "s3://$bucket/$path" \
            --storage-class STANDARD_IA
    else
        run_cmd aws s3 cp "$source" "s3://$bucket/$path/"
    fi

    log_success "Uploaded to S3"
}

# Upload to GCS
upload_gcs() {
    local source="$1"
    local bucket="$2"
    local path="$3"

    if ! command -v gsutil &>/dev/null; then
        log_error "gsutil not found. Install with: pip install google-cloud-storage"
        return 1
    fi

    log_info "Uploading to GCS: gs://$bucket/$path"

    if [ -d "$source" ]; then
        run_cmd gsutil -m rsync -r "$source" "gs://$bucket/$path"
    else
        run_cmd gsutil cp "$source" "gs://$bucket/$path/"
    fi

    log_success "Uploaded to GCS"
}

# Upload to Azure Blob Storage
upload_azure() {
    local source="$1"
    local container="$2"
    local path="$3"

    if ! command -v az &>/dev/null; then
        log_error "Azure CLI not found. Install with: pip install azure-cli"
        return 1
    fi

    log_info "Uploading to Azure: $container/$path"

    if [ -d "$source" ]; then
        run_cmd az storage blob upload-batch \
            --source "$source" \
            --destination "$container" \
            --destination-path "$path"
    else
        run_cmd az storage blob upload \
            --file "$source" \
            --container-name "$container" \
            --name "$path/$(basename "$source")"
    fi

    log_success "Uploaded to Azure"
}

# Upload to HTTP endpoint
upload_http() {
    local source="$1"
    local endpoint="$2"

    if [ -z "$endpoint" ]; then
        log_error "HTTP endpoint not specified"
        return 1
    fi

    log_info "Uploading to HTTP: $endpoint"

    local token="${VCS_UPLOAD_TOKEN:-}"
    local headers=()

    if [ -n "$token" ]; then
        headers+=(-H "Authorization: Bearer $token")
    fi

    headers+=(-H "Content-Type: application/json")

    if [ -d "$source" ]; then
        # Upload each file
        for f in "$source"/*.json "$source"/*.xml; do
            if [ -f "$f" ]; then
                log_info "  Uploading $(basename "$f")..."
                run_cmd curl -s -X POST "${headers[@]}" \
                    --data-binary "@$f" \
                    "$endpoint/upload?filename=$(basename "$f")"
            fi
        done
    else
        run_cmd curl -s -X POST "${headers[@]}" \
            --data-binary "@$source" \
            "$endpoint/upload?filename=$(basename "$source")"
    fi

    log_success "Uploaded to HTTP endpoint"
}

# Archive locally
archive_local() {
    local source="$1"
    local archive_dir="$2"
    local path="$3"

    local dest="$archive_dir/$path"
    mkdir -p "$dest"

    log_info "Archiving to: $dest"

    if [ -d "$source" ]; then
        run_cmd cp -r "$source"/* "$dest/"
    else
        run_cmd cp "$source" "$dest/"
    fi

    # Cleanup old archives
    log_info "Cleaning up archives older than $RETAIN_DAYS days..."
    find "$archive_dir" -type f -mtime +"$RETAIN_DAYS" -delete 2>/dev/null || true
    find "$archive_dir" -type d -empty -delete 2>/dev/null || true

    log_success "Archived locally"
}

# Create summary JSON
create_summary() {
    local source="$1"

    local summary='{}'
    summary=$(echo "$summary" | jq --arg ts "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '. + {timestamp: $ts}')
    summary=$(echo "$summary" | jq --arg dest "$DESTINATION" '. + {destination: $dest}')

    # Collect file info
    local files='[]'
    for f in "$source"/*.json "$source"/*.xml "$source"/*.html; do
        if [ -f "$f" ]; then
            local name
            name=$(basename "$f")
            local size
            size=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo "0")
            files=$(echo "$files" | jq --arg n "$name" --argjson s "$size" '. + [{name: $n, size: $s}]')
        fi
    done

    summary=$(echo "$summary" | jq --argjson f "$files" '. + {files: $f}')
    echo "$summary"
}

# Main function
main() {
    parse_args "$@"

    if [ ! -d "$RESULTS_DIR" ]; then
        log_error "Results directory not found: $RESULTS_DIR"
        exit 1
    fi

    log_info "VCS Results Uploader"
    log_info "Source: $RESULTS_DIR"
    log_info "Destination: $DESTINATION"
    echo ""

    local timestamp_path
    timestamp_path=$(get_timestamp_path)

    # Prepare results
    local upload_source
    upload_source=$(prepare_results "$RESULTS_DIR" "/tmp/vcs-upload-$$")

    # Upload based on destination
    case "$DESTINATION" in
        s3)
            if [ -z "$BUCKET" ]; then
                log_error "S3 bucket not specified"
                exit 1
            fi
            upload_s3 "$upload_source" "$BUCKET" "$timestamp_path"
            ;;
        gcs)
            if [ -z "$BUCKET" ]; then
                log_error "GCS bucket not specified"
                exit 1
            fi
            upload_gcs "$upload_source" "$BUCKET" "$timestamp_path"
            ;;
        azure)
            if [ -z "$BUCKET" ]; then
                log_error "Azure container not specified"
                exit 1
            fi
            upload_azure "$upload_source" "$BUCKET" "$timestamp_path"
            ;;
        http)
            upload_http "$upload_source" "$ENDPOINT"
            ;;
        local)
            archive_local "$upload_source" "$ARCHIVE_DIR" "$timestamp_path"
            ;;
        *)
            log_error "Unknown destination type: $DESTINATION"
            exit 1
            ;;
    esac

    # Create and save summary
    local summary
    summary=$(create_summary "$RESULTS_DIR")
    echo "$summary" > "$RESULTS_DIR/upload-summary.json"

    # Cleanup
    if [ "$COMPRESS" -eq 1 ] && [ -f "/tmp/vcs-upload-$$.tar.gz" ]; then
        rm -f "/tmp/vcs-upload-$$.tar.gz"
    fi

    log_success "Upload complete"
    echo ""
    echo "Summary:"
    echo "$summary" | jq .
}

# Run main
main "$@"
