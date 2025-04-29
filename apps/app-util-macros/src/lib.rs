// File: latency_tracker/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, parse_macro_input, Expr, Token};

// Define the structure for parsing macro arguments
struct TrackLatencyArgs {
    tracker: Expr,
    code_block: Expr,
}

// Implement the Parse trait for our arguments structure
impl Parse for TrackLatencyArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let tracker = input.parse()?;
        input.parse::<Token![,]>()?;
        let code_block = input.parse()?;

        Ok(TrackLatencyArgs {
            tracker,
            code_block,
        })
    }
}

/// Tracks the execution time of a code block and adds it to the provided latency tracker.
///
/// # Example
/// ```
/// use app_utils::latency::new_latency_tracker;
/// use app_util_macros::track_latency;
///
/// let (mut tracker, _consumer) = new_latency_tracker("code-block-name");
/// track_latency!(tracker, {
///     // Code to track
///     std::thread::sleep(std::time::Duration::from_millis(100));
/// });
/// ```
#[proc_macro]
pub fn track_latency(input: TokenStream) -> TokenStream {
    // Parse the input tokens
    let TrackLatencyArgs {
        tracker,
        code_block,
    } = parse_macro_input!(input as TrackLatencyArgs);

    // Generate the output code
    let expanded = quote! {
        {
            let start = std::time::Instant::now();
            let result = { #code_block };
            let duration = start.elapsed();
            #tracker.track(duration.as_micros().try_into().unwrap());
            result
        }
    };

    // Return the generated code
    expanded.into()
}
