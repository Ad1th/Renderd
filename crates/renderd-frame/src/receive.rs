//! Loss-recovering, decode-ordered frame receiver.
//!
//! [`ReassemblyBuffer`](crate::ReassemblyBuffer) completes frames and forgets
//! about the rest. A frame that loses one fragment sits incomplete until enough
//! later frames push it out of the window, and nothing stops the frames after
//! it from being decoded against a reference the decoder never got: the picture
//! smears until the next keyframe, and a keyframe — the largest thing the host
//! can send — is the only cure. On a 3 Mbps link a mid-stream keyframe is a
//! quarter of a second of link time.
//!
//! [`ReceiveWindow`] replaces that with what a lossy, slow link needs:
//!
//! * **Decode order.** Frames are handed out strictly in `frame_id` order. A
//!   frame that completes while an earlier one is still missing waits for it.
//!   A keyframe needs nothing before it, so it is handed out the moment it
//!   completes and supersedes whatever was still missing.
//! * **Retransmit requests.** The moment a fragment is known to be missing —
//!   a later fragment of the same frame, or any fragment of a later frame, has
//!   arrived — it is listed in [`ReceiveOutput::nacks`] for the host to resend.
//!   A request still unanswered after a round trip is repeated.
//! * **Keyframes only as a last resort.** Only a frame still missing well past
//!   its retransmit window (see [`RecoveryConfig`]) is given up, and only then
//!   does [`ReceiveOutput::need_keyframe`] ask for a keyframe. Until one
//!   arrives nothing that depends on the lost frame is handed out.
//!
//! The window is a pure state machine: time is passed in, and the caller turns
//! the output into decoder input and control messages.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};

use crate::error::FrameError;
use crate::flags::FragmentFlags;
use crate::header::FragmentHeader;
use crate::reassembly::ReassembledFrame;
use crate::validate::ValidateHeader;

/// Longest round trip the retransmit timers take into account.
///
/// `quinn` starts from a 333 ms guess before the first sample; a stale or
/// pathological estimate must not hold frames for seconds.
const MAX_RTT: Duration = Duration::from_millis(500);

/// Tuning for [`ReceiveWindow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryConfig {
    /// Frames held at once — incomplete, or complete but waiting for an
    /// earlier one — before the oldest is given up.
    pub max_held_frames: usize,
    /// Largest jump in `frame_id` treated as a run of lost frames worth
    /// recovering. A bigger jump is a discontinuity: everything before it is
    /// given up.
    pub max_gap: u64,
    /// Retransmit requests sent for one frame before waiting out its deadline.
    pub max_nack_rounds: u32,
    /// Added to one and a half round trips before a request is repeated. Covers
    /// the time a resent fragment waits in the host's send queue.
    pub retry_slack: Duration,
    /// Added to two round trips before a missing frame is given up.
    pub give_up_slack: Duration,
    /// How often a keyframe is asked for again while none arrives.
    pub keyframe_retry: Duration,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            max_held_frames: 64,
            max_gap: 32,
            max_nack_rounds: 3,
            retry_slack: Duration::from_millis(30),
            give_up_slack: Duration::from_millis(120),
            keyframe_retry: Duration::from_secs(1),
        }
    }
}

impl RecoveryConfig {
    /// Time after one request before the same fragments are asked for again.
    #[must_use]
    pub fn retry_interval(&self, rtt: Duration) -> Duration {
        let rtt = rtt.min(MAX_RTT);
        rtt + rtt / 2 + self.retry_slack
    }

    /// Time after a frame is first known missing before it is given up.
    #[must_use]
    pub fn give_up_after(&self, rtt: Duration) -> Duration {
        rtt.min(MAX_RTT) * 2 + self.give_up_slack
    }
}

/// The fragments of one frame the host should send again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NackRequest {
    /// Frame the fragments belong to.
    pub frame_id: u64,
    /// Missing fragment indices. Empty when no fragment of the frame arrived
    /// at all, so its size is unknown: the host resends all of it.
    pub frag_ids: Vec<u16>,
}

/// A frame whose last missing fragment just arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrivedFrame {
    /// Frame sequence identifier.
    pub frame_id: u64,
    /// Wire presentation timestamp, 24-bit microseconds.
    pub pts_offset_us: u32,
    /// Encoded size of the frame.
    pub bytes: usize,
    /// Whether the frame needed a retransmit. Its arrival time then includes a
    /// recovery round trip, which says nothing about queuing on the path.
    pub recovered: bool,
    /// Whether the frame is a keyframe.
    pub is_keyframe: bool,
}

