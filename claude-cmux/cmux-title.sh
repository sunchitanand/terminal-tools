#!/bin/bash
# Read hook event from stdin (discard)
cat 2>/dev/null > /dev/null
cmux clear-notifications 2>/dev/null
cmux set-status agent 'Working' --icon brain --color '#F97316' 2>/dev/null
true
