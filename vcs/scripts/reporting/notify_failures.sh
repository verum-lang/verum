#!/bin/bash
# =============================================================================
# Verum Compliance Suite - Failure Notifier
# =============================================================================
#
# Sends notifications when VCS tests fail. Supports multiple channels:
# - Slack
# - Discord
# - Email (via sendmail/SMTP)
# - GitHub Issues
# - PagerDuty
# - Microsoft Teams
#
# Usage:
#   ./notify_failures.sh [options]
#
# Options:
#   --results FILE       Test results file (JSON or XML)
#   --level LEVEL        Minimum level to notify: critical, warning, info
#   --channels CHANNELS  Comma-separated: slack,discord,email,github,pagerduty,teams
#   --title TITLE        Notification title
#   --context CONTEXT    Additional context (e.g., branch name, commit SHA)
#   --url URL            Build/pipeline URL
#   --dry-run            Print notifications without sending
#   --verbose            Enable verbose output
#   -h, --help           Show help message
#
# Environment Variables:
#   SLACK_WEBHOOK_URL       Slack incoming webhook URL
#   DISCORD_WEBHOOK_URL     Discord webhook URL
#   EMAIL_RECIPIENTS        Comma-separated email addresses
#   EMAIL_FROM              Sender email address
#   SMTP_HOST               SMTP server hostname
#   SMTP_PORT               SMTP server port
#   GITHUB_TOKEN            GitHub personal access token
#   GITHUB_REPOSITORY       GitHub repository (owner/repo)
#   PAGERDUTY_ROUTING_KEY   PagerDuty integration key
#   TEAMS_WEBHOOK_URL       Microsoft Teams webhook URL
#
# Reference: VCS Spec Section 23 - CI/CD Integration
# =============================================================================

set -euo pipefail

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VCS_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Configuration
RESULTS_FILE=""
LEVEL="critical"
CHANNELS="slack"
TITLE="VCS Test Failure"
CONTEXT=""
BUILD_URL=""
DRY_RUN=0
VERBOSE=0

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Logging
log_info() { echo -e "${CYAN}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Usage
usage() {
    head -n 50 "$0" | tail -n +2 | grep -E "^#" | sed 's/^# //' | sed 's/^#//'
}

# Parse arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --results)
                RESULTS_FILE="$2"
                shift 2
                ;;
            --level)
                LEVEL="$2"
                shift 2
                ;;
            --channels)
                CHANNELS="$2"
                shift 2
                ;;
            --title)
                TITLE="$2"
                shift 2
                ;;
            --context)
                CONTEXT="$2"
                shift 2
                ;;
            --url)
                BUILD_URL="$2"
                shift 2
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

# Parse test results
parse_results() {
    local file="$1"

    if [ ! -f "$file" ]; then
        log_error "Results file not found: $file"
        exit 1
    fi

    local total=0
    local passed=0
    local failed=0
    local pass_rate=0
    local failures=""

    if [[ "$file" == *.json ]]; then
        total=$(jq -r '.summary.total // 0' "$file" 2>/dev/null || echo "0")
        passed=$(jq -r '.summary.passed // 0' "$file" 2>/dev/null || echo "0")
        failed=$(jq -r '.summary.failed // 0' "$file" 2>/dev/null || echo "0")
        pass_rate=$(jq -r '.summary.pass_percentage // 0' "$file" 2>/dev/null || echo "0")

        # Get failed test names
        failures=$(jq -r '.tests[]? | select(.status == "failed") | .name' "$file" 2>/dev/null | head -10 || echo "")
    elif [[ "$file" == *.xml ]]; then
        total=$(grep -oP 'tests="\K[0-9]+' "$file" | head -1 || echo "0")
        failed=$(grep -oP 'failures="\K[0-9]+' "$file" | head -1 || echo "0")
        passed=$((total - failed))
        if [ "$total" -gt 0 ]; then
            pass_rate=$(echo "scale=1; $passed * 100 / $total" | bc)
        fi

        # Get failed test names from XML
        failures=$(grep -oP '<testcase[^>]*name="\K[^"]+(?="[^>]*>.*?<failure)' "$file" 2>/dev/null | head -10 || echo "")
    fi

    echo "total:$total"
    echo "passed:$passed"
    echo "failed:$failed"
    echo "pass_rate:$pass_rate"
    echo "failures:$failures"
}

