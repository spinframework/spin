use spin_world::spin::postgres4_0_0::postgres::{DbDataType, DbValue, ParameterValue};
use tokio_postgres::types::{FromSql, Type};
use tokio_postgres::{types::ToSql, Row};

mod convert;
mod decimal;
mod interval;
mod pg_null;

use convert::{
    date_pg_to_wit, date_wit_to_pg, datetime_pg_to_wit, datetime_wit_to_pg,
    decimal_array_pg_to_wit, decimal_array_wit_to_pg, decimal_range_pg_to_wit,
    decimal_range_wit_to_pg, decimal_wit_to_pg, jsonb_pg_to_wit, jsonb_wit_to_pg, range_pg_to_wit,
    range_wit_to_pg, time_pg_to_wit, time_wit_to_pg, timestamp_wit_to_pg, uuid_wit_to_pg,
};
use interval::Interval;
use pg_null::PgNull;

pub fn convert_data_type(pg_type: &Type) -> DbDataType {
    match *pg_type {
        Type::BOOL => DbDataType::Boolean,
        Type::BYTEA => DbDataType::Binary,
        Type::FLOAT4 => DbDataType::Floating32,
        Type::FLOAT8 => DbDataType::Floating64,
        Type::INT2 => DbDataType::Int16,
        Type::INT4 => DbDataType::Int32,
        Type::INT8 => DbDataType::Int64,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR => DbDataType::Str,
        Type::TIMESTAMP | Type::TIMESTAMPTZ => DbDataType::Datetime,
        Type::DATE => DbDataType::Date,
        Type::TIME => DbDataType::Time,
        Type::UUID => DbDataType::Uuid,
        Type::JSONB => DbDataType::Jsonb,
        Type::NUMERIC => DbDataType::Decimal,
        Type::INT4_RANGE => DbDataType::RangeInt32,
        Type::INT8_RANGE => DbDataType::RangeInt64,
        Type::NUM_RANGE => DbDataType::RangeDecimal,
        Type::INT4_ARRAY => DbDataType::ArrayInt32,
        Type::INT8_ARRAY => DbDataType::ArrayInt64,
        Type::NUMERIC_ARRAY => DbDataType::ArrayDecimal,
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY | Type::BPCHAR_ARRAY => DbDataType::ArrayStr,
        Type::INTERVAL => DbDataType::Interval,
        _ => {
            tracing::debug!("Couldn't convert Postgres type {} to WIT", pg_type.name(),);
            DbDataType::Other
        }
    }
}

fn db_value<'a, T: FromSql<'a>>(
    row: &'a Row,
    index: usize,
    convert_fn: impl Fn(T) -> DbValue,
) -> anyhow::Result<DbValue> {
    let value: Option<T> = row.try_get(index)?;
    Ok(match value {
        Some(v) => convert_fn(v),
        None => DbValue::DbNull,
    })
}

fn map_db_value<'a, T: FromSql<'a>, W>(
    row: &'a Row,
    index: usize,
    ctor: impl Fn(W) -> DbValue,
    convert_fn: impl Fn(T) -> W,
) -> anyhow::Result<DbValue> {
    let value: Option<T> = row.try_get(index)?;
    Ok(match value {
        Some(v) => ctor(convert_fn(v)),
        None => DbValue::DbNull,
    })
}

fn try_map_db_value<'a, T: FromSql<'a>, W>(
    row: &'a Row,
    index: usize,
    ctor: impl Fn(W) -> DbValue,
    convert_fn: impl Fn(T) -> anyhow::Result<W>,
) -> anyhow::Result<DbValue> {
    let value: Option<T> = row.try_get(index)?;
    Ok(match value {
        Some(v) => ctor(convert_fn(v)?),
        None => DbValue::DbNull,
    })
}

