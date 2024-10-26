use crate::hotel::SearchRequest;

// Randomly generate a new search request.
//
// [NOTE] this is a port of the original search request generation logic:
// https://github.com/delimitrou/DeathStarBench/blob/6ecb09706140f8730b5385c08f1386c654c3c526/hotelReservation/wrk2/scripts/hotel-reservation/mixed-workload_type_1.lua#L17
pub fn gen_search_request() -> SearchRequest {
    use rand::Rng;

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