# Build notification message
build_message() {
    local total="$1"
    local passed="$2"
    local failed="$3"
    local pass_rate="$4"
    local failures="$5"

    local message="$TITLE\n\n"
    message+="Results:\n"
    message+="  Total:     $total\n"
    message+="  Passed:    $passed\n"
    message+="  Failed:    $failed\n"
    message+="  Pass Rate: ${pass_rate}%\n"

    if [ -n "$CONTEXT" ]; then
        message+="\nContext: $CONTEXT\n"
    fi

    if [ -n "$BUILD_URL" ]; then
        message+="\nBuild: $BUILD_URL\n"
    fi

    if [ -n "$failures" ]; then
        message+="\nFailed Tests:\n"
        echo "$failures" | while read -r test; do
            [ -n "$test" ] && message+="  - $test\n"
        done
    fi

    echo -e "$message"
}

# Send Slack notification
notify_slack() {
    local message="$1"
    local color="$2"

    local webhook="${SLACK_WEBHOOK_URL:-}"
    if [ -z "$webhook" ]; then
        log_warn "SLACK_WEBHOOK_URL not set, skipping Slack notification"
        return 0
    fi

    log_info "Sending Slack notification..."

    local payload
    payload=$(jq -n \
        --arg text "$TITLE" \
        --arg msg "$message" \
        --arg color "$color" \
        --arg url "$BUILD_URL" \
        '{
            text: $text,
            attachments: [{
                color: $color,
                text: $msg,
                footer: "VCS Notification",
                ts: (now | floor)
            }]
        }')

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] Slack payload:"
        echo "$payload" | jq .
        return 0
    fi

    curl -s -X POST \
        -H 'Content-type: application/json' \
        --data "$payload" \
        "$webhook" > /dev/null

    log_success "Slack notification sent"
}

# Send Discord notification
notify_discord() {
    local message="$1"
    local color="$2"

    local webhook="${DISCORD_WEBHOOK_URL:-}"
    if [ -z "$webhook" ]; then
        log_warn "DISCORD_WEBHOOK_URL not set, skipping Discord notification"
        return 0
    fi

    log_info "Sending Discord notification..."

    # Convert color to Discord format (decimal)
    local discord_color=15158332  # Red default
    case "$color" in
        good|green) discord_color=3066993 ;;
        warning|yellow) discord_color=15844367 ;;
        danger|red) discord_color=15158332 ;;
    esac

    local payload
    payload=$(jq -n \
        --arg title "$TITLE" \
        --arg msg "$message" \
        --argjson color "$discord_color" \
        --arg url "$BUILD_URL" \
        '{
            embeds: [{
                title: $title,
                description: $msg,
                color: $color,
                url: (if $url != "" then $url else null end),
                footer: {text: "VCS Notification"},
                timestamp: (now | strftime("%Y-%m-%dT%H:%M:%SZ"))
            }]
        }')

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] Discord payload:"
        echo "$payload" | jq .
        return 0
    fi

    curl -s -X POST \
        -H 'Content-type: application/json' \
        --data "$payload" \
        "$webhook" > /dev/null

    log_success "Discord notification sent"
}

# Send email notification
notify_email() {
    local message="$1"

    local recipients="${EMAIL_RECIPIENTS:-}"
    if [ -z "$recipients" ]; then
        log_warn "EMAIL_RECIPIENTS not set, skipping email notification"
        return 0
    fi

    log_info "Sending email notification..."

    local from="${EMAIL_FROM:-vcs@localhost}"
    local subject="$TITLE"

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] Email:"
        echo "  To: $recipients"
        echo "  From: $from"
        echo "  Subject: $subject"
        echo "  Body: $message"
        return 0
    fi

    # Try sendmail first, then mail command
    if command -v sendmail &>/dev/null; then
        echo -e "To: $recipients\nFrom: $from\nSubject: $subject\n\n$message" | sendmail -t
    elif command -v mail &>/dev/null; then
        echo -e "$message" | mail -s "$subject" "$recipients"
    else
        log_warn "No mail command found, skipping email"
        return 1
    fi

    log_success "Email notification sent"
}

# Create GitHub issue
notify_github() {
    local message="$1"

    local token="${GITHUB_TOKEN:-}"
    local repo="${GITHUB_REPOSITORY:-}"

    if [ -z "$token" ] || [ -z "$repo" ]; then
        log_warn "GITHUB_TOKEN or GITHUB_REPOSITORY not set, skipping GitHub issue"
        return 0
    fi

    log_info "Creating GitHub issue..."

    local body
    body=$(echo "$message" | sed 's/$/\\n/' | tr -d '\n')

    local payload
    payload=$(jq -n \
        --arg title "$TITLE" \
        --arg body "$body" \
        '{
            title: $title,
            body: $body,
            labels: ["ci", "test-failure", "automated"]
        }')

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] GitHub issue payload:"
        echo "$payload" | jq .
        return 0
    fi

    curl -s -X POST \
        -H "Authorization: token $token" \
        -H "Accept: application/vnd.github.v3+json" \
        --data "$payload" \
        "https://api.github.com/repos/$repo/issues" > /dev/null

    log_success "GitHub issue created"
}

