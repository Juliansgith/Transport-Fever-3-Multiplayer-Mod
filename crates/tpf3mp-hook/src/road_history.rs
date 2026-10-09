//! Bounded private history for the opt-in 40420 road-pose experiment.
//!
//! This module stores only owned scalar pose records. Native addresses never
//! become dereferenceable history entries. The producer may insert a sample
//! only after the entire road-transform call returns and its caller verifies
//! the native writer, world, identity, and replica-report boundaries.

#![allow(unsafe_code)]

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

pub const MAX_RECORDS: usize = 4;
pub const CAPACITY: usize = 64;
pub const WINDOW_NANOS: u64 = 1_000_000_000;
pub const MAX_GAP_STEPS: f64 = 2.0;
pub const RESERVE_NANOS: u64 = 200_000_000;
pub const MAX_CATCHUP_PERCENT: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockKind {
    CGameTime,
    Other,
}

/// A room-time endpoint accepted from one completed native GameSim batch.
/// Pointer-valued fields are identities only; this type never dereferences
/// them. `tick_count` and `update_count` are read from the native actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptedAnchor {
    pub capture_generation: u64,
    pub world_epoch: u64,
    pub game_state: usize,
    pub engine: usize,
    pub clock_entity_ptr: usize,
    pub clock_entity_id: u32,
    pub clock_kind: ClockKind,
    pub native_time: i64,
    pub tick_count: u32,
    pub update_count: u32,
    pub room_step: u64,
}

/// Provenance captured around `GameState::Replicate`. This is pending
/// evidence only; creating it never makes the destination render-eligible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCopy {
    pub id: u64,
    pub source: AcceptedAnchor,
    pub destination_game_state: usize,
    pub destination_engine: usize,
    pub family_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalReadback {
    pub capture_generation: u64,
    pub world_epoch: u64,
    pub game_state: usize,
    pub engine: usize,
    pub clock_entity_ptr: usize,
    pub clock_entity_id: u32,
    pub clock_kind: ClockKind,
    pub native_time: i64,
    pub tick_count: u32,
    pub update_count: u32,
    pub family_revision: u64,
}

