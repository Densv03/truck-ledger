use super::{
    models::*,
    server::{ApiFailure, error_response, json_response, static_response},
};
use crate::db::{self, ReadError};
use std::path::Path;
use tiny_http::{Header, Method, Response};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const DASHBOARD_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/index.html"));
const DASHBOARD_JAVASCRIPT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/app.js"));
const DASHBOARD_STYLESHEET: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/styles.css"));

fn count(value: i64) -> Result<u64, ApiFailure> {
    u64::try_from(value).map_err(|_| ApiFailure::Internal)
}
fn database_failure(error: ReadError) -> ApiFailure {
    match error {
        ReadError::ProfileNotFound => ApiFailure::ProfileNotFound,
        ReadError::DatabaseUnavailable => ApiFailure::DatabaseUnavailable,
        ReadError::Internal => ApiFailure::Internal,
    }
}
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn profiles(database: &Path) -> Result<ProfilesDto, ApiFailure> {
    Ok(ProfilesDto {
        profiles: db::read_profiles(database)
            .map_err(database_failure)?
            .into_iter()
            .map(|profile| {
                Ok(ProfileDto {
                    scope_key: profile.scope_key,
                    driver_count: count(profile.driver_count)?,
                    trip_count: count(profile.trip_count)?,
                })
            })
            .collect::<Result<_, ApiFailure>>()?,
    })
}
fn drivers(database: &Path, scope: &str) -> Result<DriversDto, ApiFailure> {
    Ok(DriversDto {
        drivers: db::read_drivers(database, scope)
            .map_err(database_failure)?
            .into_iter()
            .map(|driver| DriverDto {
                raw_id: driver.raw_id,
                adr: driver.adr.to_string(),
                long_dist: driver.long_dist.to_string(),
                heavy: driver.heavy.to_string(),
                fragile: driver.fragile.to_string(),
                urgent: driver.urgent.to_string(),
                mechanical: driver.mechanical.to_string(),
                hometown: driver.hometown,
                current_city: driver.current_city,
                experience_points: driver.experience_points.to_string(),
            })
            .collect(),
    })
}

#[derive(Default)]
struct Pagination {
    limit: u32,
    offset: u32,
    driver: Option<String>,
}
fn percent_decode(input: &str) -> Result<String, ApiFailure> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ApiFailure::InvalidQuery);
            }
            let h = |b: u8| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                b'A'..=b'F' => Ok(b - b'A' + 10),
                _ => Err(ApiFailure::InvalidQuery),
            };
            output.push(h(bytes[index + 1])? * 16 + h(bytes[index + 2])?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| ApiFailure::InvalidQuery)
}
fn pagination(query: Option<&str>) -> Result<Pagination, ApiFailure> {
    let mut result = Pagination {
        limit: DEFAULT_LIMIT,
        ..Default::default()
    };
    let Some(query) = query else {
        return Ok(result);
    };
    if query.is_empty() {
        return Err(ApiFailure::InvalidQuery);
    }
    let mut seen = std::collections::BTreeSet::new();
    for item in query.split('&') {
        let (key, value) = item.split_once('=').ok_or(ApiFailure::InvalidQuery)?;
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if value.is_empty() || !seen.insert(key.clone()) {
            return Err(ApiFailure::InvalidQuery);
        }
        match key.as_str() {
            "limit" => {
                let parsed = value.parse::<u32>().map_err(|_| ApiFailure::InvalidQuery)?;
                if !(1..=MAX_LIMIT).contains(&parsed) {
                    return Err(ApiFailure::InvalidQuery);
                }
                result.limit = parsed;
            }
            "offset" => {
                result.offset = value.parse::<u32>().map_err(|_| ApiFailure::InvalidQuery)?
            }
            "driver" => result.driver = Some(value),
            _ => return Err(ApiFailure::InvalidQuery),
        }
    }
    Ok(result)
}
fn trips(database: &Path, scope: &str, page: Pagination) -> Result<TripsDto, ApiFailure> {
    let trips = db::read_trips_page(
        database,
        scope,
        page.driver.as_deref(),
        page.limit,
        page.offset,
    )
    .map_err(database_failure)?
    .into_iter()
    .map(|trip| {
        Ok(TripDto {
            driver_raw_id: trip.driver_raw_id,
            fingerprint_version: u8::try_from(trip.fingerprint_version)
                .map_err(|_| ApiFailure::Internal)?,
            fingerprint: hex(&trip.fingerprint),
            timestamp_day: trip.timestamp_day.to_string(),
            revenue: trip.revenue.to_string(),
            wage: trip.wage.to_string(),
            maintenance: trip.maintenance.to_string(),
            fuel: trip.fuel.to_string(),
            distance: trip.distance.to_string(),
            distance_on_job: trip.distance_on_job,
            cargo_count: trip.cargo_count.to_string(),
            cargo: trip.cargo,
            source_city: trip.source_city,
            source_company: trip.source_company,
            destination_city: trip.destination_city,
            destination_company: trip.destination_company,
            net: trip.net.to_string(),
        })
    })
    .collect::<Result<_, ApiFailure>>()?;
    Ok(TripsDto {
        trips,
        limit: page.limit,
        offset: page.offset,
    })
}
fn summary(database: &Path, scope: &str) -> Result<SummaryDto, ApiFailure> {
    let summary = db::read_profile_summary(database, scope).map_err(database_failure)?;
    Ok(SummaryDto {
        hired_driver_count: count(summary.hired_driver_count)?,
        trip_count: count(summary.trip_count)?,
        loaded_trip_count: count(summary.loaded_trip_count)?,
        empty_trip_count: count(summary.empty_trip_count)?,
        total_distance: summary.total_distance.to_string(),
        total_revenue: summary.total_revenue.to_string(),
        total_wage: summary.total_wage.to_string(),
        total_maintenance: summary.total_maintenance.to_string(),
        total_fuel: summary.total_fuel.to_string(),
        total_net: summary.total_net.to_string(),
    })
}
fn driver_stats(database: &Path, scope: &str) -> Result<DriverStatsDto, ApiFailure> {
    let drivers = db::read_driver_stats(database, scope)
        .map_err(database_failure)?
        .drivers
        .into_iter()
        .map(|stat| {
            let total_costs = stat
                .total_wage
                .checked_add(stat.total_maintenance)
                .and_then(|value| value.checked_add(stat.total_fuel))
                .ok_or(ApiFailure::Internal)?;
            Ok(DriverStatDto {
                raw_id: stat.raw_id,
                trip_count: count(stat.trip_count)?,
                loaded_trip_count: count(stat.loaded_trip_count)?,
                empty_trip_count: count(stat.empty_trip_count)?,
                total_distance: stat.total_distance.to_string(),
                total_revenue: stat.total_revenue.to_string(),
                total_wage: stat.total_wage.to_string(),
                total_maintenance: stat.total_maintenance.to_string(),
                total_fuel: stat.total_fuel.to_string(),
                total_costs: total_costs.to_string(),
                total_net: stat.total_net.to_string(),
            })
        })
        .collect::<Result<_, ApiFailure>>()?;
    Ok(DriverStatsDto { drivers })
}

