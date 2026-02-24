// Randomly generate requests for socialnet load generation.

use rand::{rngs::StdRng, Rng};

// Import ComposePostRequest from the socialnet crate.
use socialnet::compose_post::ComposePostRequest;
use socialnet::register_user::RegisterUserRequest;

pub fn get_compose_post_request(rng: &mut StdRng) -> ComposePostRequest {
    // Generate random user ID (similar to hotel's user range)
    let user_id = rng.gen_range(1..=1000);
    let username = format!("user_{}", user_id);

    // Generate random text content
    let text_options = vec![
        "Hello World",
        "This is a test post",
        "Check out this amazing content!",
        "Just sharing my thoughts",
        "Great day today!",
        "Working on something exciting",
        "Sharing an update",
        "Random post content",
    ];
    let text = text_options[rng.gen_range(0..text_options.len())].to_string();

    // Randomly decide if we have media
    let has_media = rng.gen_bool(0.5);
    let (media_ids, media_types) = if has_media {
        let num_media = rng.gen_range(1..=3);
        let ids: Vec<i64> = (0..num_media).map(|_| rng.gen_range(1..=100)).collect();
        let types = vec!["image".to_string(), "text".to_string(), "video".to_string()];
        let selected_types: Vec<String> = (0..num_media)
            .map(|i| types[i % types.len()].clone())
            .collect();
        (ids, selected_types)
    } else {
        (vec![], vec![])
    };

    // Random post type (0=POST, 1=REPOST, 2=REPLY, 3=DM)
    let post_type = rng.gen_range(0..4);

    ComposePostRequest {
        req_id: rng.gen_range(1..=1000000),
        username,
        user_id,
        text,
        media_ids,
        media_types,
        post_type,
        carrier: std::collections::HashMap::new(),
    }
}

pub fn get_register_user_request(rng: &mut StdRng) -> RegisterUserRequest {
    let req_id = rng.gen_range(1..=1_000_000_000);
    let first_name_options = [
        "Alice", "Bob", "Carol", "David", "Eve", "Frank", "Grace", "Heidi",
    ];
    let last_name_options = [
        "Smith", "Johnson", "Lee", "Patel", "Kim", "Garcia", "Brown", "Davis",
    ];

    let first_name = first_name_options[rng.gen_range(0..first_name_options.len())].to_string();
    let last_name = last_name_options[rng.gen_range(0..last_name_options.len())].to_string();
    let username = format!("bench_user_{}_{}", req_id, rng.gen_range(1..=1_000_000));
    let password = format!("pw_{}_{}", req_id, rng.gen_range(1..=1_000_000));

    RegisterUserRequest {
        req_id,
        first_name,
        last_name,
        username,
        password,
        carrier: std::collections::HashMap::new(),
    }
}
