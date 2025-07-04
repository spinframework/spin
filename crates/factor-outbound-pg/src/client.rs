use anyhow::Result;
use native_tls::TlsConnector;
use postgres_native_tls::MakeTlsConnector;
use spin_world::async_trait;
use spin_world::spin::postgres4_0_0::postgres::{
    self as v4, Column, DbValue, ParameterValue, RowSet,
};
use tokio_postgres::{config::SslMode, types::ToSql, Row};
use tokio_postgres::{Client as TokioClient, NoTls, Socket};

use crate::types::{convert_data_type, convert_entry, to_sql_parameter};

#[async_trait]
pub trait Client {
    async fn build_client(address: &str) -> Result<Self>
    where
        Self: Sized;

    async fn execute(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<u64, v4::Error>;

    async fn query(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<RowSet, v4::Error>;
}

#[async_trait]
impl Client for TokioClient {
    async fn build_client(address: &str) -> Result<Self>
    where
        Self: Sized,
    {
        let config = address.parse::<tokio_postgres::Config>()?;

        tracing::debug!("Build new connection: {}", address);

        if config.get_ssl_mode() == SslMode::Disable {
            let (client, connection) = config.connect(NoTls).await?;
            spawn_connection(connection);
            Ok(client)
        } else {
            let builder = TlsConnector::builder();
            let connector = MakeTlsConnector::new(builder.build()?);
            let (client, connection) = config.connect(connector).await?;
            spawn_connection(connection);
            Ok(client)
        }
    }

    async fn execute(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<u64, v4::Error> {
        let params = params
            .iter()
            .map(to_sql_parameter)
            .collect::<Result<Vec<_>>>()
            .map_err(|e| v4::Error::ValueConversionFailed(format!("{e:?}")))?;

        let params_refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();

        self.execute(&statement, params_refs.as_slice())
            .await
            .map_err(|e| v4::Error::QueryFailed(format!("{e:?}")))
    }

    async fn query(
        &self,
        statement: String,
        params: Vec<ParameterValue>,
    ) -> Result<RowSet, v4::Error> {
        let params = params
            .iter()
            .map(to_sql_parameter)
            .collect::<Result<Vec<_>>>()
            .map_err(|e| v4::Error::BadParameter(format!("{e:?}")))?;

        let params_refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|b| b.as_ref() as &(dyn ToSql + Sync))
            .collect();

        let results = self
            .query(&statement, params_refs.as_slice())
            .await
            .map_err(|e| v4::Error::QueryFailed(format!("{e:?}")))?;

        if results.is_empty() {
            return Ok(RowSet {
                columns: vec![],
                rows: vec![],
            });
        }

        let columns = infer_columns(&results[0]);
        let rows = results
            .iter()
            .map(convert_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| v4::Error::QueryFailed(format!("{e:?}")))?;

        Ok(RowSet { columns, rows })
    }
}

fn spawn_connection<T>(connection: tokio_postgres::Connection<Socket, T>)
where
    T: tokio_postgres::tls::TlsStream + std::marker::Unpin + std::marker::Send + 'static,
{
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Postgres connection error: {}", e);
        }
    });
}

