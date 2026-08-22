use serde::Serialize;

#[derive(Serialize)]
pub(super) struct ErrorBody<'a> {
    pub(super) error: ErrorDto<'a>,
}
#[derive(Serialize)]
pub(super) struct ErrorDto<'a> {
    pub(super) code: &'a str,
    pub(super) message: &'a str,
}
#[derive(Serialize)]
pub(super) struct MetadataDto {
    pub(super) api_version: &'static str,
    pub(super) schema_version: u8,
}
#[derive(Serialize)]
pub(super) struct ProfilesDto {
    pub(super) profiles: Vec<ProfileDto>,
}
#[derive(Serialize)]
pub(super) struct ProfileDto {
    pub(super) scope_key: String,
    pub(super) driver_count: u64,
    pub(super) trip_count: u64,
}
#[derive(Serialize)]
pub(super) struct DriversDto {
    pub(super) drivers: Vec<DriverDto>,
}
#[derive(Serialize)]
pub(super) struct DriverStatsDto {
    pub(super) drivers: Vec<DriverStatDto>,
}
#[derive(Serialize)]
pub(super) struct DriverStatDto {
    pub(super) raw_id: String,
    pub(super) trip_count: u64,
    pub(super) loaded_trip_count: u64,
    pub(super) empty_trip_count: u64,
    pub(super) total_distance: String,
    pub(super) total_revenue: String,
    pub(super) total_wage: String,
    pub(super) total_maintenance: String,
    pub(super) total_fuel: String,
    pub(super) total_costs: String,
    pub(super) total_net: String,
}
#[derive(Serialize)]
pub(super) struct DriverDto {
    pub(super) raw_id: String,
    pub(super) adr: String,
    pub(super) long_dist: String,
    pub(super) heavy: String,
    pub(super) fragile: String,
    pub(super) urgent: String,
    pub(super) mechanical: String,
    pub(super) hometown: String,
    pub(super) current_city: String,
    pub(super) experience_points: String,
}
#[derive(Serialize)]
pub(super) struct TripsDto {
    pub(super) trips: Vec<TripDto>,
    pub(super) limit: u32,
    pub(super) offset: u32,
}
#[derive(Serialize)]
pub(super) struct TripDto {
    pub(super) driver_raw_id: String,
    pub(super) fingerprint_version: u8,
    pub(super) fingerprint: String,
    pub(super) timestamp_day: String,
    pub(super) revenue: String,
    pub(super) wage: String,
    pub(super) maintenance: String,
    pub(super) fuel: String,
    pub(super) distance: String,
    pub(super) distance_on_job: bool,
    pub(super) cargo_count: String,
    pub(super) cargo: String,
    pub(super) source_city: String,
    pub(super) source_company: String,
    pub(super) destination_city: String,
    pub(super) destination_company: String,
    pub(super) net: String,
}
#[derive(Serialize)]
pub(super) struct SummaryDto {
    pub(super) hired_driver_count: u64,
    pub(super) trip_count: u64,
    pub(super) loaded_trip_count: u64,
    pub(super) empty_trip_count: u64,
    pub(super) total_distance: String,
    pub(super) total_revenue: String,
    pub(super) total_wage: String,
    pub(super) total_maintenance: String,
    pub(super) total_fuel: String,
    pub(super) total_net: String,
}
