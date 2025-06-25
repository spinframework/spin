use super::*;

mod rdbms_types {
    use super::*;
    use spin::postgres3_0_0::postgres as pg3;
    use spin::postgres4_0_0::postgres as pg4;

    impl From<v2::rdbms_types::Column> for v1::rdbms_types::Column {
        fn from(value: v2::rdbms_types::Column) -> Self {
            v1::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<pg4::Column> for v1::rdbms_types::Column {
        fn from(value: spin::postgres4_0_0::postgres::Column) -> Self {
            v1::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<pg4::Column> for v2::rdbms_types::Column {
        fn from(value: pg4::Column) -> Self {
            v2::rdbms_types::Column {
                name: value.name,
                data_type: value.data_type.into(),
            }
        }
    }

    impl From<pg4::Column> for pg3::Column {
        fn from(value: pg4::Column) -> Self {
            pg3::Column {
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

    impl From<pg4::DbValue> for v1::rdbms_types::DbValue {
        fn from(value: pg4::DbValue) -> v1::rdbms_types::DbValue {
            match value {
                pg4::DbValue::Boolean(b) => v1::rdbms_types::DbValue::Boolean(b),
                pg4::DbValue::Int8(i) => v1::rdbms_types::DbValue::Int8(i),
                pg4::DbValue::Int16(i) => v1::rdbms_types::DbValue::Int16(i),
                pg4::DbValue::Int32(i) => v1::rdbms_types::DbValue::Int32(i),
                pg4::DbValue::Int64(i) => v1::rdbms_types::DbValue::Int64(i),
                pg4::DbValue::Floating32(r) => v1::rdbms_types::DbValue::Floating32(r),
                pg4::DbValue::Floating64(r) => v1::rdbms_types::DbValue::Floating64(r),
                pg4::DbValue::Str(s) => v1::rdbms_types::DbValue::Str(s),
                pg4::DbValue::Binary(b) => v1::rdbms_types::DbValue::Binary(b),
                pg4::DbValue::DbNull => v1::rdbms_types::DbValue::DbNull,
                pg4::DbValue::Unsupported => v1::rdbms_types::DbValue::Unsupported,
                _ => v1::rdbms_types::DbValue::Unsupported,
            }
        }
    }

    impl From<pg4::DbValue> for v2::rdbms_types::DbValue {
        fn from(value: pg4::DbValue) -> v2::rdbms_types::DbValue {
            match value {
                pg4::DbValue::Boolean(b) => v2::rdbms_types::DbValue::Boolean(b),
                pg4::DbValue::Int8(i) => v2::rdbms_types::DbValue::Int8(i),
                pg4::DbValue::Int16(i) => v2::rdbms_types::DbValue::Int16(i),
                pg4::DbValue::Int32(i) => v2::rdbms_types::DbValue::Int32(i),
                pg4::DbValue::Int64(i) => v2::rdbms_types::DbValue::Int64(i),
                pg4::DbValue::Floating32(r) => v2::rdbms_types::DbValue::Floating32(r),
                pg4::DbValue::Floating64(r) => v2::rdbms_types::DbValue::Floating64(r),
                pg4::DbValue::Str(s) => v2::rdbms_types::DbValue::Str(s),
                pg4::DbValue::Binary(b) => v2::rdbms_types::DbValue::Binary(b),
                pg4::DbValue::DbNull => v2::rdbms_types::DbValue::DbNull,
                pg4::DbValue::Unsupported => v2::rdbms_types::DbValue::Unsupported,
                _ => v2::rdbms_types::DbValue::Unsupported,
            }
        }
    }

    impl From<pg4::DbValue> for pg3::DbValue {
        fn from(value: pg4::DbValue) -> pg3::DbValue {
            match value {
                pg4::DbValue::Boolean(b) => pg3::DbValue::Boolean(b),
                pg4::DbValue::Int8(i) => pg3::DbValue::Int8(i),
                pg4::DbValue::Int16(i) => pg3::DbValue::Int16(i),
                pg4::DbValue::Int32(i) => pg3::DbValue::Int32(i),
                pg4::DbValue::Int64(i) => pg3::DbValue::Int64(i),
                pg4::DbValue::Floating32(r) => pg3::DbValue::Floating32(r),
                pg4::DbValue::Floating64(r) => pg3::DbValue::Floating64(r),
                pg4::DbValue::Str(s) => pg3::DbValue::Str(s),
                pg4::DbValue::Binary(b) => pg3::DbValue::Binary(b),
                pg4::DbValue::Date(d) => pg3::DbValue::Date(d),
                pg4::DbValue::Datetime(dt) => pg3::DbValue::Datetime(dt),
                pg4::DbValue::Time(t) => pg3::DbValue::Time(t),
                pg4::DbValue::Timestamp(t) => pg3::DbValue::Timestamp(t),
                pg4::DbValue::Uuid(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::Jsonb(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::Decimal(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::Range32(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::Range64(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::ArrayInt32(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::ArrayInt64(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::ArrayStr(_) => pg3::DbValue::Unsupported,
                pg4::DbValue::DbNull => pg3::DbValue::DbNull,
                pg4::DbValue::Unsupported => pg3::DbValue::Unsupported,
            }
        }
    }

    impl From<pg4::DbDataType> for v1::rdbms_types::DbDataType {
        fn from(value: pg4::DbDataType) -> v1::rdbms_types::DbDataType {
            match value {
                pg4::DbDataType::Boolean => v1::rdbms_types::DbDataType::Boolean,
                pg4::DbDataType::Int8 => v1::rdbms_types::DbDataType::Int8,
                pg4::DbDataType::Int16 => v1::rdbms_types::DbDataType::Int16,
                pg4::DbDataType::Int32 => v1::rdbms_types::DbDataType::Int32,
                pg4::DbDataType::Int64 => v1::rdbms_types::DbDataType::Int64,
                pg4::DbDataType::Floating32 => v1::rdbms_types::DbDataType::Floating32,
                pg4::DbDataType::Floating64 => v1::rdbms_types::DbDataType::Floating64,
                pg4::DbDataType::Str => v1::rdbms_types::DbDataType::Str,
                pg4::DbDataType::Binary => v1::rdbms_types::DbDataType::Binary,
                pg4::DbDataType::Other => v1::rdbms_types::DbDataType::Other,
                _ => v1::rdbms_types::DbDataType::Other,
            }
        }
    }

    impl From<pg4::DbDataType> for v2::rdbms_types::DbDataType {
        fn from(value: pg4::DbDataType) -> v2::rdbms_types::DbDataType {
            match value {
                pg4::DbDataType::Boolean => v2::rdbms_types::DbDataType::Boolean,
                pg4::DbDataType::Int8 => v2::rdbms_types::DbDataType::Int8,
                pg4::DbDataType::Int16 => v2::rdbms_types::DbDataType::Int16,
                pg4::DbDataType::Int32 => v2::rdbms_types::DbDataType::Int32,
                pg4::DbDataType::Int64 => v2::rdbms_types::DbDataType::Int64,
                pg4::DbDataType::Floating32 => v2::rdbms_types::DbDataType::Floating32,
                pg4::DbDataType::Floating64 => v2::rdbms_types::DbDataType::Floating64,
                pg4::DbDataType::Str => v2::rdbms_types::DbDataType::Str,
                pg4::DbDataType::Binary => v2::rdbms_types::DbDataType::Binary,
                pg4::DbDataType::Other => v2::rdbms_types::DbDataType::Other,
                _ => v2::rdbms_types::DbDataType::Other,
            }
        }
    }

    impl From<pg4::DbDataType> for pg3::DbDataType {
        fn from(value: pg4::DbDataType) -> pg3::DbDataType {
            match value {
                pg4::DbDataType::Boolean => pg3::DbDataType::Boolean,
                pg4::DbDataType::Int8 => pg3::DbDataType::Int8,
                pg4::DbDataType::Int16 => pg3::DbDataType::Int16,
                pg4::DbDataType::Int32 => pg3::DbDataType::Int32,
                pg4::DbDataType::Int64 => pg3::DbDataType::Int64,
                pg4::DbDataType::Floating32 => pg3::DbDataType::Floating32,
                pg4::DbDataType::Floating64 => pg3::DbDataType::Floating64,
                pg4::DbDataType::Str => pg3::DbDataType::Str,
                pg4::DbDataType::Binary => pg3::DbDataType::Binary,
                pg4::DbDataType::Date => pg3::DbDataType::Date,
                pg4::DbDataType::Datetime => pg3::DbDataType::Datetime,
                pg4::DbDataType::Time => pg3::DbDataType::Time,
                pg4::DbDataType::Timestamp => pg3::DbDataType::Timestamp,
                pg4::DbDataType::Uuid => pg3::DbDataType::Other,
                pg4::DbDataType::Jsonb => pg3::DbDataType::Other,
                pg4::DbDataType::Decimal => pg3::DbDataType::Other,
                pg4::DbDataType::Range32 => pg3::DbDataType::Other,
                pg4::DbDataType::Range64 => pg3::DbDataType::Other,
                pg4::DbDataType::ArrayInt32 => pg3::DbDataType::Other,
                pg4::DbDataType::ArrayInt64 => pg3::DbDataType::Other,
                pg4::DbDataType::ArrayStr => pg3::DbDataType::Other,
                pg4::DbDataType::Other => pg3::DbDataType::Other,
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

    impl TryFrom<v1::rdbms_types::ParameterValue> for pg4::ParameterValue {
        type Error = v1::postgres::PgError;

        fn try_from(
            value: v1::rdbms_types::ParameterValue,
        ) -> Result<pg4::ParameterValue, Self::Error> {
            let converted = match value {
                v1::rdbms_types::ParameterValue::Boolean(b) => pg4::ParameterValue::Boolean(b),
                v1::rdbms_types::ParameterValue::Int8(i) => pg4::ParameterValue::Int8(i),
                v1::rdbms_types::ParameterValue::Int16(i) => pg4::ParameterValue::Int16(i),
                v1::rdbms_types::ParameterValue::Int32(i) => pg4::ParameterValue::Int32(i),
                v1::rdbms_types::ParameterValue::Int64(i) => pg4::ParameterValue::Int64(i),
                v1::rdbms_types::ParameterValue::Uint8(_)
                | v1::rdbms_types::ParameterValue::Uint16(_)
                | v1::rdbms_types::ParameterValue::Uint32(_)
                | v1::rdbms_types::ParameterValue::Uint64(_) => {
                    return Err(v1::postgres::PgError::ValueConversionFailed(
                        "Postgres does not support unsigned integers".to_owned(),
                    ));
                }
                v1::rdbms_types::ParameterValue::Floating32(r) => {
                    pg4::ParameterValue::Floating32(r)
                }
                v1::rdbms_types::ParameterValue::Floating64(r) => {
                    pg4::ParameterValue::Floating64(r)
                }
                v1::rdbms_types::ParameterValue::Str(s) => pg4::ParameterValue::Str(s),
                v1::rdbms_types::ParameterValue::Binary(b) => pg4::ParameterValue::Binary(b),
                v1::rdbms_types::ParameterValue::DbNull => pg4::ParameterValue::DbNull,
            };
            Ok(converted)
        }
    }

    impl TryFrom<v2::rdbms_types::ParameterValue> for pg4::ParameterValue {
        type Error = v2::rdbms_types::Error;

        fn try_from(
            value: v2::rdbms_types::ParameterValue,
        ) -> Result<pg4::ParameterValue, Self::Error> {
            let converted = match value {
                v2::rdbms_types::ParameterValue::Boolean(b) => pg4::ParameterValue::Boolean(b),
                v2::rdbms_types::ParameterValue::Int8(i) => pg4::ParameterValue::Int8(i),
                v2::rdbms_types::ParameterValue::Int16(i) => pg4::ParameterValue::Int16(i),
                v2::rdbms_types::ParameterValue::Int32(i) => pg4::ParameterValue::Int32(i),
                v2::rdbms_types::ParameterValue::Int64(i) => pg4::ParameterValue::Int64(i),
                v2::rdbms_types::ParameterValue::Uint8(_)
                | v2::rdbms_types::ParameterValue::Uint16(_)
                | v2::rdbms_types::ParameterValue::Uint32(_)
                | v2::rdbms_types::ParameterValue::Uint64(_) => {
                    return Err(v2::rdbms_types::Error::ValueConversionFailed(
                        "Postgres does not support unsigned integers".to_owned(),
                    ));
                }
                v2::rdbms_types::ParameterValue::Floating32(r) => {
                    pg4::ParameterValue::Floating32(r)
                }
                v2::rdbms_types::ParameterValue::Floating64(r) => {
                    pg4::ParameterValue::Floating64(r)
                }
                v2::rdbms_types::ParameterValue::Str(s) => pg4::ParameterValue::Str(s),
                v2::rdbms_types::ParameterValue::Binary(b) => pg4::ParameterValue::Binary(b),
                v2::rdbms_types::ParameterValue::DbNull => pg4::ParameterValue::DbNull,
            };
            Ok(converted)
        }
    }

    impl From<pg3::ParameterValue> for pg4::ParameterValue {
        fn from(value: pg3::ParameterValue) -> pg4::ParameterValue {
            match value {
                pg3::ParameterValue::Boolean(b) => pg4::ParameterValue::Boolean(b),
                pg3::ParameterValue::Int8(i) => pg4::ParameterValue::Int8(i),
                pg3::ParameterValue::Int16(i) => pg4::ParameterValue::Int16(i),
                pg3::ParameterValue::Int32(i) => pg4::ParameterValue::Int32(i),
                pg3::ParameterValue::Int64(i) => pg4::ParameterValue::Int64(i),
                pg3::ParameterValue::Floating32(r) => pg4::ParameterValue::Floating32(r),
                pg3::ParameterValue::Floating64(r) => pg4::ParameterValue::Floating64(r),
                pg3::ParameterValue::Str(s) => pg4::ParameterValue::Str(s),
                pg3::ParameterValue::Binary(b) => pg4::ParameterValue::Binary(b),
                pg3::ParameterValue::Date(d) => pg4::ParameterValue::Date(d),
                pg3::ParameterValue::Datetime(dt) => pg4::ParameterValue::Datetime(dt),
                pg3::ParameterValue::Time(t) => pg4::ParameterValue::Time(t),
                pg3::ParameterValue::Timestamp(t) => pg4::ParameterValue::Timestamp(t),
                pg3::ParameterValue::DbNull => pg4::ParameterValue::DbNull,
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

    impl From<pg4::Error> for v1::postgres::PgError {
        fn from(error: pg4::Error) -> v1::postgres::PgError {
            match error {
                pg4::Error::ConnectionFailed(e) => v1::postgres::PgError::ConnectionFailed(e),
                pg4::Error::BadParameter(e) => v1::postgres::PgError::BadParameter(e),
                pg4::Error::QueryFailed(e) => v1::postgres::PgError::QueryFailed(e),
                pg4::Error::ValueConversionFailed(e) => {
                    v1::postgres::PgError::ValueConversionFailed(e)
                }
                pg4::Error::Other(e) => v1::postgres::PgError::OtherError(e),
            }
        }
    }

    impl From<pg4::Error> for v2::rdbms_types::Error {
        fn from(error: pg4::Error) -> v2::rdbms_types::Error {
            match error {
                pg4::Error::ConnectionFailed(e) => v2::rdbms_types::Error::ConnectionFailed(e),
                pg4::Error::BadParameter(e) => v2::rdbms_types::Error::BadParameter(e),
                pg4::Error::QueryFailed(e) => v2::rdbms_types::Error::QueryFailed(e),
                pg4::Error::ValueConversionFailed(e) => {
                    v2::rdbms_types::Error::ValueConversionFailed(e)
                }
                pg4::Error::Other(e) => v2::rdbms_types::Error::Other(e),
            }
        }
    }

    impl From<pg4::Error> for pg3::Error {
        fn from(error: pg4::Error) -> pg3::Error {
            match error {
                pg4::Error::ConnectionFailed(e) => pg3::Error::ConnectionFailed(e),
                pg4::Error::BadParameter(e) => pg3::Error::BadParameter(e),
                pg4::Error::QueryFailed(e) => pg3::Error::QueryFailed(e),
                pg4::Error::ValueConversionFailed(e) => pg3::Error::ValueConversionFailed(e),
                pg4::Error::Other(e) => pg3::Error::Other(e),
            }
        }
    }
}

mod postgres {
    use super::*;
    use spin::postgres3_0_0::postgres as pg3;
    use spin::postgres4_0_0::postgres as pg4;

    impl From<pg4::RowSet> for v1::postgres::RowSet {
        fn from(value: pg4::RowSet) -> v1::postgres::RowSet {
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

    impl From<pg4::RowSet> for v2::rdbms_types::RowSet {
        fn from(value: pg4::RowSet) -> v2::rdbms_types::RowSet {
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

    impl From<pg4::RowSet> for pg3::RowSet {
        fn from(value: pg4::RowSet) -> pg3::RowSet {
            pg3::RowSet {
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
