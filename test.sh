#!/bin/bash

# because not all tests build right now, we only test the modules we know to build successfully.

packages=(
    "tonic"
    "tonic-build"
    "tonic-masa"
    "tonic-health"
    #"tonic-reflection"
    "tonic-types"
    "tonic-web"
    "hyper"
    # tokio's tests are flaky -- reenable when we start to modify them
    # "tokio"
    # "tokio-stream"
    # "tokio-util"
    # "tokio-io-timeout"
    # "tokio-openssl"
    "tower"
    "async-task"
)

# If testing your crate requires special feature flags, set them here
declare -A package_features=(
    ["tokio"]="--features full"
    ["tokio-util"]="--features full"
    ["tower"]="--all-features"
)

# Testing by package name can be ambiguous (e.g., we have a local crate X and
# cargo also downloads another version from crates.io). In this case, we can
# be precise about our package under test by specifying its Cargo.toml path.
declare -A package_manifest_paths=(
    ["async-task"]="./3rd_party/async-task/Cargo.toml"
)

failed_packages=()

# Initialize a variable to track the overall test status
overall_status=0

# Loop through each package and run tests
for package in "${packages[@]}"; do
    echo "======== Testing $package ========"

    # Check if the package has defined feature flags
    if [ -n "${package_features[$package]}" ]; then
        features="${package_features[$package]}"
    else
        features=""
    fi

    if [ -n "${package_manifest_paths[$package]}" ]; then
        # for packages with manifest path, test directly using manifest path
        manifest_path="--manifest-path ${package_manifest_paths[$package]}"
        cargo test $manifest_path
    else
        # otherwise, test with package name and optionally with feature flags
        cargo test -p "$package" $features
    fi
    
    
    test_status=$?
    
    # If the test failed, update the overall status to nonzero
    if [ $test_status -ne 0 ]; then
        overall_status=1
        failed_packages+=("$package")
    fi
done

if [ ${#failed_packages[@]} -ne 0 ]; then
    echo "The following packages failed their tests:"
    for package in "${failed_packages[@]}"; do
        echo "- $package"
    done
else
    echo "All packages passed their tests."
fi

exit $overall_status

