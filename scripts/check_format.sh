#!/bin/bash
# check_fmt.sh - Script to check Rust formatting using cargo fmt

echo "Checking Rust formatting..."

if cargo fmt --all -- --check; then
  echo -e "\n✅ All files are properly formatted!"
else
  status=$?
  echo -e "\n⚠️ Some files need formatting. Run \"cargo fmt --all\" to fix."
  exit $status
fi