// fn to_sql_parameter(value: &ParameterValue) -> Result<Box<dyn ToSql + Send + Sync>> {
//     match value {
//         ParameterValue::Boolean(v) => Ok(Box::new(*v)),
//         ParameterValue::Int32(v) => Ok(Box::new(*v)),
//         ParameterValue::Int64(v) => Ok(Box::new(*v)),
//         ParameterValue::Int8(v) => Ok(Box::new(*v)),
//         ParameterValue::Int16(v) => Ok(Box::new(*v)),
//         ParameterValue::Floating32(v) => Ok(Box::new(*v)),
//         ParameterValue::Floating64(v) => Ok(Box::new(*v)),
//         ParameterValue::Str(v) => Ok(Box::new(v.clone())),
//         ParameterValue::Binary(v) => Ok(Box::new(v.clone())),
//         ParameterValue::Date((y, mon, d)) => {
//             let naive_date = chrono::NaiveDate::from_ymd_opt(*y, (*mon).into(), (*d).into())
//                 .ok_or_else(|| anyhow!("invalid date y={y}, m={mon}, d={d}"))?;
//             Ok(Box::new(naive_date))
//         }
//         ParameterValue::Time((h, min, s, ns)) => {
//             let naive_time =
//                 chrono::NaiveTime::from_hms_nano_opt((*h).into(), (*min).into(), (*s).into(), *ns)
//                     .ok_or_else(|| anyhow!("invalid time {h}:{min}:{s}:{ns}"))?;
//             Ok(Box::new(naive_time))
//         }
//         ParameterValue::Datetime((y, mon, d, h, min, s, ns)) => {
//             let naive_date = chrono::NaiveDate::from_ymd_opt(*y, (*mon).into(), (*d).into())
//                 .ok_or_else(|| anyhow!("invalid date y={y}, m={mon}, d={d}"))?;
//             let naive_time =
//                 chrono::NaiveTime::from_hms_nano_opt((*h).into(), (*min).into(), (*s).into(), *ns)
//                     .ok_or_else(|| anyhow!("invalid time {h}:{min}:{s}:{ns}"))?;
//             let dt = chrono::NaiveDateTime::new(naive_date, naive_time);
//             Ok(Box::new(dt))
//         }
//         ParameterValue::Timestamp(v) => {
//             let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(*v, 0)
//                 .ok_or_else(|| anyhow!("invalid epoch timestamp {v}"))?;
//             Ok(Box::new(ts))
//         }
//         ParameterValue::Uuid(v) => {
//             let u = uuid::Uuid::parse_str(v).with_context(|| format!("invalid UUID {v}"))?;
//             Ok(Box::new(u))
//         }
//         ParameterValue::Jsonb(v) => {
//             let j: serde_json::Value = serde_json::from_slice(v)
//                 .with_context(|| format!("invalid JSON {}", String::from_utf8_lossy(v)))?;
//             Ok(Box::new(j))
//         }
//         ParameterValue::Decimal(v) => {
//             let dec = rust_decimal::Decimal::from_str_exact(v)
//                 .with_context(|| format!("invalid decimal {v}"))?;
//             Ok(Box::new(dec))
//         }
//         ParameterValue::RangeInt32((lower, upper)) => {
//             let lbound = lower.map(|(value, kind)| {
//                 postgres_range::RangeBound::new(value, range_bound_kind(kind))
//             });
//             let ubound = upper.map(|(value, kind)| {
//                 postgres_range::RangeBound::new(value, range_bound_kind(kind))
//             });
//             let r = postgres_range::Range::new(lbound, ubound);
//             Ok(Box::new(r))
//         }
//         ParameterValue::RangeInt64((lower, upper)) => {
//             let lbound = lower.map(|(value, kind)| {
//                 postgres_range::RangeBound::new(value, range_bound_kind(kind))
//             });
//             let ubound = upper.map(|(value, kind)| {
//                 postgres_range::RangeBound::new(value, range_bound_kind(kind))
//             });
//             let r = postgres_range::Range::new(lbound, ubound);
//             Ok(Box::new(r))
//         }
//         ParameterValue::RangeDecimal((lower, upper)) => {
//             let lbound = match lower {
//                 None => None,
//                 Some((value, kind)) => {
//                     let dec = rust_decimal::Decimal::from_str_exact(value)
//                         .with_context(|| format!("invalid decimal {value}"))?;
//                     let dec = RangeableDecimal(dec);
//                     Some(postgres_range::RangeBound::new(
//                         dec,
//                         range_bound_kind(*kind),
//                     ))
//                 }
//             };
//             let ubound = match upper {
//                 None => None,
//                 Some((value, kind)) => {
//                     let dec = rust_decimal::Decimal::from_str_exact(value)
//                         .with_context(|| format!("invalid decimal {value}"))?;
//                     let dec = RangeableDecimal(dec);
//                     Some(postgres_range::RangeBound::new(
//                         dec,
//                         range_bound_kind(*kind),
//                     ))
//                 }
//             };
//             let r = postgres_range::Range::new(lbound, ubound);
//             Ok(Box::new(r))
//         }
//         ParameterValue::ArrayInt32(vs) => Ok(Box::new(vs.to_owned())),
//         ParameterValue::ArrayInt64(vs) => Ok(Box::new(vs.to_owned())),
//         ParameterValue::ArrayDecimal(vs) => {
//             let decs = vs
//                 .iter()
//                 .map(|v| match v {
//                     None => Ok(None),
//                     Some(v) => rust_decimal::Decimal::from_str_exact(v)
//                         .with_context(|| format!("invalid decimal {v}"))
//                         .map(Some),
//                 })
//                 .collect::<anyhow::Result<Vec<_>>>()?;
//             Ok(Box::new(decs))
//         }
//         ParameterValue::ArrayStr(vs) => Ok(Box::new(vs.to_owned())),
//         ParameterValue::Interval(v) => Ok(Box::new(Interval(*v))),
//         ParameterValue::DbNull => Ok(Box::new(PgNull)),
//     }
// }

