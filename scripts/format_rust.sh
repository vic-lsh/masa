#!/bin/bash
# format_rust.sh - Format Rust code

CHECK_MODE=0

if [[ "$1" == "--check" ]]; then
    CHECK_MODE=1
fi

if [ $CHECK_MODE -eq 1 ]; then
    echo "Checking Rust formatting..."
    if cargo fmt --all -- --check; then
        echo "✅ Rust formatting is correct."
    else
        echo "⚠️ Rust formatting issues found."
        exit 1
    fi
else
    echo "Formatting Rust code..."
    cargo fmt
fi