fn split_url(url: &str) -> (&str, Option<&str>) {
    url.split_once('?')
        .map_or((url, None), |(path, query)| (path, Some(query)))
}
fn decode_scope(segment: &str) -> Result<String, ApiFailure> {
    if segment.is_empty() {
        Err(ApiFailure::InvalidPath)
    } else {
        percent_decode(segment).map_err(|_| ApiFailure::InvalidPath)
    }
}
fn known_route(path: &str) -> bool {
    let segments: Vec<_> = path.split('/').collect();
    matches!(
        segments.as_slice(),
        ["", "api", "v1"]
            | ["", "api", "v1", "profiles"]
            | ["", "api", "v1", "profiles", _, "drivers"]
            | ["", "api", "v1", "profiles", _, "driver-stats"]
            | ["", "api", "v1", "profiles", _, "trips"]
            | ["", "api", "v1", "profiles", _, "summary"]
    )
}
fn api_route(database: &Path, method: &Method, url: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let (path, query) = split_url(url);
    if !known_route(path) {
        return error_response(ApiFailure::NotFound);
    }
    if *method != Method::Get {
        return error_response(ApiFailure::MethodNotAllowed)
            .with_header(Header::from_bytes("Allow", "GET").unwrap());
    }
    let result = (|| {
        let segments: Vec<_> = path.split('/').collect();
        match segments.as_slice() {
            ["", "api", "v1"] if query.is_none() => Ok(json_response(
                200,
                &MetadataDto {
                    api_version: "v1",
                    schema_version: 2,
                },
            )),
            ["", "api", "v1", "profiles"] if query.is_none() => {
                Ok(json_response(200, &profiles(database)?))
            }
            ["", "api", "v1", "profiles", scope, "drivers"] if query.is_none() => Ok(
                json_response(200, &drivers(database, &decode_scope(scope)?)?),
            ),
            ["", "api", "v1", "profiles", scope, "driver-stats"] if query.is_none() => Ok(
                json_response(200, &driver_stats(database, &decode_scope(scope)?)?),
            ),
            ["", "api", "v1", "profiles", scope, "trips"] => Ok(json_response(
                200,
                &trips(database, &decode_scope(scope)?, pagination(query)?)?,
            )),
            ["", "api", "v1", "profiles", scope, "summary"] if query.is_none() => Ok(
                json_response(200, &summary(database, &decode_scope(scope)?)?),
            ),
            _ => Err(ApiFailure::InvalidQuery),
        }
    })();
    match result {
        Ok(response) => response,
        Err(error) => error_response(error),
    }
}

pub(crate) fn route(
    database: &Path,
    method: &Method,
    url: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let (path, _) = split_url(url);
    if path.starts_with("/api/") {
        return api_route(database, method, url);
    }
    if *method != Method::Get {
        return error_response(ApiFailure::MethodNotAllowed)
            .with_header(Header::from_bytes("Allow", "GET").unwrap());
    }
    match path {
        "/" => static_response("text/html; charset=utf-8", DASHBOARD_HTML),
        "/app.js" => static_response("text/javascript; charset=utf-8", DASHBOARD_JAVASCRIPT),
        "/styles.css" => static_response("text/css; charset=utf-8", DASHBOARD_STYLESHEET),
        _ => error_response(ApiFailure::NotFound),
    }
}