// fn range_bound_kind(wit_kind: v4::RangeBoundKind) -> postgres_range::BoundType {
//     match wit_kind {
//         v4::RangeBoundKind::Inclusive => postgres_range::BoundType::Inclusive,
//         v4::RangeBoundKind::Exclusive => postgres_range::BoundType::Exclusive,
//     }
// }

fn infer_columns(row: &Row) -> Vec<Column> {
    let mut result = Vec::with_capacity(row.len());
    for index in 0..row.len() {
        result.push(infer_column(row, index));
    }
    result
}

fn infer_column(row: &Row, index: usize) -> Column {
    let column = &row.columns()[index];
    let name = column.name().to_owned();
    let data_type = convert_data_type(column.type_());
    Column { name, data_type }
}

fn convert_row(row: &Row) -> anyhow::Result<Vec<DbValue>> {
    let mut result = Vec::with_capacity(row.len());
    for index in 0..row.len() {
        result.push(convert_entry(row, index)?);
    }
    Ok(result)
}

// fn db_value<'a, T: FromSql<'a>>(row: &'a Row, index: usize, convert_fn: impl Fn(T) -> DbValue) -> anyhow::Result<DbValue> {
//     let value: Option<T> = row.try_get(index)?;
//     Ok(match value {
//         Some(v) => convert_fn(v),
//         None => DbValue::DbNull,
//     })
// }

// fn map_db_value<'a, T: FromSql<'a>, W>(row: &'a Row, index: usize, ctor: impl Fn(W) -> DbValue, convert_fn: impl Fn(T) -> W) -> anyhow::Result<DbValue> {
//     let value: Option<T> = row.try_get(index)?;
//     Ok(match value {
//         Some(v) => ctor(convert_fn(v)),
//         None => DbValue::DbNull,
//     })
// }

// fn try_map_db_value<'a, T: FromSql<'a>, W>(row: &'a Row, index: usize, ctor: impl Fn(W) -> DbValue, convert_fn: impl Fn(T) -> anyhow::Result<W>) -> anyhow::Result<DbValue> {
//     let value: Option<T> = row.try_get(index)?;
//     Ok(match value {
//         Some(v) => ctor(convert_fn(v)?),
//         None => DbValue::DbNull,
//     })
// }

// fn json_db_value_to_vec(value: serde_json::Value) -> anyhow::Result<Vec<u8>> {
//     serde_json::to_vec(&value).context("invalid JSON from database")
// }

// fn range_db_value_to_range<T: Copy + postgres_range::Normalizable + PartialOrd>(value: postgres_range::Range<T>) -> (Option<(T, v4::RangeBoundKind)>, Option<(T, v4::RangeBoundKind)>) {
//     let lower = value.lower().map(tuplify_range_bound);
//     let upper = value.upper().map(tuplify_range_bound);
//     (lower, upper)
// }

// fn decimal_range_db_value_to_range(value: postgres_range::Range<RangeableDecimal>) -> (Option<(String, v4::RangeBoundKind)>, Option<(String, v4::RangeBoundKind)>) {
//     let lower = value
//         .lower()
//         .map(|b| tuplify_range_bound_map(b, |d| d.0.to_string()));
//     let upper = value
//         .upper()
//         .map(|b| tuplify_range_bound_map(b, |d| d.0.to_string()));
//     (lower, upper)
// }

// fn decimal_array_db_value_to_wit(value: Vec<Option<rust_decimal::Decimal>>) -> Vec<Option<String>> {
//     value.iter().map(|opt| opt.map(|d| d.to_string())).collect()
// }

