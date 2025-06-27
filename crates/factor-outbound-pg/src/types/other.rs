/// A catch-all type via which we return unsupported Postgres values as blobs.
#[derive(Debug)]
pub struct Other(Vec<u8>);

impl tokio_postgres::types::FromSql<'_> for Other {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'_ [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw.to_owned()))
    }

    fn accepts(_ty: &tokio_postgres::types::Type) -> bool {
        true
    }
}

impl From<Other> for Vec<u8> {
    fn from(value: Other) -> Self {
        value.0
    }
}