/// A copy whose destination actor read back exactly the source's current
/// accepted endpoint. Still pending until its own local callback (including
/// a verified zero-update callback) and every room report succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundCopy {
    pub id: u64,
    pub source: AcceptedAnchor,
    pub destination_game_state: usize,
    pub destination_engine: usize,
    pub destination_clock_entity_id: u32,
    pub destination_clock_entity_ptr: usize,
    pub capture_generation: u64,
    pub world_epoch: u64,
    pub family_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyReject {
    Identity,
    Capture,
    World,
    ClockKind,
    ClockEntity,
    Time,
    TickCount,
    UpdateCount,
    Incarnation,
    StepBoundary,
    Reports,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyCommitProof {
    pub first_step: u64,
    pub reports_ok: bool,
    pub capture_generation: u64,
    pub world_epoch: u64,
    pub game_state: usize,
    pub engine: usize,
    pub family_revision: u64,
}

pub fn bind_copy(copy: PendingCopy, readback: LocalReadback) -> Result<BoundCopy, CopyReject> {
    if copy.destination_game_state == 0
        || copy.destination_engine == 0
        || readback.game_state != copy.destination_game_state
        || readback.engine != copy.destination_engine
    {
        return Err(CopyReject::Identity);
    }
    if readback.capture_generation == 0
        || copy.source.capture_generation != readback.capture_generation
    {
        return Err(CopyReject::Capture);
    }
    if copy.source.world_epoch == 0 || copy.source.world_epoch != readback.world_epoch {
        return Err(CopyReject::World);
    }
    if copy.source.clock_kind != readback.clock_kind {
        return Err(CopyReject::ClockKind);
    }
    if copy.source.clock_entity_id == 0
        || copy.source.clock_entity_ptr == 0
        || copy.source.clock_entity_id != readback.clock_entity_id
        || readback.clock_entity_ptr == 0
    {
        return Err(CopyReject::ClockEntity);
    }
    if copy.source.native_time != readback.native_time {
        return Err(CopyReject::Time);
    }
    if copy.source.tick_count != readback.tick_count {
        return Err(CopyReject::TickCount);
    }
    if copy.source.update_count != readback.update_count {
        return Err(CopyReject::UpdateCount);
    }
    if copy.family_revision != readback.family_revision {
        return Err(CopyReject::Incarnation);
    }
    Ok(BoundCopy {
        id: copy.id,
        source: copy.source,
        destination_game_state: readback.game_state,
        destination_engine: readback.engine,
        destination_clock_entity_id: readback.clock_entity_id,
        destination_clock_entity_ptr: readback.clock_entity_ptr,
        capture_generation: readback.capture_generation,
        world_epoch: readback.world_epoch,
        family_revision: readback.family_revision,
    })
}

/// The only boundary that can turn a copied source anchor into a destination
/// anchor. A verified zero-update callback may retain the copied endpoint but
/// cannot create a new endpoint; report failure, world change, or removal
/// during the local batch leaves the destination ineligible.
pub fn commit_copy(copy: BoundCopy, proof: CopyCommitProof) -> Result<AcceptedAnchor, CopyReject> {
    if !proof.reports_ok {
        return Err(CopyReject::Reports);
    }
    if copy.source.room_step.checked_add(1) != Some(proof.first_step) {
        return Err(CopyReject::StepBoundary);
    }
    if proof.capture_generation != copy.capture_generation {
        return Err(CopyReject::Capture);
    }
    if proof.world_epoch == 0 || proof.world_epoch != copy.world_epoch {
        return Err(CopyReject::World);
    }
    if proof.game_state != copy.destination_game_state || proof.engine != copy.destination_engine {
        return Err(CopyReject::Identity);
    }
    if proof.family_revision != copy.family_revision {
        return Err(CopyReject::Incarnation);
    }
    Ok(AcceptedAnchor {
        capture_generation: proof.capture_generation,
        world_epoch: proof.world_epoch,
        game_state: proof.game_state,
        engine: proof.engine,
        clock_entity_id: copy.destination_clock_entity_id,
        clock_entity_ptr: copy.destination_clock_entity_ptr,
        clock_kind: copy.source.clock_kind,
        native_time: copy.source.native_time,
        tick_count: copy.source.tick_count,
        update_count: copy.source.update_count,
        room_step: copy.source.room_step,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FamilyKey {
    pub owner: u32,
    pub child: u32,
    pub component_index: u32,
    pub slot: i32,
    pub model_id: u32,
    /// Observed definition address used only for same-process identity checks.
    pub definition: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseSample {
    pub capture_generation: u64,
    pub world_epoch: u64,
    pub incarnation: u64,
    /// Monotonic history invalidation epoch. A contended invalidation still
    /// makes every sample from its prior epoch unusable.
    pub invalidation_generation: u64,
    pub key: FamilyKey,
    /// Fractional accepted room-step coordinate of the native interpolated
    /// time. It is derived only from two exact mapped room-time endpoints.
    pub step_position: f64,
    pub sample_nanos: u64,
    pub count: u8,
    /// Six finite f32 bit patterns per record: position then direction.
    pub records: [[u32; 6]; MAX_RECORDS],
}

impl PoseSample {
    pub fn valid(self) -> bool {
        self.capture_generation != 0
            && self.world_epoch != 0
            && self.incarnation != 0
            && self.invalidation_generation != 0
            && self.key.owner != 0
            && self.key.child != 0
            && self.key.model_id != 0
            && self.key.definition != 0
            && self.step_position.is_finite()
            && self.count != 0
            && usize::from(self.count) <= MAX_RECORDS
            && self.records[..usize::from(self.count)]
                .iter()
                .all(|record| valid_record(*record))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertResult {
    Inserted,
    Duplicate,
    Reprimed,
    Conflict,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpolatedPose {
    pub key: FamilyKey,
    pub count: u8,
    pub target_step: f64,
    pub lower_step: f64,
    pub upper_step: f64,
    pub records: [[u32; 6]; MAX_RECORDS],
}

/// Native vector storage passed to the verified read-only transform finalizer.
/// Its records stay alive for the full synchronous call and meet the native
/// 32-byte alignment requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(32))]
pub struct PrivateRecords {
    pub words: [u32; MAX_RECORDS * 6],
    pub count: u8,
}

impl PrivateRecords {
    pub fn from_pose(pose: InterpolatedPose) -> Self {
        Self {
            words: pose.records.concat_words(),
            count: pose.count,
        }
    }

    pub fn vector_header(&self) -> [usize; 3] {
        let begin = self.words.as_ptr() as usize;
        let end = begin + usize::from(self.count) * 6 * std::mem::size_of::<u32>();
        [begin, end, end]
    }
}

trait RecordWords {
    fn concat_words(self) -> [u32; MAX_RECORDS * 6];
}

impl RecordWords for [[u32; 6]; MAX_RECORDS] {
    fn concat_words(self) -> [u32; MAX_RECORDS * 6] {
        let mut words = [0; MAX_RECORDS * 6];
        for (index, record) in self.into_iter().enumerate() {
            words[index * 6..index * 6 + 6].copy_from_slice(&record);
        }
        words
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomPace {
    pub active: bool,
    pub steps_per_second: u16,
    pub speed_percent: u16,
    pub previous_speed_percent: u16,
    pub batch_two: bool,
}

impl RoomPace {
    fn effective_percent(self) -> u16 {
        if self.speed_percent == 0 {
            self.previous_speed_percent
        } else {
            self.speed_percent
        }
    }

    fn rate_steps_per_second(self) -> Option<f64> {
        let speed = self.effective_percent();
        if !self.active || self.steps_per_second == 0 || speed == 0 {
            return None;
        }
        Some(f64::from(self.steps_per_second) * f64::from(speed) / 100.0)
    }

    fn desired_lag_steps(self, rate: f64) -> f64 {
        let batch = if self.batch_two && self.effective_percent() > 200 {
            2.0
        } else {
            1.0
        };
        let interval_seconds = batch / rate;
        rate * (RESERVE_NANOS as f64 / 1_000_000_000.0 + interval_seconds)
    }
}

/// Fixed one-family ring. Its methods are deterministic and allocation-free;
/// production serializes access with a nonblocking mutex.
#[derive(Debug, Clone, Copy)]
pub struct History {
    generation: u64,
    world_epoch: u64,
    incarnation: u64,
    invalidation_generation: u64,
    frames: [Option<PoseSample>; CAPACITY],
    len: usize,
    next: usize,
    target_step: Option<f64>,
    target_wall_nanos: u64,
    last_rate: f64,
}

static HISTORY: Mutex<History> = Mutex::new(History::new());
static FAMILY_LOCK: AtomicBool = AtomicBool::new(false);
static FAMILY_OWNER: AtomicU32 = AtomicU32::new(0);
static FAMILY_CHILD: AtomicU32 = AtomicU32::new(0);
static FAMILY_REVISION: AtomicU64 = AtomicU64::new(1);
const APPLY_LEASE_COUNT_BITS: u32 = 8;
const APPLY_LEASE_COUNT_MASK: u64 = (1 << APPLY_LEASE_COUNT_BITS) - 1;
const APPLY_EPOCH_INCREMENT: u64 = 1 << APPLY_LEASE_COUNT_BITS;
#[derive(Debug)]
struct ApplyEpoch {
    state: AtomicU64,
}

impl ApplyEpoch {
    const fn new() -> Self {
        Self {
            state: AtomicU64::new(APPLY_EPOCH_INCREMENT),
        }
    }

    fn generation(&self) -> u64 {
        self.state.load(Ordering::Acquire) >> APPLY_LEASE_COUNT_BITS
    }

    fn invalidate(&self) -> u64 {
        let previous = self
            .state
            .fetch_add(APPLY_EPOCH_INCREMENT, Ordering::AcqRel);
        previous.wrapping_add(APPLY_EPOCH_INCREMENT) >> APPLY_LEASE_COUNT_BITS
    }

    fn try_lease(&self, expected_generation: u64) -> Option<ApplyLease<'_>> {
        if expected_generation == 0 {
            return None;
        }
        let mut state = self.state.load(Ordering::Acquire);
        for _ in 0..8 {
            let generation = state >> APPLY_LEASE_COUNT_BITS;
            let active = state & APPLY_LEASE_COUNT_MASK;
            if generation != expected_generation || active == APPLY_LEASE_COUNT_MASK {
                return None;
            }
            match self.state.compare_exchange_weak(
                state,
                state + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(ApplyLease {
                        epoch: self,
                        generation,
                    });
                }
                Err(actual) => state = actual,
            }
        }
        None
    }
}

static APPLY_EPOCH: ApplyEpoch = ApplyEpoch::new();
static INVALIDATIONS: AtomicU64 = AtomicU64::new(0);
static INSERTED_SAMPLES: AtomicU64 = AtomicU64::new(0);
static APPLIED_POSES: AtomicU64 = AtomicU64::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);
static HISTORY_LOCK_SKIPS: AtomicU64 = AtomicU64::new(0);
static TARGET: Mutex<Option<PresentationTarget>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationTarget {
    pub capture_generation: u64,
    pub producer_epoch: u64,
    pub world_epoch: u64,
    pub incarnation: u64,
    pub invalidation_generation: u64,
    pub step_position: f64,
}

/// A nonblocking authorization to forward one already-built private pose.
/// Its successful compare/exchange is the linearization point immediately
/// before the synchronous native call. An invalidation ordered after that CAS
/// never waits for this lease and may let this one in-flight call finish; a
/// lease acquired after invalidation cannot use the old epoch.
#[derive(Debug)]
pub struct ApplyLease<'a> {
    epoch: &'a ApplyEpoch,
    generation: u64,
}

impl ApplyLease<'_> {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Drop for ApplyLease<'_> {
    fn drop(&mut self) {
        self.epoch.state.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Current invalidation epoch, including invalidations whose history mutex
/// could not be acquired.
pub fn invalidation_generation() -> u64 {
    APPLY_EPOCH.generation()
}

/// Acquires a bounded, nonblocking forwarding lease for the observed epoch.
/// The CAS and invalidation's atomic epoch increment totally order the two
/// operations; no native writer waits for an active renderer lease.
pub fn try_apply_lease(expected_generation: u64) -> Option<ApplyLease<'static>> {
    APPLY_EPOCH.try_lease(expected_generation)
}

pub(crate) fn history_forward_eligible(
    applied_marker: bool,
    private_storage: bool,
    vector_header: bool,
    lease: bool,
) -> bool {
    applied_marker && private_storage && vector_header && lease
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub invalidations: u64,
    pub inserted_samples: u64,
    pub applied_poses: u64,
    pub fallbacks: u64,
    pub lock_skips: u64,
}

pub fn stats() -> Stats {
    Stats {
        invalidations: INVALIDATIONS.load(Ordering::Relaxed),
        inserted_samples: INSERTED_SAMPLES.load(Ordering::Relaxed),
        applied_poses: APPLIED_POSES.load(Ordering::Relaxed),
        fallbacks: FALLBACKS.load(Ordering::Relaxed),
        lock_skips: HISTORY_LOCK_SKIPS.load(Ordering::Relaxed),
    }
}

pub fn reset_stats() {
    INVALIDATIONS.store(0, Ordering::Release);
    INSERTED_SAMPLES.store(0, Ordering::Release);
    APPLIED_POSES.store(0, Ordering::Release);
    FALLBACKS.store(0, Ordering::Release);
    HISTORY_LOCK_SKIPS.store(0, Ordering::Release);
}

pub fn begin_capture() {
    invalidate();
    FAMILY_OWNER.store(0, Ordering::Release);
    FAMILY_CHILD.store(0, Ordering::Release);
    FAMILY_REVISION.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut target) = TARGET.try_lock() {
        *target = None;
    }
    reset_stats();
}

pub fn end_capture() {
    invalidate();
    FAMILY_OWNER.store(0, Ordering::Release);
    FAMILY_CHILD.store(0, Ordering::Release);
    FAMILY_REVISION.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut target) = TARGET.try_lock() {
        *target = None;
    }
}

/// Selects the single diagnostic family. A contended family transition
/// refuses this call rather than waiting on a renderer or simulation thread.
pub fn select_family(owner: u32, child: u32) -> Option<u64> {
    if owner == 0 || child == 0 {
        return None;
    }
    if FAMILY_OWNER.load(Ordering::Acquire) == owner
        && FAMILY_CHILD.load(Ordering::Acquire) == child
    {
        return Some(FAMILY_REVISION.load(Ordering::Acquire));
    }
    if FAMILY_LOCK
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let revision = if FAMILY_OWNER.load(Ordering::Acquire) == owner
        && FAMILY_CHILD.load(Ordering::Acquire) == child
    {
        FAMILY_REVISION.load(Ordering::Acquire)
    } else {
        invalidate();
        let revision = FAMILY_REVISION
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        FAMILY_OWNER.store(owner, Ordering::Release);
        FAMILY_CHILD.store(child, Ordering::Release);
        revision
    };
    FAMILY_LOCK.store(false, Ordering::Release);
    Some(revision)
}

pub fn selected_family() -> Option<(u32, u32, u64)> {
    let owner = FAMILY_OWNER.load(Ordering::Acquire);
    let child = FAMILY_CHILD.load(Ordering::Acquire);
    let revision = FAMILY_REVISION.load(Ordering::Acquire);
    (owner != 0 && child != 0 && revision != 0).then_some((owner, child, revision))
}

pub fn family_matches(owner: u32, child: u32, revision: u64) -> bool {
    owner != 0
        && child != 0
        && revision != 0
        && FAMILY_OWNER.load(Ordering::Acquire) == owner
        && FAMILY_CHILD.load(Ordering::Acquire) == child
        && FAMILY_REVISION.load(Ordering::Acquire) == revision
}

pub fn current_family_revision() -> u64 {
    FAMILY_REVISION.load(Ordering::Acquire)
}

/// Must run before the native deletion. The revision makes retained samples
/// unusable even if the bounded history mutex is contended.
pub fn note_entity_removal(entity: u32) -> bool {
    if entity == 0
        || (entity != FAMILY_OWNER.load(Ordering::Acquire)
            && entity != FAMILY_CHILD.load(Ordering::Acquire))
    {
        return false;
    }
    invalidate();
    FAMILY_REVISION.fetch_add(1, Ordering::AcqRel);
    true
}

fn advance_invalidation_epoch() -> u64 {
    INVALIDATIONS.fetch_add(1, Ordering::Relaxed);
    // This atomic increment is the invalidation linearization point. The
    // low lease count is preserved, so render calls already authorized before
    // this point may finish without making the invalidator wait.
    APPLY_EPOCH.invalidate()
}

pub fn invalidate() {
    let generation = advance_invalidation_epoch();
    match HISTORY.try_lock() {
        Ok(mut history) => history.invalidate_at(generation),
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
        }
    };
}

pub fn push(sample: PoseSample) -> InsertResult {
    push_with_commit(sample, |_| true)
}

/// Inserts while holding the history lock and publishes its evidence before
/// releasing that lock. A failed marker invalidates the candidate so another
/// renderer thread cannot consume an unrecorded sample.
pub fn push_with_commit(
    sample: PoseSample,
    publish: impl FnOnce(InsertResult) -> bool,
) -> InsertResult {
    let invalidation_epoch = invalidation_generation();
    if sample.invalidation_generation != invalidation_epoch
        || sample.incarnation != current_family_revision()
        || sample.key.owner != FAMILY_OWNER.load(Ordering::Acquire)
        || sample.key.child != FAMILY_CHILD.load(Ordering::Acquire)
    {
        FALLBACKS.fetch_add(1, Ordering::Relaxed);
        return InsertResult::Invalid;
    }
    match HISTORY.try_lock() {
        Ok(mut history) => {
            if invalidation_generation() != invalidation_epoch {
                history.invalidate_at(invalidation_generation());
                FALLBACKS.fetch_add(1, Ordering::Relaxed);
                return InsertResult::Invalid;
            }
            let result = history.push(sample);
            if !matches!(
                result,
                InsertResult::Inserted | InsertResult::Duplicate | InsertResult::Reprimed
            ) || invalidation_generation() != invalidation_epoch
                || !publish(result)
                || invalidation_generation() != invalidation_epoch
            {
                let generation = advance_invalidation_epoch();
                history.invalidate_at(generation);
                FALLBACKS.fetch_add(1, Ordering::Relaxed);
                return InsertResult::Invalid;
            }
            if matches!(result, InsertResult::Inserted | InsertResult::Reprimed) {
                INSERTED_SAMPLES.fetch_add(1, Ordering::Relaxed);
            }
            result
        }
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
            FALLBACKS.fetch_add(1, Ordering::Relaxed);
            InsertResult::Invalid
        }
    }
}

/// Computes the shared presentation coordinate exactly once from the current
/// renderer producer epoch. It advances on the room's authoritative rate and
/// cannot run past the newest confirmed pose.
pub fn advance_target(pace: RoomPace, now_nanos: u64) -> Option<f64> {
    match HISTORY.try_lock() {
        Ok(mut history) => {
            let generation = invalidation_generation();
            if history.invalidation_generation != generation {
                return None;
            }
            let target = history.advance_target(pace, now_nanos)?;
            (invalidation_generation() == generation).then_some(target)
        }
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub fn advance_target_for_epoch(
    generation: u64,
    producer_epoch: u64,
    pace: RoomPace,
    now_nanos: u64,
) -> Option<PresentationTarget> {
    if generation == 0 || producer_epoch == 0 {
        return None;
    }
    let invalidation_epoch = invalidation_generation();
    let (target_step, world_epoch, incarnation) = match HISTORY.try_lock() {
        Ok(mut history) => {
            if history.invalidation_generation != invalidation_epoch {
                return None;
            }
            let target_step = history.advance_target(pace, now_nanos)?;
            (target_step, history.world_epoch, history.incarnation)
        }
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    let target = PresentationTarget {
        capture_generation: generation,
        producer_epoch,
        world_epoch,
        incarnation,
        invalidation_generation: invalidation_epoch,
        step_position: target_step,
    };
    if invalidation_generation() != invalidation_epoch {
        return None;
    }
    match TARGET.try_lock() {
        Ok(mut current) => {
            if invalidation_generation() != invalidation_epoch {
                return None;
            }
            *current = Some(target);
            Some(target)
        }
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub fn presentation_target(generation: u64, producer_epoch: u64) -> Option<PresentationTarget> {
    let Ok(target) = TARGET.try_lock() else {
        HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    target.filter(|target| {
        target.capture_generation == generation
            && target.producer_epoch == producer_epoch
            && target.invalidation_generation == invalidation_generation()
    })
}

pub fn interpolated_pose(
    target_step: f64,
    capture_generation: u64,
    world_epoch: u64,
    incarnation: u64,
    invalidation_epoch: u64,
    key: FamilyKey,
) -> Option<InterpolatedPose> {
    if invalidation_epoch == 0
        || invalidation_epoch != invalidation_generation()
        || incarnation != current_family_revision()
        || key.owner != FAMILY_OWNER.load(Ordering::Acquire)
        || key.child != FAMILY_CHILD.load(Ordering::Acquire)
    {
        FALLBACKS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    match HISTORY.try_lock() {
        Ok(history) => {
            if history.generation != capture_generation
                || history.world_epoch != world_epoch
                || history.incarnation != incarnation
                || history.invalidation_generation != invalidation_epoch
            {
                FALLBACKS.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            let result = history
                .interpolate(target_step)
                .filter(|pose| pose.key == key);
            if result.is_none() || invalidation_generation() != invalidation_epoch {
                FALLBACKS.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            result
        }
        Err(_) => {
            HISTORY_LOCK_SKIPS.fetch_add(1, Ordering::Relaxed);
            FALLBACKS.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

pub fn note_applied() {
    APPLIED_POSES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_fallback() {
    FALLBACKS.fetch_add(1, Ordering::Relaxed);
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            world_epoch: 0,
            incarnation: 0,
            invalidation_generation: 0,
            frames: [None; CAPACITY],
            len: 0,
            next: 0,
            target_step: None,
            target_wall_nanos: 0,
            last_rate: 0.0,
        }
    }

    pub fn invalidate(&mut self) {
        self.frames = [None; CAPACITY];
        self.len = 0;
        self.next = 0;
        self.target_step = None;
        self.target_wall_nanos = 0;
        self.last_rate = 0.0;
        self.generation = 0;
        self.world_epoch = 0;
        self.incarnation = 0;
        self.invalidation_generation = 0;
    }

    fn invalidate_at(&mut self, invalidation_generation: u64) {
        self.invalidate();
        self.invalidation_generation = invalidation_generation;
    }

    fn prune_expired(&mut self, newest_nanos: u64) {
        while self.len > 0 {
            let Some(oldest) = self.frame(0) else {
                self.invalidate();
                return;
            };
            if newest_nanos.saturating_sub(oldest.sample_nanos) <= WINDOW_NANOS {
                break;
            }
            let oldest_index = (self.next + CAPACITY - self.len) % CAPACITY;
            self.frames[oldest_index] = None;
            self.len -= 1;
        }
        if self.len == 0 {
            self.next = 0;
            self.target_step = None;
            self.target_wall_nanos = 0;
        }
    }

    fn ensure_scope(&mut self, sample: PoseSample) -> bool {
        if self.generation == sample.capture_generation
            && self.world_epoch == sample.world_epoch
            && self.incarnation == sample.incarnation
            && self.invalidation_generation == sample.invalidation_generation
        {
            return false;
        }
        self.frames = [None; CAPACITY];
        self.len = 0;
        self.next = 0;
        self.target_step = None;
        self.target_wall_nanos = 0;
        self.last_rate = 0.0;
        self.generation = sample.capture_generation;
        self.world_epoch = sample.world_epoch;
        self.incarnation = sample.incarnation;
        self.invalidation_generation = sample.invalidation_generation;
        true
    }

    pub fn push(&mut self, sample: PoseSample) -> InsertResult {
        if !sample.valid() {
            return InsertResult::Invalid;
        }
        self.prune_expired(sample.sample_nanos);
        let reprime = self.ensure_scope(sample);
        if self.len > 0 {
            let Some(last) = self.frame(self.len - 1) else {
                self.invalidate();
                return InsertResult::Invalid;
            };
            if last.key != sample.key || last.count != sample.count {
                self.frames = [None; CAPACITY];
                self.len = 0;
                self.next = 0;
                self.target_step = None;
                self.generation = sample.capture_generation;
                self.world_epoch = sample.world_epoch;
                self.incarnation = sample.incarnation;
                self.push_raw(sample);
                return InsertResult::Reprimed;
            }
            if sample.step_position < last.step_position {
                self.frames = [None; CAPACITY];
                self.len = 0;
                self.next = 0;
                self.target_step = None;
                self.push_raw(sample);
                return InsertResult::Reprimed;
            }
            if sample.step_position == last.step_position {
                if same_pose(last, sample) {
                    return InsertResult::Duplicate;
                }
                self.frames = [None; CAPACITY];
                self.len = 0;
                self.next = 0;
                self.target_step = None;
                self.push_raw(sample);
                return InsertResult::Conflict;
            }
        }
        self.push_raw(sample);
        if reprime {
            InsertResult::Reprimed
        } else {
            InsertResult::Inserted
        }
    }

    fn push_raw(&mut self, sample: PoseSample) {
        self.frames[self.next] = Some(sample);
        self.next = (self.next + 1) % CAPACITY;
        self.len = self.len.saturating_add(1).min(CAPACITY);
    }

    fn frame(&self, offset: usize) -> Option<PoseSample> {
        if offset >= self.len {
            return None;
        }
        let start = (self.next + CAPACITY - self.len) % CAPACITY;
        self.frames[(start + offset) % CAPACITY]
    }

    pub fn latest_step(&self) -> Option<f64> {
        self.frame(self.len.checked_sub(1)?)
            .map(|sample| sample.step_position)
    }

    pub fn oldest_step(&self) -> Option<f64> {
        self.frame(0).map(|sample| sample.step_position)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn advance_target(&mut self, pace: RoomPace, now_nanos: u64) -> Option<f64> {
        let latest = self.latest_step()?;
        let rate = pace.rate_steps_per_second()?;
        let desired_lag = pace.desired_lag_steps(rate);
        if self.target_step.is_none() {
            let oldest = self.oldest_step()?;
            let target = latest - desired_lag;
            if target < oldest || self.interpolate(target).is_none() {
                self.target_wall_nanos = now_nanos;
                self.last_rate = rate;
                return None;
            }
            self.target_step = Some(target);
            self.target_wall_nanos = now_nanos;
            self.last_rate = rate;
            return Some(target);
        }

        let mut target = self.target_step?;
        let rate_changed = self.last_rate != rate;
        let elapsed = if rate_changed || self.target_wall_nanos == 0 {
            0.0
        } else {
            now_nanos.saturating_sub(self.target_wall_nanos) as f64 / 1_000_000_000.0
        };
        let lag = latest - target;
        let catch_up = !pace.speed_percent.eq(&0) && lag > desired_lag + 0.25;
        let effective_rate = if catch_up {
            rate * (100.0 + f64::from(MAX_CATCHUP_PERCENT)) / 100.0
        } else {
            rate
        };
        target = (target + elapsed * effective_rate).min(latest);
        self.target_step = Some(target);
        self.target_wall_nanos = now_nanos;
        self.last_rate = rate;
        self.interpolate(target).map(|_| target)
    }

    pub fn interpolate(&self, target_step: f64) -> Option<InterpolatedPose> {
        if !target_step.is_finite() || self.len < 2 {
            return None;
        }
        let mut lower: Option<PoseSample> = None;
        for index in 0..self.len {
            let sample = self.frame(index)?;
            if sample.capture_generation != self.generation
                || sample.world_epoch != self.world_epoch
                || sample.incarnation != self.incarnation
                || sample.invalidation_generation != self.invalidation_generation
                || !sample.valid()
            {
                return None;
            }
            if sample.step_position == target_step {
                return Some(InterpolatedPose {
                    key: sample.key,
                    count: sample.count,
                    target_step,
                    lower_step: target_step,
                    upper_step: target_step,
                    records: sample.records,
                });
            }
            if sample.step_position > target_step {
                let lower = lower?;
                let span = sample.step_position - lower.step_position;
                if span <= 0.0 || span > MAX_GAP_STEPS {
                    return None;
                }
                if sample.key != lower.key || sample.count != lower.count {
                    return None;
                }
                let fraction = (target_step - lower.step_position) / span;
                if !(0.0..=1.0).contains(&fraction) {
                    return None;
                }
                let mut records = [[0; 6]; MAX_RECORDS];
                for (record, (lower_record, upper_record)) in records
                    .iter_mut()
                    .zip(lower.records.iter().zip(sample.records.iter()))
                    .take(usize::from(sample.count))
                {
                    *record = interpolate_record(*lower_record, *upper_record, fraction)?;
                }
                return Some(InterpolatedPose {
                    key: sample.key,
                    count: sample.count,
                    target_step,
                    lower_step: lower.step_position,
                    upper_step: sample.step_position,
                    records,
                });
            }
            lower = Some(sample);
        }
        None
    }
}

fn same_pose(a: PoseSample, b: PoseSample) -> bool {
    a.key == b.key && a.count == b.count && a.records == b.records
}

fn valid_record(record: [u32; 6]) -> bool {
    let values = record.map(f32::from_bits);
    values.iter().all(|value| value.is_finite())
        && direction_length_squared([values[3], values[4], values[5]]) > 1.0e-10
}

fn direction_length_squared(direction: [f32; 3]) -> f32 {
    direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]
}

fn interpolate_record(a: [u32; 6], b: [u32; 6], t: f64) -> Option<[u32; 6]> {
    if !valid_record(a) || !valid_record(b) {
        return None;
    }
    let a = a.map(f32::from_bits);
    let b = b.map(f32::from_bits);
    let mut result = [0_u32; 6];
    for index in 0..3 {
        let value = (f64::from(a[index]) + (f64::from(b[index]) - f64::from(a[index])) * t) as f32;
        if !value.is_finite() {
            return None;
        }
        result[index] = value.to_bits();
    }
    let first = [a[3], a[4], a[5]];
    let second = [b[3], b[4], b[5]];
    let dot = first[0] * second[0] + first[1] * second[1] + first[2] * second[2];
    if !dot.is_finite() || dot <= -0.95 {
        return None;
    }
    let mut direction = [0.0; 3];
    for index in 0..3 {
        direction[index] = (f64::from(first[index])
            + (f64::from(second[index]) - f64::from(first[index])) * t)
            as f32;
    }
    let length_squared = direction_length_squared(direction);
    if !length_squared.is_finite() || length_squared <= 1.0e-10 {
        return None;
    }
    let inverse_length = length_squared.sqrt().recip();
    for index in 0..3 {
        let value = direction[index] * inverse_length;
        if !value.is_finite() {
            return None;
        }
        result[index + 3] = value.to_bits();
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> AcceptedAnchor {
        AcceptedAnchor {
            capture_generation: 7,
            world_epoch: 9,
            game_state: 0x1000,
            engine: 0x2000,
            clock_entity_ptr: 0x2100,
            clock_entity_id: 2,
            clock_kind: ClockKind::CGameTime,
            native_time: 500,
            tick_count: 12,
            update_count: 18,
            room_step: 91,
        }
    }

    fn pending() -> PendingCopy {
        PendingCopy {
            id: 1,
            source: anchor(),
            destination_game_state: 0x3000,
            destination_engine: 0x4000,
            family_revision: 3,
        }
    }

    fn readback() -> LocalReadback {
        LocalReadback {
            capture_generation: 7,
            world_epoch: 9,
            game_state: 0x3000,
            engine: 0x4000,
            clock_entity_ptr: 0x4100,
            clock_entity_id: 2,
            clock_kind: ClockKind::CGameTime,
            native_time: 500,
            tick_count: 12,
            update_count: 18,
            family_revision: 3,
        }
    }

    fn commit_proof(first_step: u64) -> CopyCommitProof {
        CopyCommitProof {
            first_step,
            reports_ok: true,
            capture_generation: 7,
            world_epoch: 9,
            game_state: 0x3000,
            engine: 0x4000,
            family_revision: 3,
        }
    }

    fn sample(step_position: f64, x: f32, direction: [f32; 3]) -> PoseSample {
        let mut records = [[0; 6]; MAX_RECORDS];
        records[0] = [
            x.to_bits(),
            2.0_f32.to_bits(),
            3.0_f32.to_bits(),
            direction[0].to_bits(),
            direction[1].to_bits(),
            direction[2].to_bits(),
        ];
        PoseSample {
            capture_generation: 7,
            world_epoch: 9,
            incarnation: 3,
            invalidation_generation: 1,
            key: FamilyKey {
                owner: 95_632,
                child: 88_266,
                component_index: 1,
                slot: 4,
                model_id: 46,
                definition: 0x5000,
            },
            step_position,
            sample_nanos: (step_position * 20_000_000.0) as u64,
            count: 1,
            records,
        }
    }

    #[test]
    fn a_copied_endpoint_needs_matching_local_reports_and_accepts_verified_zero() {
        let bound = bind_copy(pending(), readback()).unwrap();
        assert_eq!(
            commit_copy(bound, commit_proof(92)),
            Ok(AcceptedAnchor {
                game_state: 0x3000,
                engine: 0x4000,
                clock_entity_ptr: 0x4100,
                ..anchor()
            })
        );
        assert_eq!(
            commit_copy(bound, commit_proof(92)),
            Ok(AcceptedAnchor {
                game_state: 0x3000,
                engine: 0x4000,
                clock_entity_ptr: 0x4100,
                ..anchor()
            })
        );
        assert_eq!(
            commit_copy(
                bound,
                CopyCommitProof {
                    reports_ok: false,
                    ..commit_proof(92)
                },
            ),
            Err(CopyReject::Reports)
        );
    }

    #[test]
    fn invalidation_orders_pose_authorization_without_waiting_for_in_flight_render() {
        let epoch = ApplyEpoch::new();
        let observed = epoch.generation();
        let mut history = History::new();
        assert_eq!(
            history.push(sample(1.0, 1.0, [1.0, 0.0, 0.0])),
            InsertResult::Reprimed
        );
        assert_eq!(
            history.push(sample(2.0, 2.0, [1.0, 0.0, 0.0])),
            InsertResult::Inserted
        );
        let copied_pose = history.interpolate(1.5).unwrap();
        assert_eq!(f32::from_bits(copied_pose.records[0][0]), 1.5);

        // The renderer already copied an interpolated pose, but invalidation
        // wins before its final forwarding CAS. The exact original vector is
        // therefore still selected by the production forwarding predicate.
        epoch.invalidate();
        let stale_lease = epoch.try_lease(observed);
        assert!(!history_forward_eligible(
            true,
            true,
            true,
            stale_lease.is_some()
        ));
        let original_vector = 0x1234_usize;
        let forwarded_vector = if history_forward_eligible(true, true, true, stale_lease.is_some())
        {
            0x5678
        } else {
            original_vector
        };
        assert_eq!(forwarded_vector, original_vector);

        // If the forwarding CAS wins first, it is the operation's
        // linearization point. Invalidation remains nonblocking and permits
        // that one synchronous native call to finish.
        let accepted_epoch = epoch.generation();
        let in_flight = epoch.try_lease(accepted_epoch).unwrap();
        assert_eq!(in_flight.generation(), accepted_epoch);
        assert_eq!(epoch.invalidate(), accepted_epoch + 1);
        assert!(history_forward_eligible(true, true, true, true));
        drop(in_flight);
        assert!(epoch.try_lease(accepted_epoch).is_none());
    }

    #[test]
    fn forwarding_requires_the_marker_storage_header_and_epoch_lease() {
        assert!(!history_forward_eligible(false, true, true, true));
        assert!(!history_forward_eligible(true, false, true, true));
        assert!(!history_forward_eligible(true, true, false, true));
        assert!(!history_forward_eligible(true, true, true, false));
        assert!(history_forward_eligible(true, true, true, true));
    }

    #[test]
    fn a_changed_destination_clock_readback_never_binds_source_provenance() {
        let copy = pending();
        for readback in [
            LocalReadback {
                native_time: 501,
                ..readback()
            },
            LocalReadback {
                tick_count: 13,
                ..readback()
            },
            LocalReadback {
                update_count: 19,
                ..readback()
            },
            LocalReadback {
                clock_entity_id: 3,
                ..readback()
            },
            LocalReadback {
                clock_kind: ClockKind::Other,
                ..readback()
            },
        ] {
            assert!(bind_copy(copy, readback).is_err());
        }
    }

    #[test]
    fn removal_during_copy_or_local_step_invalidates_the_pending_incarnation() {
        let copy = pending();
        let changed_readback = LocalReadback {
            family_revision: 4,
            ..readback()
        };
        assert_eq!(
            bind_copy(copy, changed_readback),
            Err(CopyReject::Incarnation)
        );

        let bound = bind_copy(copy, readback()).unwrap();
        assert_eq!(
            commit_copy(
                bound,
                CopyCommitProof {
                    family_revision: 4,
                    ..commit_proof(92)
                },
            ),
            Err(CopyReject::Incarnation)
        );
    }

    #[test]
    fn copy_must_be_contiguous_and_stay_in_the_same_world_and_destination() {
        let bound = bind_copy(pending(), readback()).unwrap();
        assert_eq!(
            commit_copy(bound, commit_proof(93)),
            Err(CopyReject::StepBoundary)
        );
        assert_eq!(
            commit_copy(
                bound,
                CopyCommitProof {
                    world_epoch: 10,
                    ..commit_proof(92)
                },
            ),
            Err(CopyReject::World)
        );
        assert_eq!(
            commit_copy(
                bound,
                CopyCommitProof {
                    game_state: 0x3001,
                    ..commit_proof(92)
                },
            ),
            Err(CopyReject::Identity)
        );
    }

    #[test]
    fn pose_ring_rejects_conflicting_same_time_and_reprimes_on_family_change() {
        let mut history = History::new();
        assert_eq!(
            history.push(sample(1.0, 1.0, [1.0, 0.0, 0.0])),
            InsertResult::Reprimed
        );
        assert_eq!(
            history.push(sample(1.0, 2.0, [1.0, 0.0, 0.0])),
            InsertResult::Conflict
        );
        assert_eq!(history.len(), 1);
        let changed = PoseSample {
            key: FamilyKey {
                child: 88_267,
                ..sample(2.0, 3.0, [1.0, 0.0, 0.0]).key
            },
            ..sample(2.0, 3.0, [1.0, 0.0, 0.0])
        };
        assert_eq!(history.push(changed), InsertResult::Reprimed);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn interpolation_is_bounded_finite_and_normalizes_directions() {
        let mut history = History::new();
        assert_eq!(
            history.push(sample(1.0, 0.0, [1.0, 0.0, 0.0])),
            InsertResult::Reprimed
        );
        assert_eq!(
            history.push(sample(2.0, 10.0, [0.0, 1.0, 0.0])),
            InsertResult::Inserted
        );
        let result = history.interpolate(1.5).unwrap();
        assert_eq!(f32::from_bits(result.records[0][0]), 5.0);
        let direction = result.records[0][3..6]
            .iter()
            .map(|word| f32::from_bits(*word))
            .collect::<Vec<_>>();
        let length = direction
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!((length - 1.0).abs() < 1.0e-6);
        assert!(history.interpolate(0.0).is_none());
        assert!(history.interpolate(4.0).is_none());
        assert!(history.interpolate(f64::NAN).is_none());
    }

    #[test]
    fn interpolation_refuses_long_gaps_opposite_directions_and_bad_counts() {
        let mut gap = History::new();
        gap.push(sample(1.0, 0.0, [1.0, 0.0, 0.0]));
        gap.push(sample(4.0, 10.0, [1.0, 0.0, 0.0]));
        assert!(gap.interpolate(2.0).is_none());

        let mut opposite = History::new();
        opposite.push(sample(1.0, 0.0, [1.0, 0.0, 0.0]));
        opposite.push(sample(2.0, 10.0, [-1.0, 0.0, 0.0]));
        assert!(opposite.interpolate(1.5).is_none());

        let mut malformed = sample(1.0, 0.0, [1.0, 0.0, 0.0]);
        malformed.count = 5;
        assert_eq!(History::new().push(malformed), InsertResult::Invalid);
    }

    #[test]
    fn target_primes_from_history_and_is_continuous_through_speed_change_and_pause() {
        let mut history = History::new();
        for step in 0..=20 {
            history.push(sample(step as f64, step as f32, [1.0, 0.0, 0.0]));
        }
        let pace = RoomPace {
            active: true,
            steps_per_second: 5,
            speed_percent: 400,
            previous_speed_percent: 400,
            batch_two: true,
        };
        let initial = history.advance_target(pace, 1_000_000_000).unwrap();
        assert_eq!(initial, 14.0);
        let next = history.advance_target(pace, 1_050_000_000).unwrap();
        assert!((next - 15.0).abs() <= 0.01);
        let slower = RoomPace {
            speed_percent: 200,
            batch_two: false,
            ..pace
        };
        let transition = history.advance_target(slower, 1_060_000_000).unwrap();
        assert!((transition - next).abs() <= 0.01);
        let paused = RoomPace {
            speed_percent: 0,
            previous_speed_percent: 200,
            ..slower
        };
        let drained = history.advance_target(paused, 5_000_000_000).unwrap();
        assert_eq!(drained, 20.0);
        assert_eq!(history.advance_target(paused, 6_000_000_000), Some(20.0));
    }
}
