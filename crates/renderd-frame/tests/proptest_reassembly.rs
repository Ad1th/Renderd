//! Property-based tests for fragment header codec and reassembly buffer.

use bytes::Bytes;
use proptest::prelude::*;

use renderd_frame::{FragmentFlags, FragmentHeader, ReassemblyBuffer};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn test_header_encode_decode_proptest(
        frame_id in any::<u64>(),
        frag_id in 0u16..1000u16,
        frag_total in 1u16..1000u16,
        flags in any::<u8>(),
        pts_offset_us in 0u32..=0x00FF_FFFFu32,
    ) {
        let header = FragmentHeader {
            frame_id,
            frag_id,
            frag_total,
            flags,
            pts_offset_us,
        };

        let mut buf = [0u8; 16];
        if header.encode(&mut buf).is_ok() {
            let decoded = FragmentHeader::decode(&buf).unwrap();
            prop_assert_eq!(header, decoded);
        }
    }

    #[test]
    fn test_reassembly_permutation_proptest(
        payload_data in prop::collection::vec(any::<u8>(), 10..500),
        num_frags in 2..16usize,
    ) {
        let mut buffer = ReassemblyBuffer::new(64);
        let frame_id = 42u64;

        let chunk_size = payload_data.len().div_ceil(num_frags);
        let chunks: Vec<&[u8]> = payload_data.chunks(chunk_size).collect();
        let actual_frag_total = u16::try_from(chunks.len()).unwrap();

        let mut fragments = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let mut flags = FragmentFlags::new();
            if i == 0 {
                flags.set_first(true);
                flags.set_keyframe(true);
            }
            if i == chunks.len() - 1 {
                flags.set_last(true);
            }

            let header = FragmentHeader {
                frame_id,
                frag_id: u16::try_from(i).unwrap(),
                frag_total: actual_frag_total,
                flags: flags.bits(),
                pts_offset_us: 1000,
            };

            fragments.push((header, Bytes::copy_from_slice(chunk)));
        }

        let mut completed_frame = None;
        for (header, payload) in fragments {
            if let Some(frame) = buffer.insert(header, payload).unwrap() {
                completed_frame = Some(frame);
            }
        }

        prop_assert!(completed_frame.is_some());
        let frame = completed_frame.unwrap();
        prop_assert_eq!(frame.frame_id, frame_id);
        prop_assert_eq!(frame.payload.as_ref(), payload_data.as_slice());
    }
}
