use anyhow::Result;
use spin_world::spin::postgres4_0_0::postgres::{self as v4};
use tokio_postgres::types::{FromSql, ToSql, Type};

#[derive(Debug)]
pub struct Interval(pub v4::Interval);

impl ToSql for Interval {
    tokio_postgres::types::to_sql_checked!();

    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        use bytes::BufMut;

        out.put_i64(self.0.micros);
        out.put_i32(self.0.days);
        out.put_i32(self.0.months);

        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        matches!(ty, &Type::INTERVAL)
    }
}

impl FromSql<'_> for Interval {
    fn from_sql(
        _ty: &Type,
        raw: &'_ [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        const EXPECTED_LEN: usize = size_of::<i64>() + size_of::<i32>() + size_of::<i32>();

        if raw.len() != EXPECTED_LEN {
            return Err(Box::new(IntervalLengthError));
        }

        let (micro_bytes, rest) = raw.split_at(size_of::<i64>());
        let (day_bytes, rest) = rest.split_at(size_of::<i32>());
        let month_bytes = rest;
        let months = i32::from_be_bytes(month_bytes.try_into().unwrap());
        let days = i32::from_be_bytes(day_bytes.try_into().unwrap());
        let micros = i64::from_be_bytes(micro_bytes.try_into().unwrap());

        Ok(Self(v4::Interval {
            micros,
            days,
            months,
        }))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(ty, &Type::INTERVAL)
    }
}

struct IntervalLengthError;

impl std::error::Error for IntervalLengthError {}

impl std::fmt::Display for IntervalLengthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unexpected binary format for Postgres INTERVAL")
    }
}

impl std::fmt::Debug for IntervalLengthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl From<Interval> for v4::Interval {
    fn from(value: Interval) -> Self {
        value.0
    }
}

impl From<v4::Interval> for Interval {
    fn from(value: v4::Interval) -> Self {
        Self(value)
    }
}
