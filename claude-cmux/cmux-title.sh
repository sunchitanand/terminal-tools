#!/bin/bash
# Read hook event from stdin
EVENT=$(cat 2>/dev/null || echo '{}')
PROMPT=$(echo "$EVENT" | jq -r '.prompt // .message // empty' 2>/dev/null | head -c 50)
if [ -n "$PROMPT" ]; then
    cmux rename-workspace "$PROMPT" 2>/dev/null
fi
cmux clear-notifications 2>/dev/null
cmux set-status agent 'Working' --icon brain --color '#F97316' 2>/dev/null
true
