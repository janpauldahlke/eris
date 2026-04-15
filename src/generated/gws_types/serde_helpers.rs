//! serde helpers for Google JSON string encodings.

pub fn string_to_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct I64Visitor;

    impl<'de> Visitor<'de> for I64Visitor {
        type Value = i64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string or integer i64")
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<i64, E> {
            v.parse().map_err(Error::custom)
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<i64, E> {
            Ok(v)
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<i64, E> {
            i64::try_from(v).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(I64Visitor)
}

pub fn string_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct U64Visitor;

    impl<'de> Visitor<'de> for U64Visitor {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a string or integer u64")
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<u64, E> {
            v.parse().map_err(Error::custom)
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<u64, E> {
            u64::try_from(v).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(U64Visitor)
}

/// Deserialize `Option<i64>` from JSON null, missing, string, or integer (Gmail `internalDate` as string).
pub fn deserialize_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct OptVisitor;

    impl<'de> Visitor<'de> for OptVisitor {
        type Value = Option<i64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, string, or integer for optional i64")
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            string_to_i64(deserializer).map(Some)
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
            i64::try_from(v).map(Some).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(OptVisitor)
}

/// Deserialize `Option<u64>` from JSON null, string, or integer (Gmail `historyId` as string).
pub fn deserialize_opt_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct OptVisitor;

    impl<'de> Visitor<'de> for OptVisitor {
        type Value = Option<u64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, string, or integer for optional u64")
        }

        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: serde::Deserializer<'de>,
        {
            string_to_u64(deserializer).map(Some)
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
            u64::try_from(v).map(Some).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(OptVisitor)
}

/// Google `format: byte` fields are base64url or standard base64 strings.
pub fn deserialize_bytes_base64<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use base64::Engine;
    use serde::Deserialize;
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(s) => base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}
