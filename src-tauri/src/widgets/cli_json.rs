//! Allocation-aware JSON decoding for already stream-bounded CLI stdout.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::{
    fmt,
    io::{self, Write},
};

pub(super) const MAX_NODES: usize = 50_000;
pub(super) const MAX_DEPTH: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DecodeError {
    Invalid,
    Budget,
}

#[derive(Default)]
struct DecodeBudget {
    nodes: usize,
    exceeded: bool,
}
impl DecodeBudget {
    fn enter<E: de::Error>(&mut self, depth: usize) -> Result<(), E> {
        if depth > MAX_DEPTH || self.nodes == MAX_NODES {
            self.exceeded = true;
            return Err(E::custom("CLI JSON structure limit"));
        }
        self.nodes += 1;
        Ok(())
    }
}
struct Seed<'a> {
    budget: &'a mut DecodeBudget,
    depth: usize,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        self.budget.enter(self.depth)?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.into()))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Value, A::Error> {
        // Never trust an advertised sequence size when allocating.
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            // Keys count too, including duplicates replaced by JSON semantics.
            self.budget.enter(self.depth + 1)?;
            let value = object.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub(super) fn decode(text: &str) -> Result<Value, DecodeError> {
    let mut budget = DecodeBudget::default();
    let mut decoder = serde_json::Deserializer::from_str(text);
    let value = Seed {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(&mut decoder)
    .map_err(|_| {
        if budget.exceeded {
            DecodeError::Budget
        } else {
            DecodeError::Invalid
        }
    })?;
    decoder.end().map_err(|_| DecodeError::Invalid)?;
    Ok(value)
}

struct Counter {
    written: usize,
    remaining: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("CLI result byte limit"));
        }
        self.remaining -= bytes.len();
        self.written += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encoded_size<T: serde::Serialize + ?Sized>(value: &T, limit: usize) -> Option<usize> {
    let mut counter = Counter {
        written: 0,
        remaining: limit,
    };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn syntax_and_budget_errors_remain_distinct() {
        assert_eq!(decode("[1,"), Err(DecodeError::Invalid));
        assert_eq!(decode("[] {}"), Err(DecodeError::Invalid));
        assert_eq!(
            decode(&format!("[{}]", vec!["0"; MAX_NODES].join(","))),
            Err(DecodeError::Budget)
        );
        assert_eq!(
            decode(&format!(
                "{}0{}",
                "[".repeat(MAX_DEPTH + 1),
                "]".repeat(MAX_DEPTH + 1)
            )),
            Err(DecodeError::Budget)
        );
        assert_eq!(
            decode(r#"{"a":[1,true,null,1.5],"a":2}"#),
            Ok(json!({"a":2}))
        );
    }
    #[test]
    fn size_counter_matches_encoded_bytes_without_retaining_a_second_buffer() {
        let value = json!({"key":"\n🦀\\value"});
        let size = serde_json::to_vec(&value).unwrap().len();
        assert_eq!(encoded_size(&value, size), Some(size));
        assert_eq!(encoded_size(&value, size - 1), None);
    }
}
