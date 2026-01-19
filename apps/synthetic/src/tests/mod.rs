// Test runner for synthetic application

#[cfg(test)]
mod test_runner {
    /// Comprehensive test suite runner
    pub async fn run_all_tests() -> Result<(), Box<dyn std::error::Error>> {
        println!("Running comprehensive test suite for synthetic application...");

        // Run configuration compatibility tests
        println!("\n=== Configuration Compatibility Tests ===");
        config::compatibility_tests::tests::run_config_tests();

        // Run connection management tests
        println!("\n=== Connection Management Tests ===");
        // Connection tests require async runtime
        connection::tests::run_connection_tests().await;

        // Run error handling tests
        println!("\n=== Error Handling Tests ===");
        error::tests::run_error_tests();

        // Run performance tests
        println!("\n=== Performance Tests ===");
        performance::tests::PerformanceTests::run_performance_suite().await?;

        println!("\n=== All Tests Completed Successfully ===");
        Ok(())
    }
}

#[cfg(test)]
pub use test_runner::run_all_tests;
