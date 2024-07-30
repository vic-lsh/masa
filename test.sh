#!/bin/bash

# because not all tests build right now, we only test the modules we know to build successfully.

packages=(
    "tonic"
    "tonic-build"
    "tonic-deadline"
    "tonic-health"
    #"tonic-reflection"
    "tonic-types"
    "tonic-web"
)
failed_packages=()

# Initialize a variable to track the overall test status
overall_status=0

# Loop through each package and run tests
for package in "${packages[@]}"; do
    echo "======== Testing $package ========"
    
    cargo test -p "$package"
    
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

