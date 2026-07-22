#!/bin/bash

# Gong API Transcript Downloader
# Usage: ./get_call_transcript.sh "Call Title Pattern" [days_back]
# Example: ./get_call_transcript.sh "Acme" 2

set -e  # Exit on error

# Configuration (use .env)
# ACCESS_KEY=
# ACCESS_KEY_SECRET=
# BASE_URL=
OUTPUT_DIR="$(dirname "$0")"

# Create auth token
AUTH_TOKEN=$(echo -n "${ACCESS_KEY}:${ACCESS_KEY_SECRET}" | base64)

# Parse arguments
SEARCH_PATTERN="${1:-}"
DAYS_BACK="${2:-2}"

if [ -z "$SEARCH_PATTERN" ]; then
    echo "Usage: $0 <call_title_pattern> [days_back]"
    echo ""
    echo "Examples:"
    echo "  $0 'Acme'"
    echo "  $0 'Acme' 7"
    echo "  $0 'Weekly Sync' 1"
    exit 1
fi

echo "=================================================="
echo "Gong API Transcript Downloader"
echo "=================================================="
echo "Searching for: $SEARCH_PATTERN"
echo "Looking back: $DAYS_BACK days"
echo ""

# Calculate date range
FROM_DATE=$(date -u -v-${DAYS_BACK}d +"%Y-%m-%dT00:00:00Z")
TO_DATE=$(date -u +"%Y-%m-%dT23:59:59Z")

echo "Date range: $FROM_DATE to $TO_DATE"
echo ""

# Step 1: Get recent calls
echo "[1/4] Fetching recent calls..."
CALLS_JSON=$(curl -s -X GET "${BASE_URL}/v2/calls?fromDateTime=${FROM_DATE}&toDateTime=${TO_DATE}" \
  -H "Authorization: Basic ${AUTH_TOKEN}" \
  -H "Content-Type: application/json")

# Check for errors
if echo "$CALLS_JSON" | grep -q '"errors"'; then
    echo "ERROR: Failed to fetch calls"
    echo "$CALLS_JSON" | jq -r '.errors[]' 2>/dev/null || echo "$CALLS_JSON"
    exit 1
fi

# Count total calls
TOTAL_CALLS=$(echo "$CALLS_JSON" | jq -r '.calls | length' 2>/dev/null || echo "0")
echo "Found $TOTAL_CALLS total calls"
echo ""

# Step 2: Search for matching call
echo "[2/4] Searching for calls matching '$SEARCH_PATTERN'..."
MATCHING_CALLS=$(echo "$CALLS_JSON" | jq -r --arg pattern "$SEARCH_PATTERN" \
  '.calls[] | select(.title | test($pattern; "i")) | "\(.id)|\(.title)|\(.started)"')

if [ -z "$MATCHING_CALLS" ]; then
    echo "ERROR: No calls found matching '$SEARCH_PATTERN'"
    echo ""
    echo "Available calls:"
    echo "$CALLS_JSON" | jq -r '.calls[] | "  - \(.title)"' 2>/dev/null
    exit 1
fi

# Show matching calls
MATCH_COUNT=$(echo "$MATCHING_CALLS" | wc -l | tr -d ' ')
echo "Found $MATCH_COUNT matching call(s):"
echo "$MATCHING_CALLS" | while IFS='|' read -r id title started; do
    echo "  - $title ($started)"
done
echo ""

# Use the first match
FIRST_MATCH=$(echo "$MATCHING_CALLS" | head -1)
CALL_ID=$(echo "$FIRST_MATCH" | cut -d'|' -f1)
CALL_TITLE=$(echo "$FIRST_MATCH" | cut -d'|' -f2)
CALL_DATE=$(echo "$FIRST_MATCH" | cut -d'|' -f3)

echo "Selected call:"
echo "  ID: $CALL_ID"
echo "  Title: $CALL_TITLE"
echo "  Date: $CALL_DATE"
echo ""

# Create safe filename
SAFE_FILENAME=$(echo "$CALL_TITLE" | tr ' ' '_' | tr -cd '[:alnum:]_-' | cut -c1-50)
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

# Step 3: Download transcript
echo "[3/4] Downloading transcript..."
TRANSCRIPT_JSON=$(curl -s -X POST "${BASE_URL}/v2/calls/transcript" \
  -H "Authorization: Basic ${AUTH_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "filter": {
      "callIds": ["'${CALL_ID}'"]
    }
  }')

# Check for errors
if echo "$TRANSCRIPT_JSON" | grep -q '"errors"'; then
    echo "ERROR: Failed to fetch transcript"
    echo "$TRANSCRIPT_JSON" | jq -r '.errors[]' 2>/dev/null || echo "$TRANSCRIPT_JSON"
    exit 1
fi

# Check if transcript exists
TRANSCRIPT_EXISTS=$(echo "$TRANSCRIPT_JSON" | jq -r '.callTranscripts | length' 2>/dev/null || echo "0")
if [ "$TRANSCRIPT_EXISTS" = "0" ]; then
    echo "ERROR: No transcript available for this call"
    echo "The call may not have been transcribed yet."
    exit 1
fi

# Save raw JSON
JSON_FILE="${OUTPUT_DIR}/${SAFE_FILENAME}_${TIMESTAMP}.json"
echo "$TRANSCRIPT_JSON" > "$JSON_FILE"
echo "Saved raw JSON to: $JSON_FILE"

# Step 4: Format to readable text
echo "[4/4] Formatting transcript to readable text..."

READABLE_FILE="${OUTPUT_DIR}/${SAFE_FILENAME}_${TIMESTAMP}.txt"

python3 <<EOF
import json
import sys

try:
    # Read the transcript JSON
    data = json.loads('''$TRANSCRIPT_JSON''')

    # Extract the transcript
    call_transcripts = data.get('callTranscripts', [])
    if not call_transcripts:
        print("ERROR: No transcript data found", file=sys.stderr)
        sys.exit(1)

    transcript = call_transcripts[0].get('transcript', [])
    call_id = call_transcripts[0].get('callId', 'Unknown')

    # Create readable output
    output = []
    output.append('=' * 80)
    output.append('${CALL_TITLE}')
    output.append('Call ID: ${CALL_ID}')
    output.append('Date: ${CALL_DATE}')
    output.append('=' * 80)
    output.append('')

    for section in transcript:
        topic = section.get('topic')
        sentences = section.get('sentences', [])

        if topic:
            output.append(f'\\n--- Topic: {topic} ---')

        for sentence in sentences:
            start_time = sentence.get('start', 0) // 1000
            minutes = start_time // 60
            seconds = start_time % 60
            text = sentence.get('text', '')
            output.append(f'[{minutes:02d}:{seconds:02d}] {text}')

    # Write to file
    with open('${READABLE_FILE}', 'w') as f:
        f.write('\\n'.join(output))

    print(f'Saved readable transcript to: ${READABLE_FILE}')
    print(f'Total lines: {len(output)}')
    print(f'Total sections: {len(transcript)}')

except Exception as e:
    print(f"ERROR: Failed to format transcript: {e}", file=sys.stderr)
    sys.exit(1)
EOF

if [ $? -ne 0 ]; then
    echo "ERROR: Failed to format transcript"
    exit 1
fi

echo ""
echo "=================================================="
echo "SUCCESS! Transcript downloaded and formatted"
echo "=================================================="
echo ""
echo "Files created:"
echo "  JSON: $JSON_FILE"
echo "  Text: $READABLE_FILE"
echo ""
echo "To view the transcript:"
echo "  cat $READABLE_FILE"
echo "  less $READABLE_FILE"
echo ""