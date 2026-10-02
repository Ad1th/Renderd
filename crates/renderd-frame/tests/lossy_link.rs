//! `ReceiveWindow` against a simulated lossy link with a host that answers
//! retransmit requests.
//!
//! The link drops fragments at random — including the resent ones — and delays
//! every packet by half a round trip. Whatever the loss pattern, the decoder
//! must only ever be handed a frame it can decode: frames in increasing order,
//! and a non-keyframe only directly after the frame it references. And at
//! moderate loss, retransmits alone should recover almost everything.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use bytes::Bytes;
use proptest::prelude::*;
use renderd_frame::{
    FragmentHeader, ReceiveOutput, ReceiveWindow, FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG,
};

const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
const ONE_WAY: Duration = Duration::from_millis(10);

/// Deterministic xorshift, so a failing case replays exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn chance(&mut self, per_mille: u64) -> bool {
        self.next() % 1000 < per_mille
    }
}

fn fragment(frame_id: u64, frag_id: u16, frag_total: u16, key: bool) -> (FragmentHeader, Bytes) {
    let mut flags = 0;
    if frag_id == 0 {
        flags |= FLAG_FIRST_FRAG;
    }
    if frag_id + 1 == frag_total {
        flags |= FLAG_LAST_FRAG;
    }
    if key {
        flags |= FLAG_KEYFRAME;
    }
    let header = FragmentHeader {
        frame_id,
        frag_id,
        frag_total,
        flags,
        pts_offset_us: 0,
    };
    (header, Bytes::from(vec![0u8; 8]))
}

struct Outcome {
    delivered: Vec<(u64, bool)>,
    keyframe_requests: u32,
}

fn simulate(seed: u64, loss_per_mille: u64, frames: u64) -> Outcome {
    let mut rng = Rng(seed | 1);
    let start = Instant::now();
    let mut window = ReceiveWindow::default();
    window.set_rtt(ONE_WAY * 2);
    let mut out = ReceiveOutput::default();

    // Fragments in flight to the viewer: (arrival time, header, payload).
    let mut wire: VecDeque<(Instant, FragmentHeader, Bytes)> = VecDeque::new();
    // Every frame the host sent, for answering retransmit requests.
    let mut sent: Vec<(u16, bool)> = Vec::new();
    // Retransmit requests on their way to the host.
    let mut requests: VecDeque<(Instant, u64, Vec<u16>)> = VecDeque::new();
    let mut delivered = Vec::new();
    let mut keyframe_requests = 0;
    let mut force_keyframe_at: Option<Instant> = None;

    let end = start + FRAME_INTERVAL * u32::try_from(frames).unwrap() + Duration::from_secs(1);
    let mut now = start;
    let mut next_frame_id = 1u64;
    let mut next_capture = start;
    while now < end {
        // The host captures a frame.
        if now >= next_capture && next_frame_id <= frames {
            let key = next_frame_id == 1
                || next_frame_id % 120 == 0
                || force_keyframe_at.is_some_and(|t| now >= t);
            if key {
                force_keyframe_at = None;
            }
            let total = if key {
                20
            } else {
                u16::try_from(1 + rng.next() % 4).unwrap()
            };
            sent.push((total, key));
            for i in 0..total {
                if !rng.chance(loss_per_mille) {
                    let (h, p) = fragment(next_frame_id, i, total, key);
                    wire.push_back((now + ONE_WAY, h, p));
                }
            }
            next_frame_id += 1;
            next_capture += FRAME_INTERVAL;
        }

        // The host answers requests that reached it.
        while requests.front().is_some_and(|(t, _, _)| *t <= now) {
            let (_, frame_id, frag_ids) = requests.pop_front().unwrap();
            let Some(&(total, key)) = sent.get(usize::try_from(frame_id - 1).unwrap()) else {
                continue;
            };
            let ids: Vec<u16> = if frag_ids.is_empty() {
                (0..total).collect()
            } else {
                frag_ids
            };
            for i in ids {
                if !rng.chance(loss_per_mille) {
                    let (h, p) = fragment(frame_id, i, total, key);
                    wire.push_back((now + ONE_WAY, h, p));
                }
            }
        }

        // The viewer takes in whatever arrived, then runs its timers.
        while wire.front().is_some_and(|(t, _, _)| *t <= now) {
            let (_, h, p) = wire.pop_front().unwrap();
            window.insert(h, p, now, &mut out).unwrap();
        }
        window.poll(now, &mut out);

        for frame in out.frames.drain(..) {
            delivered.push((frame.frame_id, frame.is_keyframe));
        }
        for nack in out.nacks.drain(..) {
            requests.push_back((now + ONE_WAY, nack.frame_id, nack.frag_ids));
        }
        if std::mem::take(&mut out.need_keyframe) {
            keyframe_requests += 1;
            force_keyframe_at.get_or_insert(now + ONE_WAY);
        }
        out.clear();

        now += Duration::from_millis(1);
    }

    Outcome {
        delivered,
        keyframe_requests,
    }
}

fn assert_decodable(delivered: &[(u64, bool)]) -> Result<(), TestCaseError> {
    let mut last: Option<u64> = None;
    for &(id, key) in delivered {
        if let Some(prev) = last {
            prop_assert!(id > prev, "frame {id} handed out after {prev}");
            prop_assert!(
                key || id == prev + 1,
                "P-frame {id} handed out after {prev}: its reference is missing"
            );
        } else {
            prop_assert!(key, "first frame {id} is not a keyframe");
        }
        last = Some(id);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn test_decoder_only_ever_gets_decodable_frames(
        seed in any::<u64>(),
        loss_per_mille in 0u64..300,
    ) {
        let outcome = simulate(seed, loss_per_mille, 300);
        assert_decodable(&outcome.delivered)?;
    }

    /// At 2 % fragment loss, retransmits recover nearly every frame and a
    /// keyframe is almost never needed.
    #[test]
    fn test_moderate_loss_is_recovered_without_keyframes(seed in any::<u64>()) {
        let outcome = simulate(seed, 20, 600);
        assert_decodable(&outcome.delivered)?;
        prop_assert!(
            outcome.delivered.len() >= 590,
            "only {} of 600 frames reached the decoder",
            outcome.delivered.len()
        );
        prop_assert!(
            outcome.keyframe_requests <= 1,
            "{} keyframe requests",
            outcome.keyframe_requests
        );
    }
}