# Send PagerDuty alert
notify_pagerduty() {
    local message="$1"
    local severity="$2"

    local routing_key="${PAGERDUTY_ROUTING_KEY:-}"
    if [ -z "$routing_key" ]; then
        log_warn "PAGERDUTY_ROUTING_KEY not set, skipping PagerDuty alert"
        return 0
    fi

    log_info "Sending PagerDuty alert..."

    local payload
    payload=$(jq -n \
        --arg key "$routing_key" \
        --arg summary "$TITLE" \
        --arg severity "$severity" \
        --arg source "vcs" \
        --arg url "$BUILD_URL" \
        '{
            routing_key: $key,
            event_action: "trigger",
            payload: {
                summary: $summary,
                severity: $severity,
                source: $source,
                custom_details: {
                    build_url: $url
                }
            }
        }')

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] PagerDuty payload:"
        echo "$payload" | jq .
        return 0
    fi

    curl -s -X POST \
        -H 'Content-type: application/json' \
        --data "$payload" \
        "https://events.pagerduty.com/v2/enqueue" > /dev/null

    log_success "PagerDuty alert sent"
}

# Send Microsoft Teams notification
notify_teams() {
    local message="$1"
    local color="$2"

    local webhook="${TEAMS_WEBHOOK_URL:-}"
    if [ -z "$webhook" ]; then
        log_warn "TEAMS_WEBHOOK_URL not set, skipping Teams notification"
        return 0
    fi

    log_info "Sending Teams notification..."

    # Convert color to hex
    local theme_color="FF0000"
    case "$color" in
        good|green) theme_color="00FF00" ;;
        warning|yellow) theme_color="FFFF00" ;;
        danger|red) theme_color="FF0000" ;;
    esac

    local payload
    payload=$(jq -n \
        --arg title "$TITLE" \
        --arg text "$message" \
        --arg color "$theme_color" \
        --arg url "$BUILD_URL" \
        '{
            "@type": "MessageCard",
            "@context": "http://schema.org/extensions",
            themeColor: $color,
            summary: $title,
            sections: [{
                activityTitle: $title,
                text: $text
            }],
            potentialAction: [{
                "@type": "OpenUri",
                name: "View Build",
                targets: [{
                    os: "default",
                    uri: $url
                }]
            }]
        }')

    if [ "$DRY_RUN" -eq 1 ]; then
        echo "[DRY-RUN] Teams payload:"
        echo "$payload" | jq .
        return 0
    fi

    curl -s -X POST \
        -H 'Content-type: application/json' \
        --data "$payload" \
        "$webhook" > /dev/null

    log_success "Teams notification sent"
}

# Main function
main() {
    parse_args "$@"

    if [ -z "$RESULTS_FILE" ]; then
        log_error "Results file not specified"
        usage
        exit 1
    fi

    log_info "VCS Failure Notifier"
    log_info "Results: $RESULTS_FILE"
    log_info "Channels: $CHANNELS"
    echo ""

    # Parse results
    local results
    results=$(parse_results "$RESULTS_FILE")

    local total passed failed pass_rate failures
    eval "$(echo "$results" | grep -E '^(total|passed|failed|pass_rate):' | sed 's/:/=/')"
    failures=$(echo "$results" | grep '^failures:' | cut -d: -f2-)

    # Check if we need to notify
    if [ "$failed" -eq 0 ]; then
        log_success "No failures detected - no notification needed"
        exit 0
    fi

    # Build message
    local message
    message=$(build_message "$total" "$passed" "$failed" "$pass_rate" "$failures")

    # Determine color/severity
    local color="danger"
    local severity="error"
    if [ "$failed" -lt 5 ]; then
        color="warning"
        severity="warning"
    fi

    # Send notifications to each channel
    IFS=',' read -ra CHANNEL_ARRAY <<< "$CHANNELS"
    for channel in "${CHANNEL_ARRAY[@]}"; do
        channel=$(echo "$channel" | tr -d ' ')
        case "$channel" in
            slack)
                notify_slack "$message" "$color"
                ;;
            discord)
                notify_discord "$message" "$color"
                ;;
            email)
                notify_email "$message"
                ;;
            github)
                notify_github "$message"
                ;;
            pagerduty)
                notify_pagerduty "$message" "$severity"
                ;;
            teams)
                notify_teams "$message" "$color"
                ;;
            *)
                log_warn "Unknown channel: $channel"
                ;;
        esac
    done

    log_success "Notifications complete"
}

# Run main
main "$@"
