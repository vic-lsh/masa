#!/bin/bash
# format_rust.sh - Script to format Rust files in apps/ and libs/ directories

echo "Formatting Rust files in apps/ and libs/ directories..."

# Find all Rust files in the apps/ and libs/ directories
files=$(find ./apps ./libs -name "*.rs" -type f 2>/dev/null | sort)

# Count total files to format
total_files=$(echo "$files" | wc -l)
formatted_count=0

# Check if any files were found
if [ -z "$files" ]; then
  echo "No Rust files found in apps/ and libs/ directories."
  exit 0
fi

echo "Found $total_files Rust files to format."

# Format each file individually
for file in $files; do
  # echo "Formatting: $file"
  rustfmt --edition 2021 "$file"
  formatted_count=$((formatted_count + 1))
  
  # Show progress
  if [ $((formatted_count % 10)) -eq 0 ] || [ "$formatted_count" -eq "$total_files" ]; then
    echo "Progress: $formatted_count/$total_files files formatted"
  fi
done

echo -e "\n✅ Formatting complete! $formatted_count files processed."
