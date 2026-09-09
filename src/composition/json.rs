//! Trust-boundary checks before serde can discard duplicate keys.

use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::{CheckError, wire};

pub(super) fn parse(bytes: &[u8]) -> Result<Value, CheckError> {
    if bytes.len() > wire::MAX_BYTES {
        return Err(CheckError::InvalidJson);
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue { depth: 0 }
        .deserialize(&mut decoder)
        .map_err(|_| CheckError::InvalidJson)?;
    decoder.end().map_err(|_| CheckError::InvalidJson)?;
    Ok(value)
}

struct StrictValue {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for StrictValue {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
        if self.depth > wire::MAX_DEPTH {
            return Err(D::Error::custom("JSON depth budget"));
        }
        decoder.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for StrictValue {
    type Value = Value;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded JSON without duplicate keys")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Value, M::Error> {
        let mut result = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if result.contains_key(&key) {
                return Err(M::Error::custom("duplicate JSON key"));
            }
            let value = access.next_value_seed(Self {
                depth: self.depth + 1,
            })?;
            result.insert(key, value);
        }
        Ok(Value::Object(result))
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut access: S) -> Result<Value, S::Error> {
        let mut result = Vec::new();
        while let Some(value) = access.next_element_seed(Self {
            depth: self.depth + 1,
        })? {
            result.push(value);
        }
        Ok(Value::Array(result))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Value, E> {
        if value.unsigned_abs() > 9_007_199_254_740_991 {
            return Err(E::custom("unsafe JSON number"));
        }
        Ok(Value::Number(Number::from(value)))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Value, E> {
        if value > 9_007_199_254_740_991 {
            return Err(E::custom("unsafe JSON number"));
        }
        Ok(Value::Number(Number::from(value)))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Value, E> {
        if value.abs() > 9_007_199_254_740_991.0 {
            return Err(E::custom("unsafe JSON number"));
        }
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }
}
