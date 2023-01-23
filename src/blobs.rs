use serde::{Deserialize, Serialize};

// TODO(ramfox): we can't use max size b/c of the strings
// right now I'm doing a weird calculation to estimate the buffer size
// but we could limit the # of chars in a filename instead
// use postcard::experimental::max_size::MaxSize;

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
