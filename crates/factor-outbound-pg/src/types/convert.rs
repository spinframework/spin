//! Conversions between WIT representations and the SQL types as surfaced by
//! the tokio_postgres driver.

use anyhow::{anyhow, Context};
use spin_world::spin::postgres4_0_0::postgres::{self as v4};

use super::decimal::RangeableDecimal;

pub fn jsonb_pg_to_wit(value: serde_json::Value) -> anyhow::Result<Vec<u8>> {
    serde_json::to_vec(&value).context("invalid JSON from database")
}

pub fn jsonb_wit_to_pg(value: &[u8]) -> anyhow::Result<serde_json::Value> {
    serde_json::from_slice(value)
        .with_context(|| format!("invalid JSON {}", String::from_utf8_lossy(value)))
}

pub fn uuid_wit_to_pg(value: &str) -> anyhow::Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid UUID {value}"))
}

pub fn decimal_wit_to_pg(value: &str) -> anyhow::Result<rust_decimal::Decimal> {
    rust_decimal::Decimal::from_str_exact(value).with_context(|| format!("invalid decimal {value}"))
}

pub fn decimal_array_pg_to_wit(value: Vec<Option<rust_decimal::Decimal>>) -> Vec<Option<String>> {
    value.iter().map(|opt| opt.map(|d| d.to_string())).collect()
}

pub fn decimal_array_wit_to_pg(
    value: &[Option<String>],
) -> anyhow::Result<Vec<Option<rust_decimal::Decimal>>> {
    value
        .iter()
        .map(|v| match v {
            None => Ok(None),
            Some(v) => rust_decimal::Decimal::from_str_exact(v)
                .with_context(|| format!("invalid decimal {v}"))
                .map(Some),
        })
        .collect::<anyhow::Result<Vec<_>>>()
}

// Functions to convert between Postgres ranges and the WIT range representations

pub type WitRange<T> = (
    Option<(T, v4::RangeBoundKind)>,
    Option<(T, v4::RangeBoundKind)>,
);

pub fn range_pg_to_wit<T: Copy + postgres_range::Normalizable + PartialOrd>(
    value: postgres_range::Range<T>,
) -> WitRange<T> {
    let lower = value.lower().map(tuplify_range_bound);
    let upper = value.upper().map(tuplify_range_bound);
    (lower, upper)
}

pub fn range_wit_to_pg<T: postgres_range::Normalizable + PartialOrd>(
    value: WitRange<T>,
) -> postgres_range::Range<T> {
    let (lower, upper) = value;
    let lbound = lower.map(|(value, kind)| {
        postgres_range::RangeBound::new(value, range_bound_kind_wit_to_pg(kind))
    });
    let ubound = upper.map(|(value, kind)| {
        postgres_range::RangeBound::new(value, range_bound_kind_wit_to_pg(kind))
    });
    postgres_range::Range::new(lbound, ubound)
}

pub fn decimal_range_pg_to_wit(value: postgres_range::Range<RangeableDecimal>) -> WitRange<String> {
    let lower = value
        .lower()
        .map(|b| tuplify_range_bound_map(b, |d| d.to_string()));
    let upper = value
        .upper()
        .map(|b| tuplify_range_bound_map(b, |d| d.to_string()));
    (lower, upper)
}

pub fn decimal_range_wit_to_pg(
    value: &WitRange<String>,
) -> anyhow::Result<postgres_range::Range<RangeableDecimal>> {
    let (lower, upper) = value;
    let lbound = lower
        .as_ref()
        .map(decimal_range_bound_wit_to_pg)
        .transpose()?;
    let ubound = upper
        .as_ref()
        .map(decimal_range_bound_wit_to_pg)
        .transpose()?;
    Ok(postgres_range::Range::new(lbound, ubound))
}

fn decimal_range_bound_wit_to_pg<S: postgres_range::BoundSided>(
    (value, kind): &(String, v4::RangeBoundKind),
) -> anyhow::Result<postgres_range::RangeBound<S, RangeableDecimal>> {
    let dec = rust_decimal::Decimal::from_str_exact(value)
        .with_context(|| format!("invalid decimal {value}"))?;
    Ok(postgres_range::RangeBound::new(
        RangeableDecimal(dec),
        range_bound_kind_wit_to_pg(*kind),
    ))
}

