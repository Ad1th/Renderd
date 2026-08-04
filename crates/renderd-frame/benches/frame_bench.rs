#![allow(missing_docs)]
//! Micro-benchmarks for fragment header encoding, decoding, and reassembly.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use renderd_frame::{
    FragmentHeader, ReassemblyBuffer, FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG,
};

fn bench_header_codec(c: &mut Criterion) {
    let header = FragmentHeader {
        frame_id: 42,
        frag_id: 3,
        frag_total: 10,
        flags: FLAG_KEYFRAME | FLAG_FIRST_FRAG,
        pts_offset_us: 1500,
    };
    let mut buf = [0u8; 16];

    c.bench_function("fragment_header_encode", |b| {
        b.iter(|| {
            black_box(&header).encode(black_box(&mut buf)).unwrap();
        });
    });

    c.bench_function("fragment_header_decode", |b| {
        b.iter(|| {
            let decoded = FragmentHeader::decode(black_box(&buf)).unwrap();
            black_box(decoded);
        });
    });
}

fn bench_reassembly_buffer(c: &mut Criterion) {
    c.bench_function("reassembly_buffer_10_fragments", |b| {
        let payload = vec![0xABu8; 1200];
        let bytes_payload = Bytes::copy_from_slice(&payload);
        b.iter(|| {
            let mut buffer = ReassemblyBuffer::new(16);
            for i in 0..10 {
                let header = FragmentHeader {
                    frame_id: 100,
                    frag_id: i,
                    frag_total: 10,
                    flags: if i == 0 {
                        FLAG_FIRST_FRAG
                    } else if i == 9 {
                        FLAG_LAST_FRAG
                    } else {
                        0
                    },
                    pts_offset_us: 0,
                };
                let res = buffer
                    .insert(black_box(header), black_box(bytes_payload.clone()))
                    .unwrap();
                black_box(res);
            }
        });
    });
}

criterion_group!(benches, bench_header_codec, bench_reassembly_buffer);
criterion_main!(benches);
