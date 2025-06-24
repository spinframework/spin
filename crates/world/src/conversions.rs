use super::*;

mod rdbms_types {
    use super::*;

    impl From<v2::rdbms_types::Column> for v1::rdbms_types::Column {
        fn from(value: v2::rdbms_types::Column) -> Self {
            v1::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Column> for v1::rdbms_types::Column {
        fn from(value: spin::postgres4_0_0::postgres::Column) -> Self {
            v1::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Column> for v2::rdbms_types::Column {
        fn from(value: spin::postgres4_0_0::postgres::Column) -> Self {
            v2::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Column> for spin::postgres3_0_0::postgres::Column {
        fn from(value: spin::postgres4_0_0::postgres::Column) -> Self {
            spin::postgres3_0_0::postgres::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<v2::rdbms_types::DbValue> for v1::rdbms_types::DbValue {
        fn from(value: v2::rdbms_types::DbValue) -> v1::rdbms_types::DbValue {
            match value {
                v2::rdbms_types::DbValue::Boolean(b) => v1::rdbms_types::DbValue::Boolean(b),
                v2::rdbms_types::DbValue::Int8(i) => v1::rdbms_types::DbValue::Int8(i),
                v2::rdbms_types::DbValue::Int16(i) => v1::rdbms_types::DbValue::Int16(i),
                v2::rdbms_types::DbValue::Int32(i) => v1::rdbms_types::DbValue::Int32(i),
                v2::rdbms_types::DbValue::Int64(i) => v1::rdbms_types::DbValue::Int64(i),
                v2::rdbms_types::DbValue::Uint8(j) => v1::rdbms_types::DbValue::Uint8(j),
                v2::rdbms_types::DbValue::Uint16(u) => v1::rdbms_types::DbValue::Uint16(u),
                v2::rdbms_types::DbValue::Uint32(u) => v1::rdbms_types::DbValue::Uint32(u),
                v2::rdbms_types::DbValue::Uint64(u) => v1::rdbms_types::DbValue::Uint64(u),
                v2::rdbms_types::DbValue::Floating32(r) => v1::rdbms_types::DbValue::Floating32(r),
                v2::rdbms_types::DbValue::Floating64(r) => v1::rdbms_types::DbValue::Floating64(r),
                v2::rdbms_types::DbValue::Str(s) => v1::rdbms_types::DbValue::Str(s),
                v2::rdbms_types::DbValue::Binary(b) => v1::rdbms_types::DbValue::Binary(b),
                v2::rdbms_types::DbValue::DbNull => v1::rdbms_types::DbValue::DbNull,
                v2::rdbms_types::DbValue::Unsupported => v1::rdbms_types::DbValue::Unsupported,
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbValue> for v1::rdbms_types::DbValue {
        fn from(value: spin::postgres4_0_0::postgres::DbValue) -> v1::rdbms_types::DbValue {
            match value {
                spin::postgres4_0_0::postgres::DbValue::Boolean(b) => {
                    v1::rdbms_types::DbValue::Boolean(b)
                }
                spin::postgres4_0_0::postgres::DbValue::Int8(i) => {
                    v1::rdbms_types::DbValue::Int8(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int16(i) => {
                    v1::rdbms_types::DbValue::Int16(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int32(i) => {
                    v1::rdbms_types::DbValue::Int32(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int64(i) => {
                    v1::rdbms_types::DbValue::Int64(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating32(r) => {
                    v1::rdbms_types::DbValue::Floating32(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating64(r) => {
                    v1::rdbms_types::DbValue::Floating64(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Str(s) => v1::rdbms_types::DbValue::Str(s),
                spin::postgres4_0_0::postgres::DbValue::Binary(b) => {
                    v1::rdbms_types::DbValue::Binary(b)
                }
                spin::postgres4_0_0::postgres::DbValue::DbNull => v1::rdbms_types::DbValue::DbNull,
                spin::postgres4_0_0::postgres::DbValue::Unsupported => {
                    v1::rdbms_types::DbValue::Unsupported
                }
                _ => v1::rdbms_types::DbValue::Unsupported,
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbValue> for v2::rdbms_types::DbValue {
        fn from(value: spin::postgres4_0_0::postgres::DbValue) -> v2::rdbms_types::DbValue {
            match value {
                spin::postgres4_0_0::postgres::DbValue::Boolean(b) => {
                    v2::rdbms_types::DbValue::Boolean(b)
                }
                spin::postgres4_0_0::postgres::DbValue::Int8(i) => {
                    v2::rdbms_types::DbValue::Int8(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int16(i) => {
                    v2::rdbms_types::DbValue::Int16(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int32(i) => {
                    v2::rdbms_types::DbValue::Int32(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int64(i) => {
                    v2::rdbms_types::DbValue::Int64(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating32(r) => {
                    v2::rdbms_types::DbValue::Floating32(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating64(r) => {
                    v2::rdbms_types::DbValue::Floating64(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Str(s) => v2::rdbms_types::DbValue::Str(s),
                spin::postgres4_0_0::postgres::DbValue::Binary(b) => {
                    v2::rdbms_types::DbValue::Binary(b)
                }
                spin::postgres4_0_0::postgres::DbValue::DbNull => v2::rdbms_types::DbValue::DbNull,
                spin::postgres4_0_0::postgres::DbValue::Unsupported => {
                    v2::rdbms_types::DbValue::Unsupported
                }
                _ => v2::rdbms_types::DbValue::Unsupported,
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbValue> for spin::postgres3_0_0::postgres::DbValue {
        fn from(
            value: spin::postgres4_0_0::postgres::DbValue,
        ) -> spin::postgres3_0_0::postgres::DbValue {
            match value {
                spin::postgres4_0_0::postgres::DbValue::Boolean(b) => {
                    spin::postgres3_0_0::postgres::DbValue::Boolean(b)
                }
                spin::postgres4_0_0::postgres::DbValue::Int8(i) => {
                    spin::postgres3_0_0::postgres::DbValue::Int8(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int16(i) => {
                    spin::postgres3_0_0::postgres::DbValue::Int16(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int32(i) => {
                    spin::postgres3_0_0::postgres::DbValue::Int32(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Int64(i) => {
                    spin::postgres3_0_0::postgres::DbValue::Int64(i)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating32(r) => {
                    spin::postgres3_0_0::postgres::DbValue::Floating32(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Floating64(r) => {
                    spin::postgres3_0_0::postgres::DbValue::Floating64(r)
                }
                spin::postgres4_0_0::postgres::DbValue::Str(s) => {
                    spin::postgres3_0_0::postgres::DbValue::Str(s)
                }
                spin::postgres4_0_0::postgres::DbValue::Binary(b) => {
                    spin::postgres3_0_0::postgres::DbValue::Binary(b)
                }
                spin::postgres4_0_0::postgres::DbValue::Date(d) => {
                    spin::postgres3_0_0::postgres::DbValue::Date(d)
                }
                spin::postgres4_0_0::postgres::DbValue::Datetime(dt) => {
                    spin::postgres3_0_0::postgres::DbValue::Datetime(dt)
                }
                spin::postgres4_0_0::postgres::DbValue::Time(t) => {
                    spin::postgres3_0_0::postgres::DbValue::Time(t)
                }
                spin::postgres4_0_0::postgres::DbValue::Timestamp(t) => {
                    spin::postgres3_0_0::postgres::DbValue::Timestamp(t)
                }
                spin::postgres4_0_0::postgres::DbValue::Uuid(_) => {
                    spin::postgres3_0_0::postgres::DbValue::Unsupported
                }
                spin::postgres4_0_0::postgres::DbValue::Jsonb(_) => {
                    spin::postgres3_0_0::postgres::DbValue::Unsupported
                }
                spin::postgres4_0_0::postgres::DbValue::DbNull => {
                    spin::postgres3_0_0::postgres::DbValue::DbNull
                }
                spin::postgres4_0_0::postgres::DbValue::Unsupported => {
                    spin::postgres3_0_0::postgres::DbValue::Unsupported
                }
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbDataType> for v1::rdbms_types::DbDataType {
        fn from(value: spin::postgres4_0_0::postgres::DbDataType) -> v1::rdbms_types::DbDataType {
            match value {
                spin::postgres4_0_0::postgres::DbDataType::Boolean => {
                    v1::rdbms_types::DbDataType::Boolean
                }
                spin::postgres4_0_0::postgres::DbDataType::Int8 => {
                    v1::rdbms_types::DbDataType::Int8
                }
                spin::postgres4_0_0::postgres::DbDataType::Int16 => {
                    v1::rdbms_types::DbDataType::Int16
                }
                spin::postgres4_0_0::postgres::DbDataType::Int32 => {
                    v1::rdbms_types::DbDataType::Int32
                }
                spin::postgres4_0_0::postgres::DbDataType::Int64 => {
                    v1::rdbms_types::DbDataType::Int64
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating32 => {
                    v1::rdbms_types::DbDataType::Floating32
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating64 => {
                    v1::rdbms_types::DbDataType::Floating64
                }
                spin::postgres4_0_0::postgres::DbDataType::Str => v1::rdbms_types::DbDataType::Str,
                spin::postgres4_0_0::postgres::DbDataType::Binary => {
                    v1::rdbms_types::DbDataType::Binary
                }
                spin::postgres4_0_0::postgres::DbDataType::Other => {
                    v1::rdbms_types::DbDataType::Other
                }
                _ => v1::rdbms_types::DbDataType::Other,
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbDataType> for v2::rdbms_types::DbDataType {
        fn from(value: spin::postgres4_0_0::postgres::DbDataType) -> v2::rdbms_types::DbDataType {
            match value {
                spin::postgres4_0_0::postgres::DbDataType::Boolean => {
                    v2::rdbms_types::DbDataType::Boolean
                }
                spin::postgres4_0_0::postgres::DbDataType::Int8 => {
                    v2::rdbms_types::DbDataType::Int8
                }
                spin::postgres4_0_0::postgres::DbDataType::Int16 => {
                    v2::rdbms_types::DbDataType::Int16
                }
                spin::postgres4_0_0::postgres::DbDataType::Int32 => {
                    v2::rdbms_types::DbDataType::Int32
                }
                spin::postgres4_0_0::postgres::DbDataType::Int64 => {
                    v2::rdbms_types::DbDataType::Int64
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating32 => {
                    v2::rdbms_types::DbDataType::Floating32
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating64 => {
                    v2::rdbms_types::DbDataType::Floating64
                }
                spin::postgres4_0_0::postgres::DbDataType::Str => v2::rdbms_types::DbDataType::Str,
                spin::postgres4_0_0::postgres::DbDataType::Binary => {
                    v2::rdbms_types::DbDataType::Binary
                }
                spin::postgres4_0_0::postgres::DbDataType::Other => {
                    v2::rdbms_types::DbDataType::Other
                }
                _ => v2::rdbms_types::DbDataType::Other,
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::DbDataType> for spin::postgres3_0_0::postgres::DbDataType {
        fn from(
            value: spin::postgres4_0_0::postgres::DbDataType,
        ) -> spin::postgres3_0_0::postgres::DbDataType {
            match value {
                spin::postgres4_0_0::postgres::DbDataType::Boolean => {
                    spin::postgres3_0_0::postgres::DbDataType::Boolean
                }
                spin::postgres4_0_0::postgres::DbDataType::Int8 => {
                    spin::postgres3_0_0::postgres::DbDataType::Int8
                }
                spin::postgres4_0_0::postgres::DbDataType::Int16 => {
                    spin::postgres3_0_0::postgres::DbDataType::Int16
                }
                spin::postgres4_0_0::postgres::DbDataType::Int32 => {
                    spin::postgres3_0_0::postgres::DbDataType::Int32
                }
                spin::postgres4_0_0::postgres::DbDataType::Int64 => {
                    spin::postgres3_0_0::postgres::DbDataType::Int64
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating32 => {
                    spin::postgres3_0_0::postgres::DbDataType::Floating32
                }
                spin::postgres4_0_0::postgres::DbDataType::Floating64 => {
                    spin::postgres3_0_0::postgres::DbDataType::Floating64
                }
                spin::postgres4_0_0::postgres::DbDataType::Str => {
                    spin::postgres3_0_0::postgres::DbDataType::Str
                }
                spin::postgres4_0_0::postgres::DbDataType::Binary => {
                    spin::postgres3_0_0::postgres::DbDataType::Binary
                }
                spin::postgres4_0_0::postgres::DbDataType::Date => {
                    spin::postgres3_0_0::postgres::DbDataType::Date
                }
                spin::postgres4_0_0::postgres::DbDataType::Datetime => {
                    spin::postgres3_0_0::postgres::DbDataType::Datetime
                }
                spin::postgres4_0_0::postgres::DbDataType::Time => {
                    spin::postgres3_0_0::postgres::DbDataType::Time
                }
                spin::postgres4_0_0::postgres::DbDataType::Timestamp => {
                    spin::postgres3_0_0::postgres::DbDataType::Timestamp
                }
                spin::postgres4_0_0::postgres::DbDataType::Uuid => {
                    spin::postgres3_0_0::postgres::DbDataType::Other
                }
                spin::postgres4_0_0::postgres::DbDataType::Jsonb => {
                    spin::postgres3_0_0::postgres::DbDataType::Other
                }
                spin::postgres4_0_0::postgres::DbDataType::Other => {
                    spin::postgres3_0_0::postgres::DbDataType::Other
                }
            }
        }
    }

    impl From<v2::rdbms_types::DbDataType> for v1::rdbms_types::DbDataType {
        fn from(value: v2::rdbms_types::DbDataType) -> v1::rdbms_types::DbDataType {
            match value {
                v2::rdbms_types::DbDataType::Boolean => v1::rdbms_types::DbDataType::Boolean,
                v2::rdbms_types::DbDataType::Int8 => v1::rdbms_types::DbDataType::Int8,
                v2::rdbms_types::DbDataType::Int16 => v1::rdbms_types::DbDataType::Int16,
                v2::rdbms_types::DbDataType::Int32 => v1::rdbms_types::DbDataType::Int32,
                v2::rdbms_types::DbDataType::Int64 => v1::rdbms_types::DbDataType::Int64,
                v2::rdbms_types::DbDataType::Uint8 => v1::rdbms_types::DbDataType::Uint8,
                v2::rdbms_types::DbDataType::Uint16 => v1::rdbms_types::DbDataType::Uint16,
                v2::rdbms_types::DbDataType::Uint32 => v1::rdbms_types::DbDataType::Uint32,
                v2::rdbms_types::DbDataType::Uint64 => v1::rdbms_types::DbDataType::Uint64,
                v2::rdbms_types::DbDataType::Floating32 => v1::rdbms_types::DbDataType::Floating32,
                v2::rdbms_types::DbDataType::Floating64 => v1::rdbms_types::DbDataType::Floating64,
                v2::rdbms_types::DbDataType::Str => v1::rdbms_types::DbDataType::Str,
                v2::rdbms_types::DbDataType::Binary => v1::rdbms_types::DbDataType::Binary,
                v2::rdbms_types::DbDataType::Other => v1::rdbms_types::DbDataType::Other,
            }
        }
    }

    impl From<v1::rdbms_types::ParameterValue> for v2::rdbms_types::ParameterValue {
        fn from(value: v1::rdbms_types::ParameterValue) -> v2::rdbms_types::ParameterValue {
            match value {
                v1::rdbms_types::ParameterValue::Boolean(b) => {
                    v2::rdbms_types::ParameterValue::Boolean(b)
                }
                v1::rdbms_types::ParameterValue::Int8(i) => {
                    v2::rdbms_types::ParameterValue::Int8(i)
                }
                v1::rdbms_types::ParameterValue::Int16(i) => {
                    v2::rdbms_types::ParameterValue::Int16(i)
                }
                v1::rdbms_types::ParameterValue::Int32(i) => {
                    v2::rdbms_types::ParameterValue::Int32(i)
                }
                v1::rdbms_types::ParameterValue::Int64(i) => {
                    v2::rdbms_types::ParameterValue::Int64(i)
                }
                v1::rdbms_types::ParameterValue::Uint8(u) => {
                    v2::rdbms_types::ParameterValue::Uint8(u)
                }
                v1::rdbms_types::ParameterValue::Uint16(u) => {
                    v2::rdbms_types::ParameterValue::Uint16(u)
                }
                v1::rdbms_types::ParameterValue::Uint32(u) => {
                    v2::rdbms_types::ParameterValue::Uint32(u)
                }
                v1::rdbms_types::ParameterValue::Uint64(u) => {
                    v2::rdbms_types::ParameterValue::Uint64(u)
                }
                v1::rdbms_types::ParameterValue::Floating32(r) => {
                    v2::rdbms_types::ParameterValue::Floating32(r)
                }
                v1::rdbms_types::ParameterValue::Floating64(r) => {
                    v2::rdbms_types::ParameterValue::Floating64(r)
                }
                v1::rdbms_types::ParameterValue::Str(s) => v2::rdbms_types::ParameterValue::Str(s),
                v1::rdbms_types::ParameterValue::Binary(b) => {
                    v2::rdbms_types::ParameterValue::Binary(b)
                }
                v1::rdbms_types::ParameterValue::DbNull => v2::rdbms_types::ParameterValue::DbNull,
            }
        }
    }

    impl TryFrom<v1::rdbms_types::ParameterValue> for spin::postgres4_0_0::postgres::ParameterValue {
        type Error = v1::postgres::PgError;

        fn try_from(
            value: v1::rdbms_types::ParameterValue,
        ) -> Result<spin::postgres4_0_0::postgres::ParameterValue, Self::Error> {
            let converted = match value {
                v1::rdbms_types::ParameterValue::Boolean(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Boolean(b)
                }
                v1::rdbms_types::ParameterValue::Int8(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int8(i)
                }
                v1::rdbms_types::ParameterValue::Int16(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int16(i)
                }
                v1::rdbms_types::ParameterValue::Int32(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int32(i)
                }
                v1::rdbms_types::ParameterValue::Int64(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int64(i)
                }
                v1::rdbms_types::ParameterValue::Uint8(_)
                | v1::rdbms_types::ParameterValue::Uint16(_)
                | v1::rdbms_types::ParameterValue::Uint32(_)
                | v1::rdbms_types::ParameterValue::Uint64(_) => {
                    return Err(v1::postgres::PgError::ValueConversionFailed(
                        "Postgres does not support unsigned integers".to_owned(),
                    ));
                }
                v1::rdbms_types::ParameterValue::Floating32(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating32(r)
                }
                v1::rdbms_types::ParameterValue::Floating64(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating64(r)
                }
                v1::rdbms_types::ParameterValue::Str(s) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Str(s)
                }
                v1::rdbms_types::ParameterValue::Binary(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Binary(b)
                }
                v1::rdbms_types::ParameterValue::DbNull => {
                    spin::postgres4_0_0::postgres::ParameterValue::DbNull
                }
            };
            Ok(converted)
        }
    }

    impl TryFrom<v2::rdbms_types::ParameterValue> for spin::postgres4_0_0::postgres::ParameterValue {
        type Error = v2::rdbms_types::Error;

        fn try_from(
            value: v2::rdbms_types::ParameterValue,
        ) -> Result<spin::postgres4_0_0::postgres::ParameterValue, Self::Error> {
            let converted = match value {
                v2::rdbms_types::ParameterValue::Boolean(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Boolean(b)
                }
                v2::rdbms_types::ParameterValue::Int8(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int8(i)
                }
                v2::rdbms_types::ParameterValue::Int16(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int16(i)
                }
                v2::rdbms_types::ParameterValue::Int32(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int32(i)
                }
                v2::rdbms_types::ParameterValue::Int64(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int64(i)
                }
                v2::rdbms_types::ParameterValue::Uint8(_)
                | v2::rdbms_types::ParameterValue::Uint16(_)
                | v2::rdbms_types::ParameterValue::Uint32(_)
                | v2::rdbms_types::ParameterValue::Uint64(_) => {
                    return Err(v2::rdbms_types::Error::ValueConversionFailed(
                        "Postgres does not support unsigned integers".to_owned(),
                    ));
                }
                v2::rdbms_types::ParameterValue::Floating32(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating32(r)
                }
                v2::rdbms_types::ParameterValue::Floating64(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating64(r)
                }
                v2::rdbms_types::ParameterValue::Str(s) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Str(s)
                }
                v2::rdbms_types::ParameterValue::Binary(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Binary(b)
                }
                v2::rdbms_types::ParameterValue::DbNull => {
                    spin::postgres4_0_0::postgres::ParameterValue::DbNull
                }
            };
            Ok(converted)
        }
    }

    impl From<spin::postgres3_0_0::postgres::ParameterValue>
        for spin::postgres4_0_0::postgres::ParameterValue
    {
        fn from(
            value: spin::postgres3_0_0::postgres::ParameterValue,
        ) -> spin::postgres4_0_0::postgres::ParameterValue {
            match value {
                spin::postgres3_0_0::postgres::ParameterValue::Boolean(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Boolean(b)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Int8(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int8(i)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Int16(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int16(i)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Int32(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int32(i)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Int64(i) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Int64(i)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Floating32(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating32(r)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Floating64(r) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Floating64(r)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Str(s) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Str(s)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Binary(b) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Binary(b)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Date(d) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Date(d)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Datetime(dt) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Datetime(dt)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Time(t) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Time(t)
                }
                spin::postgres3_0_0::postgres::ParameterValue::Timestamp(t) => {
                    spin::postgres4_0_0::postgres::ParameterValue::Timestamp(t)
                }
                spin::postgres3_0_0::postgres::ParameterValue::DbNull => {
                    spin::postgres4_0_0::postgres::ParameterValue::DbNull
                }
            }
        }
    }

    impl From<v2::rdbms_types::Error> for v1::mysql::MysqlError {
        fn from(error: v2::rdbms_types::Error) -> v1::mysql::MysqlError {
            match error {
                v2::mysql::Error::ConnectionFailed(e) => v1::mysql::MysqlError::ConnectionFailed(e),
                v2::mysql::Error::BadParameter(e) => v1::mysql::MysqlError::BadParameter(e),
                v2::mysql::Error::QueryFailed(e) => v1::mysql::MysqlError::QueryFailed(e),
                v2::mysql::Error::ValueConversionFailed(e) => {
                    v1::mysql::MysqlError::ValueConversionFailed(e)
                }
                v2::mysql::Error::Other(e) => v1::mysql::MysqlError::OtherError(e),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Error> for v1::postgres::PgError {
        fn from(error: spin::postgres4_0_0::postgres::Error) -> v1::postgres::PgError {
            match error {
                spin::postgres4_0_0::postgres::Error::ConnectionFailed(e) => {
                    v1::postgres::PgError::ConnectionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::BadParameter(e) => {
                    v1::postgres::PgError::BadParameter(e)
                }
                spin::postgres4_0_0::postgres::Error::QueryFailed(e) => {
                    v1::postgres::PgError::QueryFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::ValueConversionFailed(e) => {
                    v1::postgres::PgError::ValueConversionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::Other(e) => {
                    v1::postgres::PgError::OtherError(e)
                }
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Error> for v2::rdbms_types::Error {
        fn from(error: spin::postgres4_0_0::postgres::Error) -> v2::rdbms_types::Error {
            match error {
                spin::postgres4_0_0::postgres::Error::ConnectionFailed(e) => {
                    v2::rdbms_types::Error::ConnectionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::BadParameter(e) => {
                    v2::rdbms_types::Error::BadParameter(e)
                }
                spin::postgres4_0_0::postgres::Error::QueryFailed(e) => {
                    v2::rdbms_types::Error::QueryFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::ValueConversionFailed(e) => {
                    v2::rdbms_types::Error::ValueConversionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::Other(e) => v2::rdbms_types::Error::Other(e),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::Error> for spin::postgres3_0_0::postgres::Error {
        fn from(
            error: spin::postgres4_0_0::postgres::Error,
        ) -> spin::postgres3_0_0::postgres::Error {
            match error {
                spin::postgres4_0_0::postgres::Error::ConnectionFailed(e) => {
                    spin::postgres3_0_0::postgres::Error::ConnectionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::BadParameter(e) => {
                    spin::postgres3_0_0::postgres::Error::BadParameter(e)
                }
                spin::postgres4_0_0::postgres::Error::QueryFailed(e) => {
                    spin::postgres3_0_0::postgres::Error::QueryFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::ValueConversionFailed(e) => {
                    spin::postgres3_0_0::postgres::Error::ValueConversionFailed(e)
                }
                spin::postgres4_0_0::postgres::Error::Other(e) => {
                    spin::postgres3_0_0::postgres::Error::Other(e)
                }
            }
        }
    }
}

mod postgres {
    use super::*;

    impl From<spin::postgres4_0_0::postgres::RowSet> for v1::postgres::RowSet {
        fn from(value: spin::postgres4_0_0::postgres::RowSet) -> v1::postgres::RowSet {
            v1::mysql::RowSet {
                columns: value.columns.into_iter().map(Into::into).collect(),
                rows: value
                    .rows
                    .into_iter()
                    .map(|r| r.into_iter().map(Into::into).collect())
                    .collect(),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::RowSet> for v2::rdbms_types::RowSet {
        fn from(value: spin::postgres4_0_0::postgres::RowSet) -> v2::rdbms_types::RowSet {
            v2::rdbms_types::RowSet {
                columns: value.columns.into_iter().map(Into::into).collect(),
                rows: value
                    .rows
                    .into_iter()
                    .map(|r| r.into_iter().map(Into::into).collect())
                    .collect(),
            }
        }
    }

    impl From<spin::postgres4_0_0::postgres::RowSet> for spin::postgres3_0_0::postgres::RowSet {
        fn from(
            value: spin::postgres4_0_0::postgres::RowSet,
        ) -> spin::postgres3_0_0::postgres::RowSet {
            spin::postgres3_0_0::postgres::RowSet {
                columns: value.columns.into_iter().map(Into::into).collect(),
                rows: value
                    .rows
                    .into_iter()
                    .map(|r| r.into_iter().map(Into::into).collect())
                    .collect(),
            }
        }
    }
}

mod mysql {
    use super::*;
    impl From<v2::mysql::RowSet> for v1::mysql::RowSet {
        fn from(value: v2::mysql::RowSet) -> v1::mysql::RowSet {
            v1::mysql::RowSet {
                columns: value.columns.into_iter().map(Into::into).collect(),
                rows: value
                    .rows
                    .into_iter()
                    .map(|r| r.into_iter().map(Into::into).collect())
                    .collect(),
            }
        }
    }
}

mod redis {
    use super::*;

    impl From<v1::redis::RedisParameter> for v2::redis::RedisParameter {
        fn from(value: v1::redis::RedisParameter) -> Self {
            match value {
                v1::redis::RedisParameter::Int64(i) => v2::redis::RedisParameter::Int64(i),
                v1::redis::RedisParameter::Binary(b) => v2::redis::RedisParameter::Binary(b),
            }
        }
    }

    impl From<v2::redis::RedisResult> for v1::redis::RedisResult {
        fn from(value: v2::redis::RedisResult) -> Self {
            match value {
                v2::redis::RedisResult::Nil => v1::redis::RedisResult::Nil,
                v2::redis::RedisResult::Status(s) => v1::redis::RedisResult::Status(s),
                v2::redis::RedisResult::Int64(i) => v1::redis::RedisResult::Int64(i),
                v2::redis::RedisResult::Binary(b) => v1::redis::RedisResult::Binary(b),
            }
        }
    }
}

mod llm {
    use super::*;

    impl From<v1::llm::InferencingParams> for v2::llm::InferencingParams {
        fn from(value: v1::llm::InferencingParams) -> Self {
            Self {
                max_tokens: value.max_tokens,
                repeat_penalty: value.repeat_penalty,
                repeat_penalty_last_n_token_count: value.repeat_penalty_last_n_token_count,
                temperature: value.temperature,
                top_k: value.top_k,
                top_p: value.top_p,
            }
        }
    }

    impl From<v2::llm::InferencingResult> for v1::llm::InferencingResult {
        fn from(value: v2::llm::InferencingResult) -> Self {
            Self {
                text: value.text,
                usage: v1::llm::InferencingUsage {
                    prompt_token_count: value.usage.prompt_token_count,
                    generated_token_count: value.usage.generated_token_count,
                },
            }
        }
    }

    impl From<v2::llm::EmbeddingsResult> for v1::llm::EmbeddingsResult {
        fn from(value: v2::llm::EmbeddingsResult) -> Self {
            Self {
                embeddings: value.embeddings,
                usage: v1::llm::EmbeddingsUsage {
                    prompt_token_count: value.usage.prompt_token_count,
                },
            }
        }
    }

    impl From<v2::llm::Error> for v1::llm::Error {
        fn from(value: v2::llm::Error) -> Self {
            match value {
                v2::llm::Error::ModelNotSupported => Self::ModelNotSupported,
                v2::llm::Error::RuntimeError(s) => Self::RuntimeError(s),
                v2::llm::Error::InvalidInput(s) => Self::InvalidInput(s),
            }
        }
    }
}
