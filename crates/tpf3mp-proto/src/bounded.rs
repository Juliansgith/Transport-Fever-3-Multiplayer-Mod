use std::{fmt, marker::PhantomData, ops::Deref};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, SeqAccess, Visitor},
};
use thiserror::Error;

/// A list of at most `MAX` items. Encodes as a plain sequence; decoding
/// refuses a longer one before allocating for it, so a peer cannot make the
/// receiver reserve memory for a length it merely claims.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedVec<T, const MAX: usize>(Vec<T>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("list of {len} items; the limit is {max}")]
pub struct TooMany {
    pub len: usize,
    pub max: usize,
}

impl<T, const MAX: usize> BoundedVec<T, MAX> {
    pub fn new(items: Vec<T>) -> Result<Self, TooMany> {
        if items.len() > MAX {
            return Err(TooMany {
                len: items.len(),
                max: MAX,
            });
        }
        Ok(Self(items))
    }

    pub fn empty() -> Self {
        Self(Vec::new())
    }

    pub fn into_inner(self) -> Vec<T> {
        self.0
    }
}

impl<T, const MAX: usize> Default for BoundedVec<T, MAX> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T, const MAX: usize> Deref for BoundedVec<T, MAX> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T, const MAX: usize> TryFrom<Vec<T>> for BoundedVec<T, MAX> {
    type Error = TooMany;

    fn try_from(items: Vec<T>) -> Result<Self, TooMany> {
        Self::new(items)
    }
}

impl<T: Serialize, const MAX: usize> Serialize for BoundedVec<T, MAX> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for BoundedVec<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SeqVisitor<T, const MAX: usize>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for SeqVisitor<T, MAX> {
            type Value = BoundedVec<T, MAX>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a list of at most {MAX} items")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let claimed = seq.size_hint().unwrap_or(0);
                if claimed > MAX {
                    return Err(de::Error::invalid_length(claimed, &self));
                }
                let mut items = Vec::with_capacity(claimed);
                while let Some(item) = seq.next_element()? {
                    if items.len() == MAX {
                        return Err(de::Error::invalid_length(MAX + 1, &self));
                    }
                    items.push(item);
                }
                Ok(BoundedVec(items))
            }
        }

        deserializer.deserialize_seq(SeqVisitor::<T, MAX>(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_as_a_plain_sequence() {
        let list = BoundedVec::<u8, 4>::new(vec![1, 2, 3]).unwrap();
        assert_eq!(postcard::to_stdvec(&list).unwrap(), vec![3, 1, 2, 3]);
        assert_eq!(
            postcard::from_bytes::<BoundedVec<u8, 4>>(&[3, 1, 2, 3]).unwrap(),
            list
        );
    }

    #[test]
    fn limit_holds_both_ways() {
        assert_eq!(
            BoundedVec::<u8, 2>::new(vec![0; 3]),
            Err(TooMany { len: 3, max: 2 })
        );
        let longer = postcard::to_stdvec(&vec![0u8; 3]).unwrap();
        assert!(postcard::from_bytes::<BoundedVec<u8, 2>>(&longer).is_err());
        // A claimed length far past the limit fails before any allocation.
        let mut huge = Vec::new();
        huge.extend([0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(postcard::from_bytes::<BoundedVec<u64, 2>>(&huge).is_err());
    }
}
