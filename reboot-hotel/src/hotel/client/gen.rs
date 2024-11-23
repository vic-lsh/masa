// Randomly generate requests.
//
// [NOTE] this is a port of the original request generation logic:
// https://github.com/delimitrou/DeathStarBench/blob/6ecb09706140f8730b5385c08f1386c654c3c526/hotelReservation/wrk2/scripts/hotel-reservation/mixed-workload_type_1.lua#L17

use crate::hotel::{PingRequest, ReservationRequest, SearchRequest};
use rand::Rng;

fn get_user() -> (String, String) {
    let mut rng = rand::thread_rng();
    let id = rng.gen_range(0..=500);

    let user_name = format!("Cornell_{}", id);

    // Create password by repeating id 10 times
    let pass_word = id.to_string().repeat(10);

    (user_name, pass_word)
}

pub fn get_ping_request() -> PingRequest {
    PingRequest {
        message: "ping".to_string(),
    }
}

pub fn get_search_request() -> SearchRequest {
    let mut rng = rand::thread_rng();

    // Generate random dates
    let in_date_day_num: i32 = rng.gen_range(9..=23);
    let out_date_day_num: i32 = rng.gen_range((in_date_day_num + 1)..=24);

    // Format in_date string
    let in_date = if in_date_day_num <= 9 {
        format!("2015-04-0{}", in_date_day_num)
    } else {
        format!("2015-04-{}", in_date_day_num)
    };

    // Format out_date string
    let out_date = if out_date_day_num <= 9 {
        format!("2015-04-0{}", out_date_day_num)
    } else {
        format!("2015-04-{}", out_date_day_num)
    };

    // Generate random coordinates
    let lat = 38.0235 + (rng.gen_range(0..=481) as f64 - 240.5) / 1000.0;
    let lon = -122.095 + (rng.gen_range(0..=325) as f64 - 157.0) / 1000.0;

    SearchRequest {
        lat,
        lon,
        in_date,
        out_date,
        locale: None,
    }
}

pub fn get_reservation_request() -> ReservationRequest {
    let mut rng = rand::thread_rng();

    // Generate random dates
    let in_date = rng.gen_range(9..=23);
    let out_date = in_date + rng.gen_range(1..=5);

    // Format dates
    let in_date_str = if in_date <= 9 {
        format!("2015-04-0{}", in_date)
    } else {
        format!("2015-04-{}", in_date)
    };

    let out_date_str = if out_date <= 9 {
        format!("2015-04-0{}", out_date)
    } else {
        format!("2015-04-{}", out_date)
    };

    let hotel_id = rng.gen_range(1..=80).to_string();

    let (user_id, password) = get_user();
    let cust_name = user_id.clone();
    let num_room = 1;

    ReservationRequest {
        username: user_id,
        password,
        customer: cust_name,
        hotels: vec![hotel_id],
        in_date: in_date_str,
        out_date: out_date_str,
        num_rooms: num_room,
    }
}
