use anyhow::Result;
use tokio_postgres::types::{FromSql, ToSql, Type};

/// Wraps the `Decimal` type to allow its use in postgres_range::Range.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct RangeableDecimal(pub rust_decimal::Decimal);

impl ToSql for RangeableDecimal {
    tokio_postgres::types::to_sql_checked!();

    fn to_sql(
        &self,
        ty: &Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.0.to_sql(ty, out)
    }

    fn accepts(ty: &Type) -> bool
    where
        Self: Sized,
    {
        <rust_decimal::Decimal as ToSql>::accepts(ty)
    }
}

impl FromSql<'_> for RangeableDecimal {
    fn from_sql(
        ty: &Type,
        raw: &'_ [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let d = <rust_decimal::Decimal as FromSql>::from_sql(ty, raw)?;
        Ok(Self(d))
    }

    fn accepts(ty: &Type) -> bool {
        <rust_decimal::Decimal as FromSql>::accepts(ty)
    }
}

impl postgres_range::Normalizable for RangeableDecimal {
    fn normalize<S>(
        bound: postgres_range::RangeBound<S, Self>,
    ) -> postgres_range::RangeBound<S, Self>
    where
        S: postgres_range::BoundSided,
    {
        bound
    }
}

impl std::fmt::Display for RangeableDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