pub fn convert_entry(row: &Row, index: usize) -> anyhow::Result<DbValue> {
    let column = &row.columns()[index];
    match column.type_() {
        &Type::BOOL => db_value(row, index, DbValue::Boolean),
        &Type::BYTEA => db_value(row, index, DbValue::Binary),
        &Type::FLOAT4 => db_value(row, index, DbValue::Floating32),
        &Type::FLOAT8 => db_value(row, index, DbValue::Floating64),
        &Type::INT2 => db_value(row, index, DbValue::Int16),
        &Type::INT4 => db_value(row, index, DbValue::Int32),
        &Type::INT8 => db_value(row, index, DbValue::Int64),
        &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR => db_value(row, index, DbValue::Str),
        &Type::TIMESTAMP | &Type::TIMESTAMPTZ => {
            try_map_db_value(row, index, DbValue::Datetime, datetime_pg_to_wit)
        }
        &Type::DATE => try_map_db_value(row, index, DbValue::Date, date_pg_to_wit),
        &Type::TIME => try_map_db_value(row, index, DbValue::Time, time_pg_to_wit),
        &Type::UUID => map_db_value(row, index, DbValue::Uuid, |v: uuid::Uuid| v.to_string()),
        &Type::JSONB => try_map_db_value(row, index, DbValue::Jsonb, jsonb_pg_to_wit),
        &Type::NUMERIC => map_db_value(row, index, DbValue::Decimal, |v: rust_decimal::Decimal| {
            v.to_string()
        }),
        &Type::INT4_RANGE => map_db_value(row, index, DbValue::RangeInt32, range_pg_to_wit),
        &Type::INT8_RANGE => map_db_value(row, index, DbValue::RangeInt64, range_pg_to_wit),
        &Type::NUM_RANGE => {
            map_db_value(row, index, DbValue::RangeDecimal, decimal_range_pg_to_wit)
        }
        &Type::INT4_ARRAY => db_value(row, index, DbValue::ArrayInt32),
        &Type::INT8_ARRAY => db_value(row, index, DbValue::ArrayInt64),
        &Type::NUMERIC_ARRAY => {
            map_db_value(row, index, DbValue::ArrayDecimal, decimal_array_pg_to_wit)
        }
        &Type::TEXT_ARRAY | &Type::VARCHAR_ARRAY | &Type::BPCHAR_ARRAY => {
            db_value(row, index, DbValue::ArrayStr)
        }
        &Type::INTERVAL => map_db_value(row, index, DbValue::Interval, |v: Interval| v.into()),
        t => {
            tracing::debug!(
                "Couldn't convert Postgres type {} in column {}",
                t.name(),
                column.name()
            );
            Ok(DbValue::Unsupported)
        }
    }
}

pub fn to_sql_parameter(value: &ParameterValue) -> anyhow::Result<Box<dyn ToSql + Send + Sync>> {
    match value {
        ParameterValue::Boolean(v) => Ok(Box::new(*v)),
        ParameterValue::Int32(v) => Ok(Box::new(*v)),
        ParameterValue::Int64(v) => Ok(Box::new(*v)),
        ParameterValue::Int8(v) => Ok(Box::new(*v)),
        ParameterValue::Int16(v) => Ok(Box::new(*v)),
        ParameterValue::Floating32(v) => Ok(Box::new(*v)),
        ParameterValue::Floating64(v) => Ok(Box::new(*v)),
        ParameterValue::Str(v) => Ok(Box::new(v.clone())),
        ParameterValue::Binary(v) => Ok(Box::new(v.clone())),
        ParameterValue::Date(v) => Ok(Box::new(date_wit_to_pg(v)?)),
        ParameterValue::Time(v) => Ok(Box::new(time_wit_to_pg(v)?)),
        ParameterValue::Datetime(v) => Ok(Box::new(datetime_wit_to_pg(v)?)),
        ParameterValue::Timestamp(v) => Ok(Box::new(timestamp_wit_to_pg(*v)?)),
        ParameterValue::Uuid(v) => Ok(Box::new(uuid_wit_to_pg(v)?)),
        ParameterValue::Jsonb(v) => Ok(Box::new(jsonb_wit_to_pg(v)?)),
        ParameterValue::Decimal(v) => Ok(Box::new(decimal_wit_to_pg(v)?)),
        ParameterValue::RangeInt32(v) => Ok(Box::new(range_wit_to_pg(*v))),
        ParameterValue::RangeInt64(v) => Ok(Box::new(range_wit_to_pg(*v))),
        ParameterValue::RangeDecimal(v) => Ok(Box::new(decimal_range_wit_to_pg(v)?)),
        ParameterValue::ArrayInt32(vs) => Ok(Box::new(vs.to_owned())),
        ParameterValue::ArrayInt64(vs) => Ok(Box::new(vs.to_owned())),
        ParameterValue::ArrayDecimal(vs) => Ok(Box::new(decimal_array_wit_to_pg(vs)?)),
        ParameterValue::ArrayStr(vs) => Ok(Box::new(vs.to_owned())),
        ParameterValue::Interval(v) => Ok(Box::new(Interval(*v))),
        ParameterValue::DbNull => Ok(Box::new(PgNull)),
    }
}