// fn convert_entry(row: &Row, index: usize) -> anyhow::Result<DbValue> {
//     let column = &row.columns()[index];
//     match column.type_() {
//         &Type::BOOL => db_value(row, index, DbValue::Boolean),
//         &Type::BYTEA => db_value(row, index, DbValue::Binary),
//         &Type::FLOAT4 => db_value(row, index, DbValue::Floating32),
//         &Type::FLOAT8 => db_value(row, index, DbValue::Floating64),
//         &Type::INT2 => db_value(row, index, DbValue::Int16),
//         &Type::INT4 => db_value(row, index, DbValue::Int32),
//         &Type::INT8 => db_value(row, index, DbValue::Int64),
//         &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR => db_value(row, index, DbValue::Str),
//         &Type::TIMESTAMP | &Type::TIMESTAMPTZ => try_map_db_value(row, index, DbValue::Datetime, tuplify_date_time),
//         &Type::DATE => try_map_db_value(row, index, DbValue::Date, tuplify_date),
//         &Type::TIME => try_map_db_value(row, index, DbValue::Time, tuplify_time),
//         &Type::UUID => map_db_value(row, index, DbValue::Uuid, |v: uuid::Uuid| v.to_string()),
//         &Type::JSONB => try_map_db_value(row, index, DbValue::Jsonb, json_db_value_to_vec),
//         &Type::NUMERIC => map_db_value(row, index, DbValue::Decimal, |v: rust_decimal::Decimal| v.to_string()),
//         &Type::INT4_RANGE => map_db_value(row, index, DbValue::RangeInt32, range_db_value_to_range),
//         &Type::INT8_RANGE => map_db_value(row, index, DbValue::RangeInt64, range_db_value_to_range),
//         &Type::NUM_RANGE => map_db_value(row, index, DbValue::RangeDecimal, decimal_range_db_value_to_range),
//         &Type::INT4_ARRAY => db_value(row, index, DbValue::ArrayInt32),
//         &Type::INT8_ARRAY => db_value(row, index, DbValue::ArrayInt64),
//         &Type::NUMERIC_ARRAY => map_db_value(row, index, DbValue::ArrayDecimal, decimal_array_db_value_to_wit),
//         &Type::TEXT_ARRAY | &Type::VARCHAR_ARRAY | &Type::BPCHAR_ARRAY => db_value(row, index, DbValue::ArrayStr),
//         &Type::INTERVAL => map_db_value(row, index, DbValue::Interval, |v: Interval| v.0),
//         t => {
//             tracing::debug!(
//                 "Couldn't convert Postgres type {} in column {}",
//                 t.name(),
//                 column.name()
//             );
//             Ok(DbValue::Unsupported)
//         }
//     }
// }

// fn tuplify_range_bound<S: postgres_range::BoundSided, T: Copy>(
//     value: &postgres_range::RangeBound<S, T>,
// ) -> (T, v4::RangeBoundKind) {
//     (value.value, wit_bound_kind(value.type_))
// }

// fn tuplify_range_bound_map<S: postgres_range::BoundSided, T, U>(
//     value: &postgres_range::RangeBound<S, T>,
//     map_fn: impl Fn(&T) -> U,
// ) -> (U, v4::RangeBoundKind) {
//     (map_fn(&value.value), wit_bound_kind(value.type_))
// }

// fn wit_bound_kind(bound_type: postgres_range::BoundType) -> v4::RangeBoundKind {
//     match bound_type {
//         postgres_range::BoundType::Inclusive => v4::RangeBoundKind::Inclusive,
//         postgres_range::BoundType::Exclusive => v4::RangeBoundKind::Exclusive,
//     }
// }

// // Functions to convert from the chrono types to the WIT interface tuples
// fn tuplify_date_time(
//     value: chrono::NaiveDateTime,
// ) -> anyhow::Result<(i32, u8, u8, u8, u8, u8, u32)> {
//     use chrono::{Datelike, Timelike};
//     Ok((
//         value.year(),
//         value.month().try_into()?,
//         value.day().try_into()?,
//         value.hour().try_into()?,
//         value.minute().try_into()?,
//         value.second().try_into()?,
//         value.nanosecond(),
//     ))
// }

// fn tuplify_date(value: chrono::NaiveDate) -> anyhow::Result<(i32, u8, u8)> {
//     use chrono::Datelike;
//     Ok((
//         value.year(),
//         value.month().try_into()?,
//         value.day().try_into()?,
//     ))
// }

// fn tuplify_time(value: chrono::NaiveTime) -> anyhow::Result<(u8, u8, u8, u32)> {
//     use chrono::Timelike;
//     Ok((
//         value.hour().try_into()?,
//         value.minute().try_into()?,
//         value.second().try_into()?,
//         value.nanosecond(),
//     ))
// }

