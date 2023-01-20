use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Blobs {
    name: String,
    pub(crate) blobs: Vec<Blob>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct Blob {
    pub(crate) name: String,
    pub(crate) hash: [u8; 32],
}

impl Blobs {
    pub fn new<I: Into<String>>(name: I) -> Self {
        Self {
            name: name.into(),
            blobs: Vec::new(),
        }
    }

    pub fn push(&mut self, b: Blob) {
        self.blobs.push(b);
    }

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
}