fn tuplify_range_bound<S: postgres_range::BoundSided, T: Copy>(
    value: &postgres_range::RangeBound<S, T>,
) -> (T, v4::RangeBoundKind) {
    (value.value, range_bound_kind_pg_to_wit(value.type_))
}

fn tuplify_range_bound_map<S: postgres_range::BoundSided, T, U>(
    value: &postgres_range::RangeBound<S, T>,
    map_fn: impl Fn(&T) -> U,
) -> (U, v4::RangeBoundKind) {
    (
        map_fn(&value.value),
        range_bound_kind_pg_to_wit(value.type_),
    )
}

fn range_bound_kind_wit_to_pg(wit_kind: v4::RangeBoundKind) -> postgres_range::BoundType {
    match wit_kind {
        v4::RangeBoundKind::Inclusive => postgres_range::BoundType::Inclusive,
        v4::RangeBoundKind::Exclusive => postgres_range::BoundType::Exclusive,
    }
}

fn range_bound_kind_pg_to_wit(bound_type: postgres_range::BoundType) -> v4::RangeBoundKind {
    match bound_type {
        postgres_range::BoundType::Inclusive => v4::RangeBoundKind::Inclusive,
        postgres_range::BoundType::Exclusive => v4::RangeBoundKind::Exclusive,
    }
}

// Functions to convert between the chrono types (Postgres-facing) and the WIT interface tuples

pub fn datetime_pg_to_wit(
    value: chrono::NaiveDateTime,
) -> anyhow::Result<(i32, u8, u8, u8, u8, u8, u32)> {
    use chrono::{Datelike, Timelike};
    Ok((
        value.year(),
        value.month().try_into()?,
        value.day().try_into()?,
        value.hour().try_into()?,
        value.minute().try_into()?,
        value.second().try_into()?,
        value.nanosecond(),
    ))
}

pub fn datetime_wit_to_pg(
    (y, mon, d, h, min, s, ns): &(i32, u8, u8, u8, u8, u8, u32),
) -> anyhow::Result<chrono::NaiveDateTime> {
    let naive_date = chrono::NaiveDate::from_ymd_opt(*y, (*mon).into(), (*d).into())
        .ok_or_else(|| anyhow!("invalid date y={y}, m={mon}, d={d}"))?;
    let naive_time =
        chrono::NaiveTime::from_hms_nano_opt((*h).into(), (*min).into(), (*s).into(), *ns)
            .ok_or_else(|| anyhow!("invalid time {h}:{min}:{s}:{ns}"))?;
    Ok(chrono::NaiveDateTime::new(naive_date, naive_time))
}

pub fn date_wit_to_pg((y, mon, d): &(i32, u8, u8)) -> anyhow::Result<chrono::NaiveDate> {
    chrono::NaiveDate::from_ymd_opt(*y, (*mon).into(), (*d).into())
        .ok_or_else(|| anyhow!("invalid date y={y}, m={mon}, d={d}"))
}

pub fn date_pg_to_wit(value: chrono::NaiveDate) -> anyhow::Result<(i32, u8, u8)> {
    use chrono::Datelike;
    Ok((
        value.year(),
        value.month().try_into()?,
        value.day().try_into()?,
    ))
}

pub fn time_wit_to_pg((h, min, s, ns): &(u8, u8, u8, u32)) -> anyhow::Result<chrono::NaiveTime> {
    chrono::NaiveTime::from_hms_nano_opt((*h).into(), (*min).into(), (*s).into(), *ns)
        .ok_or_else(|| anyhow!("invalid time {h}:{min}:{s}:{ns}"))
}

pub fn time_pg_to_wit(value: chrono::NaiveTime) -> anyhow::Result<(u8, u8, u8, u32)> {
    use chrono::Timelike;
    Ok((
        value.hour().try_into()?,
        value.minute().try_into()?,
        value.second().try_into()?,
        value.nanosecond(),
    ))
}