// /// Although the Postgres crate converts Rust Option::None to Postgres NULL,
// /// it enforces the type of the Option as it does so. (For example, trying to
// /// pass an Option::<i32>::None to a VARCHAR column fails conversion.) As we
// /// do not know expected column types, we instead use a "neutral" custom type
// /// which allows conversion to any type but always tells the Postgres crate to
// /// treat it as a SQL NULL.
// struct PgNull;

// impl ToSql for PgNull {
//     fn to_sql(
//         &self,
//         _ty: &Type,
//         _out: &mut tokio_postgres::types::private::BytesMut,
//     ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
//     where
//         Self: Sized,
//     {
//         Ok(tokio_postgres::types::IsNull::Yes)
//     }

//     fn accepts(_ty: &Type) -> bool
//     where
//         Self: Sized,
//     {
//         true
//     }

//     fn to_sql_checked(
//         &self,
//         _ty: &Type,
//         _out: &mut tokio_postgres::types::private::BytesMut,
//     ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
//         Ok(tokio_postgres::types::IsNull::Yes)
//     }
// }

// impl std::fmt::Debug for PgNull {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("NULL").finish()
//     }
// }

// #[derive(Debug)]
// struct Interval(v4::Interval);

// impl ToSql for Interval {
//     tokio_postgres::types::to_sql_checked!();

//     fn to_sql(
//         &self,
//         _ty: &Type,
//         out: &mut tokio_postgres::types::private::BytesMut,
//     ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
//     where
//         Self: Sized,
//     {
//         use bytes::BufMut;

//         out.put_i64(self.0.micros);
//         out.put_i32(self.0.days);
//         out.put_i32(self.0.months);

//         Ok(tokio_postgres::types::IsNull::No)
//     }

//     fn accepts(ty: &Type) -> bool
//     where
//         Self: Sized,
//     {
//         matches!(ty, &Type::INTERVAL)
//     }
// }

// impl FromSql<'_> for Interval {
//     fn from_sql(
//         _ty: &Type,
//         raw: &'_ [u8],
//     ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
//         const EXPECTED_LEN: usize = size_of::<i64>() + size_of::<i32>() + size_of::<i32>();

//         if raw.len() != EXPECTED_LEN {
//             return Err(Box::new(IntervalLengthError));
//         }

//         let (micro_bytes, rest) = raw.split_at(size_of::<i64>());
//         let (day_bytes, rest) = rest.split_at(size_of::<i32>());
//         let month_bytes = rest;
//         let months = i32::from_be_bytes(month_bytes.try_into().unwrap());
//         let days = i32::from_be_bytes(day_bytes.try_into().unwrap());
//         let micros = i64::from_be_bytes(micro_bytes.try_into().unwrap());

//         Ok(Self(v4::Interval {
//             micros,
//             days,
//             months,
//         }))
//     }

//     fn accepts(ty: &Type) -> bool {
//         matches!(ty, &Type::INTERVAL)
//     }
// }

// struct IntervalLengthError;

// impl std::error::Error for IntervalLengthError {}

// impl std::fmt::Display for IntervalLengthError {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.write_str("unexpected binary format for Postgres INTERVAL")
//     }
// }

// impl std::fmt::Debug for IntervalLengthError {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         std::fmt::Display::fmt(self, f)
//     }
// }

// #[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
// struct RangeableDecimal(rust_decimal::Decimal);

// impl ToSql for RangeableDecimal {
//     tokio_postgres::types::to_sql_checked!();

//     fn to_sql(
//         &self,
//         ty: &Type,
//         out: &mut tokio_postgres::types::private::BytesMut,
//     ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
//     where
//         Self: Sized,
//     {
//         self.0.to_sql(ty, out)
//     }

//     fn accepts(ty: &Type) -> bool
//     where
//         Self: Sized,
//     {
//         <rust_decimal::Decimal as ToSql>::accepts(ty)
//     }
// }

// impl FromSql<'_> for RangeableDecimal {
//     fn from_sql(
//         ty: &Type,
//         raw: &'_ [u8],
//     ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
//         let d = <rust_decimal::Decimal as FromSql>::from_sql(ty, raw)?;
//         Ok(Self(d))
//     }

//     fn accepts(ty: &Type) -> bool {
//         <rust_decimal::Decimal as FromSql>::accepts(ty)
//     }
// }

// impl postgres_range::Normalizable for RangeableDecimal {
//     fn normalize<S>(
//         bound: postgres_range::RangeBound<S, Self>,
//     ) -> postgres_range::RangeBound<S, Self>
//     where
//         S: postgres_range::BoundSided,
//     {
//         bound
//     }
// }
