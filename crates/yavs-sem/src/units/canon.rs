//! Values compared by content (`canon`): maps in key order, so that two maps with the same
//! entries compare equal whatever order they were filled in (`std` hash maps also iterate in a
//! random order per map).

use serde::Serialize;
use serde::ser::{self, Impossible};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum V {
    Unit,
    Bool(bool),
    U(u64),
    I(i64),
    /// The bits of a float.
    F(u64),
    Str(String),
    Bytes(Vec<u8>),
    None,
    Some(Box<V>),
    Variant(u32, Box<V>),
    Seq(Vec<V>),
    Map(Vec<(V, V)>),
}

#[derive(Debug)]
pub(crate) struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<M: std::fmt::Display>(m: M) -> Self {
        Error(m.to_string())
    }
}

/// The content of `t`: symbols by their number in this process, ids as they are.
pub(crate) fn canon<T: Serialize + ?Sized>(t: &T) -> Result<V, String> {
    crate::ids::plain(|| yavs_syntax::raw_symbols(|| t.serialize(Canon))).map_err(|e| e.0)
}

struct Canon;

pub(crate) struct Seq(Vec<V>, Option<u32>);

pub(crate) struct Map(Vec<(V, V)>, Option<V>);

impl ser::Serializer for Canon {
    type Ok = V;
    type Error = Error;
    type SerializeSeq = Seq;
    type SerializeTuple = Seq;
    type SerializeTupleStruct = Seq;
    type SerializeTupleVariant = Seq;
    type SerializeMap = Map;
    type SerializeStruct = Seq;
    type SerializeStructVariant = Seq;

    fn serialize_bool(self, v: bool) -> Result<V, Error> {
        Ok(V::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<V, Error> {
        Ok(V::I(v.into()))
    }
    fn serialize_i16(self, v: i16) -> Result<V, Error> {
        Ok(V::I(v.into()))
    }
    fn serialize_i32(self, v: i32) -> Result<V, Error> {
        Ok(V::I(v.into()))
    }
    fn serialize_i64(self, v: i64) -> Result<V, Error> {
        Ok(V::I(v))
    }
    fn serialize_u8(self, v: u8) -> Result<V, Error> {
        Ok(V::U(v.into()))
    }
    fn serialize_u16(self, v: u16) -> Result<V, Error> {
        Ok(V::U(v.into()))
    }
    fn serialize_u32(self, v: u32) -> Result<V, Error> {
        Ok(V::U(v.into()))
    }
    fn serialize_u64(self, v: u64) -> Result<V, Error> {
        Ok(V::U(v))
    }
    fn serialize_f32(self, v: f32) -> Result<V, Error> {
        Ok(V::F(f64::from(v).to_bits()))
    }
    fn serialize_f64(self, v: f64) -> Result<V, Error> {
        Ok(V::F(v.to_bits()))
    }
    fn serialize_char(self, v: char) -> Result<V, Error> {
        Ok(V::Str(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<V, Error> {
        Ok(V::Str(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<V, Error> {
        Ok(V::Bytes(v.to_vec()))
    }
    fn serialize_none(self) -> Result<V, Error> {
        Ok(V::None)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, v: &T) -> Result<V, Error> {
        Ok(V::Some(Box::new(v.serialize(Canon)?)))
    }
    fn serialize_unit(self) -> Result<V, Error> {
        Ok(V::Unit)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<V, Error> {
        Ok(V::Unit)
    }
    fn serialize_unit_variant(self, _: &'static str, i: u32, _: &'static str) -> Result<V, Error> {
        Ok(V::Variant(i, Box::new(V::Unit)))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<V, Error> {
        v.serialize(Canon)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        i: u32,
        _: &'static str,
        v: &T,
    ) -> Result<V, Error> {
        Ok(V::Variant(i, Box::new(v.serialize(Canon)?)))
    }
    fn serialize_seq(self, n: Option<usize>) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n.unwrap_or(0)), None))
    }
    fn serialize_tuple(self, n: usize) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n), None))
    }
    fn serialize_tuple_struct(self, _: &'static str, n: usize) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n), None))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        i: u32,
        _: &'static str,
        n: usize,
    ) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n), Some(i)))
    }
    fn serialize_map(self, n: Option<usize>) -> Result<Map, Error> {
        Ok(Map(Vec::with_capacity(n.unwrap_or(0)), None))
    }
    fn serialize_struct(self, _: &'static str, n: usize) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n), None))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        i: u32,
        _: &'static str,
        n: usize,
    ) -> Result<Seq, Error> {
        Ok(Seq(Vec::with_capacity(n), Some(i)))
    }
}

impl Seq {
    fn push<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.0.push(v.serialize(Canon)?);
        Ok(())
    }
    fn done(self) -> Result<V, Error> {
        let s = V::Seq(self.0);
        Ok(match self.1 {
            Some(i) => V::Variant(i, Box::new(s)),
            None => s,
        })
    }
}

impl ser::SerializeSeq for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeTuple for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeTupleStruct for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeTupleVariant for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeStruct for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeStructVariant for Seq {
    type Ok = V;
    type Error = Error;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        v: &T,
    ) -> Result<(), Error> {
        self.push(v)
    }
    fn end(self) -> Result<V, Error> {
        self.done()
    }
}

impl ser::SerializeMap for Map {
    type Ok = V;
    type Error = Error;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, k: &T) -> Result<(), Error> {
        self.1 = Some(k.serialize(Canon)?);
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, v: &T) -> Result<(), Error> {
        let k = self
            .1
            .take()
            .ok_or_else(|| Error("value without key".into()))?;
        self.0.push((k, v.serialize(Canon)?));
        Ok(())
    }
    fn end(mut self) -> Result<V, Error> {
        self.0.sort();
        Ok(V::Map(self.0))
    }
}

/// Where `a` and `b` first differ, for messages.
pub(crate) fn first_diff(a: &V, b: &V) -> String {
    match (a, b) {
        (V::Seq(x), V::Seq(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                if p != q {
                    return format!("[{i}]{}", first_diff(p, q));
                }
            }
            String::new()
        }
        (V::Map(x), V::Map(y)) => {
            for (k, v) in x {
                match y.iter().find(|e| e.0 == *k) {
                    None => return format!(" key {k:?} only here"),
                    Some((_, w)) if w != v => return format!("{{{k:?}}}{}", first_diff(v, w)),
                    _ => {}
                }
            }
            for (k, _) in y {
                if !x.iter().any(|e| e.0 == *k) {
                    return format!(" key {k:?} only there");
                }
            }
            String::new()
        }
        (V::Some(p), V::Some(q)) => first_diff(p, q),
        (V::Variant(i, p), V::Variant(j, q)) if i == j => format!("<{i}>{}", first_diff(p, q)),
        _ => {
            let (mut x, mut y) = (format!("{a:?}"), format!("{b:?}"));
            x.truncate(300);
            y.truncate(300);
            format!(": {x} vs {y}")
        }
    }
}

// Never used: the compound types above are the only ones `Canon` makes.
#[allow(dead_code)]
type Unused = Impossible<V, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_compare_by_content() {
        let mut a = std::collections::HashMap::new();
        let mut b = std::collections::HashMap::new();
        for i in 0..100u32 {
            a.insert(i, i * 2);
        }
        for i in (0..100u32).rev() {
            b.insert(i, i * 2);
        }
        assert_eq!(canon(&a), canon(&b));
        b.insert(7, 0);
        assert_ne!(canon(&a), canon(&b));
    }
}