/// Everything one call into [`ReceiveWindow`] produced.
#[derive(Debug, Default)]
pub struct ReceiveOutput {
    /// Frames ready for the decoder, in decode order.
    pub frames: Vec<ReassembledFrame>,
    /// Frames that finished arriving, in completion order.
    pub arrived: Vec<ArrivedFrame>,
    /// Fragments to ask the host for.
    pub nacks: Vec<NackRequest>,
    /// Frames given up on as lost.
    pub lost_frames: u64,
    /// A reference frame was lost for good: ask the host for a keyframe.
    pub need_keyframe: bool,
}

impl ReceiveOutput {
    /// Empties every list, keeping their allocations.
    pub fn clear(&mut self) {
        self.frames.clear();
        self.arrived.clear();
        self.nacks.clear();
        self.lost_frames = 0;
        self.need_keyframe = false;
    }
}

/// One frame the window knows about.
#[derive(Debug)]
struct Slot {
    /// `None` until a fragment of this frame arrives: the frame is then only
    /// known to exist because a later frame did.
    frag_total: Option<u16>,
    fragments: Vec<Option<Bytes>>,
    received: u16,
    /// Highest fragment index received so far. Meaningful when `received > 0`.
    highest_frag: u16,
    is_keyframe: bool,
    pts_offset_us: u32,
    /// The whole frame, once every fragment arrived, while it waits for an
    /// earlier frame.
    complete: Option<ReassembledFrame>,
    /// When a fragment of this frame was first known to be missing.
    missing_since: Option<Instant>,
    /// Per fragment: already asked for in the current request round.
    asked: Vec<bool>,
    /// The whole frame was asked for while its size was unknown.
    asked_whole: bool,
    nack_rounds: u32,
    last_nack: Option<Instant>,
}

impl Slot {
    const fn unknown() -> Self {
        Self {
            frag_total: None,
            fragments: Vec::new(),
            received: 0,
            highest_frag: 0,
            is_keyframe: false,
            pts_offset_us: 0,
            complete: None,
            missing_since: None,
            asked: Vec::new(),
            asked_whole: false,
            nack_rounds: 0,
            last_nack: None,
        }
    }

    fn size(&mut self, frag_total: u16) {
        self.frag_total = Some(frag_total);
        self.fragments = vec![None; usize::from(frag_total)];
        self.asked = vec![false; usize::from(frag_total)];
    }

    /// Fragment indices known to be missing. `tail_known` means a later frame
    /// has been seen, so fragments past the highest one received are missing
    /// too rather than merely not here yet.
    fn missing(&self, tail_known: bool) -> Vec<u16> {
        let Some(total) = self.frag_total else {
            return Vec::new();
        };
        if self.complete.is_some() {
            return Vec::new();
        }
        let limit = if tail_known {
            total
        } else if self.received == 0 {
            0
        } else {
            self.highest_frag + 1
        };
        (0..limit)
            .filter(|&i| self.fragments[usize::from(i)].is_none())
            .collect()
    }

    /// Whether the frame is known to be missing something: some fragment is
    /// missing, or (with no fragment at all) a later frame proved it exists.
    fn is_missing(&self, tail_known: bool) -> bool {
        if self.complete.is_some() {
            return false;
        }
        if self.frag_total.is_none() {
            return true;
        }
        !self.missing(tail_known).is_empty()
    }
}

/// Decode-ordered frame receiver with retransmit-based loss recovery.
#[derive(Debug)]
pub struct ReceiveWindow {
    config: RecoveryConfig,
    slots: BTreeMap<u64, Slot>,
    /// The next frame the decoder needs. Every frame below it was handed out
    /// or given up; their fragments are stale.
    next_id: Option<u64>,
    /// Only a keyframe can be handed out: the reference chain is broken.
    need_keyframe: bool,
    highest_seen: Option<u64>,
    /// When a keyframe was last asked for, or when frames started arriving
    /// without one.
    keyframe_asked: Option<Instant>,
    rtt: Duration,
    lost_frames: u64,
    recovered_frames: u64,
}

impl Default for ReceiveWindow {
    fn default() -> Self {
        Self::new(RecoveryConfig::default())
    }
}

