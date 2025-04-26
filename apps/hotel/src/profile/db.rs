use mongodb::{options::ClientOptions, Client};
use serde::{Deserialize, Serialize};

use crate::server::hotel_tonic;

#[derive(Debug, Serialize, Deserialize)]
pub struct Hotel {
    #[serde(rename = "id")]
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(rename = "phoneNumber")]
    pub(crate) phone_number: String,
    pub(crate) description: String,
    pub(crate) address: Address,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Address {
    #[serde(rename = "streetNumber")]
    pub(crate) street_number: String,
    #[serde(rename = "streetName")]
    pub(crate) street_name: String,
    pub(crate) city: String,
    pub(crate) state: String,
    pub(crate) country: String,
    #[serde(rename = "postalCode")]
    pub(crate) postal_code: String,
    pub(crate) lat: f32,
    pub(crate) lon: f32,
}

impl From<Address> for hotel_tonic::profile::Address {
    fn from(a: Address) -> Self {
        Self {
            street_number: a.street_number,
            street_name: a.street_name,
            city: a.city,
            state: a.state,
            country: a.country,
            postal_code: a.postal_code,
            lat: a.lat,
            lon: a.lon,
        }
    }
}

impl From<Hotel> for hotel_tonic::profile::Hotel {
    fn from(h: Hotel) -> Self {
        Self {
            id: h.id,
            name: h.name,
            phone_number: h.phone_number,
            description: h.description,
            address: Some(h.address.into()),
            images: vec![],
        }
    }
}

fn generate_test_data() -> Vec<Hotel> {
    let mut new_profiles = vec![
        Hotel {
            id: "1".to_string(),
            name: "Clift Hotel".to_string(),
            phone_number: "(415) 775-4700".to_string(),
            description: "A 6-minute walk from Union Square and 4 minutes from a Muni Metro station, this luxury hotel designed by Philippe Starck features an artsy furniture collection in the lobby, including work by Salvador Dali.".to_string(),
            address: Address {
                street_number: "495".to_string(),
                street_name: "Geary St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94102".to_string(),
                lat: 37.7867,
                lon: -122.4112,
            },
        },
        Hotel {
            id: "2".to_string(),
            name: "W San Francisco".to_string(),
            phone_number: "(415) 777-5300".to_string(),
            description: "Less than a block from the Yerba Buena Center for the Arts, this trendy hotel is a 12-minute walk from Union Square.".to_string(),
            address: Address {
                street_number: "181".to_string(),
                street_name: "3rd St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94103".to_string(),
                lat: 37.7854,
                lon: -122.4005,
            },
        },
        Hotel {
            id: "3".to_string(),
            name: "Hotel Zetta".to_string(),
            phone_number: "(415) 543-8555".to_string(),
            description: "A 3-minute walk from the Powell Street cable-car turnaround and BART rail station, this hip hotel 9 minutes from Union Square combines high-tech lodging with artsy touches.".to_string(),
            address: Address {
                street_number: "55".to_string(),
                street_name: "5th St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94103".to_string(),
                lat: 37.7834,
                lon: -122.4071,
            },
        },
        Hotel {
            id: "4".to_string(),
            name: "Hotel Vitale".to_string(),
            phone_number: "(415) 278-3700".to_string(),
            description: "This waterfront hotel with Bay Bridge views is 3 blocks from the Financial District and a 4-minute walk from the Ferry Building.".to_string(),
            address: Address {
                street_number: "8".to_string(),
                street_name: "Mission St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94105".to_string(),
                lat: 37.7936,
                lon: -122.3930,
            },
        },
        Hotel {
            id: "5".to_string(),
            name: "Phoenix Hotel".to_string(),
            phone_number: "(415) 776-1380".to_string(),
            description: "Located in the Tenderloin neighborhood, a 10-minute walk from a BART rail station, this retro motor lodge has hosted many rock musicians and other celebrities since the 1950s. It's a 4-minute walk from the historic Great American Music Hall nightclub.".to_string(),
            address: Address {
                street_number: "601".to_string(),
                street_name: "Eddy St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94109".to_string(),
                lat: 37.7831,
                lon: -122.4181,
            },
        },
        Hotel {
            id: "6".to_string(),
            name: "St. Regis San Francisco".to_string(),
            phone_number: "(415) 284-4000".to_string(),
            description: "St. Regis Museum Tower is a 42-story, 484 ft skyscraper in the South of Market district of San Francisco, California, adjacent to Yerba Buena Gardens, Moscone Center, PacBell Building and the San Francisco Museum of Modern Art.".to_string(),
            address: Address {
                street_number: "125".to_string(),
                street_name: "3rd St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94109".to_string(),
                lat: 37.7863,
                lon: -122.4015,
            },
        },
    ];

    // Generate additional hotels
    for i in 7..=80 {
        let hotel_id = i.to_string();
        let phone_number = format!("(415) 284-40{}", hotel_id);

        let lat = 37.7835 + (i as f32) / 500.0 * 3.0;
        let lon = -122.41 + (i as f32) / 500.0 * 4.0;

        new_profiles.push(Hotel {
            id: hotel_id,
            name: "St. Regis San Francisco".to_string(),
            phone_number,
            description: "St. Regis Museum Tower is a 42-story, 484 ft skyscraper in the South of Market district of San Francisco, California, adjacent to Yerba Buena Gardens, Moscone Center, PacBell Building and the San Francisco Museum of Modern Art.".to_string(),
            address: Address {
                street_number: "125".to_string(),
                street_name: "3rd St".to_string(),
                city: "San Francisco".to_string(),
                state: "CA".to_string(),
                country: "United States".to_string(),
                postal_code: "94109".to_string(),
                lat,
                lon,
            },
        });
    }

    new_profiles
}

pub async fn initialize_database(url: &str) -> Result<Client, mongodb::error::Error> {
    log::info!("Generating test data...");

    log::info!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(&url).await?;
    let client = Client::with_options(client_options)?;
    log::info!("Successfully connected to MongoDB");

    let new_profiles = generate_test_data();
    let collection = client.database("profile-db").collection::<Hotel>("hotels");
    collection.insert_many(&new_profiles, None).await?;
    log::info!("Successfully inserted test data into profile DB");

    Ok(client)
}
