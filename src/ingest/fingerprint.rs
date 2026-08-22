use crate::domain::Trip;
use sha2::{Digest, Sha256};

pub(crate) fn fingerprint(driver: &str, t: &Trip) -> [u8; 32] {
    let mut b = Vec::new();
    b.extend_from_slice(b"truck-ledger.trip.v1\0");
    fn i(b: &mut Vec<u8>, x: i64) {
        b.extend_from_slice(&x.to_be_bytes())
    }
    fn st(b: &mut Vec<u8>, s: &str) {
        b.extend_from_slice(&(s.len() as u64).to_be_bytes());
        b.extend_from_slice(s.as_bytes())
    }
    st(&mut b, driver);
    for x in [
        t.timestamp_day,
        t.revenue,
        t.wage,
        t.maintenance,
        t.fuel,
        t.distance,
    ] {
        i(&mut b, x)
    }
    b.push(u8::from(t.distance_on_job));
    i(&mut b, t.cargo_count);
    for x in [
        &t.cargo,
        &t.source_city,
        &t.source_company,
        &t.destination_city,
        &t.destination_company,
    ] {
        st(&mut b, x)
    }
    Sha256::digest(b).into()
}
