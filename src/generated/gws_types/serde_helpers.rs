//! serde helpers for Google JSON string encodings.

pub fn string_to_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct OptI64Visitor;

    impl<'de> Visitor<'de> for OptI64Visitor {
        type Value = Option<i64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, a string, or integer i64")
        }

        fn visit_none<E: Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }

        fn visit_unit<E: Error>(self) -> Result<Option<i64>, E> {
            Ok(None)
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Option<i64>, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<Option<i64>, E> {
            Ok(Some(v))
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<Option<i64>, E> {
            i64::try_from(v).map(Some).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(OptI64Visitor)
}

pub fn string_to_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::de::Visitor;
    use std::fmt;

    struct OptU64Visitor;

    impl<'de> Visitor<'de> for OptU64Visitor {
        type Value = Option<u64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("null, a string, or integer u64")
        }

        fn visit_none<E: Error>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }

        fn visit_unit<E: Error>(self) -> Result<Option<u64>, E> {
            Ok(None)
        }

        fn visit_str<E: Error>(self, v: &str) -> Result<Option<u64>, E> {
            v.parse().map(Some).map_err(Error::custom)
        }

        fn visit_u64<E: Error>(self, v: u64) -> Result<Option<u64>, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: Error>(self, v: i64) -> Result<Option<u64>, E> {
            u64::try_from(v).map(Some).map_err(Error::custom)
        }
    }

    deserializer.deserialize_any(OptU64Visitor)
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
