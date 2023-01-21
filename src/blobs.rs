use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Blobs {
    /// The name of this collection
    pub(crate) name: String,
    /// Links to other blobs in this collection
    pub(crate) blobs: Vec<Blob>,
    /// The total size of the raw_data referred to by all links
    pub(crate) raw_size: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Blob {
    /// The name of this blob of data
    pub(crate) name: String,
    /// The hash of the blob of data
    pub(crate) hash: [u8; 32],
}

impl Blobs {
    /// The length of the Collection structure itself.
    ///
    /// **Not** the length of the raw data refered to by this collection of links.
    pub fn len(&self) -> usize {
        let mut len = self.name.len();
        // TODO: should we just estimate this?
        // e/g, expect name to be max 400 bytes (4 chars per byte * 100 chars)
        // 400 * self.blobs.len()??
        for blob in &self.blobs {
            len += blob.name.len() + 32;
        }
        len
    }

    /// The size of the raw data refered to by this collection of links
    pub fn raw_data_size(&self) -> usize {
        self.raw_size
    }
}
