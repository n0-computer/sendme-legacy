#[cfg(test)]
mod tests {
    use bao::decode::AsyncSliceDecoder;
    use bao::encode::SliceExtractor;
    use blake3::guts::CHUNK_LEN;
    use proptest::prelude::*;
    use std::io::{Cursor, Read};
    use tokio::io::AsyncReadExt;

    fn create_test_data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i / CHUNK_LEN) as u8).collect()
    }

    /// Encode a slice of the given data and return the hash and the encoded slice, using the bao encoder
    fn encode_slice(data: &[u8], slice_start: u64, slice_len: u64) -> (blake3::Hash, Vec<u8>) {
        let (encoded, hash) = bao::encode::encode(data);
        let mut extractor = SliceExtractor::new(Cursor::new(&encoded), slice_start, slice_len);
        let mut slice = vec![];
        extractor.read_to_end(&mut slice).unwrap();
        (hash, slice)
    }

    /// Test implementation for the test_decode_all test, to be called by both proptest and hardcoded tests
    async fn test_decode_all_async_impl(len: u64) {
        // create a slice encoding the entire data - equivalent to the bao inline encoding
        let test_data = create_test_data(len as usize);
        let (hash, encoded) = encode_slice(&test_data, 0, len);

        // test once with and once without reading size, to make sure that calling size is not required to drive
        // the internal state machine

        // test validation and reading - without reading size
        let mut cursor = std::io::Cursor::new(&encoded);
        let mut reader = AsyncSliceDecoder::new(&mut cursor, &hash, 0, len);
        let mut data = vec![];
        reader.read_to_end(&mut data).await.unwrap();
        assert_eq!(data, test_data);

        // test validation and reading - with reading size
        let mut cursor = std::io::Cursor::new(&encoded);
        let mut reader = AsyncSliceDecoder::new(&mut cursor, &hash, 0, len);
        assert_eq!(reader.read_size().await.unwrap(), test_data.len() as u64);
        let mut data = vec![];
        reader.read_to_end(&mut data).await.unwrap();
        assert_eq!(data, test_data);

        // check that we have read the entire slice
        assert_eq!(cursor.position(), encoded.len() as u64);

        // add some garbage
        let mut encoded_with_garbage = encoded.clone();
        encoded_with_garbage.extend_from_slice(vec![0u8; 1234].as_slice());
        let mut cursor = std::io::Cursor::new(&encoded_with_garbage);

        // check that reading with a size > end works
        let mut reader = AsyncSliceDecoder::new(&mut cursor, &hash, 0, u64::MAX);
        assert_eq!(reader.read_size().await.unwrap(), test_data.len() as u64);
        let mut data = vec![];
        reader.read_to_end(&mut data).await.unwrap();
        assert_eq!(data, test_data);

        // check that we have read just the encoded data, and not some extra bytes
        let inner = reader.into_inner();
        assert_eq!(inner.position(), encoded.len() as u64);
    }

    /// Test implementation for the test_decode_part test, to be called by both proptest and hardcoded tests
    async fn test_decode_part_async_impl(len: u64, slice_start: u64, slice_len: u64) {
        let test_data = create_test_data(len as usize);
        // create a slice encoding the given range
        let (hash, slice) = encode_slice(&test_data, slice_start, slice_len);
        // SliceIter::print_bao_encoded(len, slice_start..slice_start + slice_len, &slice);

        let mut cursor = std::io::Cursor::new(&slice);
        let mut reader = AsyncSliceDecoder::new(&mut cursor, &hash, slice_start, slice_len);
        assert_eq!(reader.read_size().await.unwrap(), test_data.len() as u64);
        let mut data = vec![];
        reader.read_to_end(&mut data).await.unwrap();
        // check that we have read the entire slice
        assert_eq!(cursor.position(), slice.len() as u64);
        // check that we have read the correct data
        let start = slice_start.min(len) as usize;
        let end = (slice_start + slice_len).min(len) as usize;
        assert_eq!(data, test_data[start..end]);
    }

    /// Generate a random size, start and len
    fn size_start_len() -> impl Strategy<Value = (u64, u64, u64)> {
        (0u64..65536).prop_flat_map(|size| {
            let start = 0u64..size;
            let len = 0u64..size;
            (Just(size), start, len)
        })
    }

    fn test_decode_all_impl(size: u64) {
        futures::executor::block_on(test_decode_all_async_impl(size));
    }

    fn test_decode_part_impl(size: u64, start: u64, len: u64) {
        futures::executor::block_on(test_decode_part_async_impl(size, start, len));
    }

    proptest! {
        #[test]
        fn test_decode_all(len in 1u64..32768) {
            test_decode_all_impl(len)
        }

        #[test]
        fn test_decode_part((size, start, len) in size_start_len()) {
            test_decode_part_impl(size, start, len);
        }
    }

    /// manual tests for decode_all for a few interesting cases
    #[test]
    fn test_decode_all_manual() {
        test_decode_all_impl(0);
        test_decode_all_impl(1);
        test_decode_all_impl(1024);
        test_decode_all_impl(1025);
        test_decode_all_impl(2049);
        test_decode_all_impl(12343465);
    }

    /// manual tests for decode_part for a few interesting cases
    #[test]
    fn test_decode_part_manual() {
        test_decode_part_impl(1, 0, 0);
        test_decode_part_impl(2048, 0, 1024);
        test_decode_part_impl(2048, 1024, 1024);
        test_decode_part_impl(4096, 0, 1024);
        test_decode_part_impl(4096, 1024, 1024);
        test_decode_part_impl(548, 520, 505);
        test_decode_part_impl(2126, 2048, 1);
        test_decode_part_impl(3073, 1024, 1025);
    }
}
