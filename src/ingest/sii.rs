use crate::domain::{Driver, Error, Trip};
use std::collections::BTreeMap;

pub(crate) fn decode_input(bytes: &[u8]) -> Result<String, Error> {
    if bytes.starts_with(b"BSII") {
        return Err(Error::Input(
            "unsupported input format BSII; Phase 1 accepts ScsC or SiiNunit".into(),
        ));
    }
    let output = if bytes.starts_with(b"ScsC") {
        sii_decode::file_type::decode_until_siin(bytes)
            .map_err(|e| Error::Decode(format!("ScsC decode failed: {e}")))?
    } else if bytes.starts_with(b"SiiNunit") {
        bytes.to_vec()
    } else {
        return Err(Error::Input(
            "unsupported input format; expected ScsC or SiiNunit header".into(),
        ));
    };
    String::from_utf8(output).map_err(|e| Error::Parse(format!("decoded SII is not UTF-8: {e}")))
}

#[derive(Default)]
struct Block {
    kind: String,
    id: String,
    fields: BTreeMap<String, String>,
}
fn unquote(v: &str) -> Result<String, Error> {
    let v = v.trim();
    if !v.starts_with('"') {
        return Ok(v.to_string());
    }
    if !v.ends_with('"') || v.len() < 2 {
        return Err(Error::Parse("malformed quoted string".into()));
    }
    let mut out = String::new();
    let mut esc = false;
    for c in v[1..v.len() - 1].chars() {
        if esc {
            match c {
                '"' | '\\' => out.push(c),
                _ => return Err(Error::Parse("unsupported string escape".into())),
            };
            esc = false
        } else if c == '\\' {
            esc = true
        } else {
            out.push(c)
        }
    }
    if esc {
        return Err(Error::Parse("unterminated string escape".into()));
    }
    Ok(out)
}
fn parse_blocks(s: &str) -> Result<Vec<Block>, Error> {
    if !s.starts_with("SiiNunit") {
        return Err(Error::Parse("missing SiiNunit header".into()));
    }
    let mut blocks = Vec::new();
    let mut current: Option<Block> = None;
    for (n, line) in s.lines().enumerate().skip(1) {
        let t = line.trim();
        if t.is_empty() || t == "{" {
            continue;
        }
        if t == "}" {
            if let Some(b) = current.take() {
                blocks.push(b)
            }
            continue;
        }
        if current.is_none() {
            let (head, _) = t
                .split_once('{')
                .ok_or_else(|| Error::Parse(format!("line {}: malformed block", n + 1)))?;
            let (kind, id) = head
                .split_once(':')
                .ok_or_else(|| Error::Parse(format!("line {}: malformed block", n + 1)))?;
            current = Some(Block {
                kind: kind.trim().into(),
                id: id.trim().into(),
                ..Default::default()
            });
            continue;
        }
        let (k, v) = t
            .split_once(':')
            .ok_or_else(|| Error::Parse(format!("line {}: malformed field", n + 1)))?;
        current
            .as_mut()
            .unwrap()
            .fields
            .insert(k.trim().into(), v.trim().into());
    }
    if current.is_some() {
        return Err(Error::Parse("unterminated block".into()));
    }
    Ok(blocks)
}
fn reqs(b: &Block, k: &str, ctx: &str) -> Result<String, Error> {
    b.fields
        .get(k)
        .ok_or_else(|| Error::Parse(format!("{ctx} {} {} missing field {k}", b.kind, b.id)))
        .and_then(|v| unquote(v))
}
fn reqi(b: &Block, k: &str, ctx: &str) -> Result<i64, Error> {
    reqs(b, k, ctx)?
        .parse()
        .map_err(|_| Error::Parse(format!("{ctx} {} {} field {k} must be i64", b.kind, b.id)))
}
fn reqb(b: &Block, k: &str, ctx: &str) -> Result<bool, Error> {
    match reqs(b, k, ctx)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Error::Parse(format!(
            "{ctx} {} {} field {k} must be boolean",
            b.kind, b.id
        ))),
    }
}
pub(crate) fn extract(s: &str) -> Result<Vec<Driver>, Error> {
    let blocks = parse_blocks(s)?;
    let mut map = BTreeMap::new();
    for b in blocks {
        map.insert(b.id.clone(), b);
    }
    let mut result = Vec::new();
    for b in map.values().filter(|b| b.kind == "driver_ai") {
        let hometown = reqs(b, "hometown", "driver")?;
        if hometown.is_empty() {
            continue;
        }
        let raw_id = b.id.clone();
        let log = reqs(b, "profit_log", &format!("driver {raw_id}"))?;
        let logb = map
            .get(&log)
            .filter(|x| x.kind == "profit_log")
            .ok_or_else(|| {
                Error::Parse(format!("driver {raw_id}: profit_log target {log} missing"))
            })?;
        let mut refs: Vec<_> = logb
            .fields
            .iter()
            .filter_map(|(k, v)| {
                k.strip_prefix("stats_data[")
                    .and_then(|x| x.strip_suffix(']'))
                    .and_then(|x| x.parse::<usize>().ok())
                    .map(|i| (i, v))
            })
            .collect();
        refs.sort_by_key(|x| x.0);
        let mut trips = Vec::new();
        for (_, r) in refs {
            let id = unquote(r)?;
            let t = map
                .get(&id)
                .filter(|x| x.kind == "profit_log_entry")
                .ok_or_else(|| {
                    Error::Parse(format!("driver {raw_id}: trip target {id} missing"))
                })?;
            let trip = Trip {
                timestamp_day: reqi(t, "timestamp_day", &raw_id)?,
                revenue: reqi(t, "revenue", &raw_id)?,
                wage: reqi(t, "wage", &raw_id)?,
                maintenance: reqi(t, "maintenance", &raw_id)?,
                fuel: reqi(t, "fuel", &raw_id)?,
                distance: reqi(t, "distance", &raw_id)?,
                distance_on_job: reqb(t, "distance_on_job", &raw_id)?,
                cargo_count: reqi(t, "cargo_count", &raw_id)?,
                cargo: reqs(t, "cargo", &raw_id)?,
                source_city: reqs(t, "source_city", &raw_id)?,
                source_company: reqs(t, "source_company", &raw_id)?,
                destination_city: reqs(t, "destination_city", &raw_id)?,
                destination_company: reqs(t, "destination_company", &raw_id)?,
            };
            trip.net()?;
            trips.push(trip)
        }
        result.push(Driver {
            raw_id,
            adr: reqi(b, "adr", "driver")?,
            long_dist: reqi(b, "long_dist", "driver")?,
            heavy: reqi(b, "heavy", "driver")?,
            fragile: reqi(b, "fragile", "driver")?,
            urgent: reqi(b, "urgent", "driver")?,
            mechanical: reqi(b, "mechanical", "driver")?,
            hometown,
            current_city: reqs(b, "current_city", "driver")?,
            experience_points: reqi(b, "experience_points", "driver")?,
            trips,
        })
    }
    Ok(result)
}
