//! Bytes shared by several holders, and ranges of them that deserializers hand out instead of
//! copies (the compilation cache decodes straight from a memory-mapped entry,
//! `docs/decisions.md` "Zero-copy cache entries").

use std::sync::Arc;

/// An immutable byte buffer many values may keep a range of: a mapped cache entry, or a `Vec`.
#[derive(Clone)]
pub struct Buffer(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl Buffer {
    pub fn new(bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> Buffer {
        Buffer(Arc::new(bytes))
    }
}

impl std::ops::Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Buffer({} bytes)", self.len())
    }
}

// The buffer being deserialized from, so that byte strings can keep a range of it.
thread_local! {
    static SOURCE: std::cell::RefCell<Option<Buffer>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` (which deserializes [`Slice`]s from `buf`'s bytes) so that they share `buf`.
pub fn sharing<R>(buf: &Buffer, f: impl FnOnce() -> R) -> R {
    let before = SOURCE.with(|s| s.borrow_mut().replace(buf.clone()));
    let r = f();
    SOURCE.with(|s| *s.borrow_mut() = before);
    r
}

/// A range of a shared [`Buffer`].
#[derive(Clone, Debug)]
pub struct Slice {
    buf: Buffer,
    start: usize,
    end: usize,
}

impl Slice {
    /// Bytes `start..end` of `buf`.
    pub fn of(buf: &Buffer, start: usize, end: usize) -> Slice {
        assert!(start <= end && end <= buf.len(), "slice out of its buffer");
        Slice {
            buf: buf.clone(),
            start,
            end,
        }
    }

    /// Bytes `start..end` of this slice.
    pub fn range(&self, start: usize, end: usize) -> Slice {
        assert!(start <= end && end <= self.len(), "slice out of its slice");
        Slice {
            buf: self.buf.clone(),
            start: self.start + start,
            end: self.start + end,
        }
    }

    pub fn get(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

impl From<Vec<u8>> for Slice {
    fn from(v: Vec<u8>) -> Slice {
        let end = v.len();
        Slice {
            buf: Buffer::new(v),
            start: 0,
            end,
        }
    }
}

impl serde::Serialize for Slice {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(self.get())
    }
}

/// A byte string, deserialized in one piece (not byte by byte as a `Vec<u8>` would be): a range
/// of the buffer set by [`sharing`] when the bytes lie in it, else a copy.
impl<'de> serde::Deserialize<'de> for Slice {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Slice;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_borrowed_bytes<E: serde::de::Error>(self, v: &'de [u8]) -> Result<Slice, E> {
                let at = SOURCE.with(|s| {
                    let s = s.borrow();
                    let buf = s.as_ref()?;
                    let start = (v.as_ptr() as usize).checked_sub(buf.as_ptr() as usize)?;
                    (start + v.len() <= buf.len()).then(|| (buf.clone(), start))
                });
                Ok(match at {
                    Some((buf, start)) => Slice {
                        buf,
                        start,
                        end: start + v.len(),
                    },
                    None => self.visit_bytes(v)?,
                })
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Slice, E> {
                Ok(v.to_vec().into())
            }
        }
        d.deserialize_bytes(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Two(Slice, Slice);

    #[test]
    fn slices_share_the_buffer_they_are_read_from() {
        let bytes = postcard::to_stdvec(&Two(vec![1, 2, 3].into(), vec![4, 5].into())).unwrap();
        let buf = Buffer::new(bytes);
        let Two(a, b) = sharing(&buf, || postcard::from_bytes::<Two>(&buf)).unwrap();
        assert_eq!((a.get(), b.get()), (&[1, 2, 3][..], &[4, 5][..]));
        // Ranges of the buffer: no copy.
        let at = |s: &Slice| s.get().as_ptr() as usize - buf.as_ptr() as usize;
        assert!(at(&a) < buf.len() && at(&b) < buf.len());
        // Without a buffer to share the bytes are copied and still right.
        let Two(c, d) = postcard::from_bytes::<Two>(&buf).unwrap();
        assert_eq!((c.get(), d.get()), (&[1, 2, 3][..], &[4, 5][..]));
        let p = c.get().as_ptr() as usize;
        assert!(!(buf.as_ptr() as usize..buf.as_ptr() as usize + buf.len()).contains(&p));
    }
}