impl ReceiveWindow {
    /// Creates an empty window waiting for its first keyframe.
    #[must_use]
    pub const fn new(config: RecoveryConfig) -> Self {
        Self {
            config,
            slots: BTreeMap::new(),
            next_id: None,
            need_keyframe: true,
            highest_seen: None,
            keyframe_asked: None,
            rtt: Duration::from_millis(20),
            lost_frames: 0,
            recovered_frames: 0,
        }
    }

    /// Updates the round-trip time the retransmit timers are based on.
    pub fn set_rtt(&mut self, rtt: Duration) {
        self.rtt = rtt.min(MAX_RTT);
    }

    /// Frames currently held, incomplete or waiting for an earlier frame.
    #[must_use]
    pub fn held_frames(&self) -> usize {
        self.slots.len()
    }

    /// Frames given up on since construction.
    #[must_use]
    pub const fn lost_frames(&self) -> u64 {
        self.lost_frames
    }

    /// Frames that completed only thanks to a retransmit.
    #[must_use]
    pub const fn recovered_frames(&self) -> u64 {
        self.recovered_frames
    }

    /// Whether only a keyframe can be handed out right now.
    #[must_use]
    pub const fn awaiting_keyframe(&self) -> bool {
        self.need_keyframe
    }

    /// Takes in one fragment.
    ///
    /// # Errors
    /// Returns [`FrameError`] if the header is invalid or its `frag_total`
    /// disagrees with earlier fragments of the same frame. A duplicate or stale
    /// fragment is not an error: retransmits make both routine.
    pub fn insert(
        &mut self,
        header: FragmentHeader,
        payload: Bytes,
        now: Instant,
        out: &mut ReceiveOutput,
    ) -> Result<(), FrameError> {
        header.validate()?;
        let frame_id = header.frame_id;
        if self.next_id.is_some_and(|next| frame_id < next) {
            return Ok(());
        }
        let flags = FragmentFlags::from_bits(header.flags);
        self.note_new_frame(frame_id, flags.is_keyframe(), now, out);

        let slot = self.slots.entry(frame_id).or_insert_with(Slot::unknown);
        if slot.frag_total.is_none() {
            slot.size(header.frag_total);
        } else if slot.frag_total != Some(header.frag_total) {
            return Err(FrameError::FragmentTotalMismatch {
                frame_id,
                expected: slot.frag_total.unwrap_or_default(),
                got: header.frag_total,
            });
        }
        let idx = usize::from(header.frag_id);
        if slot.complete.is_some() || slot.fragments[idx].is_some() {
            return Ok(());
        }
        slot.is_keyframe |= flags.is_keyframe();
        if header.frag_id == 0 || slot.received == 0 {
            slot.pts_offset_us = header.pts_offset_us;
        }
        slot.fragments[idx] = Some(payload);
        slot.received += 1;
        slot.highest_frag = if slot.received == 1 {
            header.frag_id
        } else {
            slot.highest_frag.max(header.frag_id)
        };

        if Some(slot.received) == slot.frag_total {
            let recovered = slot.nack_rounds > 0 || slot.asked_whole;
            let total: usize = slot.fragments.iter().flatten().map(Bytes::len).sum();
            let mut assembled = BytesMut::with_capacity(total);
            for fragment in slot.fragments.drain(..).flatten() {
                assembled.extend_from_slice(&fragment);
            }
            let frame = ReassembledFrame {
                frame_id,
                is_keyframe: slot.is_keyframe,
                pts_offset_us: slot.pts_offset_us,
                payload: assembled.freeze(),
            };
            out.arrived.push(ArrivedFrame {
                frame_id,
                pts_offset_us: frame.pts_offset_us,
                bytes: total,
                recovered,
                is_keyframe: frame.is_keyframe,
            });
            if recovered {
                self.recovered_frames += 1;
            }
            slot.complete = Some(frame);
            slot.missing_since = None;
        }

        self.release(out);
        self.request_missing(now, out);
        self.enforce_capacity(now, out);
        Ok(())
    }

    /// Runs the timers: repeats unanswered retransmit requests, gives up frames
    /// past their deadline, and repeats a keyframe request nobody answered.
    pub fn poll(&mut self, now: Instant, out: &mut ReceiveOutput) {
        self.give_up_expired(now, out);
        self.request_missing(now, out);
        if self.keyframe_retry_due(now) {
            self.keyframe_asked = Some(now);
            out.need_keyframe = true;
        }
    }

