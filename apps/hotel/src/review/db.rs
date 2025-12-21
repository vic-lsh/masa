use mongodb::{bson::doc, options::ClientOptions, Client};
use serde::{Deserialize, Serialize};

use crate::server::hotel_tonic;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Image {
    url: String,
    default: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Review {
    #[serde(rename = "reviewId")]
    review_id: String,
    #[serde(rename = "hotelId")]
    hotel_id: String,
    name: String,
    rating: f32,
    description: String,
    images: Vec<Image>,
}

impl From<Image> for hotel_tonic::review::Image {
    fn from(i: Image) -> Self {
        Self {
            url: i.url,
            default: i.default,
        }
    }
}

impl From<Review> for hotel_tonic::review::ReviewComm {
    fn from(r: Review) -> Self {
        Self {
            review_id: r.review_id,
            hotel_id: r.hotel_id,
            name: r.name,
            rating: r.rating,
            description: r.description,
            images: r.images.into_iter().map(|img| img.into()).collect(),
        }
    }
}

fn generate_test_data() -> Vec<Review> {
    log::info!("Generating test review data...");

    let mut reviews: Vec<Review> = vec![
        Review {
            review_id: "1".to_string(),
            hotel_id: "1".to_string(),
            name: "John Smith".to_string(),
            rating: 4.5,
            description: "Great hotel with excellent service!".to_string(),
            images: vec![
                Image {
                    url: "https://example.com/img1.jpg".to_string(),
                    default: true,
                },
                Image {
                    url: "https://example.com/img2.jpg".to_string(),
                    default: false,
                },
            ],
        },
        Review {
            review_id: "2".to_string(),
            hotel_id: "1".to_string(),
            name: "Alice Johnson".to_string(),
            rating: 5.0,
            description: "Wonderful stay, would come back again!".to_string(),
            images: vec![Image {
                url: "https://example.com/img3.jpg".to_string(),
                default: true,
            }],
        },
        Review {
            review_id: "3".to_string(),
            hotel_id: "2".to_string(),
            name: "Bob Miller".to_string(),
            rating: 3.8,
            description: "Nice location but rooms need updating.".to_string(),
            images: vec![],
        },
    ];

    // Generate additional test reviews for various hotels
    for i in 3..=20 {
        let hotel_id = ((i % 10) + 1).to_string();
        let review_id = (i + 3).to_string();

        let (name, rating, description) = match i % 5 {
            0 => (
                "Michael Brown".to_string(),
                4.2,
                "Clean rooms and friendly staff.".to_string(),
            ),
            1 => (
                "Sarah Davis".to_string(),
                3.5,
                "Average experience, but good value.".to_string(),
            ),
            2 => (
                "Emily Wilson".to_string(),
                4.8,
                "Luxurious accommodations with amazing amenities!".to_string(),
            ),
            3 => (
                "David Garcia".to_string(),
                2.5,
                "Disappointing experience. Not as advertised.".to_string(),
            ),
            _ => (
                "Jennifer Lee".to_string(),
                4.0,
                "Great location near all attractions.".to_string(),
            ),
        };

        let has_images = i % 3 != 0;
        let images = if has_images {
            vec![
                Image {
                    url: format!("https://example.com/hotel{}/img{}.jpg", hotel_id, i),
                    default: true,
                },
                Image {
                    url: format!("https://example.com/hotel{}/img{}_2.jpg", hotel_id, i),
                    default: false,
                },
            ]
        } else {
            vec![]
        };

        reviews.push(Review {
            review_id,
            hotel_id,
            name,
            rating,
            description,
            images,
        });
    }

    reviews
}

pub async fn initialize_database(url: &str) -> Result<Client, Box<dyn std::error::Error>> {
    log::info!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(&url).await?;
    let client = Client::with_options(client_options)?;
    log::info!("Successfully connected to MongoDB");

    let collection = client.database("review-db").collection::<Review>("reviews");

    // empty collection first
    collection.delete_many(doc! {}, None).await?;

    let reviews = generate_test_data();
    collection.insert_many(&reviews, None).await?;
    log::info!("Successfully inserted test data into review DB");

    // Create index on hotelId for faster queries
    // collection
    //     .create_index(
    //         doc! { "hotelId": 1 },
    //         None,
    //     )
    //     .await?;

    // log::info!("Created index on hotelId field");

    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::hotel_tonic;

    #[test]
    fn reviews_round_trip_through_json() {
        let reviews = generate_test_data();
        let serialized = serde_json::to_vec(&reviews).expect("serialize reviews");
        let decoded: Vec<Review> =
            serde_json::from_slice(&serialized).expect("deserialize cached reviews");

        assert_eq!(decoded, reviews);
    }

    #[test]
    fn proto_conversion_maps_all_fields() {
        let review = generate_test_data().into_iter().next().expect("test review");
        let proto: hotel_tonic::review::ReviewComm = review.clone().into();

        assert_eq!(proto.review_id, review.review_id);
        assert_eq!(proto.hotel_id, review.hotel_id);
        assert_eq!(proto.name, review.name);
        assert!((proto.rating - review.rating).abs() < f32::EPSILON);
        assert_eq!(proto.description, review.description);
        assert_eq!(proto.images.len(), review.images.len());
    }
}
