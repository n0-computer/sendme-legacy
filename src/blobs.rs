use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Collection {
    /// The name of this collection
    pub(crate) name: String,
    /// Links to the blobs in this collection
    pub(crate) blobs: Vec<Blob>,
    /// The total size of the raw_data referred to by all links
    pub(crate) total_blobs_size: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
// #[serde(from = "SerdeBlob")]
// #[serde(into = "SerdeBlob")]
pub(crate) struct Blob {
    /// The name of this blob of data
    pub(crate) name: String,
    /// The hash of the blob of data
    #[serde(with = "hash_serde")]
    pub(crate) hash: bao::Hash,
}

#[derive(Deserialize, Serialize, Clone)]
struct SerdeBlob {
    name: String,
    hash: [u8; 32],
}

impl From<Blob> for SerdeBlob {
    fn from(b: Blob) -> Self {
        Self {
            name: b.name,
            hash: *b.hash.as_bytes(),
        }
    }
}

impl From<SerdeBlob> for Blob {
    fn from(s: SerdeBlob) -> Self {
        Self {
            name: s.name,
            hash: bao::Hash::from(s.hash),
        }
    }
}

mod hash_serde {
    use serde::{de, Deserializer, Serializer};

    pub fn serialize<S>(h: &bao::Hash, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_bytes(h.as_bytes())
    }

    pub fn deserialize<'de, D>(d: D) -> Result<bao::Hash, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HashVisitor;

        impl<'de> de::Visitor<'de> for HashVisitor {
            type Value = bao::Hash;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an array of bytes containing hash data")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let b: [u8; 32] = v.try_into().map_err(E::custom)?;
                Ok(bao::Hash::from(b))
            }
        }

        d.deserialize_bytes(HashVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_blob() {
        let b = Blob {
            name: "test".to_string(),
            hash: bao::Hash::from_hex(
                "3aa61c409fd7717c9d9c639202af2fae470c0ef669be7ba2caea5779cb534e9d",
            )
            .unwrap(),
        };

        let mut buf = bytes::BytesMut::zeroed(1024);
        postcard::to_slice(&b, &mut buf).unwrap();
        let deserialize_b: Blob = postcard::from_bytes(&buf).unwrap();
        assert_eq!(b, deserialize_b);
    }
}
