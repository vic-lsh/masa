#!/bin/bash
# check_fmt.sh - Script to check Rust formatting and list mal-formatted files

echo "Checking Rust formatting..."

# Find all Rust files in the project; ignore 3rd_party files
files=$(find ./apps ./libs -name "*.rs" -type f 2>/dev/null | sort)

# Track if we found any mal-formatted files
mal_formatted=false

# Check each file individually to identify which ones have issues
for file in $files; do
  if ! rustfmt --edition 2021 --check "$file" &>/dev/null; then
    echo "❌ $file is not properly formatted"
    mal_formatted=true
  fi
done

# Output summary and exit with appropriate code
if [ "$mal_formatted" = true ]; then
  echo -e "\n⚠️ Some files need formatting. Run \"cargo fmt\" locally to fix."
  exit 1
else
  echo -e "\n✅ All files are properly formatted!"
  exit 0
fi
