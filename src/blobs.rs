use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Collection {
    /// The name of this collection
    pub(crate) name: String,
    /// Links to the blobs in this collection
    pub(crate) blobs: Vec<Blob>,
    /// The total size of the raw_data referred to by all links
    pub(crate) raw_size: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SerdeBlob")]
#[serde(into = "SerdeBlob")]
pub(crate) struct Blob {
    /// The name of this blob of data
    pub(crate) name: String,
    /// The hash of the blob of data
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
    fn from(d: SerdeBlob) -> Self {
        Self {
            name: d.name,
            hash: bao::Hash::from(d.hash),
        }
    }
}