pub fn timestamp_wit_to_pg(value: i64) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(value, 0)
        .ok_or_else(|| anyhow!("invalid epoch timestamp {value}"))
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn timestamp_is_interpreted_as_seconds() {
        let ts = timestamp_wit_to_pg(0).expect("should have converted 0");
        assert_eq!(
            chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            ts.date_naive()
        );

        let ts = timestamp_wit_to_pg(60 * 60 + 2 * 60 + 3).expect("should have converted 1h2m3s");
        assert_eq!(chrono::NaiveTime::from_hms_opt(1, 2, 3).unwrap(), ts.time());

        let ts = timestamp_wit_to_pg(-60 * 60 * 24).expect("should have converted -1d");
        assert_eq!(
            chrono::NaiveDate::from_ymd_opt(1969, 12, 31).unwrap(),
            ts.date_naive()
        );

        let ts = timestamp_wit_to_pg(60 * 60 * 24 * 365).expect("should have converted -1d");
        assert_eq!(
            chrono::NaiveDate::from_ymd_opt(1971, 1, 1).unwrap(),
            ts.date_naive()
        );
    }

    #[test]
    fn can_convert_decimal_range_wit_to_pg() {
        let range = (
            Some(("123.45".to_string(), v4::RangeBoundKind::Inclusive)),
            Some(("456.789789".to_string(), v4::RangeBoundKind::Exclusive)),
        );

        let pg = decimal_range_wit_to_pg(&range).expect("should have converted decimal range");

        let (Some(pg_lower), Some(pg_upper)) = (pg.lower(), pg.upper()) else {
            panic!("both PG bounds should have been Some");
        };

        assert_eq!(rust_decimal::Decimal::new(12345, 2), pg_lower.value.0);
        assert_eq!(postgres_range::BoundType::Inclusive, pg_lower.type_);
        assert_eq!(rust_decimal::Decimal::new(456789789, 6), pg_upper.value.0);
        assert_eq!(postgres_range::BoundType::Exclusive, pg_upper.type_);
    }

    #[test]
    fn can_convert_decimal_range_pg_to_wit() {
        let lo = rust_decimal::Decimal::new(123456, 3);
        let hi = rust_decimal::Decimal::new(789987654, 2);

        let pg = postgres_range::Range::new(
            Some(postgres_range::RangeBound::new(
                RangeableDecimal(lo),
                postgres_range::BoundType::Exclusive,
            )),
            Some(postgres_range::RangeBound::new(
                RangeableDecimal(hi),
                postgres_range::BoundType::Inclusive,
            )),
        );

        let range = decimal_range_pg_to_wit(pg);

        let (Some(lower), Some(upper)) = range else {
            panic!("both WIT bounds should have been Some");
        };

        assert_eq!("123.456", lower.0);
        assert_eq!(v4::RangeBoundKind::Exclusive, lower.1);
        assert_eq!("7899876.54", upper.0);
        assert_eq!(v4::RangeBoundKind::Inclusive, upper.1);
    }

    #[test]
    fn can_convert_decimal_array_wit_to_pg() {
        let arr = vec![
            Some("12.34".to_string()),
            None,
            Some("123456789.987654321".to_string()),
        ];

        let pg = decimal_array_wit_to_pg(&arr).expect("should have converted decimal array");

        assert_eq!(arr.len(), pg.len());
        assert_eq!(
            rust_decimal::Decimal::new(1234, 2),
            *pg[0].as_ref().expect("some should convert to some")
        );
        assert!(pg[1].is_none(), "none should convert to none");
        assert_eq!(
            rust_decimal::Decimal::new(123456789987654321, 9),
            *pg[2].as_ref().expect("some should convert to some")
        );
    }

    #[test]
    fn can_convert_decimal_array_pg_to_wit() {
        let pg = vec![
            Some(rust_decimal::Decimal::new(1234, 2)),
            None,
            Some(rust_decimal::Decimal::new(123456789987654321, 9)),
        ];

        let arr = decimal_array_pg_to_wit(pg);

        assert_eq!(3, arr.len());
        assert_eq!(
            "12.34",
            *arr[0].as_ref().expect("some should convert to some")
        );
        assert!(arr[1].is_none(), "none should convert to none");
        assert_eq!(
            "123456789.987654321",
            *arr[2].as_ref().expect("some should convert to some")
        );
    }
}
