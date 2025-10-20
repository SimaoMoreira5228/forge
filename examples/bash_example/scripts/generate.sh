#!/bin/bash
set -e

# Parse arguments
COUNT=5
while [[ $# -gt 0 ]]; do
    case $1 in
        --count)
            COUNT="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Create output directory
mkdir -p generated

# Generate random data
echo "Generating $COUNT lines of data..."
for i in $(seq 1 $COUNT); do
    echo "Line $i: $(date +%s%N | sha256sum | head -c 16)"
done > generated/data.txt

echo "Generated generated/data.txt with $COUNT lines"