    /// The next instant [`Self::poll`] has something to do, if any.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        let retry = self.config.retry_interval(self.rtt);
        let give_up = self.config.give_up_after(self.rtt);
        let highest = self.highest_seen.unwrap_or(0);
        let mut next: Option<Instant> = None;
        let mut consider = |t: Instant| next = Some(next.map_or(t, |n| n.min(t)));
        for (&id, slot) in &self.slots {
            let Some(since) = slot.missing_since else {
                continue;
            };
            if !self.recoverable(id, slot) || !slot.is_missing(id < highest) {
                continue;
            }
            consider(since + give_up);
            if self.worth_asking(slot) && slot.nack_rounds < self.config.max_nack_rounds {
                if let Some(last) = slot.last_nack {
                    consider(last + retry);
                }
            }
        }
        if self.need_keyframe && !self.slots.is_empty() && !self.keyframe_in_flight() {
            if let Some(asked) = self.keyframe_asked {
                consider(asked + self.config.keyframe_retry);
            }
        }
        next
    }

    /// Drops everything not yet handed out and waits for a keyframe, because
    /// the caller could not keep up and skipped frames itself.
    pub fn require_keyframe(&mut self, now: Instant) {
        if let Some(highest) = self.highest_seen {
            self.slots
                .retain(|&id, slot| id > highest || slot.is_keyframe);
        }
        self.need_keyframe = true;
        self.keyframe_asked = Some(now);
        if let Some(highest) = self.highest_seen {
            let floor = self
                .slots
                .keys()
                .next()
                .copied()
                .unwrap_or(highest + 1)
                .min(highest + 1);
            self.next_id = Some(self.next_id.map_or(floor, |next| next.max(floor)));
        }
    }

    /// Records that `frame_id` exists and notes the frames a jump past the
    /// highest one seen proves missing.
    fn note_new_frame(
        &mut self,
        frame_id: u64,
        is_keyframe: bool,
        now: Instant,
        out: &mut ReceiveOutput,
    ) {
        if self.keyframe_asked.is_none() {
            self.keyframe_asked = Some(now);
        }
        let Some(highest) = self.highest_seen else {
            self.highest_seen = Some(frame_id);
            return;
        };
        if frame_id <= highest {
            return;
        }
        self.highest_seen = Some(frame_id);

        // Everything already held now has its tail accounted for.
        for slot in self.slots.values_mut() {
            if slot.missing_since.is_none() && slot.is_missing(true) {
                slot.missing_since = Some(now);
            }
        }

        let gap = frame_id - highest - 1;
        if gap == 0 {
            return;
        }
        if gap > self.config.max_gap {
            // Too far to be worth recovering frame by frame. A keyframe needs
            // none of it; anything else can only wait for one.
            let dropped = self.drop_below(frame_id);
            if !is_keyframe {
                self.lost_frames += dropped;
                out.lost_frames += dropped;
                self.break_chain(frame_id, now, out);
            }
            return;
        }
        let floor = self.next_id.unwrap_or(0);
        for id in (highest + 1)..frame_id {
            if id >= floor {
                let slot = self.slots.entry(id).or_insert_with(Slot::unknown);
                slot.missing_since.get_or_insert(now);
            }
        }
    }

    /// Hands out every frame the decoder can take now.
    fn release(&mut self, out: &mut ReceiveOutput) {
        loop {
            if !self.need_keyframe {
                if let Some(next) = self.next_id {
                    if self
                        .slots
                        .get(&next)
                        .is_some_and(|slot| slot.complete.is_some())
                    {
                        if let Some(frame) = self.slots.remove(&next).and_then(|s| s.complete) {
                            out.frames.push(frame);
                        }
                        self.next_id = Some(next + 1);
                        continue;
                    }
                }
            }
            // A complete keyframe needs nothing before it.
            let keyframe = self
                .slots
                .iter()
                .find(|(_, slot)| {
                    slot.is_keyframe
                        && slot
                            .complete
                            .as_ref()
                            .is_some_and(|frame| frame.is_keyframe)
                })
                .map(|(&id, _)| id);
            let Some(id) = keyframe else {
                break;
            };
            let superseded = self.drop_below(id);
            // Frames abandoned for this keyframe were missing; with no
            // keyframe needed they were not lost, merely unnecessary.
            let _ = superseded;
            if let Some(frame) = self.slots.remove(&id).and_then(|s| s.complete) {
                out.frames.push(frame);
            }
            self.next_id = Some(id + 1);
            self.need_keyframe = false;
            self.keyframe_asked = None;
        }
    }

    /// Lists every fragment that should be asked for now.
    fn request_missing(&mut self, now: Instant, out: &mut ReceiveOutput) {
        let retry = self.config.retry_interval(self.rtt);
        let max_rounds = self.config.max_nack_rounds;
        let highest = self.highest_seen.unwrap_or(0);
        let ids: Vec<u64> = self.slots.keys().copied().collect();
        for id in ids {
            let eligible = self
                .slots
                .get(&id)
                .is_some_and(|slot| self.recoverable(id, slot) && self.worth_asking(slot));
            if !eligible {
                continue;
            }
            let Some(slot) = self.slots.get_mut(&id) else {
                continue;
            };
            let tail_known = id < highest;
            if slot.frag_total.is_none() {
                let retry_due = slot.last_nack.is_some_and(|last| now >= last + retry);
                if (!slot.asked_whole || retry_due) && slot.nack_rounds < max_rounds {
                    slot.asked_whole = true;
                    slot.nack_rounds += 1;
                    slot.last_nack = Some(now);
                    slot.missing_since.get_or_insert(now);
                    out.nacks.push(NackRequest {
                        frame_id: id,
                        frag_ids: Vec::new(),
                    });
                }
                continue;
            }
            let missing = slot.missing(tail_known);
            if missing.is_empty() {
                // Every hole filled; what is left is still on its way.
                slot.missing_since = None;
                continue;
            }
            slot.missing_since.get_or_insert(now);
            let retry_due = slot.last_nack.is_some_and(|last| now >= last + retry);
            let ask: Vec<u16> = if retry_due && slot.nack_rounds < max_rounds {
                slot.nack_rounds += 1;
                missing
            } else {
                missing
                    .into_iter()
                    .filter(|&i| !slot.asked[usize::from(i)])
                    .collect()
            };
            if ask.is_empty() {
                continue;
            }
            if slot.nack_rounds == 0 {
                slot.nack_rounds = 1;
            }
            for &i in &ask {
                slot.asked[usize::from(i)] = true;
            }
            slot.last_nack = Some(now);
            out.nacks.push(NackRequest {
                frame_id: id,
                frag_ids: ask,
            });
        }
    }

    /// Whether `slot` could still turn into something the decoder can use.
    fn recoverable(&self, id: u64, slot: &Slot) -> bool {
        if slot.complete.is_some() {
            return false;
        }
        // A keyframe already on its way supersedes every frame before it.
        !self
            .slots
            .range((id + 1)..)
            .any(|(_, later)| later.is_keyframe)
    }

    /// Whether asking for `slot` again is useful. With the chain broken only a
    /// keyframe helps, so only frames that are, or might be, one are asked for.
    const fn worth_asking(&self, slot: &Slot) -> bool {
        !self.need_keyframe || slot.is_keyframe || slot.frag_total.is_none()
    }

    fn keyframe_in_flight(&self) -> bool {
        self.slots.values().any(|slot| slot.is_keyframe)
    }

    fn keyframe_retry_due(&self, now: Instant) -> bool {
        self.need_keyframe
            && !self.slots.is_empty()
            && !self.keyframe_in_flight()
            && self
                .keyframe_asked
                .is_some_and(|asked| now >= asked + self.config.keyframe_retry)
    }

    /// Gives up every frame still missing past its deadline.
    fn give_up_expired(&mut self, now: Instant, out: &mut ReceiveOutput) {
        let give_up = self.config.give_up_after(self.rtt);
        let highest = self.highest_seen.unwrap_or(0);
        let expired = self
            .slots
            .iter()
            .filter(|&(&id, slot)| {
                self.recoverable(id, slot)
                    && slot.is_missing(id < highest)
                    && slot
                        .missing_since
                        .is_some_and(|since| now >= since + give_up)
            })
            .map(|(&id, _)| id)
            .max();
        if let Some(id) = expired {
            self.give_up_through(id, now, out);
        }
    }

    /// Keeps the window within [`RecoveryConfig::max_held_frames`].
    fn enforce_capacity(&mut self, now: Instant, out: &mut ReceiveOutput) {
        while self.slots.len() > self.config.max_held_frames {
            let Some(&oldest) = self.slots.keys().next() else {
                break;
            };
            self.give_up_through(oldest, now, out);
        }
    }

    /// Abandons every frame up to and including `id`.
    fn give_up_through(&mut self, id: u64, now: Instant, out: &mut ReceiveOutput) {
        let dropped = self.drop_below(id + 1);
        self.lost_frames += dropped;
        out.lost_frames += dropped;
        self.break_chain(id + 1, now, out);
        self.release(out);
    }

    /// Marks the reference chain broken before `floor` and asks for a keyframe
    /// unless one was already asked for.
    fn break_chain(&mut self, floor: u64, now: Instant, out: &mut ReceiveOutput) {
        self.next_id = Some(self.next_id.map_or(floor, |next| next.max(floor)));
        if !self.need_keyframe || self.keyframe_asked.is_none() {
            out.need_keyframe = true;
            self.keyframe_asked = Some(now);
        }
        self.need_keyframe = true;
    }

    /// Removes every slot below `id` and returns how many were incomplete.
    fn drop_below(&mut self, id: u64) -> u64 {
        let keep = self.slots.split_off(&id);
        let dropped = std::mem::replace(&mut self.slots, keep);
        dropped
            .values()
            .filter(|slot| slot.complete.is_none())
            .count() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::{FLAG_FIRST_FRAG, FLAG_KEYFRAME, FLAG_LAST_FRAG};

    fn header(frame_id: u64, frag_id: u16, frag_total: u16, key: bool) -> FragmentHeader {
        let mut flags = 0;
        if frag_id == 0 {
            flags |= FLAG_FIRST_FRAG;
        }
        if frag_id == frag_total - 1 {
            flags |= FLAG_LAST_FRAG;
        }
        if key {
            flags |= FLAG_KEYFRAME;
        }
        FragmentHeader {
            frame_id,
            frag_id,
            frag_total,
            flags,
            pts_offset_us: u32::try_from(frame_id).unwrap() * 16_667,
        }
    }

    struct Rx {
        window: ReceiveWindow,
        now: Instant,
        out: ReceiveOutput,
    }

    impl Rx {
        fn new() -> Self {
            let mut window = ReceiveWindow::default();
            window.set_rtt(Duration::from_millis(10));
            Self {
                window,
                now: Instant::now(),
                out: ReceiveOutput::default(),
            }
        }

        fn frag(&mut self, frame_id: u64, frag_id: u16, total: u16, key: bool) {
            let payload = Bytes::from(vec![u8::try_from(frag_id % 256).unwrap(); 4]);
            self.window
                .insert(
                    header(frame_id, frag_id, total, key),
                    payload,
                    self.now,
                    &mut self.out,
                )
                .unwrap();
        }

        fn frame(&mut self, frame_id: u64, total: u16, key: bool) {
            for i in 0..total {
                self.frag(frame_id, i, total, key);
            }
        }

        fn advance(&mut self, ms: u64) {
            self.now += Duration::from_millis(ms);
            self.window.poll(self.now, &mut self.out);
        }

        fn delivered(&mut self) -> Vec<u64> {
            let ids = self.out.frames.iter().map(|f| f.frame_id).collect();
            self.out.frames.clear();
            ids
        }

        fn nacks(&mut self) -> Vec<NackRequest> {
            std::mem::take(&mut self.out.nacks)
        }

        fn take_keyframe_request(&mut self) -> bool {
            std::mem::take(&mut self.out.need_keyframe)
        }
    }

    #[test]
    fn test_in_order_stream_is_delivered_frame_by_frame() {
        let mut rx = Rx::new();
        rx.frame(1, 3, true);
        assert_eq!(rx.delivered(), vec![1]);
        for id in 2..=5 {
            rx.frame(id, 2, false);
            assert_eq!(rx.delivered(), vec![id]);
        }
        assert!(rx.nacks().is_empty());
        assert!(!rx.take_keyframe_request());
        assert_eq!(rx.window.held_frames(), 0);
    }

    #[test]
    fn test_nothing_is_delivered_before_the_first_keyframe() {
        let mut rx = Rx::new();
        rx.frame(1, 1, false);
        rx.frame(2, 1, false);
        assert!(rx.delivered().is_empty());
        rx.frame(3, 2, true);
        assert_eq!(rx.delivered(), vec![3]);
        rx.frame(4, 1, false);
        assert_eq!(rx.delivered(), vec![4]);
    }

    /// A hole inside a frame is asked for the moment a later fragment shows it.
    #[test]
    fn test_hole_inside_a_frame_is_nacked_immediately() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 4, false);
        rx.frag(2, 2, 4, false);
        assert_eq!(
            rx.nacks(),
            vec![NackRequest {
                frame_id: 2,
                frag_ids: vec![1]
            }]
        );
        // Fragment 3 is merely not here yet: nothing proves it lost.
        rx.frag(2, 1, 4, false);
        assert!(rx.nacks().is_empty());
        rx.frag(2, 3, 4, false);
        assert_eq!(rx.delivered(), vec![2]);
    }

    /// Frames behind a missing one wait for it, and the whole run is released
    /// in order once the retransmit lands — no keyframe involved.
    #[test]
    fn test_retransmit_releases_held_frames_in_order() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        // Frame 2 loses its last fragment; frames 3 and 4 arrive whole.
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        assert_eq!(
            rx.nacks(),
            vec![NackRequest {
                frame_id: 2,
                frag_ids: vec![1]
            }]
        );
        rx.frame(4, 1, false);
        assert!(rx.delivered().is_empty(), "3 and 4 depend on 2");
        assert!(rx.nacks().is_empty(), "already asked");

        rx.advance(5);
        rx.frag(2, 1, 2, false);
        assert_eq!(rx.delivered(), vec![2, 3, 4]);
        assert!(!rx.take_keyframe_request());
        assert_eq!(rx.window.recovered_frames(), 1);
        assert_eq!(rx.window.lost_frames(), 0);
    }

    /// A frame lost whole is only visible as a jump in `frame_id`; it is
    /// asked for in full.
    #[test]
    fn test_whole_frame_loss_is_nacked_in_full() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frame(3, 1, false);
        assert_eq!(
            rx.nacks(),
            vec![NackRequest {
                frame_id: 2,
                frag_ids: vec![]
            }]
        );
        rx.frame(2, 2, false);
        assert_eq!(rx.delivered(), vec![2, 3]);
    }

    #[test]
    fn test_unanswered_request_is_repeated_after_a_round_trip() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        assert_eq!(rx.nacks().len(), 1);
        rx.advance(10);
        assert!(rx.nacks().is_empty(), "too early to repeat");
        rx.advance(40);
        assert_eq!(
            rx.nacks(),
            vec![NackRequest {
                frame_id: 2,
                frag_ids: vec![1]
            }]
        );
    }

    /// Only a frame missing past its deadline costs a keyframe, and nothing
    /// that depends on it is handed out until one arrives.
    #[test]
    fn test_give_up_asks_for_a_keyframe_and_blocks_dependents() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        rx.nacks();
        rx.advance(100);
        assert!(!rx.take_keyframe_request(), "still within the deadline");
        rx.advance(100);
        assert!(rx.take_keyframe_request());
        assert_eq!(rx.window.lost_frames(), 1);
        assert!(rx.delivered().is_empty());

        // A late retransmit of the abandoned frame changes nothing.
        rx.frag(2, 1, 2, false);
        rx.frame(4, 1, false);
        assert!(rx.delivered().is_empty());

        rx.frame(5, 3, true);
        assert_eq!(rx.delivered(), vec![5]);
        rx.frame(6, 1, false);
        assert_eq!(rx.delivered(), vec![6]);
    }

    /// A keyframe that completes while earlier frames are still missing is
    /// shown at once instead of waiting for frames it does not need.
    #[test]
    fn test_keyframe_supersedes_missing_frames() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.frame(3, 2, true);
        assert_eq!(rx.delivered(), vec![3]);
        assert_eq!(rx.window.held_frames(), 0);
        assert!(!rx.take_keyframe_request());
        // No point asking for a frame the keyframe already replaced.
        rx.advance(500);
        assert!(!rx.take_keyframe_request());
    }

    /// Frames the host skipped on purpose are followed by a keyframe; that gap
    /// is not chased.
    #[test]
    fn test_gap_before_a_keyframe_is_not_nacked() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(5, 0, 3, true);
        assert!(rx.nacks().is_empty());
        rx.frag(5, 1, 3, true);
        rx.frag(5, 2, 3, true);
        assert_eq!(rx.delivered(), vec![5]);
    }

    /// The tail of the newest frame only counts as missing once something
    /// later arrives — a tail-loss probe from the host, typically.
    #[test]
    fn test_tail_loss_is_detected_by_a_later_fragment() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 3, false);
        assert!(rx.nacks().is_empty());
        // The host re-sends frame 2's last fragment after going quiet.
        rx.frag(2, 2, 3, false);
        assert_eq!(
            rx.nacks(),
            vec![NackRequest {
                frame_id: 2,
                frag_ids: vec![1]
            }]
        );
    }

    #[test]
    fn test_duplicates_and_stale_fragments_are_ignored() {
        let mut rx = Rx::new();
        rx.frame(1, 2, true);
        rx.delivered();
        rx.frag(1, 0, 2, true);
        rx.frag(2, 0, 2, false);
        rx.frag(2, 0, 2, false);
        rx.frag(2, 1, 2, false);
        assert_eq!(rx.delivered(), vec![2]);
        assert_eq!(rx.window.held_frames(), 0);
    }

    #[test]
    fn test_mismatched_frag_total_is_rejected() {
        let mut rx = Rx::new();
        rx.frag(1, 0, 2, true);
        let err = rx
            .window
            .insert(header(1, 4, 5, true), Bytes::new(), rx.now, &mut rx.out)
            .unwrap_err();
        assert!(matches!(err, FrameError::FragmentTotalMismatch { .. }));
    }

    #[test]
    fn test_held_frames_are_bounded() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        for id in 3..100 {
            rx.frame(id, 1, false);
        }
        assert!(rx.window.held_frames() <= RecoveryConfig::default().max_held_frames);
        assert!(rx.take_keyframe_request());
    }

    #[test]
    fn test_big_jump_is_a_discontinuity_not_a_run_of_nacks() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frame(500, 1, false);
        assert!(rx.nacks().is_empty());
        assert!(rx.take_keyframe_request());
        rx.frame(501, 1, true);
        assert_eq!(rx.delivered(), vec![501]);
    }

    /// While waiting for a keyframe, unknown frames — which might be that
    /// keyframe — are still chased, but plain P-frames are not.
    #[test]
    fn test_only_possible_keyframes_are_chased_while_the_chain_is_broken() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        rx.nacks();
        rx.advance(300);
        assert!(rx.take_keyframe_request());
        rx.nacks();
        rx.frag(4, 0, 2, false);
        rx.frame(6, 1, false);
        let nacks = rx.nacks();
        assert_eq!(
            nacks,
            vec![NackRequest {
                frame_id: 5,
                frag_ids: vec![]
            }],
            "frame 4 is a known P-frame and useless; frame 5 might be the keyframe"
        );
    }

    #[test]
    fn test_keyframe_request_repeats_while_none_arrives() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        rx.advance(300);
        assert!(rx.take_keyframe_request());
        rx.frame(4, 1, false);
        rx.advance(500);
        assert!(!rx.take_keyframe_request());
        rx.advance(600);
        assert!(rx.take_keyframe_request(), "asked again after a second");
    }

    #[test]
    fn test_require_keyframe_drops_pending_non_key_frames() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        rx.frag(2, 0, 2, false);
        rx.window.require_keyframe(rx.now);
        assert_eq!(rx.window.held_frames(), 0);
        rx.frame(3, 1, false);
        assert!(rx.delivered().is_empty());
        rx.frame(4, 1, true);
        assert_eq!(rx.delivered(), vec![4]);
    }

    #[test]
    fn test_next_deadline_tracks_retries_and_give_up() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.delivered();
        assert!(rx.window.next_deadline().is_none());
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        let deadline = rx.window.next_deadline().expect("a retry is pending");
        assert!(deadline > rx.now);
        assert!(deadline <= rx.now + Duration::from_millis(50));
    }

    #[test]
    fn test_arrivals_report_recovery() {
        let mut rx = Rx::new();
        rx.frame(1, 1, true);
        rx.frag(2, 0, 2, false);
        rx.frame(3, 1, false);
        rx.out.arrived.clear();
        rx.frag(2, 1, 2, false);
        assert_eq!(rx.out.arrived.len(), 1);
        assert!(rx.out.arrived[0].recovered);
    }
}
