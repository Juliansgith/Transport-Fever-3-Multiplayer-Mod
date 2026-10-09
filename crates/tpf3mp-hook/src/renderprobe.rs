//! Bounded observe-only renderer feasibility probe for Steam build 40420.
//!
//! This probe records the renderer's producer frame, its joined Lambda7 work
//! context, a small sample of moving fat ModelInstance values, and an optional
//! bounded road-vehicle transform seam. It never changes a renderer result,
//! writes ECS memory, follows Engine's generation vector, or retains a native
//! pointer after its call scope. Neither sample proves that the observed path
//! carries a vehicle's root/world transform.

#![allow(unsafe_code)]

use std::{
    fs,
    io::Read,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
};

pub const ENV: &str = "TPF3MP_HOOK_RENDER_PROBE";
/// Adds observe-only native GameTime labels to selected road-vehicle rows.
/// Requires `ENV=1` and the timeline watcher; no clock or render value changes.
pub const NATIVE_TIME_ENV: &str = "TPF3MP_HOOK_NATIVE_TIME_PROBE";
/// Applies a diagnostic translation to a cloned road-transform vector during
/// an explicitly armed renderer timeline capture.
pub const ROAD_OFFSET_ENV: &str = "TPF3MP_HOOK_ROAD_OFFSET_PROBE";
/// Uses only previously committed, copied road-position samples during an
/// explicitly armed five-second timeline capture. Requires both probe modes.
pub const ROAD_HISTORY_ENV: &str = "TPF3MP_HOOK_ROAD_HISTORY";
pub const ENTITY_FILE: &str = "render-probe.entities";
pub const MOVING_INSTANCE_LIMIT: usize = 4;
pub const MAX_TARGET_ENTITIES: usize = 128;
pub const MAX_ENTITY_FILE_BYTES: usize = 2_048;
pub const MAX_DISCOVERY_INSPECTIONS: u32 = 512;
pub const MAX_SELECTED_POSE_INSPECTIONS: u32 = 4_096;
pub const MAX_ROAD_VEHICLE_SAMPLES: u32 = 512;
const MAX_ROAD_VEHICLE_KEYS: usize = 2;
const MAX_WORLD_POSITION_RECORDS: usize = 4;
const MAX_NATIVE_TIME_ENDPOINTS: usize = 64;
const MAX_NATIVE_WRITER_ENGINES: usize = 16;
const MAX_NATIVE_BATCH_UPDATES: usize = 16;
const ROAD_OFFSET_X_METERS: f32 = 5.0;
const MAX_DISPATCH_CONTEXTS: usize = 16;
const THREAD_SAMPLE_LIMIT: usize = 4;
const EMPTY: u8 = 0;
const READY: u8 = 1;
const TIME_REJECT_NONE: u8 = 0;
const TIME_REJECT_NESTED: u8 = 2;
const TIME_REJECT_GETTER_COUNT: u8 = 3;
const TIME_REJECT_CLOCK_KEY: u8 = 4;
const TIME_REJECT_NO_BODY: u8 = 5;
const TIME_REJECT_ENGINE: u8 = 6;
const TIME_REJECT_WRITER: u8 = 7;
const TIME_REJECT_PREVIOUS_INVALID: u8 = 8;
const TIME_REJECT_ALPHA: u8 = 9;
const TIME_REJECT_UNMAPPED: u8 = 10;
const TIME_REJECT_GAP: u8 = 11;
const TIME_REJECT_INTERPOLATION: u8 = 12;
const TIME_REJECT_MAP_BUSY: u8 = 13;
const TIME_REJECT_CAPTURE: u8 = 14;
const TIME_REJECT_SAMPLE: u8 = 15;
const TIME_REJECT_COUNT_GAP: u8 = 16;
const TIME_REJECT_REPORT: u8 = 17;
const TIME_REJECT_WORLD: u8 = 18;
const TIME_REJECT_MAP_CONFLICT: u8 = 19;
const TIME_REJECT_ZERO_UNKNOWN: u8 = 20;
const TIME_REJECT_REPLICA: u8 = 21;
const TIME_REJECT_REPLICA_UNAVAILABLE: u8 = 22;

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_DISPATCH: AtomicU64 = AtomicU64::new(1);
static ACTIVE_DISPATCHES: AtomicU32 = AtomicU32::new(0);
static CONTEXT_MISSES: AtomicU64 = AtomicU64::new(0);
static DISCOVERY_INSPECTIONS: AtomicU32 = AtomicU32::new(0);
static SELECTED_POSE_INSPECTIONS: AtomicU32 = AtomicU32::new(0);
static ALLOWLIST_GENERATION: AtomicU64 = AtomicU64::new(0);
static ALLOWLIST_VALID: AtomicBool = AtomicBool::new(false);
static ALLOWLIST_STATUS: AtomicU8 = AtomicU8::new(AllowlistStatus::Disabled as u8);
static ALLOWLIST_LEN: AtomicUsize = AtomicUsize::new(0);
static ALLOWLIST_IDS: [AtomicU32; MAX_TARGET_ENTITIES] =
    [const { AtomicU32::new(0) }; MAX_TARGET_ENTITIES];
static NEW_UPDATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MODEL_LIST_INDEX_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DISPATCH_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static BODY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static GET_INSTANCE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static ROAD_VF3_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static ROAD_PATH_HELPER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static ROAD_USER_TRANSFORMS_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static NATIVE_TIME_ENABLED: AtomicBool = AtomicBool::new(false);
static NATIVE_TIME_FUNCTION_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static NATIVE_TIME_CURRENT_GETTER: AtomicUsize = AtomicUsize::new(0);
static NATIVE_TIME_PREVIOUS_GETTER: AtomicUsize = AtomicUsize::new(0);
static ROAD_OFFSET_ENABLED: AtomicBool = AtomicBool::new(false);
static ROAD_HISTORY_ENABLED: AtomicBool = AtomicBool::new(false);
static ROAD_HISTORY_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static GAME_STATE_REPLICATE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static ENGINE_REMOVE_ENTITY_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static NEXT_REPLICA_COPY: AtomicU64 = AtomicU64::new(1);
static LATEST_REPLICA_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static PENDING_REPLICA_COPY: Mutex<Option<PendingReplicaCopy>> = Mutex::new(None);
static REGISTRY: OnceLock<DispatchRegistry<MAX_DISPATCH_CONTEXTS>> = OnceLock::new();
static SAMPLES: Mutex<SampleSet> = Mutex::new(SampleSet::new());
static ROAD_SAMPLES: Mutex<RoadVehicleSamples> = Mutex::new(RoadVehicleSamples::new());
static NATIVE_TIME_MAP: Mutex<NativeTimeMap> = Mutex::new(NativeTimeMap::new());
static NATIVE_TIME_WORLD_EPOCH: AtomicU64 = AtomicU64::new(1);
static NATIVE_WRITER_SLOTS: [NativeWriterSlot; MAX_NATIVE_WRITER_ENGINES] =
    [const { NativeWriterSlot::new() }; MAX_NATIVE_WRITER_ENGINES];
static NATIVE_WRITER_SKIPS: AtomicU64 = AtomicU64::new(0);
static NATIVE_MAP_TRACKING_FAILED: AtomicBool = AtomicBool::new(false);
static SAMPLE_SKIPS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ClockKey {
    engine: usize,
    entity_ptr: usize,
    entity_id: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NativeClockSample {
    key: ClockKey,
    time: i64,
    tick_count: u32,
    update_count: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NativeTimeEndpoint {
    used: bool,
    capture_generation: u64,
    world_epoch: u64,
    game_state: usize,
    engine: usize,
    clock: ClockKey,
    time: i64,
    tick_count: u32,
    room_step: u64,
    update_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct PendingReplicaCopy {
    evidence: crate::road_history::PendingCopy,
}

#[derive(Debug)]
struct NativeTimeMap {
    capture_generation: u64,
    world_mark: Option<crate::step::WorldMark>,
    world_epoch: u64,
    endpoints: [NativeTimeEndpoint; MAX_NATIVE_TIME_ENDPOINTS],
    next: usize,
    len: usize,
}

impl NativeTimeMap {
    const fn new() -> Self {
        Self {
            capture_generation: 0,
            world_mark: None,
            world_epoch: 0,
            endpoints: [NativeTimeEndpoint {
                used: false,
                capture_generation: 0,
                world_epoch: 0,
                game_state: 0,
                engine: 0,
                clock: ClockKey {
                    engine: 0,
                    entity_ptr: 0,
                    entity_id: 0,
                },
                time: 0,
                tick_count: 0,
                room_step: 0,
                update_count: 0,
            }; MAX_NATIVE_TIME_ENDPOINTS],
            next: 0,
            len: 0,
        }
    }

    fn clear_endpoints(&mut self) {
        self.endpoints = [NativeTimeEndpoint::default(); MAX_NATIVE_TIME_ENDPOINTS];
        self.next = 0;
        self.len = 0;
    }

    fn ensure_capture(&mut self, generation: u64) {
        if self.capture_generation == generation {
            return;
        }
        self.capture_generation = generation;
        self.world_mark = None;
        self.world_epoch = NATIVE_TIME_WORLD_EPOCH
            .fetch_add(1, Ordering::Relaxed)
            .max(1);
        self.clear_endpoints();
    }

    fn set_world(&mut self, mark: Option<crate::step::WorldMark>) -> bool {
        if self.world_mark == mark {
            return false;
        }
        self.world_mark = mark;
        self.world_epoch = NATIVE_TIME_WORLD_EPOCH
            .fetch_add(1, Ordering::Relaxed)
            .max(1);
        self.clear_endpoints();
        true
    }

    fn invalidate(&mut self) {
        self.clear_endpoints();
    }

    fn invalidate_engine(&mut self, engine: usize) {
        if engine == 0 {
            return;
        }
        for endpoint in &mut self.endpoints {
            if endpoint.used && endpoint.engine == engine {
                *endpoint = NativeTimeEndpoint::default();
            }
        }
    }

    fn latest_for(
        &self,
        generation: u64,
        world_epoch: u64,
        game_state: usize,
        engine: usize,
    ) -> Option<NativeTimeEndpoint> {
        self.endpoints
            .iter()
            .take(self.len)
            .filter(|endpoint| {
                endpoint.used
                    && endpoint.capture_generation == generation
                    && endpoint.world_epoch == world_epoch
                    && endpoint.game_state == game_state
                    && endpoint.engine == engine
            })
            .max_by_key(|endpoint| endpoint.room_step)
            .copied()
    }

    fn insert(&mut self, endpoint: NativeTimeEndpoint) -> bool {
        if !endpoint.used || endpoint.engine == 0 || endpoint.game_state == 0 {
            return false;
        }
        if let Some(present) = self.endpoints.iter().take(self.len).find(|present| {
            present.used
                && present.capture_generation == endpoint.capture_generation
                && present.world_epoch == endpoint.world_epoch
                && present.game_state == endpoint.game_state
                && present.engine == endpoint.engine
                && present.clock == endpoint.clock
                && present.time == endpoint.time
        }) {
            if present.room_step == endpoint.room_step
                && present.tick_count == endpoint.tick_count
                && present.update_count == endpoint.update_count
            {
                return true;
            }
            return false;
        }
        self.endpoints[self.next] = endpoint;
        self.next = (self.next + 1) % self.endpoints.len();
        self.len = self.len.saturating_add(1).min(self.endpoints.len());
        true
    }

    fn lookup(
        &self,
        generation: u64,
        world_epoch: u64,
        game_state: usize,
        engine: usize,
        clock: ClockKey,
        time: i64,
    ) -> Option<NativeTimeEndpoint> {
        self.endpoints
            .iter()
            .take(self.len)
            .find(|entry| {
                entry.used
                    && entry.capture_generation == generation
                    && entry.world_epoch == world_epoch
                    && entry.game_state == game_state
                    && entry.engine == engine
                    && entry.clock == clock
                    && entry.time == time
            })
            .copied()
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeBatchPoints {
    entries: [NativeTimeEndpoint; MAX_NATIVE_BATCH_UPDATES],
    len: usize,
}

impl NativeBatchPoints {
    const fn empty() -> Self {
        Self {
            entries: [NativeTimeEndpoint {
                used: false,
                capture_generation: 0,
                world_epoch: 0,
                game_state: 0,
                engine: 0,
                clock: ClockKey {
                    engine: 0,
                    entity_ptr: 0,
                    entity_id: 0,
                },
                time: 0,
                tick_count: 0,
                room_step: 0,
                update_count: 0,
            }; MAX_NATIVE_BATCH_UPDATES],
            len: 0,
        }
    }
}

fn native_batch_points(
    candidate: NativeTimeCandidate,
    first_step: u64,
    world_epoch: u64,
) -> Result<NativeBatchPoints, u8> {
    if candidate.updates == 0 {
        return if candidate.before.time == candidate.after.time
            && candidate.before.update_count == candidate.after.update_count
            && candidate.before.tick_count == candidate.after.tick_count
        {
            Ok(NativeBatchPoints::empty())
        } else {
            Err(TIME_REJECT_SAMPLE)
        };
    }
    let step_time = i64::try_from(candidate.frame_time_micros / 1_000).ok();
    if candidate.updates as usize > MAX_NATIVE_BATCH_UPDATES
        || candidate.before.key != candidate.after.key
        || candidate.before.key.engine != candidate.engine
        || step_time.and_then(|step| step.checked_mul(i64::from(candidate.updates)))
            != candidate.after.time.checked_sub(candidate.before.time)
        || candidate
            .after
            .update_count
            .wrapping_sub(candidate.before.update_count)
            != candidate.updates
        || candidate
            .after
            .tick_count
            .wrapping_sub(candidate.before.tick_count)
            != candidate.updates
    {
        return Err(TIME_REJECT_COUNT_GAP);
    }
    let mut points = NativeBatchPoints::empty();
    let Some(step_time) = step_time else {
        return Err(TIME_REJECT_COUNT_GAP);
    };
    for offset in 1..=candidate.updates {
        let Some(time) = candidate.before.time.checked_add(
            step_time
                .checked_mul(i64::from(offset))
                .ok_or(TIME_REJECT_COUNT_GAP)?,
        ) else {
            return Err(TIME_REJECT_COUNT_GAP);
        };
        points.entries[points.len] = NativeTimeEndpoint {
            used: true,
            capture_generation: candidate.capture_generation,
            world_epoch,
            game_state: candidate.game_state,
            engine: candidate.engine,
            clock: candidate.after.key,
            time,
            tick_count: candidate.before.tick_count.wrapping_add(offset),
            room_step: first_step.saturating_add(u64::from(offset - 1)),
            update_count: candidate.before.update_count.wrapping_add(offset),
        };
        points.len += 1;
    }
    Ok(points)
}

#[derive(Debug)]
struct NativeWriterSlot {
    engine: AtomicUsize,
    version: AtomicU64,
    active: AtomicU32,
    map_rebuild_state: AtomicU64,
}

impl NativeWriterSlot {
    const fn new() -> Self {
        Self {
            engine: AtomicUsize::new(0),
            version: AtomicU64::new(0),
            active: AtomicU32::new(0),
            map_rebuild_state: AtomicU64::new(0),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NativeWriterSnapshot {
    known: bool,
    stable: bool,
    version: u64,
    active: u32,
}

impl NativeWriterSnapshot {
    const EMPTY: Self = Self {
        known: false,
        stable: false,
        version: 0,
        active: 0,
    };
}

#[derive(Debug, Clone, Copy, Default)]
struct NativeWriterToken {
    slot: usize,
    engine: usize,
    entry_version: u64,
    valid: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct NativeTimeObservation {
    present: bool,
    nested: bool,
    generation: u64,
    current_calls: u8,
    previous_calls: u8,
    current: Option<i64>,
    previous: Option<i64>,
    current_key: Option<ClockKey>,
    previous_key: Option<ClockKey>,
    previous_valid: u8,
    alpha_bits: u32,
    interp_return: i64,
    sample_nanos: u64,
    previous_step: u64,
    current_step: u64,
    previous_update_count: u32,
    current_update_count: u32,
    world_epoch: u64,
    label_valid: bool,
    writer_start: NativeWriterSnapshot,
    writer_end: NativeWriterSnapshot,
    rejection: u8,
}

impl NativeTimeObservation {
    const EMPTY: Self = Self {
        present: false,
        nested: false,
        generation: 0,
        current_calls: 0,
        previous_calls: 0,
        current: None,
        previous: None,
        current_key: None,
        previous_key: None,
        previous_valid: 0,
        alpha_bits: 0,
        interp_return: 0,
        sample_nanos: 0,
        previous_step: 0,
        current_step: 0,
        previous_update_count: 0,
        current_update_count: 0,
        world_epoch: 0,
        label_valid: false,
        writer_start: NativeWriterSnapshot::EMPTY,
        writer_end: NativeWriterSnapshot::EMPTY,
        rejection: 0,
    };
}

#[derive(Debug, Clone, Copy, Default)]
struct NativeTimeCandidate {
    present: bool,
    sample_valid: bool,
    nested: bool,
    capture_generation: u64,
    native_this: usize,
    game_state: usize,
    engine: usize,
    updates: u32,
    frame_time_micros: usize,
    room: bool,
    before: NativeClockSample,
    after: NativeClockSample,
    bound_copy: Option<crate::road_history::BoundCopy>,
    pending_copy_id: u64,
    sample_nanos: u64,
    writer_version_start: u64,
    writer_version_return: u64,
    writer_version_end: u64,
    writer_active_return: u32,
    writer_active_end: u32,
    writer_stable: bool,
    rejection: u8,
}

#[derive(Debug, Clone, Copy, Default)]
struct NativeTimeReadScope {
    active: bool,
    nested: bool,
    capture_generation: u64,
    clock_object: usize,
    current_calls: u8,
    previous_calls: u8,
    current: Option<i64>,
    previous: Option<i64>,
    current_key: Option<ClockKey>,
    previous_key: Option<ClockKey>,
    previous_valid: u8,
    rejection: u8,
}

impl NativeTimeReadScope {
    const EMPTY: Self = Self {
        active: false,
        nested: false,
        capture_generation: 0,
        clock_object: 0,
        current_calls: 0,
        previous_calls: 0,
        current: None,
        previous: None,
        current_key: None,
        previous_key: None,
        previous_valid: 0,
        rejection: 0,
    };
}

#[derive(Debug, Clone, Copy, Default)]
struct NativeStepContext {
    active: bool,
    capture: bool,
    nested: bool,
    capture_generation: u64,
    native_this: usize,
    game_state: usize,
    engine: usize,
    room: bool,
    updates: u32,
    frame_time_micros: usize,
    clock_object: usize,
    before: Option<NativeClockSample>,
    bound_copy: Option<crate::road_history::BoundCopy>,
    pending_copy_id: u64,
    writer: NativeWriterToken,
}

impl NativeStepContext {
    const EMPTY: Self = Self {
        active: false,
        capture: false,
        nested: false,
        capture_generation: 0,
        native_this: 0,
        game_state: 0,
        engine: 0,
        room: false,
        updates: 0,
        frame_time_micros: 0,
        clock_object: 0,
        before: None,
        bound_copy: None,
        pending_copy_id: 0,
        writer: NativeWriterToken {
            slot: usize::MAX,
            engine: 0,
            entry_version: 0,
            valid: false,
        },
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum AllowlistStatus {
    Disabled = 0,
    Ready = 1,
    Missing = 2,
    Oversized = 3,
    Malformed = 4,
    Empty = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntityAllowlist {
    ids: [u32; MAX_TARGET_ENTITIES],
    len: usize,
    status: AllowlistStatus,
}

impl EntityAllowlist {
    const fn invalid(status: AllowlistStatus) -> Self {
        Self {
            ids: [0; MAX_TARGET_ENTITIES],
            len: 0,
            status,
        }
    }

    pub(crate) fn is_valid(self) -> bool {
        self.status == AllowlistStatus::Ready
    }

    pub(crate) fn status(self) -> AllowlistStatus {
        self.status
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RenderMeta {
    capture_generation: u64,
    epoch: u64,
    renderer: usize,
    game_state: usize,
    engine: usize,
    type_index: i32,
    binding_valid: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct ProducerContext {
    captured: bool,
    meta: RenderMeta,
}

thread_local! {
    static PRODUCER: std::cell::Cell<ProducerContext> = const {
        std::cell::Cell::new(ProducerContext { captured: false, meta: RenderMeta {
            capture_generation: 0, epoch: 0, renderer: 0, game_state: 0,
            engine: 0, type_index: -1, binding_valid: false,
        } })
    };
    static BODY: std::cell::Cell<BodyContext> = const {
        std::cell::Cell::new(BodyContext::EMPTY)
    };
    static ROAD_SCOPE: std::cell::Cell<RoadObservation> = const {
        std::cell::Cell::new(RoadObservation::EMPTY)
    };
    static NATIVE_TIME_SCOPE: std::cell::Cell<NativeTimeReadScope> = const {
        std::cell::Cell::new(NativeTimeReadScope::EMPTY)
    };
    static NATIVE_STEP_CONTEXT: std::cell::Cell<NativeStepContext> = const {
        std::cell::Cell::new(NativeStepContext::EMPTY)
    };
    static NATIVE_STEP_CANDIDATE: std::cell::Cell<Option<NativeTimeCandidate>> = const {
        std::cell::Cell::new(None)
    };
}

struct ProducerScope(ProducerContext);

impl ProducerScope {
    fn enter(context: ProducerContext) -> Self {
        let previous = PRODUCER.with(|slot| slot.replace(context));
        Self(previous)
    }

    fn current() -> ProducerContext {
        PRODUCER.with(std::cell::Cell::get)
    }
}

impl Drop for ProducerScope {
    fn drop(&mut self) {
        PRODUCER.with(|slot| slot.set(self.0));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RenderKey {
    entity: u32,
    entity_component_index: u32,
    slot: i32,
    model_id: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct BodyContext {
    registry_index: usize,
    key: usize,
    dispatch_id: u64,
    meta: RenderMeta,
    fresh_instance: FreshInstance,
    active: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FreshInstance {
    present: bool,
    entity: u32,
    entity_component_index: u32,
    slot: i32,
    returned: usize,
    capture_generation: u64,
    dispatch_id: u64,
}

impl BodyContext {
    const EMPTY: Self = Self {
        registry_index: usize::MAX,
        key: 0,
        dispatch_id: 0,
        meta: RenderMeta {
            capture_generation: 0,
            epoch: 0,
            renderer: 0,
            game_state: 0,
            engine: 0,
            type_index: -1,
            binding_valid: false,
        },
        fresh_instance: FreshInstance {
            present: false,
            entity: 0,
            entity_component_index: 0,
            slot: 0,
            returned: 0,
            capture_generation: 0,
            dispatch_id: 0,
        },
        active: false,
    };
}

fn same_live_body(
    expected: BodyContext,
    current: BodyContext,
    generation: u64,
    current_generation: u64,
    registry_observable: bool,
) -> bool {
    expected.active
        && expected.key != 0
        && expected.meta.binding_valid
        && expected.meta.capture_generation == generation
        && generation != 0
        && generation == current_generation
        && current.active
        && current.key == expected.key
        && current.dispatch_id == expected.dispatch_id
        && current.meta == expected.meta
        && current.meta.binding_valid
        && registry_observable
}

fn road_body_observable(expected: BodyContext, generation: u64) -> bool {
    let current_generation = crate::timeline::current_generation();
    BODY.with(|cell| {
        let current = cell.get();
        let registry_observable = current.active
            && current.key != 0
            && registry().remains_observable(current.registry_index, current.key, generation);
        same_live_body(
            expected,
            current,
            generation,
            current_generation,
            registry_observable,
        )
    })
}

fn road_scope_observable(observation: RoadObservation) -> bool {
    if !observation.active
        || !observation.fresh_instance.present
        || observation.meta.capture_generation == 0
        || observation.meta.capture_generation != crate::timeline::current_generation()
    {
        return false;
    }
    let current = BODY.with(|cell| cell.get());
    current.dispatch_id == observation.fresh_instance.dispatch_id
        && current.meta == observation.meta
        && road_body_observable(current, observation.meta.capture_generation)
}

fn read_between_live_checks<T>(
    is_live: impl Fn() -> bool,
    read: impl FnOnce() -> Option<T>,
) -> Option<T> {
    if !is_live() {
        return None;
    }
    let value = read()?;
    is_live().then_some(value)
}

fn road_call_span(started_nanos: u64, finished_nanos: u64) -> (u64, u64) {
    let started_nanos = started_nanos.min(finished_nanos);
    (started_nanos, finished_nanos.saturating_sub(started_nanos))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VehicleKey {
    owner: u32,
    child: u32,
    component: u32,
    slot: i32,
    model_id: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VehicleSampleEntry {
    used: bool,
    key: VehicleKey,
    last_dispatch_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VehicleSampleDecision {
    Selected,
    Discover,
    Skip,
}

struct RoadVehicleSamples {
    capture_generation: u64,
    entries: [VehicleSampleEntry; MAX_ROAD_VEHICLE_KEYS],
    samples: u32,
    skips: u64,
    refused: u64,
}

impl RoadVehicleSamples {
    const fn new() -> Self {
        Self {
            capture_generation: 0,
            entries: [VehicleSampleEntry {
                used: false,
                key: VehicleKey {
                    owner: 0,
                    child: 0,
                    component: 0,
                    slot: 0,
                    model_id: 0,
                },
                last_dispatch_id: 0,
            }; MAX_ROAD_VEHICLE_KEYS],
            samples: 0,
            skips: 0,
            refused: 0,
        }
    }

    fn clear(&mut self, generation: u64) {
        self.capture_generation = generation;
        self.entries = [VehicleSampleEntry::default(); MAX_ROAD_VEHICLE_KEYS];
        self.samples = 0;
        self.skips = 0;
        self.refused = 0;
    }

    fn ensure_generation(&mut self, generation: u64) {
        if self.capture_generation != generation {
            self.clear(generation);
        }
    }

    fn decide(
        &mut self,
        generation: u64,
        key: VehicleKey,
        dispatch_id: u64,
        moving: bool,
    ) -> VehicleSampleDecision {
        self.ensure_generation(generation);
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.used
                && entry.key.owner == key.owner
                && entry.key.child == key.child
                && entry.key.component == key.component
                && entry.key.slot == key.slot
        }) {
            entry.key.model_id = key.model_id;
            if entry.last_dispatch_id == dispatch_id {
                return VehicleSampleDecision::Skip;
            }
            entry.last_dispatch_id = dispatch_id;
            return VehicleSampleDecision::Selected;
        }
        if !moving {
            return VehicleSampleDecision::Skip;
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.used && entry.key.owner == key.owner)
        {
            return VehicleSampleDecision::Skip;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| !entry.used) else {
            return VehicleSampleDecision::Skip;
        };
        *entry = VehicleSampleEntry {
            used: true,
            key,
            last_dispatch_id: dispatch_id,
        };
        VehicleSampleDecision::Discover
    }

    fn note_sample(&mut self) -> bool {
        if self.samples >= MAX_ROAD_VEHICLE_SAMPLES {
            self.skips = self.skips.saturating_add(1);
            return false;
        }
        self.samples += 1;
        true
    }

    fn note_skip(&mut self) {
        self.skips = self.skips.saturating_add(1);
    }

    fn note_refused(&mut self) {
        self.refused = self.refused.saturating_add(1);
    }
}

fn decide_road_sample(
    samples: &mut RoadVehicleSamples,
    captured_generation: u64,
    current_generation: u64,
    key: VehicleKey,
    dispatch_id: u64,
    moving: bool,
) -> Option<VehicleSampleDecision> {
    if captured_generation == 0 || captured_generation != current_generation {
        return None;
    }
    samples.ensure_generation(captured_generation);
    Some(samples.decide(captured_generation, key, dispatch_id, moving))
}

#[derive(Debug, Clone, Copy, Default)]
struct RoadObservation {
    active: bool,
    invalid_reason: u8,
    started_nanos: u64,
    meta: RenderMeta,
    context: usize,
    transformator: usize,
    model_input: usize,
    output: usize,
    child: u32,
    model_id: u32,
    model_instance: usize,
    fresh_instance: FreshInstance,
    alpha_bits: u32,
    owner: u32,
    path_seen: bool,
    path_valid_before: u8,
    path_valid_after: u8,
    path_current_before: [u32; 10],
    path_previous_before: [u32; 10],
    path_current_after: [u32; 10],
    path_previous_after: [u32; 10],
    path_result_pointer: usize,
    path_result_word0: u64,
    path_result_word1: u32,
    helper_engine: usize,
    helper_out: usize,
    helper_type_index: u32,
    helper_network: usize,
    helper_move_path: usize,
    helper_alpha_bits: u32,
    helper_calls: u8,
    finish_definition: usize,
    finish_output: usize,
    finish_alpha_bits: u32,
    finish_return: usize,
    finish_calls: u8,
    world_records_total: u16,
    world_records_copied: u8,
    world_records_truncated: bool,
    world_points: [u32; 24],
    native_time: NativeTimeObservation,
    native_writer_start: NativeWriterSnapshot,
    native_writer_end: NativeWriterSnapshot,
    history_family_revision: u64,
    history_invalidation_generation: u64,
    vf3_writer_start: NativeWriterSnapshot,
    vf3_writer_end: NativeWriterSnapshot,
    invalid: bool,
}

impl RoadObservation {
    const EMPTY: Self = Self {
        active: false,
        invalid_reason: 0,
        started_nanos: 0,
        meta: RenderMeta {
            capture_generation: 0,
            epoch: 0,
            renderer: 0,
            game_state: 0,
            engine: 0,
            type_index: -1,
            binding_valid: false,
        },
        context: 0,
        transformator: 0,
        model_input: 0,
        output: 0,
        child: 0,
        model_id: 0,
        model_instance: 0,
        fresh_instance: FreshInstance {
            present: false,
            entity: 0,
            entity_component_index: 0,
            slot: 0,
            returned: 0,
            capture_generation: 0,
            dispatch_id: 0,
        },
        alpha_bits: 0,
        owner: 0,
        path_seen: false,
        path_valid_before: 0,
        path_valid_after: 0,
        path_current_before: [0; 10],
        path_previous_before: [0; 10],
        path_current_after: [0; 10],
        path_previous_after: [0; 10],
        path_result_pointer: 0,
        path_result_word0: 0,
        path_result_word1: 0,
        helper_engine: 0,
        helper_out: 0,
        helper_type_index: 0,
        helper_network: 0,
        helper_move_path: 0,
        helper_alpha_bits: 0,
        helper_calls: 0,
        finish_definition: 0,
        finish_output: 0,
        finish_alpha_bits: 0,
        finish_return: 0,
        finish_calls: 0,
        world_records_total: 0,
        world_records_copied: 0,
        world_records_truncated: false,
        world_points: [0; 24],
        native_time: NativeTimeObservation::EMPTY,
        native_writer_start: NativeWriterSnapshot::EMPTY,
        native_writer_end: NativeWriterSnapshot::EMPTY,
        history_family_revision: 0,
        history_invalidation_generation: 0,
        vf3_writer_start: NativeWriterSnapshot::EMPTY,
        vf3_writer_end: NativeWriterSnapshot::EMPTY,
        invalid: false,
    };
}

fn path_read_eligible(
    observation: RoadObservation,
    current_generation: u64,
    body_matches: bool,
    engine: usize,
    owner_allowlisted: bool,
) -> bool {
    observation.active
        && !observation.invalid
        && !observation.path_seen
        && observation.helper_calls == 0
        && observation.meta.capture_generation != 0
        && observation.meta.capture_generation == current_generation
        && body_matches
        && engine != 0
        && engine == observation.meta.engine
        && owner_allowlisted
}

fn finish_read_eligible(
    observation: RoadObservation,
    current_generation: u64,
    owner_allowlisted: bool,
) -> bool {
    observation.active
        && !observation.invalid
        && observation.path_seen
        && observation.finish_calls == 0
        && observation.meta.capture_generation != 0
        && observation.meta.capture_generation == current_generation
        && owner_allowlisted
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn history_writer_matches(
    observation: RoadObservation,
    time: NativeTimeObservation,
    current: NativeWriterSnapshot,
) -> bool {
    let start = observation.vf3_writer_start;
    current.known
        && current.stable
        && current.active == 0
        && start.known
        && start.stable
        && start.active == 0
        && current == start
        && observation.vf3_writer_end == NativeWriterSnapshot::EMPTY
        && observation.helper_engine != 0
        && observation.helper_engine == observation.meta.engine
        && time.writer_start == start
        && time.writer_end == start
        && observation.native_writer_start == start
        && observation.native_writer_end == start
}

struct RoadScope(RoadObservation);

impl RoadScope {
    fn enter(observation: RoadObservation) -> Self {
        Self(ROAD_SCOPE.with(|slot| slot.replace(observation)))
    }

    fn current() -> RoadObservation {
        ROAD_SCOPE.with(std::cell::Cell::get)
    }

    fn update(update: impl FnOnce(&mut RoadObservation)) {
        ROAD_SCOPE.with(|slot| {
            let mut observation = slot.get();
            update(&mut observation);
            slot.set(observation);
        });
    }
}

impl Drop for RoadScope {
    fn drop(&mut self) {
        ROAD_SCOPE.with(|slot| slot.set(self.0));
    }
}

#[inline]
pub(crate) fn native_time_probe_enabled() -> bool {
    NATIVE_TIME_ENABLED.load(Ordering::Acquire)
}

fn native_writer_slot(engine: usize) -> Option<usize> {
    if engine == 0 {
        return None;
    }
    for (index, slot) in NATIVE_WRITER_SLOTS.iter().enumerate() {
        let existing = slot.engine.load(Ordering::Acquire);
        if existing == engine {
            return Some(index);
        }
        if existing == 0
            && slot
                .engine
                .compare_exchange(0, engine, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            return Some(index);
        }
    }
    NATIVE_WRITER_SKIPS.fetch_add(1, Ordering::Relaxed);
    None
}

fn mark_native_map_rebuild_pending(engine: usize) -> Option<u64> {
    if let Some(index) = native_writer_slot(engine) {
        Some(mark_map_rebuild_state(
            &NATIVE_WRITER_SLOTS[index].map_rebuild_state,
        ))
    } else {
        NATIVE_MAP_TRACKING_FAILED.store(true, Ordering::Release);
        None
    }
}

fn native_map_rebuild_state(engine: usize) -> Option<u64> {
    if NATIVE_MAP_TRACKING_FAILED.load(Ordering::Acquire) {
        return None;
    }
    native_writer_slot(engine).map(|index| {
        NATIVE_WRITER_SLOTS[index]
            .map_rebuild_state
            .load(Ordering::Acquire)
    })
}

fn native_map_rebuild_pending(engine: usize) -> bool {
    native_map_rebuild_state(engine).is_none_or(|state| state & 1 != 0)
}

fn clear_native_map_rebuild_pending(engine: usize, expected: u64) -> bool {
    if expected & 1 == 0 {
        return true;
    }
    let Some(index) = native_writer_slot(engine) else {
        return false;
    };
    clear_map_rebuild_state(&NATIVE_WRITER_SLOTS[index].map_rebuild_state, expected)
}

fn mark_map_rebuild_state(state: &AtomicU64) -> u64 {
    let previous = state
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.wrapping_add(2) | 1)
        })
        .unwrap_or_else(|current| current);
    previous.wrapping_add(2) | 1
}

fn clear_map_rebuild_state(state: &AtomicU64, expected: u64) -> bool {
    expected & 1 == 0
        || state
            .compare_exchange(
                expected,
                expected.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
}

fn block_native_time_labels(engine: usize) {
    if !ROAD_HISTORY_ENABLED.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
        map.invalidate_engine(engine);
    }
    let _ = mark_native_map_rebuild_pending(engine);
}

fn invalidate_rebuilt_engine(map: &mut NativeTimeMap, engine: usize, rebuild_state: Option<u64>) {
    if rebuild_state.is_none_or(|state| state & 1 != 0) {
        map.invalidate_engine(engine);
    }
}

#[derive(Debug, Clone, Copy)]
struct ZeroEndpointQuery {
    rebuild_state: Option<u64>,
    generation: u64,
    world_epoch: u64,
    game_state: usize,
    engine: usize,
    clock: ClockKey,
    time: i64,
    next_step: u64,
}

fn lookup_zero_endpoint(
    map: &NativeTimeMap,
    query: ZeroEndpointQuery,
) -> Option<NativeTimeEndpoint> {
    if query.rebuild_state.is_none_or(|state| state & 1 != 0) {
        return None;
    }
    map.lookup(
        query.generation,
        query.world_epoch,
        query.game_state,
        query.engine,
        query.clock,
        query.time,
    )
    .filter(|endpoint| endpoint.room_step.saturating_add(1) == query.next_step)
}

fn native_writer_snapshot(engine: usize) -> NativeWriterSnapshot {
    let Some(index) = native_writer_slot(engine) else {
        return NativeWriterSnapshot::default();
    };
    let slot = &NATIVE_WRITER_SLOTS[index];
    for _ in 0..3 {
        let before = slot.version.load(Ordering::Acquire);
        let active = slot.active.load(Ordering::Acquire);
        let after = slot.version.load(Ordering::Acquire);
        if before == after && before.is_multiple_of(2) {
            return NativeWriterSnapshot {
                known: true,
                stable: true,
                version: after,
                active,
            };
        }
    }
    NativeWriterSnapshot {
        known: true,
        stable: false,
        version: slot.version.load(Ordering::Acquire),
        active: slot.active.load(Ordering::Acquire),
    }
}

fn native_writer_enter(game_sim_this: usize) -> NativeWriterToken {
    let (game_state, engine) = crate::timeline::native_world_fields(game_sim_this);
    let _ = game_state;
    native_writer_enter_engine(engine)
}

fn native_writer_enter_engine(engine: usize) -> NativeWriterToken {
    let Some(index) = native_writer_slot(engine) else {
        return NativeWriterToken::default();
    };
    let slot = &NATIVE_WRITER_SLOTS[index];
    slot.version.fetch_add(1, Ordering::AcqRel);
    slot.active.fetch_add(1, Ordering::AcqRel);
    let entry_version = slot.version.fetch_add(1, Ordering::AcqRel) + 1;
    NativeWriterToken {
        slot: index,
        engine,
        entry_version,
        valid: true,
    }
}

fn native_writer_exit(token: NativeWriterToken) -> NativeWriterSnapshot {
    if !token.valid || token.slot >= NATIVE_WRITER_SLOTS.len() {
        return NativeWriterSnapshot::default();
    }
    let slot = &NATIVE_WRITER_SLOTS[token.slot];
    if slot.engine.load(Ordering::Acquire) != token.engine {
        return NativeWriterSnapshot::default();
    }
    slot.version.fetch_add(1, Ordering::AcqRel);
    let _ = slot.active.fetch_sub(1, Ordering::AcqRel);
    slot.version.fetch_add(1, Ordering::AcqRel);
    native_writer_snapshot(token.engine)
}

/// Replicate runs inside the same writer bracket as GameSim::Step: the enter
/// and exit helpers each advance the version twice. The token's entry version
/// is therefore pre-version + 2, and a completed copy is pre-version + 4.
fn completed_native_copy_bracket(
    before: NativeWriterSnapshot,
    token: NativeWriterToken,
    after: NativeWriterSnapshot,
) -> bool {
    before.known
        && before.stable
        && before.active == 0
        && token.valid
        && token.engine != 0
        && token.entry_version == before.version.saturating_add(2)
        && after.known
        && after.stable
        && after.active == 0
        && after.version == before.version.saturating_add(4)
}

fn read_game_state_engine(game_state: usize) -> Option<usize> {
    game_state
        .checked_add(0x18)
        .and_then(crate::image::guarded::read::<usize>)
        .filter(|engine| *engine != 0)
}

#[cfg(all(windows, target_arch = "x86_64"))]
type GameStateReplicateFn = unsafe extern "system" fn(usize, usize, u32) -> usize;

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn game_state_replicate_detour(
    source_state: usize,
    destination_state: usize,
    replica: u32,
) -> usize {
    let original_address = GAME_STATE_REPLICATE_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: the pinned 40420 GameState::Replicate trampoline preserves its
    // three native arguments and the incidental RAX return value.
    let original = unsafe { std::mem::transmute::<usize, GameStateReplicateFn>(original_address) };
    if !ROAD_HISTORY_ENABLED.load(Ordering::Acquire) {
        // SAFETY: transparent native forwarding when the candidate is off.
        return unsafe { original(source_state, destination_state, replica) };
    }
    let attempt_id = NEXT_REPLICA_COPY.fetch_add(1, Ordering::Relaxed).max(1);
    LATEST_REPLICA_ATTEMPT.store(attempt_id, Ordering::Release);
    clear_pending_replica_copy(None);
    let destination_engine = read_game_state_engine(destination_state);
    let destination_rebuild_state = destination_engine.and_then(mark_native_map_rebuild_pending);
    let destination_tracking = destination_rebuild_state.is_some();
    if let Some(engine) = destination_engine {
        if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
            map.invalidate_engine(engine);
        }
    } else {
        NATIVE_MAP_TRACKING_FAILED.store(true, Ordering::Release);
    }
    if !crate::timeline::active() || !native_time_probe_enabled() {
        // SAFETY: transparent native forwarding outside the armed capture.
        return unsafe { original(source_state, destination_state, replica) };
    }
    let generation = crate::timeline::current_generation();
    let source_engine = read_game_state_engine(source_state);
    let family = crate::road_history::selected_family();
    let candidate = match (source_engine, destination_engine, family) {
        (Some(source_engine), Some(destination_engine), Some((owner, child, revision)))
            if source_state != destination_state
                && source_engine != destination_engine
                && destination_tracking
                && crate::road_history::family_matches(owner, child, revision)
                && allowlist_has_only_owner(generation, owner) =>
        {
            let source_writer = native_writer_snapshot(source_engine);
            let destination_writer = native_writer_snapshot(destination_engine);
            if !source_writer.known
                || !source_writer.stable
                || source_writer.active != 0
                || !destination_writer.known
                || !destination_writer.stable
                || destination_writer.active != 0
            {
                None
            } else if let Ok(map) = NATIVE_TIME_MAP.try_lock() {
                let source_endpoint =
                    map.latest_for(generation, map.world_epoch, source_state, source_engine);
                if map.capture_generation != generation || map.world_mark.is_none() {
                    None
                } else if let Some(source_endpoint) = source_endpoint {
                    let world_epoch = map.world_epoch;
                    Some((
                        source_engine,
                        destination_engine,
                        owner,
                        child,
                        revision,
                        source_writer,
                        destination_writer,
                        world_epoch,
                        source_endpoint,
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    let Some((
        source_engine,
        destination_engine,
        owner,
        child,
        revision,
        source_writer,
        destination_writer,
        world_epoch,
        source_endpoint,
    )) = candidate
    else {
        // SAFETY: no lineage was captured, so run the native copy unchanged.
        return unsafe { original(source_state, destination_state, replica) };
    };

    let destination_writer_token = native_writer_enter_engine(destination_engine);
    // SAFETY: one original call with every native argument unchanged.
    let result = unsafe { original(source_state, destination_state, replica) };
    let destination_after = native_writer_exit(destination_writer_token);
    let source_after = native_writer_snapshot(source_engine);
    let source_state_engine_after = read_game_state_engine(source_state);
    let destination_state_engine_after = read_game_state_engine(destination_state);
    let map_still_matches = NATIVE_TIME_MAP.try_lock().is_ok_and(|map| {
        map.capture_generation == generation
            && map.world_mark.is_some()
            && map.world_epoch == world_epoch
            && map.latest_for(generation, world_epoch, source_state, source_engine)
                == Some(source_endpoint)
    });
    let stable = crate::timeline::active()
        && crate::timeline::current_generation() == generation
        && destination_rebuild_state
            .is_some_and(|state| native_map_rebuild_state(destination_engine) == Some(state))
        && crate::road_history::family_matches(owner, child, revision)
        && source_state_engine_after == Some(source_engine)
        && destination_state_engine_after == Some(destination_engine)
        && source_after.known
        && source_after.stable
        && source_after.active == 0
        && source_after.version == source_writer.version
        && completed_native_copy_bracket(
            destination_writer,
            destination_writer_token,
            destination_after,
        )
        && map_still_matches;
    if stable {
        let copy = crate::road_history::PendingCopy {
            id: attempt_id,
            source: crate::road_history::AcceptedAnchor {
                capture_generation: generation,
                world_epoch,
                game_state: source_state,
                engine: source_engine,
                clock_entity_ptr: source_endpoint.clock.entity_ptr,
                clock_entity_id: source_endpoint.clock.entity_id,
                clock_kind: crate::road_history::ClockKind::CGameTime,
                native_time: source_endpoint.time,
                tick_count: source_endpoint.tick_count,
                update_count: source_endpoint.update_count,
                room_step: source_endpoint.room_step,
            },
            destination_game_state: destination_state,
            destination_engine,
            family_revision: revision,
        };
        if let Ok(mut pending) = PENDING_REPLICA_COPY.try_lock()
            && LATEST_REPLICA_ATTEMPT.load(Ordering::Acquire) == attempt_id
            && record_replica_copy(copy, false, 1, 0).is_some()
        {
            *pending = Some(PendingReplicaCopy { evidence: copy });
        }
    }
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
type EngineRemoveEntityFn = unsafe extern "system" fn(usize, i32, u8) -> usize;

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn engine_remove_entity_detour(
    engine: usize,
    entity: i32,
    flag: u8,
) -> usize {
    let original_address = ENGINE_REMOVE_ENTITY_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: the exact pinned 40420 trampoline is called with its native
    // Engine, signed entity ID and byte flag; RAX is preserved unchanged.
    let original = unsafe { std::mem::transmute::<usize, EngineRemoveEntityFn>(original_address) };
    if ROAD_HISTORY_ENABLED.load(Ordering::Acquire) && crate::timeline::active() {
        crate::road_history::note_entity_removal(u32::try_from(entity).unwrap_or(0));
    }
    // SAFETY: original native arguments are passed through exactly once.
    unsafe { original(engine, entity, flag) }
}

fn read_clock_key(clock_object: usize) -> Option<ClockKey> {
    let engine = clock_object
        .checked_add(8)
        .and_then(crate::image::guarded::read::<usize>)?;
    let entity_ptr = clock_object
        .checked_add(0x10)
        .and_then(crate::image::guarded::read::<usize>)?;
    let entity_id = crate::image::guarded::read::<u32>(entity_ptr)?;
    (engine != 0 && entity_ptr != 0 && entity_id != 0).then_some(ClockKey {
        engine,
        entity_ptr,
        entity_id,
    })
}

fn read_native_clock(clock_object: usize) -> Option<NativeClockSample> {
    let key = read_clock_key(clock_object)?;
    let getter = NATIVE_TIME_CURRENT_GETTER.load(Ordering::Acquire);
    if getter == 0 {
        return None;
    }
    type CurrentTimeFn = unsafe extern "system" fn(usize) -> i64;
    // SAFETY: install verifies this exact 40420 CGameTime getter at the
    // current-getter call site; `clock_object` is guarded and was supplied by
    // GameSim::Step's own speed call.
    let time = unsafe { std::mem::transmute::<usize, CurrentTimeFn>(getter)(clock_object) };
    let counters = crate::ticks::read_counters(clock_object).ok()?;
    (read_clock_key(clock_object) == Some(key)).then_some(NativeClockSample {
        key,
        time,
        tick_count: counters.tick_count,
        update_count: counters.update_count,
    })
}

fn clear_pending_replica_copy(id: Option<u64>) {
    let Ok(mut pending) = PENDING_REPLICA_COPY.try_lock() else {
        return;
    };
    if id.is_none_or(|id| pending.is_some_and(|copy| copy.evidence.id == id)) {
        *pending = None;
    }
}

fn bind_pending_replica_for_step(context: &mut NativeStepContext, before: NativeClockSample) {
    if !ROAD_HISTORY_ENABLED.load(Ordering::Acquire)
        || !context.capture
        || !context.room
        || context.capture_generation == 0
    {
        return;
    }
    let copy = match PENDING_REPLICA_COPY.try_lock() {
        Ok(pending) => *pending,
        Err(_) => return,
    };
    let Some(copy) = copy else {
        return;
    };
    if copy.evidence.id != LATEST_REPLICA_ATTEMPT.load(Ordering::Acquire) {
        clear_pending_replica_copy(Some(copy.evidence.id));
        return;
    }
    if copy.evidence.destination_game_state != context.game_state {
        return;
    }
    if copy.evidence.destination_engine != context.engine {
        clear_pending_replica_copy(Some(copy.evidence.id));
        return;
    }
    let Some((owner, child, revision)) = crate::road_history::selected_family() else {
        clear_pending_replica_copy(Some(copy.evidence.id));
        return;
    };
    if !crate::road_history::family_matches(owner, child, revision)
        || revision != copy.evidence.family_revision
    {
        clear_pending_replica_copy(Some(copy.evidence.id));
        return;
    }
    let world = match NATIVE_TIME_MAP.try_lock() {
        Ok(map)
            if map.capture_generation == context.capture_generation && map.world_mark.is_some() =>
        {
            Some(map.world_epoch)
        }
        _ => None,
    };
    let Some(world_epoch) = world else {
        return;
    };
    let readback = crate::road_history::LocalReadback {
        capture_generation: context.capture_generation,
        world_epoch,
        game_state: context.game_state,
        engine: context.engine,
        clock_entity_ptr: before.key.entity_ptr,
        clock_entity_id: before.key.entity_id,
        clock_kind: crate::road_history::ClockKind::CGameTime,
        native_time: before.time,
        tick_count: before.tick_count,
        update_count: before.update_count,
        family_revision: revision,
    };
    match crate::road_history::bind_copy(copy.evidence, readback) {
        Ok(bound) => {
            context.bound_copy = Some(bound);
            context.pending_copy_id = copy.evidence.id;
        }
        Err(_) => clear_pending_replica_copy(Some(copy.evidence.id)),
    }
}

/// Tracks exactly one original GameSim::Step call while the diagnostic is
/// opted in. Writer counters remain active between captures so a capture
/// beginning mid-call still sees the writer span.
pub(crate) struct NativeStepScope {
    previous: NativeStepContext,
    writer: NativeWriterToken,
    finished: bool,
}

impl NativeStepScope {
    pub(crate) fn enter(
        game_sim_this: usize,
        updates: crate::step::Updates,
        room: bool,
        frame_time_micros: usize,
    ) -> Option<Self> {
        if !native_time_probe_enabled() {
            return None;
        }
        let (game_state, engine) = crate::timeline::native_world_fields(game_sim_this);
        let writer = native_writer_enter(game_sim_this);
        let (selected_updates, selected_known) = match updates {
            crate::step::Updates::Exactly(updates) => (updates, true),
            crate::step::Updates::Own => (0, false),
        };
        let previous = NATIVE_STEP_CONTEXT.with(|cell| {
            let parent = cell.get();
            let context = NativeStepContext {
                active: true,
                capture: crate::timeline::active()
                    && room
                    && selected_known
                    && crate::timeline::current_generation() != 0,
                nested: parent.active,
                capture_generation: crate::timeline::current_generation(),
                native_this: game_sim_this,
                game_state,
                engine,
                room,
                updates: selected_updates,
                frame_time_micros,
                clock_object: 0,
                before: None,
                bound_copy: None,
                pending_copy_id: 0,
                writer,
            };
            cell.replace(context)
        });
        NATIVE_STEP_CANDIDATE.with(|cell| cell.set(None));
        Some(Self {
            previous,
            writer,
            finished: false,
        })
    }

    pub(crate) fn finish(mut self) {
        self.finish_inner();
        self.finished = true;
    }

    fn finish_inner(&mut self) {
        if self.finished {
            return;
        }
        let context = NATIVE_STEP_CONTEXT.with(std::cell::Cell::get);
        let mut candidate = NativeTimeCandidate::default();
        if context.capture
            && crate::timeline::active()
            && crate::timeline::current_generation() == context.capture_generation
        {
            candidate.present = true;
            candidate.nested = context.nested;
            candidate.capture_generation = context.capture_generation;
            candidate.native_this = context.native_this;
            candidate.game_state = context.game_state;
            candidate.engine = context.engine;
            candidate.updates = context.updates;
            candidate.frame_time_micros = context.frame_time_micros;
            candidate.room = context.room;
            candidate.bound_copy = context.bound_copy;
            candidate.pending_copy_id = context.pending_copy_id;
            candidate.sample_nanos = crate::timeline::monotonic_nanos();
            candidate.writer_version_start = context.writer.entry_version;
            if context.nested {
                candidate.rejection = TIME_REJECT_NESTED;
            } else if let Some(before) = context.before {
                candidate.before = before;
                if let Some(after) = read_native_clock(context.clock_object) {
                    candidate.after = after;
                    let (game_state, engine) =
                        crate::timeline::native_world_fields(context.native_this);
                    let writer_return = native_writer_snapshot(context.engine);
                    candidate.writer_version_return = writer_return.version;
                    candidate.writer_active_return = writer_return.active;
                    let writer_return_stable = self.writer.valid
                        && writer_return.known
                        && writer_return.stable
                        && writer_return.version == context.writer.entry_version
                        && writer_return.active == 1;
                    let world_stable = game_state == context.game_state
                        && engine == context.engine
                        && before.key == after.key
                        && after.key.engine == context.engine;
                    candidate.sample_valid = writer_return_stable && world_stable;
                    candidate.writer_stable = writer_return_stable;
                    if !writer_return_stable {
                        candidate.rejection = TIME_REJECT_WRITER;
                    } else if !world_stable {
                        candidate.rejection = TIME_REJECT_WORLD;
                    }
                } else {
                    candidate.rejection = TIME_REJECT_SAMPLE;
                }
            } else {
                candidate.rejection = TIME_REJECT_SAMPLE;
            }
        }
        let writer_end = native_writer_exit(self.writer);
        if candidate.present {
            candidate.writer_version_end = writer_end.version;
            candidate.writer_active_end = writer_end.active;
            candidate.writer_stable &= writer_end.known
                && writer_end.stable
                && writer_end.version == context.writer.entry_version.saturating_add(2)
                && writer_end.active == 0;
            if !candidate.writer_stable && candidate.rejection == TIME_REJECT_NONE {
                candidate.sample_valid = false;
                candidate.rejection = TIME_REJECT_WRITER;
            }
            NATIVE_STEP_CANDIDATE.with(|cell| cell.set(Some(candidate)));
        }
        NATIVE_STEP_CONTEXT.with(|cell| cell.set(self.previous));
    }
}

impl Drop for NativeStepScope {
    fn drop(&mut self) {
        if !self.finished {
            self.finish_inner();
            self.finished = true;
        }
    }
}

pub(crate) fn native_step_before(clock_object: usize) {
    if !native_time_probe_enabled() || !crate::timeline::active() {
        return;
    }
    NATIVE_STEP_CONTEXT.with(|cell| {
        let mut context = cell.get();
        if !context.capture || context.capture_generation != crate::timeline::current_generation() {
            return;
        }
        context.clock_object = clock_object;
        context.before = read_native_clock(clock_object);
        if let Some(before) = context.before {
            bind_pending_replica_for_step(&mut context, before);
        }
        cell.set(context);
    });
}

fn take_native_step_candidate() -> Option<NativeTimeCandidate> {
    NATIVE_STEP_CANDIDATE.with(|cell| cell.replace(None))
}

pub(crate) fn native_time_observe_world(mark: Option<crate::step::WorldMark>) {
    if !native_time_probe_enabled() || !crate::timeline::active() {
        return;
    }
    let generation = crate::timeline::current_generation();
    if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
        map.ensure_capture(generation);
        let changed = map.set_world(mark);
        drop(map);
        if changed {
            clear_pending_replica_copy(None);
            if ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire) {
                crate::road_history::invalidate();
            }
        }
    }
}

fn copied_anchor_endpoint(
    copy: crate::road_history::BoundCopy,
    candidate: NativeTimeCandidate,
    first_step: u64,
    world_epoch: u64,
    reports_succeeded: bool,
) -> Result<NativeTimeEndpoint, u8> {
    if copy.world_epoch != world_epoch {
        return Err(TIME_REJECT_WORLD);
    }
    let current_revision = crate::road_history::current_family_revision();
    let accepted = crate::road_history::commit_copy(
        copy,
        crate::road_history::CopyCommitProof {
            first_step,
            reports_ok: reports_succeeded,
            capture_generation: candidate.capture_generation,
            world_epoch,
            game_state: candidate.game_state,
            engine: candidate.engine,
            family_revision: current_revision,
        },
    )
    .map_err(|_| TIME_REJECT_REPLICA)?;
    if !candidate.before.key.engine.eq(&accepted.engine)
        || candidate.before.key.entity_ptr != accepted.clock_entity_ptr
        || candidate.before.key.entity_id != accepted.clock_entity_id
        || candidate.before.time != accepted.native_time
        || candidate.before.tick_count != accepted.tick_count
        || candidate.before.update_count != accepted.update_count
    {
        return Err(TIME_REJECT_REPLICA);
    }
    Ok(NativeTimeEndpoint {
        used: true,
        capture_generation: accepted.capture_generation,
        world_epoch: accepted.world_epoch,
        game_state: accepted.game_state,
        engine: accepted.engine,
        clock: candidate.before.key,
        time: accepted.native_time,
        tick_count: accepted.tick_count,
        room_step: accepted.room_step,
        update_count: accepted.update_count,
    })
}

fn record_replica_copy(
    copy: crate::road_history::PendingCopy,
    accepted: bool,
    stage: u8,
    rejection: u8,
) -> Option<u64> {
    crate::timeline::record_with(crate::timeline::Kind::NativeReplicaCopy, copy.id, || {
        crate::timeline::Fields {
            this: copy.source.game_state,
            data: copy.destination_game_state,
            native_replica_source_state: copy.source.game_state,
            native_replica_source_engine: copy.source.engine,
            native_replica_destination_state: copy.destination_game_state,
            native_replica_destination_engine: copy.destination_engine,
            native_replica_room_step: copy.source.room_step,
            native_replica_family_revision: copy.family_revision,
            native_replica_accepted: accepted,
            result_byte: stage,
            road_history_incarnation: copy.family_revision,
            road_history_rejection: rejection,
            ..crate::timeline::Fields::default()
        }
    })
}

fn copy_as_pending(copy: crate::road_history::BoundCopy) -> crate::road_history::PendingCopy {
    crate::road_history::PendingCopy {
        id: copy.id,
        source: copy.source,
        destination_game_state: copy.destination_game_state,
        destination_engine: copy.destination_engine,
        family_revision: copy.family_revision,
    }
}

/// Publishes a native-time endpoint only after the step driver confirms that
/// every room report succeeded and the loaded world is unchanged. The first
/// observed `before` timestamp is deliberately not assigned a room step: only
/// times reached by completed, exactly-counted updates enter the map.
pub(crate) fn commit_native_step(
    first_step: Option<u64>,
    world_mark: Option<crate::step::WorldMark>,
    reports_succeeded: bool,
) {
    let Some(candidate) = take_native_step_candidate() else {
        return;
    };
    if !candidate.present || !crate::timeline::active() {
        return;
    }
    let mut rejection = candidate.rejection;
    let mut previous_step = 0;
    let mut current_step = 0;
    let mut world_epoch = 0;
    let mut label_valid = false;
    let mut clear_pending = None;
    let step_start = first_step.unwrap_or(0);
    let map_rebuild_state = if ROAD_HISTORY_ENABLED.load(Ordering::Acquire) {
        native_map_rebuild_state(candidate.engine)
    } else {
        Some(0)
    };
    let map_rebuild_pending = map_rebuild_state.is_none_or(|state| state & 1 != 0);

    if !reports_succeeded || first_step.is_none() {
        if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
            map.invalidate();
        }
        clear_pending = (candidate.pending_copy_id != 0).then_some(candidate.pending_copy_id);
        rejection = TIME_REJECT_REPORT;
    } else if world_mark.is_none() {
        rejection = TIME_REJECT_WORLD;
    } else if candidate.capture_generation != crate::timeline::current_generation() {
        rejection = TIME_REJECT_CAPTURE;
    } else if candidate.nested || !candidate.room {
        rejection = TIME_REJECT_NESTED;
    } else if !candidate.sample_valid || !candidate.writer_stable {
        if rejection == TIME_REJECT_NONE {
            rejection = TIME_REJECT_WRITER;
        }
    } else if candidate.updates == 0 {
        if native_batch_points(candidate, step_start, 0).is_err() {
            rejection = TIME_REJECT_SAMPLE;
        } else if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
            map.ensure_capture(candidate.capture_generation);
            map.set_world(world_mark);
            invalidate_rebuilt_engine(&mut map, candidate.engine, map_rebuild_state);
            world_epoch = map.world_epoch;
            if let Some(copy) = candidate.bound_copy {
                match copied_anchor_endpoint(copy, candidate, step_start, world_epoch, true) {
                    Ok(endpoint) if map.insert(endpoint) => {
                        previous_step = endpoint.room_step;
                        current_step = endpoint.room_step;
                        if record_replica_copy(copy_as_pending(copy), true, 2, 0).is_some() {
                            label_valid = true;
                            rejection = TIME_REJECT_NONE;
                            clear_pending = Some(copy.id);
                        } else {
                            map.invalidate_engine(candidate.engine);
                            previous_step = 0;
                            current_step = 0;
                            rejection = TIME_REJECT_REPLICA_UNAVAILABLE;
                            clear_pending = Some(copy.id);
                        }
                    }
                    _ => {
                        map.invalidate_engine(candidate.engine);
                        rejection = TIME_REJECT_REPLICA;
                        clear_pending = Some(copy.id);
                    }
                }
            } else {
                if let Some(endpoint) = lookup_zero_endpoint(
                    &map,
                    ZeroEndpointQuery {
                        rebuild_state: map_rebuild_state,
                        generation: candidate.capture_generation,
                        world_epoch,
                        game_state: candidate.game_state,
                        engine: candidate.engine,
                        clock: candidate.before.key,
                        time: candidate.before.time,
                        next_step: step_start,
                    },
                ) {
                    previous_step = endpoint.room_step;
                    current_step = endpoint.room_step;
                    label_valid = true;
                    rejection = TIME_REJECT_NONE;
                } else {
                    rejection = if map_rebuild_pending {
                        TIME_REJECT_REPLICA_UNAVAILABLE
                    } else {
                        TIME_REJECT_ZERO_UNKNOWN
                    };
                }
            }
        } else {
            rejection = TIME_REJECT_MAP_BUSY;
        }
    } else if let Ok(mut map) = NATIVE_TIME_MAP.try_lock() {
        map.ensure_capture(candidate.capture_generation);
        map.set_world(world_mark);
        invalidate_rebuilt_engine(&mut map, candidate.engine, map_rebuild_state);
        world_epoch = map.world_epoch;
        match native_batch_points(candidate, step_start, world_epoch) {
            Ok(points) => {
                let copied = candidate.bound_copy.and_then(|copy| {
                    copied_anchor_endpoint(copy, candidate, step_start, world_epoch, true)
                        .ok()
                        .map(|endpoint| (copy, endpoint))
                });
                let mut inserted_all = candidate.bound_copy.is_none() || copied.is_some();
                if let Some((_, endpoint)) = copied {
                    inserted_all &= map.insert(endpoint);
                }
                for endpoint in points.entries.iter().take(points.len).copied() {
                    inserted_all &= map.insert(endpoint);
                }
                if inserted_all {
                    previous_step = step_start.saturating_sub(1);
                    current_step = step_start.saturating_add(u64::from(candidate.updates) - 1);
                    if let Some((copy, _)) = copied {
                        if record_replica_copy(copy_as_pending(copy), true, 2, 0).is_some() {
                            label_valid = true;
                            rejection = TIME_REJECT_NONE;
                            clear_pending = Some(copy.id);
                        } else {
                            map.invalidate_engine(candidate.engine);
                            previous_step = 0;
                            current_step = 0;
                            rejection = TIME_REJECT_REPLICA_UNAVAILABLE;
                            clear_pending = Some(copy.id);
                        }
                    } else {
                        label_valid = true;
                        rejection = TIME_REJECT_NONE;
                    }
                } else {
                    if let Some((copy, _)) = copied {
                        map.invalidate_engine(candidate.engine);
                        clear_pending = Some(copy.id);
                        rejection = TIME_REJECT_REPLICA;
                    } else if let Some(copy) = candidate.bound_copy {
                        map.invalidate_engine(candidate.engine);
                        clear_pending = Some(copy.id);
                        rejection = TIME_REJECT_REPLICA;
                    } else {
                        map.invalidate();
                        rejection = TIME_REJECT_MAP_CONFLICT;
                    }
                }
            }
            Err(code) => rejection = code,
        }
    } else {
        rejection = TIME_REJECT_MAP_BUSY;
    }

    if candidate.bound_copy.is_some() && !label_valid {
        clear_pending = candidate.bound_copy.map(|copy| copy.id);
    }
    if let Some(copy_id) = clear_pending {
        clear_pending_replica_copy(Some(copy_id));
    }

    if ROAD_HISTORY_ENABLED.load(Ordering::Acquire)
        && (native_map_rebuild_state(candidate.engine) != map_rebuild_state
            || map_rebuild_state.is_none())
    {
        label_valid = false;
        rejection = TIME_REJECT_REPLICA_UNAVAILABLE;
        block_native_time_labels(candidate.engine);
    } else if !label_valid && rejection != TIME_REJECT_ZERO_UNKNOWN {
        block_native_time_labels(candidate.engine);
    }

    let row_recorded =
        crate::timeline::record_with(crate::timeline::Kind::NativeTimeStep, 0, || {
            crate::timeline::Fields {
                this: candidate.native_this,
                arg_a: candidate.frame_time_micros,
                native_time_before: candidate.before.time,
                native_time_after: candidate.after.time,
                native_time_delta: candidate.after.time.saturating_sub(candidate.before.time),
                native_time_update_before: candidate.before.update_count,
                native_time_update_after: candidate.after.update_count,
                native_time_tick_before: candidate.before.tick_count,
                native_time_tick_after: candidate.after.tick_count,
                native_time_selected_updates: candidate.updates,
                native_time_first_step: step_start,
                native_time_last_step: if candidate.updates == 0 {
                    step_start.saturating_sub(1)
                } else {
                    step_start.saturating_add(u64::from(candidate.updates) - 1)
                },
                native_time_previous_step: previous_step,
                native_time_current_step: current_step,
                native_time_world_epoch: world_epoch,
                native_time_sample_nanos: candidate.sample_nanos,
                native_time_current_engine: candidate.after.key.engine,
                native_time_current_entity_ptr: candidate.after.key.entity_ptr,
                native_time_current_entity_id: candidate.after.key.entity_id,
                native_time_writer_version_start: candidate.writer_version_start,
                native_time_writer_version_end: candidate.writer_version_end,
                native_time_writer_active_start: candidate.writer_active_return,
                native_time_writer_active_end: candidate.writer_active_end,
                native_replica_source_state: candidate
                    .bound_copy
                    .map(|copy| copy.source.game_state)
                    .unwrap_or(0),
                native_replica_source_engine: candidate
                    .bound_copy
                    .map(|copy| copy.source.engine)
                    .unwrap_or(0),
                native_replica_destination_state: candidate
                    .bound_copy
                    .map(|copy| copy.destination_game_state)
                    .unwrap_or(0),
                native_replica_destination_engine: candidate
                    .bound_copy
                    .map(|copy| copy.destination_engine)
                    .unwrap_or(0),
                native_replica_room_step: candidate
                    .bound_copy
                    .map(|copy| copy.source.room_step)
                    .unwrap_or(0),
                native_replica_family_revision: candidate
                    .bound_copy
                    .map(|copy| copy.family_revision)
                    .unwrap_or(0),
                native_replica_accepted: candidate.bound_copy.is_some() && label_valid,
                road_history_rejection: if candidate.bound_copy.is_some() && !label_valid {
                    TIME_REJECT_REPLICA
                } else {
                    0
                },
                native_time_label_valid: label_valid,
                native_time_rejection: rejection,
                selected_updates: u64::from(candidate.updates),
                selected_known: true,
                room: candidate.room,
                native_game_state: candidate.game_state,
                native_engine: candidate.engine,
                ..crate::timeline::Fields::default()
            }
        })
        .is_some();
    if !row_recorded {
        block_native_time_labels(candidate.engine);
    } else if label_valid
        && map_rebuild_pending
        && let Some(state) = map_rebuild_state
    {
        let _ = clear_native_map_rebuild_pending(candidate.engine, state);
    }
}

fn native_time_scope_update(update: impl FnOnce(&mut NativeTimeReadScope)) {
    NATIVE_TIME_SCOPE.with(|cell| {
        let mut scope = cell.get();
        update(&mut scope);
        cell.set(scope);
    });
}

fn native_time_label(
    mut observation: NativeTimeObservation,
    meta: RenderMeta,
    helper_engine: usize,
) -> NativeTimeObservation {
    if !observation.present {
        return observation;
    }
    if observation.rejection != TIME_REJECT_NONE {
        return observation;
    }
    let reject = if observation.nested {
        TIME_REJECT_NESTED
    } else if observation.current_calls != 1 || observation.previous_calls != 1 {
        TIME_REJECT_GETTER_COUNT
    } else if observation.current_key.is_none()
        || observation.current_key != observation.previous_key
        || observation
            .current_key
            .is_some_and(|key| key.engine != meta.engine)
        || helper_engine != meta.engine
    {
        TIME_REJECT_ENGINE
    } else if observation.previous_valid == 0 {
        TIME_REJECT_PREVIOUS_INVALID
    } else if !f32::from_bits(observation.alpha_bits).is_finite()
        || !(0.0..=1.0).contains(&f32::from_bits(observation.alpha_bits))
    {
        TIME_REJECT_ALPHA
    } else if !observation.writer_start.known
        || !observation.writer_start.stable
        || !observation.writer_end.known
        || !observation.writer_end.stable
        || observation.writer_start.active != 0
        || observation.writer_end.active != 0
        || observation.writer_start.version != observation.writer_end.version
    {
        TIME_REJECT_WRITER
    } else {
        TIME_REJECT_UNMAPPED
    };
    observation.rejection = reject;
    if reject != TIME_REJECT_UNMAPPED {
        return observation;
    }
    if !crate::timeline::active()
        || observation.generation == 0
        || observation.generation != crate::timeline::current_generation()
    {
        observation.rejection = TIME_REJECT_CAPTURE;
        return observation;
    }
    if ROAD_HISTORY_ENABLED.load(Ordering::Acquire) && native_map_rebuild_pending(meta.engine) {
        observation.rejection = TIME_REJECT_REPLICA_UNAVAILABLE;
        return observation;
    }
    let Some(current_time) = observation.current else {
        observation.rejection = TIME_REJECT_SAMPLE;
        return observation;
    };
    let Some(previous_time) = observation.previous else {
        observation.rejection = TIME_REJECT_SAMPLE;
        return observation;
    };
    let Some(clock) = observation.current_key else {
        observation.rejection = TIME_REJECT_CLOCK_KEY;
        return observation;
    };
    let Ok(map) = NATIVE_TIME_MAP.try_lock() else {
        observation.rejection = TIME_REJECT_MAP_BUSY;
        return observation;
    };
    let epoch = map.world_epoch;
    let previous = map.lookup(
        observation.generation,
        epoch,
        meta.game_state,
        meta.engine,
        clock,
        previous_time,
    );
    let current = map.lookup(
        observation.generation,
        epoch,
        meta.game_state,
        meta.engine,
        clock,
        current_time,
    );
    let (Some(previous), Some(current)) = (previous, current) else {
        return observation;
    };
    let delta_time = current_time.saturating_sub(previous_time);
    if delta_time < 0 || current.room_step < previous.room_step {
        observation.rejection = TIME_REJECT_GAP;
        return observation;
    }
    let alpha = f64::from(f32::from_bits(observation.alpha_bits));
    let expected = (previous_time as f64 + (delta_time as f64 * alpha)).round();
    if !expected.is_finite() || (expected - observation.interp_return as f64).abs() > 1.0 {
        observation.rejection = TIME_REJECT_INTERPOLATION;
        return observation;
    }
    observation.previous_step = previous.room_step;
    observation.current_step = current.room_step;
    observation.previous_update_count = previous.update_count;
    observation.current_update_count = current.update_count;
    observation.world_epoch = epoch;
    observation.label_valid = true;
    observation.rejection = TIME_REJECT_NONE;
    observation
}

#[cfg(all(windows, target_arch = "x86_64"))]
type NativeTimeInterpolationFn = unsafe extern "system" fn(usize, f32) -> i64;
#[cfg(all(windows, target_arch = "x86_64"))]
type NativeTimeCurrentFn = unsafe extern "system" fn(usize) -> i64;
#[cfg(all(windows, target_arch = "x86_64"))]
type NativeTimePreviousFn = unsafe extern "system" fn(usize, usize) -> usize;

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn native_time_current_observer(clock: usize) -> i64 {
    let original_address = NATIVE_TIME_CURRENT_GETTER.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: profile checks the one current-time getter call site.
    let original = unsafe { std::mem::transmute::<usize, NativeTimeCurrentFn>(original_address) };
    let scoped = NATIVE_TIME_SCOPE.with(std::cell::Cell::get);
    if !scoped.active
        || scoped.nested
        || !crate::timeline::active()
        || scoped.capture_generation != crate::timeline::current_generation()
    {
        // SAFETY: transparent call when there is no active captured scope.
        return unsafe { original(clock) };
    }
    let before = read_clock_key(clock);
    // SAFETY: exactly one original native getter call.
    let result = unsafe { original(clock) };
    let after = read_clock_key(clock);
    native_time_scope_update(|scope| {
        scope.current_calls = scope.current_calls.saturating_add(1);
        if clock != scope.clock_object
            || scope.current_calls != 1
            || before.is_none()
            || before != after
        {
            scope.rejection = TIME_REJECT_CLOCK_KEY;
        } else {
            scope.current = Some(result);
            scope.current_key = before;
        }
    });
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn native_time_previous_observer(clock: usize, output: usize) -> usize {
    let original_address = NATIVE_TIME_PREVIOUS_GETTER.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: profile checks the one previous-time getter call site.
    let original = unsafe { std::mem::transmute::<usize, NativeTimePreviousFn>(original_address) };
    let scoped = NATIVE_TIME_SCOPE.with(std::cell::Cell::get);
    if !scoped.active
        || scoped.nested
        || !crate::timeline::active()
        || scoped.capture_generation != crate::timeline::current_generation()
    {
        // SAFETY: transparent call when there is no active captured scope.
        return unsafe { original(clock, output) };
    }
    let before = read_clock_key(clock);
    // SAFETY: exactly one original call writes the native caller-owned result.
    let result = unsafe { original(clock, output) };
    let after = read_clock_key(clock);
    let bytes = crate::image::guarded::read::<[u8; 16]>(output);
    native_time_scope_update(|scope| {
        scope.previous_calls = scope.previous_calls.saturating_add(1);
        if clock != scope.clock_object
            || scope.previous_calls != 1
            || before.is_none()
            || before != after
        {
            scope.rejection = TIME_REJECT_CLOCK_KEY;
        } else if let Some(bytes) = bytes {
            scope.previous = Some(i64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8])));
            scope.previous_valid = bytes[8];
            scope.previous_key = before;
        } else {
            scope.rejection = TIME_REJECT_SAMPLE;
        }
    });
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn native_time_interpolation_detour(clock: usize, alpha: f32) -> i64 {
    let original_address = NATIVE_TIME_FUNCTION_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: profile checks the 40420 CGameTime interpolation call.
    let original =
        unsafe { std::mem::transmute::<usize, NativeTimeInterpolationFn>(original_address) };
    if !NATIVE_TIME_ENABLED.load(Ordering::Acquire) || !crate::timeline::active() {
        // SAFETY: transparent call when observation is disabled.
        return unsafe { original(clock, alpha) };
    }
    let body = BODY.with(std::cell::Cell::get);
    let generation = crate::timeline::current_generation();
    let active = road_body_observable(body, generation);
    let nested_parent = NATIVE_TIME_SCOPE.with(|cell| {
        let mut parent = cell.get();
        let nested = parent.active;
        if nested {
            parent.nested = true;
            cell.set(parent);
        }
        parent
    });
    let writer_start = native_writer_snapshot(body.meta.engine);
    let current_scope = NativeTimeReadScope {
        active: active && !nested_parent.active,
        nested: nested_parent.active,
        capture_generation: generation,
        clock_object: clock,
        current_calls: 0,
        previous_calls: 0,
        current: None,
        previous: None,
        current_key: None,
        previous_key: None,
        previous_valid: 0,
        rejection: if active {
            TIME_REJECT_NONE
        } else {
            TIME_REJECT_NO_BODY
        },
    };
    let previous = NATIVE_TIME_SCOPE.with(|cell| cell.replace(current_scope));
    let mut observation = NativeTimeObservation {
        present: true,
        nested: nested_parent.active,
        generation,
        alpha_bits: alpha.to_bits(),
        writer_start,
        ..NativeTimeObservation::default()
    };
    // SAFETY: original call receives the same this and XMM1 alpha exactly once.
    let result = unsafe { original(clock, alpha) };
    let captured = NATIVE_TIME_SCOPE.with(|cell| cell.get());
    let writer_end = native_writer_snapshot(body.meta.engine);
    observation.current_calls = captured.current_calls;
    observation.previous_calls = captured.previous_calls;
    observation.current = captured.current;
    observation.previous = captured.previous;
    observation.current_key = captured.current_key;
    observation.previous_key = captured.previous_key;
    observation.previous_valid = captured.previous_valid;
    observation.interp_return = result;
    observation.sample_nanos = crate::timeline::monotonic_nanos();
    observation.writer_end = writer_end;
    observation.nested |= captured.nested;
    observation.rejection = captured.rejection;
    if writer_start.active != 0
        || writer_end.active != 0
        || !writer_start.known
        || !writer_end.known
        || !writer_start.stable
        || !writer_end.stable
        || writer_start.version != writer_end.version
    {
        observation.rejection = TIME_REJECT_WRITER;
    }
    if crate::timeline::current_generation() != generation
        || !road_body_observable(body, generation)
    {
        observation.rejection = TIME_REJECT_CAPTURE;
    }
    RoadScope::update(|scope| {
        if scope.active && scope.meta == body.meta {
            scope.native_time = observation;
            scope.native_writer_start = writer_start;
            scope.native_writer_end = writer_end;
        }
    });
    NATIVE_TIME_SCOPE.with(|cell| cell.set(previous));
    result
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DispatchSnapshot {
    pub dispatch_id: u64,
    pub key: usize,
    pub meta: RenderMeta,
    pub task_count: i32,
    pub worker_count: i32,
    pub started: u32,
    pub completed: u32,
    pub max_active: u32,
    pub thread_ids: [u32; THREAD_SAMPLE_LIMIT],
    pub invalid: bool,
}

struct DispatchSlot {
    state: AtomicU8,
    key: AtomicUsize,
    dispatch_id: AtomicU64,
    capture_generation: AtomicU64,
    epoch: AtomicU64,
    renderer: AtomicUsize,
    game_state: AtomicUsize,
    engine: AtomicUsize,
    type_index: AtomicI32,
    task_count: AtomicI32,
    worker_count: AtomicI32,
    started: AtomicU32,
    completed: AtomicU32,
    active: AtomicU32,
    max_active: AtomicU32,
    thread_ids: [AtomicU32; THREAD_SAMPLE_LIMIT],
    binding_valid: AtomicBool,
    invalid: AtomicBool,
}

impl DispatchSlot {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(EMPTY),
            key: AtomicUsize::new(0),
            dispatch_id: AtomicU64::new(0),
            capture_generation: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            renderer: AtomicUsize::new(0),
            game_state: AtomicUsize::new(0),
            engine: AtomicUsize::new(0),
            type_index: AtomicI32::new(-1),
            task_count: AtomicI32::new(0),
            worker_count: AtomicI32::new(0),
            started: AtomicU32::new(0),
            completed: AtomicU32::new(0),
            active: AtomicU32::new(0),
            max_active: AtomicU32::new(0),
            thread_ids: std::array::from_fn(|_| AtomicU32::new(0)),
            binding_valid: AtomicBool::new(false),
            invalid: AtomicBool::new(false),
        }
    }

    fn reset(&self) {
        self.key.store(0, Ordering::Relaxed);
        self.dispatch_id.store(0, Ordering::Relaxed);
        self.capture_generation.store(0, Ordering::Relaxed);
        self.epoch.store(0, Ordering::Relaxed);
        self.renderer.store(0, Ordering::Relaxed);
        self.game_state.store(0, Ordering::Relaxed);
        self.engine.store(0, Ordering::Relaxed);
        self.type_index.store(-1, Ordering::Relaxed);
        self.task_count.store(0, Ordering::Relaxed);
        self.worker_count.store(0, Ordering::Relaxed);
        self.started.store(0, Ordering::Relaxed);
        self.completed.store(0, Ordering::Relaxed);
        self.active.store(0, Ordering::Relaxed);
        self.max_active.store(0, Ordering::Relaxed);
        for thread in &self.thread_ids {
            thread.store(0, Ordering::Relaxed);
        }
        self.binding_valid.store(false, Ordering::Relaxed);
        self.invalid.store(false, Ordering::Relaxed);
    }

    fn metadata(&self) -> RenderMeta {
        RenderMeta {
            capture_generation: self.capture_generation.load(Ordering::Acquire),
            epoch: self.epoch.load(Ordering::Acquire),
            renderer: self.renderer.load(Ordering::Acquire),
            game_state: self.game_state.load(Ordering::Acquire),
            engine: self.engine.load(Ordering::Acquire),
            type_index: self.type_index.load(Ordering::Acquire),
            binding_valid: self.binding_valid.load(Ordering::Acquire),
        }
    }

    fn snapshot(&self) -> DispatchSnapshot {
        DispatchSnapshot {
            dispatch_id: self.dispatch_id.load(Ordering::Acquire),
            key: self.key.load(Ordering::Acquire),
            meta: self.metadata(),
            task_count: self.task_count.load(Ordering::Acquire),
            worker_count: self.worker_count.load(Ordering::Acquire),
            started: self.started.load(Ordering::Acquire),
            completed: self.completed.load(Ordering::Acquire),
            max_active: self.max_active.load(Ordering::Acquire),
            thread_ids: std::array::from_fn(|index| self.thread_ids[index].load(Ordering::Acquire)),
            invalid: self.invalid.load(Ordering::Acquire),
        }
    }

    fn start_body(&self, thread_id: u32) {
        self.started.fetch_add(1, Ordering::AcqRel);
        let active = self.active.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        atomic_max(&self.max_active, active);
        record_thread(&self.thread_ids, thread_id);
    }

    fn finish_body(&self) {
        self.completed.fetch_add(1, Ordering::AcqRel);
        let result = self
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active > 0).then_some(active - 1)
            });
        if result.is_err() {
            self.invalid.store(true, Ordering::Release);
        }
    }
}

fn atomic_max(counter: &AtomicU32, value: u32) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |before| {
        (value > before).then_some(value)
    });
}

fn reserve_inspection(counter: &AtomicU32, limit: u32) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |inspected| {
            (inspected < limit).then_some(inspected + 1)
        })
        .is_ok()
}

pub(crate) fn load_entity_allowlist(path: &Path) -> EntityAllowlist {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return EntityAllowlist::invalid(AllowlistStatus::Missing);
        }
        Err(_) => return EntityAllowlist::invalid(AllowlistStatus::Malformed),
    };
    if metadata.len() > MAX_ENTITY_FILE_BYTES as u64 {
        return EntityAllowlist::invalid(AllowlistStatus::Oversized);
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return EntityAllowlist::invalid(AllowlistStatus::Missing);
        }
        Err(_) => return EntityAllowlist::invalid(AllowlistStatus::Malformed),
    };
    let mut bytes = [0_u8; MAX_ENTITY_FILE_BYTES + 1];
    let mut limited = file.take(bytes.len() as u64);
    let mut count = 0;
    while count < bytes.len() {
        let read = match limited.read(&mut bytes[count..]) {
            Ok(read) => read,
            Err(_) => return EntityAllowlist::invalid(AllowlistStatus::Malformed),
        };
        if read == 0 {
            break;
        }
        count += read;
    }
    if count > MAX_ENTITY_FILE_BYTES {
        EntityAllowlist::invalid(AllowlistStatus::Oversized)
    } else {
        parse_entity_allowlist(&bytes[..count])
    }
}

fn parse_entity_allowlist(bytes: &[u8]) -> EntityAllowlist {
    if bytes.len() > MAX_ENTITY_FILE_BYTES {
        return EntityAllowlist::invalid(AllowlistStatus::Oversized);
    }
    if bytes.is_empty() {
        return EntityAllowlist::invalid(AllowlistStatus::Empty);
    }
    if !bytes.is_ascii() {
        return EntityAllowlist::invalid(AllowlistStatus::Malformed);
    }

    let mut parsed = EntityAllowlist::invalid(AllowlistStatus::Malformed);
    let lines = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    for raw_line in lines.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            return EntityAllowlist::invalid(AllowlistStatus::Malformed);
        }
        if line.len() > 10 || !line.iter().all(u8::is_ascii_digit) {
            return EntityAllowlist::invalid(AllowlistStatus::Malformed);
        }
        let Ok(text) = std::str::from_utf8(line) else {
            return EntityAllowlist::invalid(AllowlistStatus::Malformed);
        };
        let Ok(entity) = text.parse::<u32>() else {
            return EntityAllowlist::invalid(AllowlistStatus::Malformed);
        };
        if entity == 0 || parsed.ids[..parsed.len].contains(&entity) {
            return EntityAllowlist::invalid(AllowlistStatus::Malformed);
        }
        if parsed.len == MAX_TARGET_ENTITIES {
            return EntityAllowlist::invalid(AllowlistStatus::Oversized);
        }
        parsed.ids[parsed.len] = entity;
        parsed.len += 1;
    }
    if parsed.len == 0 {
        return EntityAllowlist::invalid(AllowlistStatus::Empty);
    }
    parsed.status = AllowlistStatus::Ready;
    parsed
}

fn publish_entity_allowlist(generation: u64, allowlist: EntityAllowlist) {
    ALLOWLIST_VALID.store(false, Ordering::Release);
    ALLOWLIST_GENERATION.store(0, Ordering::Release);
    for (index, slot) in ALLOWLIST_IDS.iter().enumerate() {
        slot.store(
            allowlist.ids.get(index).copied().unwrap_or(0),
            Ordering::Relaxed,
        );
    }
    ALLOWLIST_LEN.store(allowlist.len, Ordering::Relaxed);
    ALLOWLIST_STATUS.store(allowlist.status as u8, Ordering::Relaxed);
    ALLOWLIST_VALID.store(allowlist.is_valid(), Ordering::Relaxed);
    ALLOWLIST_GENERATION.store(generation, Ordering::Release);
}

fn allowlisted_entity_index(generation: u64, entity: u32) -> Option<usize> {
    if !allowlist_ready(generation) {
        return None;
    }
    let len = ALLOWLIST_LEN
        .load(Ordering::Acquire)
        .min(MAX_TARGET_ENTITIES);
    ALLOWLIST_IDS[..len]
        .iter()
        .position(|candidate| candidate.load(Ordering::Relaxed) == entity)
}

fn only_allowlisted_owner(ready: bool, count: usize, first_id: u32, owner: u32) -> bool {
    ready && count == 1 && owner != 0 && first_id == owner
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn allowlist_has_only_owner(generation: u64, owner: u32) -> bool {
    only_allowlisted_owner(
        allowlist_ready(generation),
        ALLOWLIST_LEN.load(Ordering::Acquire),
        ALLOWLIST_IDS[0].load(Ordering::Relaxed),
        owner,
    )
}

fn allowlist_ready(generation: u64) -> bool {
    generation != 0
        && ALLOWLIST_GENERATION.load(Ordering::Acquire) == generation
        && ALLOWLIST_VALID.load(Ordering::Acquire)
}

fn capture_allowlist_status(generation: u64) -> (AllowlistStatus, usize, bool) {
    if generation == 0 || ALLOWLIST_GENERATION.load(Ordering::Acquire) != generation {
        return (AllowlistStatus::Disabled, 0, false);
    }
    let status = match ALLOWLIST_STATUS.load(Ordering::Acquire) {
        1 => AllowlistStatus::Ready,
        2 => AllowlistStatus::Missing,
        3 => AllowlistStatus::Oversized,
        4 => AllowlistStatus::Malformed,
        5 => AllowlistStatus::Empty,
        _ => AllowlistStatus::Disabled,
    };
    (
        status,
        ALLOWLIST_LEN
            .load(Ordering::Acquire)
            .min(MAX_TARGET_ENTITIES),
        ALLOWLIST_VALID.load(Ordering::Acquire),
    )
}

pub(crate) fn begin_capture(generation: u64, allowlist: EntityAllowlist) {
    ROAD_HISTORY_CAPTURE_ACTIVE.store(false, Ordering::Release);
    publish_entity_allowlist(generation, allowlist);
    if let Ok(mut samples) = SAMPLES.try_lock() {
        sync_sample_generation(&mut samples, generation);
    } else {
        SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
    }
    ROAD_SAMPLES
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clear(generation);
    if ROAD_HISTORY_ENABLED.load(Ordering::Acquire)
        && native_time_probe_enabled()
        && allowlist.is_valid()
        && allowlist.len == 1
        && allowlist.ids[0] != 0
    {
        crate::road_history::begin_capture();
        ROAD_HISTORY_CAPTURE_ACTIVE.store(true, Ordering::Release);
    } else if ROAD_HISTORY_ENABLED.load(Ordering::Acquire) {
        crate::road_history::begin_capture();
        crate::log::line(
            "road-history capture disabled: it requires one valid owner ID in render-probe.entities and active native-time labels",
        );
    }
    if !allowlist.is_valid() {
        crate::log::line(&format!(
            "renderer feasibility pose sampling disabled for this capture: entity allowlist status {:?}",
            allowlist.status()
        ));
    }
}

pub(crate) fn end_capture() {
    ROAD_HISTORY_CAPTURE_ACTIVE.store(false, Ordering::Release);
    clear_pending_replica_copy(None);
    if ROAD_HISTORY_ENABLED.load(Ordering::Acquire) {
        crate::road_history::end_capture();
    }
}

pub(crate) fn allowlist_info(generation: u64) -> (AllowlistStatus, usize, bool) {
    capture_allowlist_status(generation)
}

fn record_thread(samples: &[AtomicU32; THREAD_SAMPLE_LIMIT], thread_id: u32) {
    if thread_id == 0
        || samples
            .iter()
            .any(|sample| sample.load(Ordering::Acquire) == thread_id)
    {
        return;
    }
    for sample in samples {
        if sample
            .compare_exchange(0, thread_id, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterError {
    EmptyKey,
    DuplicateKey,
    Full,
}

struct DispatchRegistry<const N: usize> {
    writer: Mutex<()>,
    slots: [DispatchSlot; N],
}

impl<const N: usize> DispatchRegistry<N> {
    fn new() -> Self {
        Self {
            writer: Mutex::new(()),
            slots: std::array::from_fn(|_| DispatchSlot::new()),
        }
    }

    fn register(
        &self,
        key: usize,
        dispatch_id: u64,
        meta: RenderMeta,
        task_count: i32,
        worker_count: i32,
    ) -> Result<usize, RegisterError> {
        if key == 0 {
            return Err(RegisterError::EmptyKey);
        }
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(existing) = self.slots.iter().find(|slot| {
            slot.state.load(Ordering::Acquire) == READY && slot.key.load(Ordering::Acquire) == key
        }) {
            existing.invalid.store(true, Ordering::Release);
            return Err(RegisterError::DuplicateKey);
        }
        let Some((index, slot)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.state.load(Ordering::Acquire) == EMPTY)
        else {
            return Err(RegisterError::Full);
        };
        slot.reset();
        slot.dispatch_id.store(dispatch_id, Ordering::Relaxed);
        slot.capture_generation
            .store(meta.capture_generation, Ordering::Relaxed);
        slot.epoch.store(meta.epoch, Ordering::Relaxed);
        slot.renderer.store(meta.renderer, Ordering::Relaxed);
        slot.game_state.store(meta.game_state, Ordering::Relaxed);
        slot.engine.store(meta.engine, Ordering::Relaxed);
        slot.type_index.store(meta.type_index, Ordering::Relaxed);
        slot.task_count.store(task_count, Ordering::Relaxed);
        slot.worker_count.store(worker_count, Ordering::Relaxed);
        slot.binding_valid
            .store(meta.binding_valid, Ordering::Relaxed);
        slot.invalid.store(!meta.binding_valid, Ordering::Relaxed);
        slot.key.store(key, Ordering::Relaxed);
        slot.state.store(READY, Ordering::Release);
        Ok(index)
    }

    fn find(&self, key: usize) -> Option<usize> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            (slot.state.load(Ordering::Acquire) == READY
                && slot.key.load(Ordering::Acquire) == key
                && slot.binding_valid.load(Ordering::Acquire)
                && !slot.invalid.load(Ordering::Acquire))
            .then_some(index)
        })
    }

    fn remains_observable(&self, index: usize, key: usize, generation: u64) -> bool {
        self.slots.get(index).is_some_and(|slot| {
            slot.state.load(Ordering::Acquire) == READY
                && slot.key.load(Ordering::Acquire) == key
                && slot.capture_generation.load(Ordering::Acquire) == generation
                && slot.binding_valid.load(Ordering::Acquire)
                && !slot.invalid.load(Ordering::Acquire)
        })
    }

    fn begin_body(&self, index: usize, thread_id: u32) {
        if let Some(slot) = self.slots.get(index) {
            slot.start_body(thread_id);
        }
    }

    fn end_body(&self, index: usize) {
        if let Some(slot) = self.slots.get(index) {
            slot.finish_body();
        }
    }

    fn mark_invalid(&self, index: usize) {
        if let Some(slot) = self.slots.get(index) {
            slot.invalid.store(true, Ordering::Release);
        }
    }

    fn finish(&self, index: usize, expected_key: usize) -> Option<DispatchSnapshot> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let slot = self.slots.get(index)?;
        if slot.state.load(Ordering::Acquire) != READY {
            return None;
        }
        if slot.key.load(Ordering::Acquire) != expected_key {
            // The joined call's slot no longer matches its key. Fail the
            // observation and clear the bounded slot so no native context
            // address survives its dispatch scope.
            slot.invalid.store(true, Ordering::Release);
            slot.state.store(EMPTY, Ordering::Release);
            slot.key.store(0, Ordering::Release);
            return None;
        }
        let mut snapshot = slot.snapshot();
        if snapshot.started != snapshot.completed || slot.active.load(Ordering::Acquire) != 0 {
            slot.invalid.store(true, Ordering::Release);
            snapshot.invalid = true;
        }
        slot.state.store(EMPTY, Ordering::Release);
        slot.key.store(0, Ordering::Release);
        Some(snapshot)
    }
}

fn registry() -> &'static DispatchRegistry<MAX_DISPATCH_CONTEXTS> {
    REGISTRY.get_or_init(DispatchRegistry::new)
}

#[derive(Debug, Clone, Copy, Default)]
struct SampleEntry {
    used: bool,
    key: RenderKey,
    last_dispatch_id: u64,
}

struct SampleSet {
    capture_generation: u64,
    entries: [SampleEntry; MOVING_INSTANCE_LIMIT],
    last_discovery_dispatch: [u64; MAX_TARGET_ENTITIES],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoseInspection {
    Selected,
    Discover,
    Skip,
}

impl SampleSet {
    const fn new() -> Self {
        Self {
            capture_generation: 0,
            entries: [SampleEntry {
                used: false,
                key: RenderKey {
                    entity: 0,
                    entity_component_index: 0,
                    slot: 0,
                    model_id: 0,
                },
                last_dispatch_id: 0,
            }; MOVING_INSTANCE_LIMIT],
            last_discovery_dispatch: [0; MAX_TARGET_ENTITIES],
        }
    }

    fn clear(&mut self, generation: u64) {
        self.capture_generation = generation;
        self.entries = [SampleEntry::default(); MOVING_INSTANCE_LIMIT];
        self.last_discovery_dispatch = [0; MAX_TARGET_ENTITIES];
    }

    /// The generation field is authoritative while this SampleSet is locked.
    fn ensure_generation(&mut self, generation: u64) -> bool {
        let changed = self.capture_generation != generation;
        if changed {
            self.clear(generation);
        }
        changed
    }

    fn begin_inspection(
        &mut self,
        generation: u64,
        allowlist_index: usize,
        entity: u32,
        component: u32,
        slot: i32,
        dispatch_id: u64,
    ) -> PoseInspection {
        self.ensure_generation(generation);
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.used
                && entry.key.entity == entity
                && entry.key.entity_component_index == component
                && entry.key.slot == slot
        }) {
            if entry.last_dispatch_id == dispatch_id {
                return PoseInspection::Skip;
            }
            entry.last_dispatch_id = dispatch_id;
            return PoseInspection::Selected;
        }
        if allowlist_index >= self.last_discovery_dispatch.len()
            || self.last_discovery_dispatch[allowlist_index] == dispatch_id
        {
            return PoseInspection::Skip;
        }
        self.last_discovery_dispatch[allowlist_index] = dispatch_id;
        PoseInspection::Discover
    }

    fn record_pose(
        &mut self,
        generation: u64,
        key: RenderKey,
        dispatch_id: u64,
        moving: bool,
    ) -> bool {
        self.ensure_generation(generation);
        if let Some(entry) = self.entries.iter_mut().find(|entry| {
            entry.used
                && entry.key.entity == key.entity
                && entry.key.entity_component_index == key.entity_component_index
                && entry.key.slot == key.slot
        }) {
            entry.key.model_id = key.model_id;
            return entry.last_dispatch_id == dispatch_id;
        }
        if !moving {
            return false;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| !entry.used) else {
            return false;
        };
        *entry = SampleEntry {
            used: true,
            key,
            last_dispatch_id: dispatch_id,
        };
        true
    }
}

/// Call while holding `SAMPLES`. The lock-protected SampleSet decides whether
/// budget counters reset; `SAMPLE_GENERATION` is only a fast-path hint.
fn sync_sample_generation(samples: &mut SampleSet, generation: u64) {
    sync_sample_generation_with_counters(
        samples,
        generation,
        &DISCOVERY_INSPECTIONS,
        &SELECTED_POSE_INSPECTIONS,
    );
    SAMPLE_GENERATION.store(generation, Ordering::Release);
}

/// The generation transition and its read-budget reset must happen under the
/// same SampleSet lock. A fast-path observer can be stale by the time it gets
/// that lock, so only the caller that actually advances the locked generation
/// resets either counter.
fn sync_sample_generation_with_counters(
    samples: &mut SampleSet,
    generation: u64,
    discovery_inspections: &AtomicU32,
    selected_pose_inspections: &AtomicU32,
) -> bool {
    if !samples.ensure_generation(generation) {
        return false;
    }
    discovery_inspections.store(0, Ordering::Release);
    selected_pose_inspections.store(0, Ordering::Release);
    true
}

pub fn requested_from(value: Option<&str>) -> bool {
    value == Some("1")
}

pub fn road_offset_requested_from(value: Option<&str>) -> bool {
    requested_from(value)
}

pub fn native_time_requested_from(value: Option<&str>) -> bool {
    requested_from(value)
}

pub fn requested_with_features(
    render_probe: Option<&str>,
    road_offset_probe: Option<&str>,
    timeline: Option<&str>,
) -> bool {
    requested_with_timeline(render_probe, timeline)
        || requested_with_timeline(road_offset_probe, timeline)
}

pub fn requested_with_native_time(
    render_probe: Option<&str>,
    road_offset_probe: Option<&str>,
    native_time_probe: Option<&str>,
    timeline: Option<&str>,
) -> bool {
    requested_with_features(render_probe, road_offset_probe, timeline)
        || (native_time_requested_from(native_time_probe)
            && requested_with_timeline(render_probe, timeline))
}

pub fn road_history_requested_with_features(
    render_probe: Option<&str>,
    native_time_probe: Option<&str>,
    road_history: Option<&str>,
    timeline: Option<&str>,
) -> bool {
    requested_from(road_history)
        && native_time_requested_from(native_time_probe)
        && requested_with_timeline(render_probe, timeline)
}

pub fn requested_with_timeline(render_probe: Option<&str>, timeline: Option<&str>) -> bool {
    requested_from(render_probe) && crate::timeline::watch_requested(timeline)
}

pub fn requested() -> bool {
    requested_from(std::env::var(ENV).ok().as_deref())
        || road_offset_requested_from(std::env::var(ROAD_OFFSET_ENV).ok().as_deref())
        || requested_from(std::env::var(ROAD_HISTORY_ENV).ok().as_deref())
}

pub fn road_offset_enabled() -> bool {
    ROAD_OFFSET_ENABLED.load(Ordering::Acquire)
}

pub(crate) fn road_history_enabled() -> bool {
    ROAD_HISTORY_ENABLED.load(Ordering::Acquire)
}

pub(crate) fn road_history_capture_active() -> bool {
    ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire)
}

#[cfg(all(windows, target_arch = "x86_64"))]
type NewUpdateFn = unsafe extern "system" fn(usize, usize, usize, usize, usize, u8);
#[cfg(all(windows, target_arch = "x86_64"))]
type IndexLookupFn = unsafe extern "system" fn(usize, usize) -> i32;
#[cfg(all(windows, target_arch = "x86_64"))]
type DispatchFn = unsafe extern "system" fn(usize, usize, i32, i32, usize, u8, u8) -> usize;
#[cfg(all(windows, target_arch = "x86_64"))]
type BodyFn = unsafe extern "system" fn(usize, usize, usize) -> usize;
#[cfg(all(windows, target_arch = "x86_64"))]
type GetInstanceFn = unsafe extern "system" fn(usize, i32, usize) -> usize;
#[cfg(all(windows, target_arch = "x86_64"))]
type RoadVf3Fn = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
#[cfg(all(windows, target_arch = "x86_64"))]
type RoadPathHelperFn = unsafe extern "system" fn(usize, usize, u32, usize, usize, f32) -> usize;
#[cfg(all(windows, target_arch = "x86_64"))]
type RoadUserTransformsFn = unsafe extern "system" fn(usize, usize, f32, usize) -> usize;

#[cfg(all(windows, target_arch = "x86_64"))]
static SAMPLE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn new_update_detour(
    renderer: usize,
    model_transformator: usize,
    octree_skip: usize,
    environment: usize,
    callback: usize,
    flag: u8,
) {
    let original = NEW_UPDATE_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    // SAFETY: installation stores the exact 40420 NewUpdate trampoline.
    let original = unsafe { std::mem::transmute::<usize, NewUpdateFn>(original) };
    let generation = crate::timeline::current_generation();
    if generation == 0 {
        // SAFETY: the original arguments are forwarded unchanged.
        unsafe {
            original(
                renderer,
                model_transformator,
                octree_skip,
                environment,
                callback,
                flag,
            )
        };
        return;
    }
    if SAMPLE_GENERATION.load(Ordering::Acquire) != generation {
        if let Ok(mut samples) = SAMPLES.try_lock() {
            // Recheck after taking the lock. Another NewUpdate may have
            // already initialized this generation after our fast-path read.
            if crate::timeline::current_generation() == generation {
                sync_sample_generation(&mut samples, generation);
            }
        } else {
            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let meta = RenderMeta {
        capture_generation: generation,
        epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
        renderer,
        game_state: 0,
        engine: 0,
        type_index: -1,
        binding_valid: false,
    };
    let context = ProducerContext {
        captured: true,
        meta,
    };
    let scope = ProducerScope::enter(context);
    if ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire) {
        let pace = crate::pacing::current();
        let room_pace = crate::road_history::RoomPace {
            active: crate::pacing::installed() && pace.active,
            steps_per_second: pace.steps_per_second,
            speed_percent: pace.speed.0,
            previous_speed_percent: pace.previous_speed.0,
            batch_two: pace.batch_two,
        };
        let _ = crate::road_history::advance_target_for_epoch(
            generation,
            meta.epoch,
            room_pace,
            crate::timeline::monotonic_nanos(),
        );
    }
    crate::timeline::record_with(crate::timeline::Kind::RenderUpdateEntry, meta.epoch, || {
        render_fields(meta, 0, 0)
    });
    // SAFETY: all six original 40420 ABI arguments are preserved.
    unsafe {
        original(
            renderer,
            model_transformator,
            octree_skip,
            environment,
            callback,
            flag,
        )
    };
    let final_meta = ProducerScope::current().meta;
    crate::timeline::record_with(crate::timeline::Kind::RenderUpdateExit, meta.epoch, || {
        let mut fields = render_fields(final_meta, 0, 0);
        fields.render_invalid = !final_meta.binding_valid;
        fields
    });
    drop(scope);
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(naked)]
unsafe extern "system" fn model_list_index_adapter(
    _engine_component_store: usize,
    _rtti: usize,
) -> i32 {
    core::arch::naked_asm!("mov r8, r13", "jmp {observer}", observer = sym model_list_index_observer);
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn model_list_index_observer(
    engine_component_store: usize,
    rtti: usize,
    game_state: usize,
) -> i32 {
    let original = MODEL_LIST_INDEX_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return -1;
    }
    // SAFETY: installation checked this exact call still resolves to 0xa4c80.
    let lookup = unsafe { std::mem::transmute::<usize, IndexLookupFn>(original) };
    // SAFETY: caller's original two arguments, unchanged.
    let index = unsafe { lookup(engine_component_store, rtti) };
    let meta = PRODUCER.with(|cell| {
        let mut producer = cell.get();
        if !producer.captured {
            return None;
        }
        let engine_from_state = game_state
            .checked_add(0x18)
            .and_then(crate::image::guarded::read::<usize>)
            .unwrap_or(0);
        let engine_from_call = engine_component_store.saturating_sub(0x48);
        producer.meta.game_state = game_state;
        producer.meta.engine = engine_from_state;
        producer.meta.type_index = index;
        producer.meta.binding_valid = game_state != 0
            && engine_from_state != 0
            && engine_from_state == engine_from_call
            && index >= 0;
        cell.set(producer);
        Some(producer.meta)
    });
    if let Some(meta) = meta {
        crate::timeline::record_with(
            crate::timeline::Kind::RenderModelListBinding,
            meta.epoch,
            || {
                let mut fields = render_fields(meta, 0, 0);
                fields.render_invalid = !meta.binding_valid;
                fields
            },
        );
    }
    index
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn dispatch_detour(
    pool: usize,
    binder: usize,
    task_count: i32,
    worker_count: i32,
    output: usize,
    flag_a: u8,
    flag_b: u8,
) -> usize {
    let original = DISPATCH_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    // SAFETY: installation stores the exact 40420 helper target.
    let original = unsafe { std::mem::transmute::<usize, DispatchFn>(original) };
    let producer = ProducerScope::current();
    if !producer.captured {
        // SAFETY: dispatch call is forwarded byte-for-byte when no capture
        // produced its frame context.
        return unsafe {
            original(
                pool,
                binder,
                task_count,
                worker_count,
                output,
                flag_a,
                flag_b,
            )
        };
    }
    let context_key = binder
        .checked_add(8)
        .and_then(crate::image::guarded::read::<usize>)
        .unwrap_or(0);
    let dispatch_id = NEXT_DISPATCH.fetch_add(1, Ordering::Relaxed);
    let registration = registry().register(
        context_key,
        dispatch_id,
        producer.meta,
        task_count,
        worker_count,
    );
    let slot_index = registration.as_ref().ok().copied();
    if slot_index.is_some() {
        ACTIVE_DISPATCHES.fetch_add(1, Ordering::AcqRel);
    }
    crate::timeline::record_with(
        crate::timeline::Kind::RenderDispatchEntry,
        dispatch_id,
        || {
            let mut fields = render_fields(producer.meta, context_key, dispatch_id);
            fields.render_task_count = task_count;
            fields.render_worker_count = worker_count;
            fields.render_invalid = slot_index.is_none() || !producer.meta.binding_valid;
            fields.result_byte = register_error_code(registration.err());
            fields
        },
    );
    if let Some(error) = registration.err() {
        crate::timeline::record_with(
            crate::timeline::Kind::RenderProbeRefused,
            dispatch_id,
            || {
                let mut fields = render_fields(producer.meta, context_key, dispatch_id);
                fields.render_invalid = true;
                fields.result_byte = register_error_code(Some(error));
                fields.render_task_count = task_count;
                fields.render_worker_count = worker_count;
                fields
            },
        );
    }
    // SAFETY: all seven original arguments are forwarded unchanged; the
    // helper joins its submitted futures before returning.
    let result = unsafe {
        original(
            pool,
            binder,
            task_count,
            worker_count,
            output,
            flag_a,
            flag_b,
        )
    };
    let snapshot = slot_index.and_then(|index| {
        let snapshot = registry().finish(index, context_key);
        let _ = ACTIVE_DISPATCHES.fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active > 0).then_some(active - 1)
        });
        snapshot
    });
    crate::timeline::record_with(
        crate::timeline::Kind::RenderDispatchExit,
        dispatch_id,
        || {
            let mut fields = render_fields(producer.meta, context_key, dispatch_id);
            fields.render_task_count = task_count;
            fields.render_worker_count = worker_count;
            if let Some(snapshot) = snapshot {
                fields.render_body_started = snapshot.started;
                fields.render_body_completed = snapshot.completed;
                fields.render_body_max_active = snapshot.max_active;
                fields.render_thread_ids = snapshot.thread_ids;
                fields.render_invalid = snapshot.invalid;
                fields.render_context = snapshot.key;
            } else {
                fields.render_invalid = true;
                fields.result_byte = 5;
            }
            fields.render_sample_skips = SAMPLE_SKIPS.load(Ordering::Acquire);
            fields.render_context_misses = CONTEXT_MISSES.load(Ordering::Acquire);
            fields.arg_a = result;
            fields
        },
    );
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn render_body_detour(context: usize, a: usize, b: usize) -> usize {
    let original = BODY_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    // SAFETY: installation stores the exact 40420 body trampoline.
    let original = unsafe { std::mem::transmute::<usize, BodyFn>(original) };
    if !crate::timeline::active() && ACTIVE_DISPATCHES.load(Ordering::Acquire) == 0 {
        // SAFETY: no captured renderer dispatch needs thread correlation.
        return unsafe { original(context, a, b) };
    }
    let Some(index) = registry().find(context) else {
        if crate::timeline::active() {
            CONTEXT_MISSES.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: context miss does not alter the native call.
        return unsafe { original(context, a, b) };
    };
    let slot = &registry().slots[index];
    let meta = slot.metadata();
    if !registry().remains_observable(index, context, meta.capture_generation) {
        // A duplicate registration or guarded read may have invalidated this
        // context after find() but before this worker arrived.
        if crate::timeline::active() {
            CONTEXT_MISSES.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: invalid probe metadata never changes the native body call.
        return unsafe { original(context, a, b) };
    }
    let thread_id = crate::timeline::thread_id();
    registry().begin_body(index, thread_id);
    let body_context = BodyContext {
        registry_index: index,
        key: context,
        dispatch_id: slot.dispatch_id.load(Ordering::Acquire),
        meta,
        fresh_instance: FreshInstance::default(),
        active: true,
    };
    let previous = BODY.with(|cell| cell.replace(body_context));
    // SAFETY: the three native body arguments are unchanged.
    let result = unsafe { original(context, a, b) };
    BODY.with(|cell| cell.set(previous));
    registry().end_body(index);
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(naked)]
unsafe extern "system" fn get_instance_adapter(
    _model_list: usize,
    _slot: i32,
    _out: usize,
) -> usize {
    core::arch::naked_asm!("mov r9, r13", "jmp {observer}", observer = sym get_instance_observer);
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn get_instance_observer(
    model_list: usize,
    slot: i32,
    out: usize,
    entity_ref: usize,
) -> usize {
    let original = GET_INSTANCE_ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    // SAFETY: installation checked the call still targets 0x2fdb80.
    let get_instance = unsafe { std::mem::transmute::<usize, GetInstanceFn>(original) };
    // The pointer is consumed only during this call; the probe copies its
    // three integer fields before returning from this adapter.
    let body_context = BODY.with(std::cell::Cell::get);
    let generation = crate::timeline::current_generation();
    let observing = body_context.active
        && body_context.meta.binding_valid
        && generation == body_context.meta.capture_generation
        && registry().remains_observable(
            body_context.registry_index,
            body_context.key,
            body_context.meta.capture_generation,
        );
    let identity = observing
        .then(|| crate::image::guarded::read::<[u32; 3]>(entity_ref))
        .flatten();
    if body_context.active {
        BODY.with(|cell| {
            let mut current = cell.get();
            if current.key == body_context.key && current.dispatch_id == body_context.dispatch_id {
                current.fresh_instance = FreshInstance::default();
                cell.set(current);
            }
        });
    }
    // SAFETY: native arguments and ordering are exactly preserved.
    let returned = unsafe { get_instance(model_list, slot, out) };
    if observing
        && let Some([entity, entity_component_index, instance_slot]) = identity
        && returned != 0
        && registry().remains_observable(
            body_context.registry_index,
            body_context.key,
            body_context.meta.capture_generation,
        )
    {
        BODY.with(|cell| {
            let mut current = cell.get();
            if current.active
                && current.key == body_context.key
                && current.dispatch_id == body_context.dispatch_id
            {
                current.fresh_instance = FreshInstance {
                    present: true,
                    entity,
                    entity_component_index,
                    slot: i32::from_ne_bytes(instance_slot.to_ne_bytes()),
                    returned,
                    capture_generation: body_context.meta.capture_generation,
                    dispatch_id: body_context.dispatch_id,
                };
                cell.set(current);
            }
        });
    }
    if observing && allowlist_ready(generation) && slot < 0 {
        if let Some([entity, entity_component_index, instance_slot]) = identity {
            if let Some(allowlist_index) = allowlisted_entity_index(generation, entity) {
                let instance_slot = i32::from_ne_bytes(instance_slot.to_ne_bytes());
                let inspection = match SAMPLES.try_lock() {
                    Ok(mut samples) => {
                        if crate::timeline::current_generation()
                            != body_context.meta.capture_generation
                            || !registry().remains_observable(
                                body_context.registry_index,
                                body_context.key,
                                body_context.meta.capture_generation,
                            )
                        {
                            PoseInspection::Skip
                        } else {
                            sync_sample_generation(
                                &mut samples,
                                body_context.meta.capture_generation,
                            );
                            samples.begin_inspection(
                                body_context.meta.capture_generation,
                                allowlist_index,
                                entity,
                                entity_component_index,
                                instance_slot,
                                body_context.dispatch_id,
                            )
                        }
                    }
                    Err(_) => {
                        SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
                        PoseInspection::Skip
                    }
                };
                let within_budget = match inspection {
                    PoseInspection::Selected => reserve_inspection(
                        &SELECTED_POSE_INSPECTIONS,
                        MAX_SELECTED_POSE_INSPECTIONS,
                    ),
                    PoseInspection::Discover => {
                        reserve_inspection(&DISCOVERY_INSPECTIONS, MAX_DISCOVERY_INSPECTIONS)
                    }
                    PoseInspection::Skip => false,
                };
                if within_budget {
                    let model_id = crate::image::guarded::read::<u32>(returned);
                    match model_id.and_then(|model_id| read_pose(returned, model_id)) {
                        Some(pose) => record_pose(
                            body_context,
                            entity,
                            entity_component_index,
                            instance_slot,
                            pose,
                        ),
                        None => {
                            registry().mark_invalid(body_context.registry_index);
                            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                } else if inspection != PoseInspection::Skip {
                    SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
                }
            }
        } else if identity.is_none() {
            registry().mark_invalid(body_context.registry_index);
            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
        }
    }
    returned
}

#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_MODEL_INPUT: u8 = 1;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_ASSOCIATION: u8 = 2;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_ALPHA: u8 = 3;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_CONTEXT: u8 = 4;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_ENGINE: u8 = 5;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_OWNER: u8 = 6;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_PATH: u8 = 7;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_PATH_RESULT: u8 = 8;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_REPEATED_HELPER: u8 = 9;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_WORLD_VECTOR: u8 = 10;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_WORLD_FLOAT: u8 = 11;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_FINISH_MISSING: u8 = 12;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_DISPATCH: u8 = 13;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_INVALID_REPEATED_FINISH: u8 = 14;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_NONE: u8 = 0;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_SCOPE: u8 = 1;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_TARGET: u8 = 2;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_TIME: u8 = 3;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_WRITER: u8 = 4;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_VECTOR: u8 = 5;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_COUNT: u8 = 6;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_GAP: u8 = 7;
#[cfg(all(windows, target_arch = "x86_64"))]
const ROAD_HISTORY_REJECT_MARKER: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorldVectorCopy {
    total: u16,
    copied: u8,
    truncated: bool,
    words: [u32; 24],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(32))]
struct PrivateWorldRecords {
    /// Six raw f32 words per record: position followed by direction.
    words: [u32; 24],
    count: u8,
}

fn translated_world_records(copy: WorldVectorCopy, offset_x: f32) -> Option<PrivateWorldRecords> {
    let count = usize::from(copy.total);
    if count == 0
        || count > MAX_WORLD_POSITION_RECORDS
        || usize::from(copy.copied) != count
        || copy.truncated
        || !offset_x.is_finite()
    {
        return None;
    }
    let mut translated = copy.words;
    for record in 0..count {
        let start = record * 6;
        for word in &translated[start..start + 6] {
            if !f32::from_bits(*word).is_finite() {
                return None;
            }
        }
        let x = f32::from_bits(translated[start]);
        let shifted_x = x + offset_x;
        if !shifted_x.is_finite() {
            return None;
        }
        translated[start] = shifted_x.to_bits();
    }
    Some(PrivateWorldRecords {
        words: translated,
        count: u8::try_from(count).ok()?,
    })
}

fn private_world_vector_header(records: &PrivateWorldRecords) -> [usize; 3] {
    let begin = records.words.as_ptr() as usize;
    let end = begin + usize::from(records.count) * 24;
    [begin, end, end]
}

fn road_offset_scope_eligible(
    enabled: bool,
    capture_active: bool,
    current_generation: u64,
    observation: RoadObservation,
    registry_live: bool,
    only_allowlisted_owner: bool,
) -> bool {
    enabled
        && capture_active
        && current_generation != 0
        && observation.active
        && observation.meta.capture_generation == current_generation
        && !observation.invalid
        && observation.path_seen
        && observation.owner != 0
        && observation.finish_calls == 0
        && registry_live
        && only_allowlisted_owner
}

fn should_forward_translated(
    scope_eligible: bool,
    vector_valid: bool,
    applied_event_recorded: bool,
) -> bool {
    scope_eligible && vector_valid && applied_event_recorded
}

fn vector24_record_count(begin: usize, end: usize, capacity: usize) -> Option<usize> {
    if begin == 0 && end == 0 && capacity == 0 {
        return Some(0);
    }
    if begin == 0
        || begin > end
        || end > capacity
        || !begin.is_multiple_of(4)
        || !end.is_multiple_of(4)
        || !capacity.is_multiple_of(4)
    {
        return None;
    }
    let used = end.checked_sub(begin)?;
    let reserved = capacity.checked_sub(begin)?;
    if used % 24 != 0 || reserved % 24 != 0 || used > reserved {
        return None;
    }
    let count = used / 24;
    (count <= 4_096).then_some(count)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn read_path_snapshot_live(
    path: usize,
    observation: RoadObservation,
) -> Option<([u32; 10], [u32; 10], u8)> {
    let current = read_between_live_checks(
        || road_scope_observable(observation),
        || {
            path.checked_add(0x4c)
                .and_then(crate::image::guarded::read::<[u32; 10]>)
        },
    )?;
    let previous = read_between_live_checks(
        || road_scope_observable(observation),
        || {
            path.checked_add(0x74)
                .and_then(crate::image::guarded::read::<[u32; 10]>)
        },
    )?;
    let valid = read_between_live_checks(
        || road_scope_observable(observation),
        || {
            path.checked_add(0x9c)
                .and_then(crate::image::guarded::read::<u8>)
        },
    )?;
    Some((current, previous, valid))
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn read_path_result_live(result: usize, observation: RoadObservation) -> Option<(u64, u32)> {
    let first = read_between_live_checks(
        || road_scope_observable(observation),
        || crate::image::guarded::read::<u64>(result),
    )?;
    let second = read_between_live_checks(
        || road_scope_observable(observation),
        || {
            result
                .checked_add(8)
                .and_then(crate::image::guarded::read::<u32>)
        },
    )?;
    Some((first, second))
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn read_world_vector_live(
    vector: usize,
    observation: RoadObservation,
) -> Result<WorldVectorCopy, u8> {
    let [begin, end, capacity] = read_between_live_checks(
        || road_scope_observable(observation),
        || crate::image::guarded::read::<[usize; 3]>(vector),
    )
    .ok_or(ROAD_INVALID_DISPATCH)?;
    let total = vector24_record_count(begin, end, capacity).ok_or(ROAD_INVALID_WORLD_VECTOR)?;
    let copied = total.min(MAX_WORLD_POSITION_RECORDS);
    let mut copy = WorldVectorCopy {
        total: u16::try_from(total).unwrap_or(u16::MAX),
        copied: u8::try_from(copied).unwrap_or(u8::MAX),
        truncated: total > copied,
        words: [0; 24],
    };
    for index in 0..copied {
        let address = begin
            .checked_add(index.checked_mul(24).ok_or(ROAD_INVALID_WORLD_VECTOR)?)
            .ok_or(ROAD_INVALID_WORLD_VECTOR)?;
        let words = read_between_live_checks(
            || road_scope_observable(observation),
            || crate::image::guarded::read::<[u32; 6]>(address),
        )
        .ok_or(ROAD_INVALID_DISPATCH)?;
        if words.iter().any(|word| !f32::from_bits(*word).is_finite()) {
            return Err(ROAD_INVALID_WORLD_FLOAT);
        }
        let start = index * 6;
        copy.words[start..start + 6].copy_from_slice(&words);
    }
    Ok(copy)
}

fn association_matches(
    fresh: FreshInstance,
    generation: u64,
    dispatch_id: u64,
    child: u32,
    model_instance: usize,
) -> bool {
    fresh.present
        && fresh.capture_generation == generation
        && fresh.dispatch_id == dispatch_id
        && fresh.entity == child
        && fresh.returned != 0
        && fresh.returned == model_instance
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn road_vf3_detour(
    transformator: usize,
    context: usize,
    model_input: usize,
    output: usize,
) -> usize {
    let original_address = ROAD_VF3_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: installation stores the exact 40420 vf3 trampoline.
    let original = unsafe { std::mem::transmute::<usize, RoadVf3Fn>(original_address) };
    if !crate::timeline::active() {
        // SAFETY: the original four arguments and one native call are preserved.
        return unsafe { original(transformator, context, model_input, output) };
    }

    // Start the span before any native vf3 input reads or nested transform
    // work; the return timestamp is taken immediately after the trampoline.
    let started_nanos = crate::timeline::monotonic_nanos();
    let body = BODY.with(|cell| {
        let mut body = cell.get();
        let fresh = body.fresh_instance;
        body.fresh_instance = FreshInstance::default();
        cell.set(body);
        (body, fresh)
    });
    let (body, fresh_instance) = body;
    let generation = crate::timeline::current_generation();
    let history_capture = ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire)
        && generation != 0
        && generation == crate::timeline::current_generation();
    let history_invalidation_generation = if history_capture {
        crate::road_history::invalidation_generation()
    } else {
        0
    };
    let vf3_writer_start = if history_capture {
        native_writer_snapshot(body.meta.engine)
    } else {
        NativeWriterSnapshot::EMPTY
    };
    let child = read_between_live_checks(
        || road_body_observable(body, generation),
        || crate::image::guarded::read::<u32>(model_input),
    );
    let model_instance = read_between_live_checks(
        || road_body_observable(body, generation),
        || {
            model_input
                .checked_add(8)
                .and_then(crate::image::guarded::read::<usize>)
        },
    );
    let model_id = model_instance
        .filter(|address| *address != 0)
        .and_then(|address| {
            read_between_live_checks(
                || road_body_observable(body, generation),
                || crate::image::guarded::read::<u32>(address),
            )
        });
    let alpha = read_between_live_checks(
        || road_body_observable(body, generation),
        || {
            context
                .checked_add(0x10)
                .and_then(crate::image::guarded::read::<f32>)
        },
    );
    let dispatch_valid = road_body_observable(body, generation);
    let identity_valid = match (child, model_instance) {
        (Some(child), Some(model_instance)) => association_matches(
            fresh_instance,
            generation,
            body.dispatch_id,
            child,
            model_instance,
        ),
        _ => false,
    };
    let alpha_bits = alpha.map(f32::to_bits).unwrap_or(0);
    let alpha_valid = alpha.is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value));
    let reason = if !dispatch_valid {
        ROAD_INVALID_DISPATCH
    } else if child.is_none() || model_instance.is_none() || model_id.is_none() {
        ROAD_INVALID_MODEL_INPUT
    } else if !identity_valid {
        ROAD_INVALID_ASSOCIATION
    } else if !alpha_valid {
        ROAD_INVALID_ALPHA
    } else {
        0
    };
    if reason != 0 {
        if body.active {
            note_road_refused(generation);
        }
        // SAFETY: a failed observation does not change the native call.
        return unsafe { original(transformator, context, model_input, output) };
    }

    let observation = RoadObservation {
        active: true,
        invalid_reason: 0,
        started_nanos,
        meta: body.meta,
        context,
        transformator,
        model_input,
        output,
        child: child.unwrap_or(0),
        model_id: model_id.unwrap_or(0),
        model_instance: model_instance.unwrap_or(0),
        fresh_instance,
        alpha_bits,
        history_invalidation_generation,
        vf3_writer_start,
        ..RoadObservation::EMPTY
    };
    let scope = RoadScope::enter(observation);
    // SAFETY: the original four arguments and exactly one native call are preserved.
    let result = unsafe { original(transformator, context, model_input, output) };
    let finished_nanos = crate::timeline::monotonic_nanos();
    let vf3_writer_end = if history_capture {
        native_writer_snapshot(body.meta.engine)
    } else {
        NativeWriterSnapshot::EMPTY
    };
    let mut observation = RoadScope::current();
    observation.vf3_writer_end = vf3_writer_end;
    if observation.active && !observation.invalid {
        if !observation.path_seen {
            observation.invalid = true;
            observation.invalid_reason = ROAD_INVALID_PATH;
        } else if observation.helper_calls != 1 {
            observation.invalid = true;
            observation.invalid_reason = ROAD_INVALID_REPEATED_HELPER;
        } else if observation.finish_calls == 0 {
            observation.invalid = true;
            observation.invalid_reason = ROAD_INVALID_FINISH_MISSING;
        } else if observation.finish_calls != 1 {
            observation.invalid = true;
            observation.invalid_reason = ROAD_INVALID_REPEATED_FINISH;
        }
    }
    if observation.active {
        record_road_observation(observation, result, finished_nanos);
    }
    drop(scope);
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(naked)]
unsafe extern "system" fn road_path_helper_adapter(
    _out: usize,
    _engine: usize,
    _type_index: u32,
    _network: usize,
    _move_path: usize,
    _alpha: f32,
) -> usize {
    core::arch::naked_asm!(
        "mov eax, dword ptr [rsp + 0x7c]",
        "mov dword ptr [rsp + 0x38], eax",
        "jmp {observer}",
        observer = sym road_path_helper_observer
    );
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn road_path_helper_observer(
    out: usize,
    engine: usize,
    type_index: u32,
    network: usize,
    move_path: usize,
    alpha: f32,
    owner: u32,
) -> usize {
    let original_address = ROAD_PATH_HELPER_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: this exact function accepts the six native arguments; owner is
    // the observer-only seventh stack value injected into the unused slot.
    let original = unsafe { std::mem::transmute::<usize, RoadPathHelperFn>(original_address) };
    let observation = RoadScope::current();
    let scoped = crate::timeline::active() && road_scope_observable(observation);
    // Recheck the body registry using its native Lambda7 context key; the
    // dispatch ID is diagnostic metadata and is not the registry key.
    let body_matches = road_scope_observable(observation);
    let allowlisted =
        scoped && allowlisted_entity_index(observation.meta.capture_generation, owner).is_some();
    let history_family_revision = if ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire)
        && allowlisted
        && allowlist_has_only_owner(observation.meta.capture_generation, owner)
    {
        crate::road_history::select_family(owner, observation.child).unwrap_or(0)
    } else {
        0
    };
    let path_snapshot = if path_read_eligible(
        observation,
        crate::timeline::current_generation(),
        body_matches,
        engine,
        allowlisted,
    ) {
        read_path_snapshot_live(move_path, observation)
    } else {
        None
    };
    let capture_path = path_snapshot.is_some();
    if scoped {
        RoadScope::update(|scope| {
            scope.owner = owner;
            scope.history_family_revision = history_family_revision;
            scope.helper_out = out;
            scope.helper_engine = engine;
            scope.helper_type_index = type_index;
            scope.helper_network = network;
            scope.helper_move_path = move_path;
            scope.helper_alpha_bits = alpha.to_bits();
            scope.helper_calls = scope.helper_calls.saturating_add(1);
            if !body_matches {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_CONTEXT;
            } else if engine != scope.meta.engine || engine == 0 {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_ENGINE;
            } else if scope.path_seen {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_REPEATED_HELPER;
            } else if !allowlisted {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_OWNER;
            } else if alpha.to_bits() != scope.alpha_bits || path_snapshot.is_none() {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_PATH;
            } else if let Some((current, previous, valid)) = path_snapshot {
                scope.path_seen = true;
                scope.path_valid_before = valid;
                scope.path_current_before = current;
                scope.path_previous_before = previous;
                scope.invalid_reason = 0;
            }
        });
    }
    // SAFETY: original six native arguments are forwarded once and unchanged;
    // injected owner is observer metadata only.
    let result = unsafe { original(out, engine, type_index, network, move_path, alpha) };
    if capture_path {
        let after = read_path_snapshot_live(move_path, observation);
        let result_words = read_path_result_live(result, observation);
        RoadScope::update(|scope| match (after, result_words) {
            (Some((current, previous, valid)), Some((word0, word1))) => {
                scope.path_valid_after = valid;
                scope.path_current_after = current;
                scope.path_previous_after = previous;
                scope.path_result_pointer = result;
                scope.path_result_word0 = word0;
                scope.path_result_word1 = word1;
            }
            _ => {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_PATH_RESULT;
            }
        });
    }
    result
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "system" fn road_user_transforms_observer(
    definition: usize,
    world_vector: usize,
    alpha: f32,
    output: usize,
) -> usize {
    let original_address = ROAD_USER_TRANSFORMS_ORIGINAL.load(Ordering::Acquire);
    if original_address == 0 {
        return 0;
    }
    // SAFETY: installation verifies this exact 40420 call target and ABI.
    let original = unsafe { std::mem::transmute::<usize, RoadUserTransformsFn>(original_address) };
    let observation = RoadScope::current();
    let capture_active = crate::timeline::active();
    let generation = crate::timeline::current_generation();
    let registry_live = capture_active && road_scope_observable(observation);
    let allowlist_index =
        allowlisted_entity_index(observation.meta.capture_generation, observation.owner);
    let in_scope =
        registry_live && !observation.invalid && observation.path_seen && allowlist_index.is_some();
    let observing = in_scope && finish_read_eligible(observation, generation, true);
    let offset_requested = ROAD_OFFSET_ENABLED.load(Ordering::Acquire);
    let only_allowlisted_owner = offset_requested
        && allowlist_has_only_owner(observation.meta.capture_generation, observation.owner);
    let offset_eligible = road_offset_scope_eligible(
        offset_requested,
        capture_active,
        generation,
        observation,
        registry_live,
        only_allowlisted_owner,
    );
    let history_capture = ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire);
    let history_candidate = history_capture
        && in_scope
        && allowlist_has_only_owner(observation.meta.capture_generation, observation.owner)
        && crate::road_history::family_matches(
            observation.owner,
            observation.child,
            observation.history_family_revision,
        );
    let history_invalidation_epoch =
        history_candidate.then(crate::road_history::invalidation_generation);
    let history_attempt = history_capture
        && in_scope
        && allowlist_has_only_owner(observation.meta.capture_generation, observation.owner);
    let history_target = history_candidate
        .then(|| {
            crate::road_history::presentation_target(
                observation.meta.capture_generation,
                observation.meta.epoch,
            )
        })
        .flatten();
    let history_time = history_candidate.then(|| {
        native_time_label(
            observation.native_time,
            observation.meta,
            observation.helper_engine,
        )
    });
    let history_writer_now =
        history_candidate.then(|| native_writer_snapshot(observation.helper_engine));
    let history_stamp_eligible = match (history_target, history_time, history_writer_now) {
        (Some(target), Some(time), Some(writer)) => {
            target.capture_generation == generation
                && target.producer_epoch == observation.meta.epoch
                && target.world_epoch != 0
                && target.world_epoch == time.world_epoch
                && target.incarnation == observation.history_family_revision
                && Some(target.invalidation_generation) == history_invalidation_epoch
                && time.label_valid
                && history_writer_matches(observation, time, writer)
        }
        _ => false,
    };
    // A repeated callback still reaches the native function, but is already
    // ineligible for a row. The offset experiment independently copies every
    // eligible invocation; it does not use the moving-sample discovery gate.
    let copied = (observing || offset_eligible || history_stamp_eligible)
        .then(|| read_world_vector_live(world_vector, observation));
    let translated = offset_eligible
        .then(|| {
            copied
                .as_ref()?
                .as_ref()
                .ok()
                .and_then(|copy| translated_world_records(*copy, ROAD_OFFSET_X_METERS))
        })
        .flatten();
    let translated_header = translated.as_ref().map(private_world_vector_header);
    let offset_event = match (translated.as_ref(), translated_header.as_ref()) {
        (Some(records), Some(_header)) => crate::timeline::record_with_guard(
            crate::timeline::Kind::RoadOffsetApplied,
            observation.fresh_instance.dispatch_id,
            || {
                let current = RoadScope::current();
                let current_generation = crate::timeline::current_generation();
                let current_live = crate::timeline::active() && road_scope_observable(current);
                let current_single_owner =
                    allowlist_has_only_owner(current_generation, current.owner);
                road_offset_scope_eligible(
                    ROAD_OFFSET_ENABLED.load(Ordering::Acquire),
                    crate::timeline::active(),
                    current_generation,
                    current,
                    current_live,
                    current_single_owner,
                ) && current.meta.capture_generation == observation.meta.capture_generation
                    && current.context == observation.context
                    && current.owner == observation.owner
                    && current.child == observation.child
                    && current.fresh_instance == observation.fresh_instance
            },
            || crate::timeline::Fields {
                arg_a: usize::from(records.count),
                arg_b: observation.fresh_instance.dispatch_id as usize,
                arg_c: observation.owner as usize,
                render_epoch: observation.meta.epoch,
                render_vehicle_owner: observation.owner,
                render_vehicle_child: observation.child,
                render_vehicle_component_index: observation.fresh_instance.entity_component_index,
                render_vehicle_slot: observation.fresh_instance.slot,
                render_vehicle_model_id: observation.model_id,
                render_vehicle_alpha_bits: alpha.to_bits(),
                render_vehicle_finish_alpha_bits: alpha.to_bits(),
                render_vehicle_world_records_total: u16::from(records.count),
                render_vehicle_world_records_copied: records.count,
                render_vehicle_offset_x_bits: ROAD_OFFSET_X_METERS.to_bits(),
                render_vehicle_world_points: records.words,
                ..crate::timeline::Fields::default()
            },
        ),
        _ => None,
    };
    let forward_translated = should_forward_translated(
        offset_eligible,
        translated.is_some() && translated_header.is_some(),
        offset_event.is_some(),
    );
    let history_input_count = copied.as_ref().and_then(|copy| {
        copy.as_ref()
            .ok()
            .filter(|copy| {
                copy.total != 0
                    && copy.total <= MAX_WORLD_POSITION_RECORDS as u16
                    && copy.total == u16::from(copy.copied)
                    && !copy.truncated
            })
            .map(|copy| copy.copied)
    });
    let history_pose_candidate = if history_stamp_eligible {
        match (history_target, history_input_count, history_time) {
            (Some(target), Some(_input_count), Some(time)) => {
                crate::road_history::interpolated_pose(
                    target.step_position,
                    target.capture_generation,
                    target.world_epoch,
                    target.incarnation,
                    target.invalidation_generation,
                    crate::road_history::FamilyKey {
                        owner: observation.owner,
                        child: observation.child,
                        component_index: observation.fresh_instance.entity_component_index,
                        slot: observation.fresh_instance.slot,
                        model_id: observation.model_id,
                        definition,
                    },
                )
                .filter(|_| time.label_valid)
            }
            _ => None,
        }
    } else {
        None
    };
    let history_pose =
        history_pose_candidate.filter(|pose| Some(pose.count) == history_input_count);
    let history_storage = history_pose.map(crate::road_history::PrivateRecords::from_pose);
    let history_header = history_storage
        .as_ref()
        .map(crate::road_history::PrivateRecords::vector_header);
    let mut history_apply_lease = None;
    let history_applied_event = match (history_pose, history_target, history_time, history_header) {
        (Some(pose), Some(target), Some(time), Some(_header)) => {
            crate::timeline::record_with_guard(
                crate::timeline::Kind::RoadHistoryApplied,
                observation.fresh_instance.dispatch_id,
                || {
                    let current = RoadScope::current();
                    let writer = native_writer_snapshot(current.helper_engine);
                    crate::timeline::active()
                        && ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire)
                        && road_scope_observable(current)
                        && current.meta == observation.meta
                        && current.context == observation.context
                        && current.fresh_instance == observation.fresh_instance
                        && current.owner == observation.owner
                        && current.child == observation.child
                        && current.helper_move_path == observation.helper_move_path
                        && current.finish_calls == 0
                        && !current.invalid
                        && crate::road_history::family_matches(
                            observation.owner,
                            observation.child,
                            observation.history_family_revision,
                        )
                        && crate::road_history::presentation_target(
                            observation.meta.capture_generation,
                            observation.meta.epoch,
                        ) == Some(target)
                        && time.label_valid
                        && history_writer_matches(observation, time, writer)
                        && {
                            history_apply_lease = crate::road_history::try_apply_lease(
                                target.invalidation_generation,
                            );
                            history_apply_lease.is_some()
                        }
                },
                || {
                    let mut words = [0_u32; 24];
                    for (index, record) in pose
                        .records
                        .iter()
                        .take(usize::from(pose.count))
                        .enumerate()
                    {
                        words[index * 6..index * 6 + 6].copy_from_slice(record);
                    }
                    crate::timeline::Fields {
                        this: observation.owner as usize,
                        data: observation.child as usize,
                        arg_a: usize::from(pose.count),
                        arg_b: pose.lower_step.to_bits() as usize,
                        arg_c: pose.upper_step.to_bits() as usize,
                        render_epoch: observation.meta.epoch,
                        render_game_state: observation.meta.game_state,
                        render_engine: observation.meta.engine,
                        render_vehicle_owner: observation.owner,
                        render_vehicle_child: observation.child,
                        render_vehicle_component_index: observation
                            .fresh_instance
                            .entity_component_index,
                        render_vehicle_slot: observation.fresh_instance.slot,
                        render_vehicle_model_id: observation.model_id,
                        render_vehicle_finish_definition: definition,
                        render_vehicle_alpha_bits: observation.alpha_bits,
                        render_vehicle_finish_alpha_bits: alpha.to_bits(),
                        render_vehicle_world_records_total: u16::from(pose.count),
                        render_vehicle_world_records_copied: pose.count,
                        render_vehicle_world_points: words,
                        native_time_before: time.previous.unwrap_or(0),
                        native_time_after: time.current.unwrap_or(0),
                        native_time_interp_return: time.interp_return,
                        native_time_alpha_bits: time.alpha_bits,
                        native_time_label_valid: time.label_valid,
                        native_time_previous_step: time.previous_step,
                        native_time_current_step: time.current_step,
                        native_time_world_epoch: time.world_epoch,
                        native_time_current_engine: observation.helper_engine,
                        road_history_target_step_bits: target.step_position.to_bits(),
                        road_history_lower_step_bits: pose.lower_step.to_bits(),
                        road_history_upper_step_bits: pose.upper_step.to_bits(),
                        road_history_incarnation: target.incarnation,
                        road_history_record_count: pose.count,
                        road_history_rejection: ROAD_HISTORY_REJECT_NONE,
                        ..crate::timeline::Fields::default()
                    }
                },
            )
        }
        _ => None,
    };
    let history_forward = crate::road_history::history_forward_eligible(
        history_applied_event.is_some(),
        history_storage.is_some(),
        history_header.is_some(),
        history_apply_lease.is_some(),
    );
    if history_attempt && !history_forward {
        let rejection = if !history_candidate {
            ROAD_HISTORY_REJECT_SCOPE
        } else if history_target.is_none() {
            ROAD_HISTORY_REJECT_TARGET
        } else if history_time.is_none_or(|time| !time.label_valid) {
            ROAD_HISTORY_REJECT_TIME
        } else if !history_stamp_eligible {
            ROAD_HISTORY_REJECT_WRITER
        } else if history_input_count.is_none() {
            ROAD_HISTORY_REJECT_VECTOR
        } else if history_pose_candidate.is_some() && history_pose.is_none() {
            ROAD_HISTORY_REJECT_COUNT
        } else if history_pose.is_none() {
            ROAD_HISTORY_REJECT_GAP
        } else {
            ROAD_HISTORY_REJECT_MARKER
        };
        let _ = crate::timeline::record_with(
            crate::timeline::Kind::RoadHistoryFallback,
            observation.fresh_instance.dispatch_id,
            || crate::timeline::Fields {
                this: observation.owner as usize,
                data: observation.child as usize,
                result_byte: rejection,
                render_epoch: observation.meta.epoch,
                render_game_state: observation.meta.game_state,
                render_engine: observation.meta.engine,
                render_vehicle_owner: observation.owner,
                render_vehicle_child: observation.child,
                render_vehicle_component_index: observation.fresh_instance.entity_component_index,
                render_vehicle_slot: observation.fresh_instance.slot,
                render_vehicle_model_id: observation.model_id,
                render_vehicle_finish_definition: definition,
                render_vehicle_alpha_bits: observation.alpha_bits,
                road_history_target_step_bits: history_target
                    .map(|target| target.step_position.to_bits())
                    .unwrap_or(0),
                road_history_incarnation: observation.history_family_revision,
                road_history_record_count: history_input_count.unwrap_or(0),
                road_history_rejection: rejection,
                ..crate::timeline::Fields::default()
            },
        );
    }
    // A successful applied marker is reserved inside the timeline buffer only
    // after its live-scope guard succeeds. This call immediately consumes that
    // reservation; otherwise the original vector is forwarded byte-for-byte.
    let result = match (history_forward, history_header, history_storage.as_ref()) {
        (true, Some(header), Some(_storage)) => {
            // SAFETY: the owned, 32-byte-aligned records remain alive for the
            // complete synchronous native finalizer call.
            let result = unsafe { original(definition, header.as_ptr() as usize, alpha, output) };
            crate::road_history::note_applied();
            drop(history_apply_lease);
            result
        }
        _ => match (forward_translated, translated.as_ref(), translated_header) {
            (true, Some(_records), Some(header)) => {
                // SAFETY: `translated` owns aligned stack storage through this
                // synchronous native call; its header and records remain alive.
                unsafe { original(definition, header.as_ptr() as usize, alpha, output) }
            }
            _ => {
                // SAFETY: the exact original arguments are forwarded once.
                unsafe { original(definition, world_vector, alpha, output) }
            }
        },
    };
    if in_scope {
        RoadScope::update(|scope| {
            let repeated = scope.finish_calls != 0;
            scope.finish_calls = scope.finish_calls.saturating_add(1);
            scope.finish_definition = definition;
            scope.finish_output = output;
            scope.finish_alpha_bits = alpha.to_bits();
            scope.finish_return = result;
            if repeated {
                scope.invalid = true;
                scope.invalid_reason = ROAD_INVALID_REPEATED_FINISH;
            } else {
                match copied {
                    Some(Ok(vector)) => append_world_vector(scope, vector),
                    Some(Err(reason)) => {
                        scope.invalid = true;
                        scope.invalid_reason = reason;
                    }
                    None => {
                        scope.invalid = true;
                        scope.invalid_reason = ROAD_INVALID_WORLD_VECTOR;
                    }
                }
            }
        });
    }
    result
}

fn append_world_vector(scope: &mut RoadObservation, vector: WorldVectorCopy) {
    scope.world_records_total = scope.world_records_total.saturating_add(vector.total);
    scope.world_records_truncated |= vector.truncated;
    let available =
        MAX_WORLD_POSITION_RECORDS.saturating_sub(usize::from(scope.world_records_copied));
    let count = usize::from(vector.copied).min(available);
    let output_start = usize::from(scope.world_records_copied) * 6;
    let input_count = count * 6;
    scope.world_points[output_start..output_start + input_count]
        .copy_from_slice(&vector.words[..input_count]);
    scope.world_records_copied = scope
        .world_records_copied
        .saturating_add(u8::try_from(count).unwrap_or(u8::MAX));
    scope.world_records_truncated |= usize::from(vector.copied) > count;
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn note_road_refused(generation: u64) {
    if generation == 0 || crate::timeline::current_generation() != generation {
        return;
    }
    if let Ok(mut samples) = ROAD_SAMPLES.try_lock() {
        if crate::timeline::current_generation() != generation {
            return;
        }
        samples.ensure_generation(generation);
        samples.note_refused();
    } else {
        SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn record_road_observation(
    observation: RoadObservation,
    native_result: usize,
    finished_nanos: u64,
) {
    let native_time = native_time_label(
        observation.native_time,
        observation.meta,
        observation.helper_engine,
    );
    let generation = observation.meta.capture_generation;
    let current_generation = crate::timeline::current_generation();
    if generation == 0 || generation != current_generation {
        return;
    }
    let body_matches = road_scope_observable(observation);
    if generation == 0
        || generation != current_generation
        || !body_matches
        || observation.invalid
        || observation.owner == 0
        || !allowlist_ready(generation)
        || allowlisted_entity_index(generation, observation.owner).is_none()
        || observation.helper_calls != 1
        || observation.finish_calls != 1
    {
        if let Ok(mut samples) = ROAD_SAMPLES.try_lock() {
            if crate::timeline::current_generation() == generation {
                samples.ensure_generation(generation);
                samples.note_refused();
            }
        } else {
            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    record_road_history_sample(observation, native_time, finished_nanos);
    let moving = observation.path_valid_before != 0
        && observation.path_current_before != observation.path_previous_before;
    let key = VehicleKey {
        owner: observation.owner,
        child: observation.child,
        component: observation.fresh_instance.entity_component_index,
        slot: observation.fresh_instance.slot,
        model_id: observation.model_id,
    };
    let decision = match ROAD_SAMPLES.try_lock() {
        Ok(mut samples) => {
            let Some(decision) = decide_road_sample(
                &mut samples,
                generation,
                crate::timeline::current_generation(),
                key,
                observation.fresh_instance.dispatch_id,
                moving,
            ) else {
                return;
            };
            if decision == VehicleSampleDecision::Skip {
                samples.note_skip();
            }
            if decision != VehicleSampleDecision::Skip && !samples.note_sample() {
                VehicleSampleDecision::Skip
            } else {
                decision
            }
        }
        Err(_) => {
            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if decision == VehicleSampleDecision::Skip {
        return;
    }
    let (started_nanos, duration) = road_call_span(observation.started_nanos, finished_nanos);
    let _ = crate::timeline::record_with(
        crate::timeline::Kind::RoadVehicleSample,
        observation.fresh_instance.dispatch_id,
        || {
            let mut fields = render_fields(
                observation.meta,
                observation.context,
                observation.fresh_instance.dispatch_id,
            );
            fields.this = observation.transformator;
            fields.data = observation.context;
            fields.arg_a = observation.model_input;
            fields.arg_b = observation.output;
            fields.arg_c = native_result;
            fields.elapsed_nanos = duration;
            fields.render_invalid = false;
            fields.render_vehicle_owner = observation.owner;
            fields.render_vehicle_child = observation.child;
            fields.render_vehicle_component_index =
                observation.fresh_instance.entity_component_index;
            fields.render_vehicle_slot = observation.fresh_instance.slot;
            fields.render_vehicle_model_id = key.model_id;
            fields.render_vehicle_get_instance = observation.model_instance;
            fields.render_vehicle_started_nanos = started_nanos;
            fields.render_vehicle_duration_nanos = duration;
            fields.render_vehicle_alpha_bits = observation.alpha_bits;
            fields.render_vehicle_path_valid_before = observation.path_valid_before;
            fields.render_vehicle_path_valid_after = observation.path_valid_after;
            fields.render_vehicle_path_current_before = observation.path_current_before;
            fields.render_vehicle_path_previous_before = observation.path_previous_before;
            fields.render_vehicle_path_current_after = observation.path_current_after;
            fields.render_vehicle_path_previous_after = observation.path_previous_after;
            fields.render_vehicle_path_result_pointer = observation.path_result_pointer;
            fields.render_vehicle_path_result_word0 = observation.path_result_word0;
            fields.render_vehicle_path_result_word1 = observation.path_result_word1;
            fields.render_vehicle_helper_engine = observation.helper_engine;
            fields.render_vehicle_helper_out = observation.helper_out;
            fields.render_vehicle_helper_type_index = observation.helper_type_index;
            fields.render_vehicle_helper_network = observation.helper_network;
            fields.render_vehicle_helper_move_path = observation.helper_move_path;
            fields.render_vehicle_helper_alpha_bits = observation.helper_alpha_bits;
            fields.render_vehicle_helper_calls = observation.helper_calls;
            fields.render_vehicle_finish_definition = observation.finish_definition;
            fields.render_vehicle_finish_output = observation.finish_output;
            fields.render_vehicle_finish_alpha_bits = observation.finish_alpha_bits;
            fields.render_vehicle_finish_return = observation.finish_return;
            fields.render_vehicle_finish_calls = observation.finish_calls;
            fields.render_vehicle_world_records_total = observation.world_records_total;
            fields.render_vehicle_world_records_copied = observation.world_records_copied;
            fields.render_vehicle_world_records_truncated = observation.world_records_truncated;
            fields.render_vehicle_world_points = observation.world_points;
            fields.render_vehicle_invalid_reason = observation.invalid_reason;
            fields.native_time_before = native_time.previous.unwrap_or(0);
            fields.native_time_after = native_time.current.unwrap_or(0);
            fields.native_time_delta = native_time
                .current
                .zip(native_time.previous)
                .map(|(current, previous)| current.saturating_sub(previous))
                .unwrap_or(0);
            fields.native_time_interp_return = native_time.interp_return;
            fields.native_time_alpha_bits = native_time.alpha_bits;
            fields.native_time_prior_valid = native_time.previous_valid;
            fields.native_time_current_calls = native_time.current_calls;
            fields.native_time_previous_calls = native_time.previous_calls;
            fields.native_time_rejection = native_time.rejection;
            fields.native_time_label_valid = native_time.label_valid;
            fields.native_time_update_before = native_time.previous_update_count;
            fields.native_time_update_after = native_time.current_update_count;
            fields.native_time_selected_updates = native_time
                .current_step
                .saturating_sub(native_time.previous_step)
                .try_into()
                .unwrap_or(u32::MAX);
            fields.native_time_previous_step = native_time.previous_step;
            fields.native_time_current_step = native_time.current_step;
            fields.native_time_world_epoch = native_time.world_epoch;
            fields.native_time_sample_nanos = native_time.sample_nanos;
            if let Some(key) = native_time.current_key {
                fields.native_time_current_engine = key.engine;
                fields.native_time_current_entity_ptr = key.entity_ptr;
                fields.native_time_current_entity_id = key.entity_id;
            }
            if let Some(key) = native_time.previous_key {
                fields.native_time_previous_engine = key.engine;
                fields.native_time_previous_entity_ptr = key.entity_ptr;
                fields.native_time_previous_entity_id = key.entity_id;
            }
            fields.native_time_helper_engine = observation.helper_engine;
            fields.native_time_writer_version_start = native_time.writer_start.version;
            fields.native_time_writer_version_end = native_time.writer_end.version;
            fields.native_time_writer_active_start = native_time.writer_start.active;
            fields.native_time_writer_active_end = native_time.writer_end.active;
            fields
        },
    );
}

fn native_time_step_position(observation: NativeTimeObservation) -> Option<f64> {
    if !observation.label_valid || observation.current_step < observation.previous_step {
        return None;
    }
    let previous = observation.previous?;
    let current = observation.current?;
    let delta = current.checked_sub(previous)?;
    if delta == 0 {
        return (observation.previous_step == observation.current_step
            && observation.interp_return == current)
            .then_some(current_step_as_f64(observation.current_step));
    }
    if delta < 0 {
        return None;
    }
    let elapsed = observation.interp_return.checked_sub(previous)?;
    let fraction = elapsed as f64 / delta as f64;
    if !(0.0..=1.0).contains(&fraction) {
        return None;
    }
    let steps = observation.current_step - observation.previous_step;
    let position = observation.previous_step as f64 + steps as f64 * fraction;
    position.is_finite().then_some(position)
}

fn current_step_as_f64(step: u64) -> f64 {
    step as f64
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn record_road_history_sample(
    observation: RoadObservation,
    native_time: NativeTimeObservation,
    finished_nanos: u64,
) {
    if !ROAD_HISTORY_CAPTURE_ACTIVE.load(Ordering::Acquire)
        || observation.invalid
        || observation.history_family_revision == 0
        || observation.owner == 0
        || observation.world_records_total == 0
        || observation.world_records_total > MAX_WORLD_POSITION_RECORDS as u16
        || usize::from(observation.world_records_copied)
            != usize::from(observation.world_records_total)
        || observation.world_records_truncated
        || !road_scope_observable(observation)
        || !crate::road_history::family_matches(
            observation.owner,
            observation.child,
            observation.history_family_revision,
        )
    {
        return;
    }
    let Some(step_position) = native_time_step_position(native_time) else {
        return;
    };
    let writer = observation.vf3_writer_start;
    if !writer.known
        || !writer.stable
        || writer.active != 0
        || observation.vf3_writer_end != writer
        || observation.native_writer_start != writer
        || observation.native_writer_end != writer
    {
        return;
    }
    let mut records = [[0_u32; 6]; crate::road_history::MAX_RECORDS];
    let count = usize::from(observation.world_records_copied);
    for (index, record) in records.iter_mut().take(count).enumerate() {
        let start = index * 6;
        record.copy_from_slice(&observation.world_points[start..start + 6]);
    }
    let sample = crate::road_history::PoseSample {
        capture_generation: observation.meta.capture_generation,
        world_epoch: native_time.world_epoch,
        incarnation: observation.history_family_revision,
        invalidation_generation: observation.history_invalidation_generation,
        key: crate::road_history::FamilyKey {
            owner: observation.owner,
            child: observation.child,
            component_index: observation.fresh_instance.entity_component_index,
            slot: observation.fresh_instance.slot,
            model_id: observation.model_id,
            definition: observation.finish_definition,
        },
        step_position,
        sample_nanos: finished_nanos,
        count: observation.world_records_copied,
        records,
    };
    let _ = crate::road_history::push_with_commit(sample, |result| {
        crate::timeline::record_with(
            crate::timeline::Kind::RoadHistorySample,
            observation.fresh_instance.dispatch_id,
            || {
                let mut fields = crate::timeline::Fields {
                    this: observation.owner as usize,
                    data: observation.child as usize,
                    result_byte: match result {
                        crate::road_history::InsertResult::Inserted => 1,
                        crate::road_history::InsertResult::Duplicate => 2,
                        crate::road_history::InsertResult::Reprimed => 3,
                        crate::road_history::InsertResult::Conflict => 4,
                        crate::road_history::InsertResult::Invalid => 5,
                    },
                    render_epoch: observation.meta.epoch,
                    render_game_state: observation.meta.game_state,
                    render_engine: observation.meta.engine,
                    render_vehicle_owner: observation.owner,
                    render_vehicle_child: observation.child,
                    render_vehicle_component_index: observation
                        .fresh_instance
                        .entity_component_index,
                    render_vehicle_slot: observation.fresh_instance.slot,
                    render_vehicle_model_id: observation.model_id,
                    render_vehicle_finish_definition: observation.finish_definition,
                    render_vehicle_alpha_bits: observation.alpha_bits,
                    render_vehicle_world_records_total: observation.world_records_total,
                    render_vehicle_world_records_copied: observation.world_records_copied,
                    render_vehicle_world_points: observation.world_points,
                    road_history_target_step_bits: step_position.to_bits(),
                    road_history_incarnation: observation.history_family_revision,
                    road_history_record_count: observation.world_records_copied,
                    native_time_before: native_time.previous.unwrap_or(0),
                    native_time_after: native_time.current.unwrap_or(0),
                    native_time_interp_return: native_time.interp_return,
                    native_time_world_epoch: native_time.world_epoch,
                    native_time_label_valid: native_time.label_valid,
                    native_time_previous_step: native_time.previous_step,
                    native_time_current_step: native_time.current_step,
                    ..crate::timeline::Fields::default()
                };
                fields.native_engine = observation.meta.engine;
                fields
            },
        )
        .is_some()
    });
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[derive(Debug, Clone, Copy)]
struct Pose {
    model_id: u32,
    current: [u32; 16],
    previous: [u32; 16],
    valid: u8,
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn read_pose(address: usize, model_id: u32) -> Option<Pose> {
    Some(Pose {
        model_id,
        current: crate::image::guarded::read(address.checked_add(4)?)?,
        previous: crate::image::guarded::read(address.checked_add(0x44)?)?,
        valid: crate::image::guarded::read(address.checked_add(0x84)?)?,
    })
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn record_pose(
    context: BodyContext,
    entity: u32,
    entity_component_index: u32,
    instance_slot: i32,
    pose: Pose,
) {
    let key = RenderKey {
        entity,
        entity_component_index,
        slot: instance_slot,
        model_id: pose.model_id,
    };
    let current_hash = pose_hash(&pose.current);
    let previous_hash = pose_hash(&pose.previous);
    let mut samples = match SAMPLES.try_lock() {
        Ok(samples) => samples,
        Err(_) => {
            SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if crate::timeline::current_generation() != context.meta.capture_generation
        || !registry().remains_observable(
            context.registry_index,
            context.key,
            context.meta.capture_generation,
        )
    {
        SAMPLE_SKIPS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    sync_sample_generation(&mut samples, context.meta.capture_generation);
    let should_record = samples.record_pose(
        context.meta.capture_generation,
        key,
        context.dispatch_id,
        pose.valid != 0 && pose.current != pose.previous,
    );
    if !should_record {
        return;
    }
    drop(samples);
    crate::timeline::record_with(
        crate::timeline::Kind::RenderPoseSample,
        context.dispatch_id,
        || {
            let mut fields = render_fields(context.meta, context.key, context.dispatch_id);
            fields.render_entity = entity;
            fields.render_entity_component_index = entity_component_index;
            fields.render_instance_slot = key.slot;
            fields.render_model_id = pose.model_id;
            fields.render_current_pose_hash = current_hash;
            fields.render_previous_pose_hash = previous_hash;
            fields.render_current_pose_bits = pose.current;
            fields.render_previous_pose_bits = pose.previous;
            fields.render_instance_valid = pose.valid;
            fields
        },
    );
}

fn pose_hash(bits: &[u32; 16]) -> u64 {
    bits.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, word| {
        word.to_le_bytes().iter().fold(hash, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
        })
    })
}

fn render_fields(meta: RenderMeta, context: usize, dispatch_id: u64) -> crate::timeline::Fields {
    let (allowlist_status, allowlist_count, allowlist_valid) =
        capture_allowlist_status(meta.capture_generation);
    let discovery_inspections = DISCOVERY_INSPECTIONS.load(Ordering::Acquire);
    let selected_pose_inspections = SELECTED_POSE_INSPECTIONS.load(Ordering::Acquire);
    let road_sample_stats = ROAD_SAMPLES
        .try_lock()
        .ok()
        .filter(|samples| samples.capture_generation == meta.capture_generation)
        .map(|samples| (samples.samples, samples.skips, samples.refused))
        .unwrap_or((0, 0, 0));
    crate::timeline::Fields {
        render_epoch: meta.epoch,
        render_context: context,
        render_game_state: meta.game_state,
        render_engine: meta.engine,
        render_type_index: meta.type_index,
        arg_a: dispatch_id as usize,
        render_sample_skips: SAMPLE_SKIPS.load(Ordering::Acquire),
        render_context_misses: CONTEXT_MISSES.load(Ordering::Acquire),
        render_allowlist_status: allowlist_status as u8,
        render_allowlist_valid: allowlist_valid,
        render_allowlist_count: allowlist_count as u16,
        render_discovery_inspections: discovery_inspections,
        render_selected_pose_inspections: selected_pose_inspections,
        render_pose_inspections: discovery_inspections.saturating_add(selected_pose_inspections),
        render_vehicle_samples_total: u64::from(road_sample_stats.0),
        render_vehicle_sample_skips: road_sample_stats.1,
        render_vehicle_refused_total: road_sample_stats.2,
        this: meta.renderer,
        ..crate::timeline::Fields::default()
    }
}

fn register_error_code(error: Option<RegisterError>) -> u8 {
    match error {
        None => 0,
        Some(RegisterError::EmptyKey) => 1,
        Some(RegisterError::DuplicateKey) => 2,
        Some(RegisterError::Full) => 3,
    }
}

/// Installs renderer observers only with an explicit renderer or road-offset
/// opt-in, the timeline watcher, and the reviewed 40420 sites.
#[cfg(all(windows, target_arch = "x86_64"))]
pub(super) unsafe fn install(
    base: usize,
    profile: &tpf3mp_hookcore::profile::Profile,
    resolved: &tpf3mp_hookcore::profile::ResolvedProfile,
) -> Result<(), String> {
    let render_probe_option = std::env::var(ENV).ok();
    let road_offset_option = std::env::var(ROAD_OFFSET_ENV).ok();
    let native_time_option = std::env::var(NATIVE_TIME_ENV).ok();
    let road_history_option = std::env::var(ROAD_HISTORY_ENV).ok();
    let timeline_option = std::env::var(crate::timeline::ENV).ok();
    let road_offset_requested = road_offset_requested_from(road_offset_option.as_deref());
    let native_time_requested = native_time_requested_from(native_time_option.as_deref());
    let road_history_requested = requested_from(road_history_option.as_deref());
    if road_history_requested
        && !road_history_requested_with_features(
            render_probe_option.as_deref(),
            native_time_option.as_deref(),
            road_history_option.as_deref(),
            timeline_option.as_deref(),
        )
    {
        return Err(format!(
            "{ROAD_HISTORY_ENV}=1 requires {ENV}=1, {NATIVE_TIME_ENV}=1, and {}=watch",
            crate::timeline::ENV
        ));
    }
    if road_history_requested && road_offset_requested {
        return Err(format!(
            "{ROAD_HISTORY_ENV}=1 and {ROAD_OFFSET_ENV}=1 cannot be combined"
        ));
    }
    if road_history_requested && !crate::pacing::installed() {
        return Err(format!(
            "{ROAD_HISTORY_ENV}=1 requires {smooth}=1",
            smooth = crate::pacing::ENV
        ));
    }
    if native_time_requested
        && !requested_with_timeline(render_probe_option.as_deref(), timeline_option.as_deref())
    {
        return Err(format!(
            "{NATIVE_TIME_ENV}=1 requires {ENV}=1 and {}=watch",
            crate::timeline::ENV
        ));
    }
    if !requested_with_native_time(
        render_probe_option.as_deref(),
        road_offset_option.as_deref(),
        native_time_option.as_deref(),
        timeline_option.as_deref(),
    ) && !road_history_requested
    {
        return Ok(());
    }
    if !crate::timeline::watch_requested(timeline_option.as_deref()) {
        return Err(format!(
            "{ENV}=1 or {ROAD_OFFSET_ENV}=1 requires {}=watch",
            crate::timeline::ENV
        ));
    }
    if profile.build.sha256 != "74861ac43b041aebc5179154345b3cf1ec83154c8e6cc58e0d9e02ff5fa602e4" {
        return Err("the renderer probe is pinned to the exact Steam 40420 build".into());
    }
    let address = |name: &str| -> Result<u64, String> {
        resolved
            .get(name)
            .map(|target| target.address)
            .ok_or_else(|| format!("the profile did not resolve {name}"))
    };
    const NEW_UPDATE_RVA: u64 = 0x2ffc60;
    const MODEL_LIST_INDEX_CALL_RVA: u64 = 0x3003b2;
    const MODEL_LIST_INDEX_FUNCTION_RVA: u64 = 0xa4c80;
    const DISPATCH_CALL_RVA: u64 = 0x301b3c;
    const DISPATCH_FUNCTION_RVA: u64 = 0x2ee900;
    const BODY_RVA: u64 = 0x2f5d90;
    const TRANSFORMATOR_SLOT_CALL_RVA: u64 = 0x2f640d;
    const GET_INSTANCE_CALL_RVA: u64 = 0x2f617e;
    const GET_INSTANCE_FUNCTION_RVA: u64 = 0x2fdb80;
    const ROAD_VF3_RVA: u64 = 0xc8ff70;
    const ROAD_PATH_HELPER_CALL_RVA: u64 = 0xc90391;
    const ROAD_PATH_HELPER_FUNCTION_RVA: u64 = 0x25c38e0;
    const ROAD_USER_TRANSFORMS_CALL_RVA: u64 = 0xc91371;
    const ROAD_USER_TRANSFORMS_FUNCTION_RVA: u64 = 0xcb8690;
    const NATIVE_TIME_INTERPOLATION_CALL_RVA: u64 = 0xc9040f;
    const NATIVE_TIME_INTERPOLATION_FUNCTION_RVA: u64 = 0xc456d0;
    const NATIVE_TIME_CURRENT_CALL_RVA: u64 = 0xc456e7;
    const NATIVE_TIME_CURRENT_FUNCTION_RVA: u64 = 0x2a9650;
    const NATIVE_TIME_PREVIOUS_CALL_RVA: u64 = 0xc456f7;
    const NATIVE_TIME_PREVIOUS_FUNCTION_RVA: u64 = 0x2a9620;
    const GAME_STATE_REPLICATE_RVA: u64 = 0x255e60;
    const ENGINE_REMOVE_ENTITY_RVA: u64 = 0x2bbc700;
    for (name, actual, expected) in [
        (
            "CRenderer::NewUpdate",
            address("CRenderer::NewUpdate")?,
            NEW_UPDATE_RVA,
        ),
        (
            "CRenderer::NewUpdate/ModelInstanceList index call",
            address("CRenderer::NewUpdate/ModelInstanceList index call")?,
            MODEL_LIST_INDEX_CALL_RVA,
        ),
        (
            "CRenderer::NewUpdate/Lambda7 dispatch call",
            address("CRenderer::NewUpdate/Lambda7 dispatch call")?,
            DISPATCH_CALL_RVA,
        ),
        (
            "CRenderer::NewUpdate/ModelInstanceList index function",
            address("CRenderer::NewUpdate/ModelInstanceList index function")?,
            MODEL_LIST_INDEX_FUNCTION_RVA,
        ),
        (
            "RenderModelInstance::_Do_call",
            address("RenderModelInstance::_Do_call")?,
            BODY_RVA,
        ),
        (
            "RenderModelInstance::transformator slot call",
            address("RenderModelInstance::transformator slot call")?,
            TRANSFORMATOR_SLOT_CALL_RVA,
        ),
        (
            "RenderModelInstance/GetInstance call",
            address("RenderModelInstance/GetInstance call")?,
            GET_INSTANCE_CALL_RVA,
        ),
        (
            "ModelInstanceList::GetInstance",
            address("ModelInstanceList::GetInstance")?,
            GET_INSTANCE_FUNCTION_RVA,
        ),
        (
            "RoadVehicleTransformator::vf3",
            address("RoadVehicleTransformator::vf3")?,
            ROAD_VF3_RVA,
        ),
        (
            "RoadVehicleTransformator::path helper call",
            address("RoadVehicleTransformator::path helper call")?,
            ROAD_PATH_HELPER_CALL_RVA,
        ),
        (
            "RoadVehicleTransformator::user transforms call",
            address("RoadVehicleTransformator::user transforms call")?,
            ROAD_USER_TRANSFORMS_CALL_RVA,
        ),
        (
            "RoadVehicleTransformator::vf3/CGameTime interpolation call",
            address("RoadVehicleTransformator::vf3/CGameTime interpolation call")?,
            NATIVE_TIME_INTERPOLATION_CALL_RVA,
        ),
        (
            "CGameTime interpolation/current getter call",
            address("CGameTime interpolation/current getter call")?,
            NATIVE_TIME_CURRENT_CALL_RVA,
        ),
        (
            "CGameTime interpolation/previous getter call",
            address("CGameTime interpolation/previous getter call")?,
            NATIVE_TIME_PREVIOUS_CALL_RVA,
        ),
    ] {
        if actual != expected {
            return Err(format!(
                "{name} resolved to {actual:#x}, expected verified 40420 address {expected:#x}"
            ));
        }
    }
    if road_history_requested {
        for (name, actual, expected) in [
            (
                "GameState::Replicate",
                address("GameState::Replicate")?,
                GAME_STATE_REPLICATE_RVA,
            ),
            (
                "ecs::Engine::RemoveEntity",
                address("ecs::Engine::RemoveEntity")?,
                ENGINE_REMOVE_ENTITY_RVA,
            ),
        ] {
            if actual != expected {
                return Err(format!(
                    "{name} resolved to {actual:#x}, expected verified 40420 address {expected:#x}"
                ));
            }
        }
    }
    let registry = REGISTRY.get_or_init(DispatchRegistry::new);
    let _ = registry;
    let _ = &SAMPLES;

    let native_time_interpolation = if native_time_requested {
        Some(
            unsafe {
                tpf3mp_hookcore::detour::CallRedirect::install(
                    (base + NATIVE_TIME_INTERPOLATION_CALL_RVA as usize) as *mut u8,
                    base + NATIVE_TIME_INTERPOLATION_FUNCTION_RVA as usize,
                    native_time_interpolation_detour as *const u8,
                )
            }
            .map_err(|error| format!("redirecting road vf3 CGameTime interpolation: {error:?}"))?,
        )
    } else {
        None
    };
    let native_time_current = if native_time_requested {
        Some(
            unsafe {
                tpf3mp_hookcore::detour::CallRedirect::install(
                    (base + NATIVE_TIME_CURRENT_CALL_RVA as usize) as *mut u8,
                    base + NATIVE_TIME_CURRENT_FUNCTION_RVA as usize,
                    native_time_current_observer as *const u8,
                )
            }
            .map_err(|error| format!("redirecting CGameTime current-time getter: {error:?}"))?,
        )
    } else {
        None
    };
    let native_time_previous = if native_time_requested {
        Some(
            unsafe {
                tpf3mp_hookcore::detour::CallRedirect::install(
                    (base + NATIVE_TIME_PREVIOUS_CALL_RVA as usize) as *mut u8,
                    base + NATIVE_TIME_PREVIOUS_FUNCTION_RVA as usize,
                    native_time_previous_observer as *const u8,
                )
            }
            .map_err(|error| format!("redirecting CGameTime previous-time getter: {error:?}"))?,
        )
    } else {
        None
    };

    let model_list_index = unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            (base + MODEL_LIST_INDEX_CALL_RVA as usize) as *mut u8,
            base + MODEL_LIST_INDEX_FUNCTION_RVA as usize,
            model_list_index_adapter as *const u8,
        )
    }
    .map_err(|error| format!("redirecting ModelInstanceList index lookup: {error:?}"))?;
    let dispatch = unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            (base + DISPATCH_CALL_RVA as usize) as *mut u8,
            base + DISPATCH_FUNCTION_RVA as usize,
            dispatch_detour as *const u8,
        )
    }
    .map_err(|error| format!("redirecting Lambda7 dispatch: {error:?}"))?;
    let get_instance = unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            (base + GET_INSTANCE_CALL_RVA as usize) as *mut u8,
            base + GET_INSTANCE_FUNCTION_RVA as usize,
            get_instance_adapter as *const u8,
        )
    }
    .map_err(|error| format!("redirecting GetInstance: {error:?}"))?;
    let road_path_helper = unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            (base + ROAD_PATH_HELPER_CALL_RVA as usize) as *mut u8,
            base + ROAD_PATH_HELPER_FUNCTION_RVA as usize,
            road_path_helper_adapter as *const u8,
        )
    }
    .map_err(|error| format!("redirecting road-vehicle path helper: {error:?}"))?;
    let road_user_transforms = unsafe {
        tpf3mp_hookcore::detour::CallRedirect::install(
            (base + ROAD_USER_TRANSFORMS_CALL_RVA as usize) as *mut u8,
            base + ROAD_USER_TRANSFORMS_FUNCTION_RVA as usize,
            road_user_transforms_observer as *const u8,
        )
    }
    .map_err(|error| format!("redirecting road-vehicle user transforms: {error:?}"))?;
    let body = unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(
            (base + BODY_RVA as usize) as *mut u8,
            render_body_detour as *const u8,
        )
    }
    .map_err(|error| format!("detouring RenderModelInstance body: {error:?}"))?;
    let new_update = unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(
            (base + NEW_UPDATE_RVA as usize) as *mut u8,
            new_update_detour as *const u8,
        )
    }
    .map_err(|error| format!("detouring CRenderer::NewUpdate: {error:?}"))?;
    let road_vf3 = unsafe {
        tpf3mp_hookcore::detour::InlineDetour::install(
            (base + ROAD_VF3_RVA as usize) as *mut u8,
            road_vf3_detour as *const u8,
        )
    }
    .map_err(|error| format!("detouring road-vehicle vf3: {error:?}"))?;
    let game_state_replicate = if road_history_requested {
        Some(
            unsafe {
                tpf3mp_hookcore::detour::InlineDetour::install(
                    (base + GAME_STATE_REPLICATE_RVA as usize) as *mut u8,
                    game_state_replicate_detour as *const u8,
                )
            }
            .map_err(|error| format!("detouring GameState::Replicate: {error:?}"))?,
        )
    } else {
        None
    };
    let engine_remove_entity = if road_history_requested {
        Some(
            unsafe {
                tpf3mp_hookcore::detour::InlineDetour::install(
                    (base + ENGINE_REMOVE_ENTITY_RVA as usize) as *mut u8,
                    engine_remove_entity_detour as *const u8,
                )
            }
            .map_err(|error| format!("detouring ecs::Engine::RemoveEntity: {error:?}"))?,
        )
    } else {
        None
    };

    MODEL_LIST_INDEX_ORIGINAL.store(
        base + MODEL_LIST_INDEX_FUNCTION_RVA as usize,
        Ordering::Release,
    );
    DISPATCH_ORIGINAL.store(base + DISPATCH_FUNCTION_RVA as usize, Ordering::Release);
    GET_INSTANCE_ORIGINAL.store(base + GET_INSTANCE_FUNCTION_RVA as usize, Ordering::Release);
    ROAD_PATH_HELPER_ORIGINAL.store(
        base + ROAD_PATH_HELPER_FUNCTION_RVA as usize,
        Ordering::Release,
    );
    ROAD_USER_TRANSFORMS_ORIGINAL.store(
        base + ROAD_USER_TRANSFORMS_FUNCTION_RVA as usize,
        Ordering::Release,
    );
    ROAD_OFFSET_ENABLED.store(road_offset_requested, Ordering::Release);
    ROAD_HISTORY_ENABLED.store(road_history_requested, Ordering::Release);
    if let Some(detour) = &game_state_replicate {
        GAME_STATE_REPLICATE_ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
    }
    if let Some(detour) = &engine_remove_entity {
        ENGINE_REMOVE_ENTITY_ORIGINAL.store(detour.trampoline() as usize, Ordering::Release);
    }
    BODY_ORIGINAL.store(body.trampoline() as usize, Ordering::Release);
    NEW_UPDATE_ORIGINAL.store(new_update.trampoline() as usize, Ordering::Release);
    ROAD_VF3_ORIGINAL.store(road_vf3.trampoline() as usize, Ordering::Release);
    if native_time_requested {
        NATIVE_TIME_FUNCTION_ORIGINAL.store(
            base + NATIVE_TIME_INTERPOLATION_FUNCTION_RVA as usize,
            Ordering::Release,
        );
        NATIVE_TIME_CURRENT_GETTER.store(
            base + NATIVE_TIME_CURRENT_FUNCTION_RVA as usize,
            Ordering::Release,
        );
        NATIVE_TIME_PREVIOUS_GETTER.store(
            base + NATIVE_TIME_PREVIOUS_FUNCTION_RVA as usize,
            Ordering::Release,
        );
        NATIVE_TIME_ENABLED.store(true, Ordering::Release);
        crate::log::line(
            "native GameTime labels enabled: observe-only, five-second timeline capture required",
        );
    }
    if road_history_requested {
        crate::log::line(
            "road-pose history mutation enabled only for an explicitly armed five-second capture with a one-owner allowlist; original GameState replication and entity-removal hooks are active",
        );
    }
    if let Some(redirect) = native_time_interpolation {
        std::mem::forget(redirect);
    }
    if let Some(redirect) = native_time_current {
        std::mem::forget(redirect);
    }
    if let Some(redirect) = native_time_previous {
        std::mem::forget(redirect);
    }
    std::mem::forget(model_list_index);
    std::mem::forget(dispatch);
    std::mem::forget(get_instance);
    std::mem::forget(road_path_helper);
    std::mem::forget(road_user_transforms);
    std::mem::forget(body);
    std::mem::forget(new_update);
    std::mem::forget(road_vf3);
    if let Some(detour) = game_state_replicate {
        std::mem::forget(detour);
    }
    if let Some(detour) = engine_remove_entity {
        std::mem::forget(detour);
    }
    Ok(())
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub(super) unsafe fn install(
    _base: usize,
    _profile: &tpf3mp_hookcore::profile::Profile,
    _resolved: &tpf3mp_hookcore::profile::ResolvedProfile,
) -> Result<(), String> {
    Err("the renderer probe is available only on Windows x64".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(all(windows, target_arch = "x86_64"))]
    static ROAD_FORWARD_TEST_LOCK: Mutex<()> = Mutex::new(());
    #[cfg(all(windows, target_arch = "x86_64"))]
    static ROAD_FORWARD_CAPTURE: Mutex<[usize; 6]> = Mutex::new([0; 6]);
    #[cfg(all(windows, target_arch = "x86_64"))]
    static NATIVE_TIME_FORWARD_TEST_LOCK: Mutex<()> = Mutex::new(());
    #[cfg(all(windows, target_arch = "x86_64"))]
    static NATIVE_TIME_INTERPOLATION_CALLS: AtomicUsize = AtomicUsize::new(0);
    #[cfg(all(windows, target_arch = "x86_64"))]
    static NATIVE_TIME_CURRENT_CALLS: AtomicUsize = AtomicUsize::new(0);
    #[cfg(all(windows, target_arch = "x86_64"))]
    static NATIVE_TIME_PREVIOUS_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[cfg(all(windows, target_arch = "x86_64"))]
    struct RestoreOriginal {
        slot: &'static AtomicUsize,
        value: usize,
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    impl Drop for RestoreOriginal {
        fn drop(&mut self) {
            self.slot.store(self.value, Ordering::Release);
        }
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_road_vf3(
        transformator: usize,
        context: usize,
        model_input: usize,
        output: usize,
    ) -> usize {
        *ROAD_FORWARD_CAPTURE.lock().unwrap() = [transformator, context, model_input, output, 0, 0];
        0xaaaa
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_native_interpolation(clock: usize, alpha: f32) -> i64 {
        NATIVE_TIME_INTERPOLATION_CALLS.fetch_add(1, Ordering::Relaxed);
        i64::from((clock as u32) ^ alpha.to_bits())
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_native_current(clock: usize) -> i64 {
        NATIVE_TIME_CURRENT_CALLS.fetch_add(1, Ordering::Relaxed);
        clock as i64 + 0x7654
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_native_previous(clock: usize, output: usize) -> usize {
        NATIVE_TIME_PREVIOUS_CALLS.fetch_add(1, Ordering::Relaxed);
        let words = 0x1122_3344_5566_7788_i64.to_le_bytes();
        // SAFETY: the caller provides its own sixteen-byte scratch output.
        unsafe {
            std::ptr::copy_nonoverlapping(words.as_ptr(), output as *mut u8, words.len());
            (output as *mut u8).add(8).write(1);
        }
        let _ = clock;
        0xcafe
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn unscoped_native_time_observers_forward_once_and_keep_native_returns() {
        let _guard = NATIVE_TIME_FORWARD_TEST_LOCK.lock().unwrap();
        let _interpolation_restore = RestoreOriginal {
            slot: &NATIVE_TIME_FUNCTION_ORIGINAL,
            value: NATIVE_TIME_FUNCTION_ORIGINAL.swap(
                capture_native_interpolation as *const () as usize,
                Ordering::AcqRel,
            ),
        };
        let _current_restore = RestoreOriginal {
            slot: &NATIVE_TIME_CURRENT_GETTER,
            value: NATIVE_TIME_CURRENT_GETTER.swap(
                capture_native_current as *const () as usize,
                Ordering::AcqRel,
            ),
        };
        let _previous_restore = RestoreOriginal {
            slot: &NATIVE_TIME_PREVIOUS_GETTER,
            value: NATIVE_TIME_PREVIOUS_GETTER.swap(
                capture_native_previous as *const () as usize,
                Ordering::AcqRel,
            ),
        };
        let old_enabled = NATIVE_TIME_ENABLED.swap(false, Ordering::AcqRel);
        let old_scope = NATIVE_TIME_SCOPE.with(|cell| {
            cell.replace(NativeTimeReadScope {
                active: false,
                nested: true,
                ..NativeTimeReadScope::EMPTY
            })
        });
        NATIVE_TIME_INTERPOLATION_CALLS.store(0, Ordering::Relaxed);
        NATIVE_TIME_CURRENT_CALLS.store(0, Ordering::Relaxed);
        NATIVE_TIME_PREVIOUS_CALLS.store(0, Ordering::Relaxed);

        // SAFETY: function pointers above have the callsite observer ABIs and
        // use only local scalar/scratch arguments in this test.
        let interp = unsafe { native_time_interpolation_detour(0x1234, 0.5) };
        let current = unsafe { native_time_current_observer(0x2345) };
        let mut output = [0_u8; 16];
        let previous =
            unsafe { native_time_previous_observer(0x3456, output.as_mut_ptr() as usize) };
        NATIVE_TIME_SCOPE.with(|cell| cell.set(old_scope));
        NATIVE_TIME_ENABLED.store(old_enabled, Ordering::Release);

        assert_eq!(interp, i64::from(0x1234_u32 ^ 0.5_f32.to_bits()));
        assert_eq!(current, 0x2345 + 0x7654);
        assert_eq!(previous, 0xcafe);
        assert_eq!(
            i64::from_le_bytes(output[..8].try_into().unwrap()),
            0x1122_3344_5566_7788
        );
        assert_eq!(output[8], 1);
        assert_eq!(NATIVE_TIME_INTERPOLATION_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(NATIVE_TIME_CURRENT_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(NATIVE_TIME_PREVIOUS_CALLS.load(Ordering::Relaxed), 1);
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_road_path_helper(
        out: usize,
        engine: usize,
        type_index: u32,
        network: usize,
        move_path: usize,
        alpha: f32,
    ) -> usize {
        *ROAD_FORWARD_CAPTURE.lock().unwrap() = [
            out,
            engine,
            type_index as usize,
            network,
            move_path,
            alpha.to_bits() as usize,
        ];
        0xbbbb
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    unsafe extern "system" fn capture_road_user_transforms(
        definition: usize,
        world_vector: usize,
        alpha: f32,
        output: usize,
    ) -> usize {
        *ROAD_FORWARD_CAPTURE.lock().unwrap() = [
            definition,
            world_vector,
            alpha.to_bits() as usize,
            output,
            0,
            0,
        ];
        0xcccc
    }

    fn meta(epoch: u64) -> RenderMeta {
        RenderMeta {
            capture_generation: 1,
            epoch,
            renderer: 0x1110,
            game_state: 0x2220,
            engine: 0x3330,
            type_index: 7,
            binding_valid: true,
        }
    }

    #[test]
    fn probe_is_off_unless_exactly_armed() {
        assert!(!requested_from(None));
        assert!(!requested_from(Some("0")));
        assert!(!requested_from(Some("true")));
        assert!(!requested_from(Some("1 ")));
        assert!(requested_from(Some("1")));
        assert!(!requested_with_timeline(Some("1"), None));
        assert!(!requested_with_timeline(Some("1"), Some("true")));
        assert!(requested_with_timeline(Some("1"), Some("watch")));
        assert!(!road_offset_requested_from(None));
        assert!(!road_offset_requested_from(Some("true")));
        assert!(road_offset_requested_from(Some("1")));
        assert!(!requested_with_features(None, Some("1"), None));
        assert!(!requested_with_features(None, Some("1"), Some("true")));
        assert!(requested_with_features(None, Some("1"), Some("watch")));
        assert!(requested_with_features(Some("1"), None, Some("watch")));
        assert!(only_allowlisted_owner(true, 1, 50, 50));
        assert!(!only_allowlisted_owner(false, 1, 50, 50));
        assert!(!only_allowlisted_owner(true, 2, 50, 50));
        assert!(!only_allowlisted_owner(true, 1, 51, 50));
        assert!(!only_allowlisted_owner(true, 1, 0, 0));
    }

    #[test]
    fn registry_correlates_distinct_contexts_and_rejects_overlapping_key() {
        let registry = DispatchRegistry::<2>::new();
        let first = registry.register(0x1000, 1, meta(1), 8, 4).unwrap();
        let second = registry.register(0x2000, 2, meta(2), 6, 3).unwrap();
        assert_eq!(registry.find(0x1000), Some(first));
        assert_eq!(registry.find(0x2000), Some(second));
        assert_eq!(
            registry.register(0x1000, 3, meta(3), 2, 1),
            Err(RegisterError::DuplicateKey)
        );
        assert!(registry.slots[first].invalid.load(Ordering::Acquire));
        assert_eq!(registry.find(0x1000), None);
        assert!(!registry.remains_observable(first, 0x1000, 1));
        let _ = registry.finish(first, 0x1000).unwrap();
        let _ = registry.finish(second, 0x2000).unwrap();
    }

    #[test]
    fn concurrent_registration_never_assigns_one_context_to_two_frames() {
        let registry = std::sync::Arc::new(DispatchRegistry::<4>::new());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = (0..2)
            .map(|epoch| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry.register(0x1234, epoch + 1, meta(epoch + 1), 1, 1)
                })
            })
            .collect();
        barrier.wait();
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == Err(RegisterError::DuplicateKey))
                .count(),
            1
        );
        let index = results
            .iter()
            .find_map(|result| result.as_ref().ok())
            .copied()
            .unwrap();
        assert!(registry.slots[index].invalid.load(Ordering::Acquire));
        assert_eq!(registry.find(0x1234), None);
        let _ = registry.finish(index, 0x1234).unwrap();
    }

    #[test]
    fn registry_is_bounded_and_context_addresses_may_be_reused_after_join() {
        let registry = DispatchRegistry::<1>::new();
        let first = registry.register(0x1000, 1, meta(1), 1, 1).unwrap();
        assert_eq!(
            registry.register(0x2000, 2, meta(2), 1, 1),
            Err(RegisterError::Full)
        );
        let completed = registry.finish(first, 0x1000).unwrap();
        assert_eq!(completed.key, 0x1000);
        let reused = registry.register(0x1000, 3, meta(3), 1, 1).unwrap();
        registry.begin_body(reused, 7);
        registry.end_body(reused);
        let completed = registry.finish(reused, 0x1000).unwrap();
        assert_eq!(completed.started, 1);
        assert_eq!(completed.completed, 1);
        assert_eq!(completed.thread_ids[0], 7);
    }

    #[test]
    fn dispatch_exit_detects_unjoined_body_and_context_misses_are_explicit() {
        let registry = DispatchRegistry::<1>::new();
        assert_eq!(registry.find(0x9999), None);
        let index = registry.register(0x1000, 1, meta(1), 1, 1).unwrap();
        assert!(registry.remains_observable(index, 0x1000, 1));
        registry.mark_invalid(index);
        assert!(!registry.remains_observable(index, 0x1000, 1));
        assert_eq!(registry.find(0x1000), None);
        let _ = registry.finish(index, 0x1000).unwrap();

        let index = registry.register(0x2000, 2, meta(2), 1, 1).unwrap();
        registry.begin_body(index, 10);
        let snapshot = registry.finish(index, 0x2000).unwrap();
        assert!(snapshot.invalid);
        assert_eq!(snapshot.started, 1);
        assert_eq!(snapshot.completed, 0);
    }

    #[test]
    fn selected_moving_key_is_sampled_when_it_collapses_to_an_equal_pair() {
        let mut samples = SampleSet::new();
        let key = RenderKey {
            entity: 1,
            entity_component_index: 3,
            slot: -2,
            model_id: 5,
        };
        assert_eq!(
            samples.begin_inspection(4, 0, 1, 3, -2, 10),
            PoseInspection::Discover
        );
        assert!(samples.record_pose(4, key, 10, true));
        assert_eq!(
            samples.begin_inspection(4, 0, 1, 3, -2, 11),
            PoseInspection::Selected
        );
        assert!(samples.record_pose(4, key, 11, false));
        assert_eq!(
            samples.begin_inspection(4, 0, 1, 3, -2, 11),
            PoseInspection::Skip,
            "one key is sampled at most once per dispatch"
        );
        assert_eq!(
            samples.begin_inspection(5, 0, 1, 3, -2, 12),
            PoseInspection::Discover,
            "a new capture starts discovery afresh"
        );
        assert_eq!(pose_hash(&[0; 16]), pose_hash(&[0; 16]));
        assert_ne!(pose_hash(&[0; 16]), pose_hash(&[1; 16]));
    }

    #[test]
    fn discovery_requires_native_valid_pose_but_selected_invalid_pairs_are_recordable() {
        let mut samples = SampleSet::new();
        let key = RenderKey {
            entity: 88,
            entity_component_index: 6,
            slot: -1,
            model_id: 12,
        };
        assert_eq!(
            samples.begin_inspection(3, 0, 88, 6, -1, 1),
            PoseInspection::Discover
        );
        assert!(
            !samples.record_pose(3, key, 1, false),
            "an invalid previous matrix cannot select a key as moving"
        );
        assert_eq!(
            samples.begin_inspection(3, 0, 88, 6, -1, 2),
            PoseInspection::Discover
        );
        assert!(samples.record_pose(3, key, 2, true));
        assert_eq!(
            samples.begin_inspection(3, 0, 88, 6, -1, 3),
            PoseInspection::Selected
        );
        assert!(
            samples.record_pose(3, key, 3, false),
            "an already selected key still emits an invalid/reset sample"
        );
    }

    #[test]
    fn a_selected_key_is_sampled_once_per_dispatch_through_a_five_second_window() {
        let mut samples = SampleSet::new();
        let key = RenderKey {
            entity: 77,
            entity_component_index: 4,
            slot: -1,
            model_id: 12,
        };
        let mut recorded = 0;
        for dispatch in 1..=300 {
            let inspection = samples.begin_inspection(9, 0, 77, 4, -1, dispatch);
            assert_eq!(
                inspection,
                if dispatch == 1 {
                    PoseInspection::Discover
                } else {
                    PoseInspection::Selected
                }
            );
            assert!(samples.record_pose(9, key, dispatch, dispatch == 1));
            recorded += 1;
            assert_eq!(
                samples.begin_inspection(9, 0, 77, 4, -1, dispatch),
                PoseInspection::Skip
            );
        }
        assert_eq!(recorded, 300);
    }

    #[test]
    fn concurrent_stale_generation_observers_initialize_once_and_preserve_new_keys() {
        let samples = std::sync::Arc::new(std::sync::Mutex::new(SampleSet::new()));
        assert!(samples.lock().unwrap().ensure_generation(4));
        let observed_generation = 4;
        let new_generation = 5;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let resets = std::sync::Arc::new(AtomicU32::new(0));
        let discovery = std::sync::Arc::new(AtomicU32::new(MAX_DISCOVERY_INSPECTIONS));
        let selected = std::sync::Arc::new(AtomicU32::new(MAX_SELECTED_POSE_INSPECTIONS));
        let workers: Vec<_> = (0..2)
            .map(|worker_index| {
                let samples = samples.clone();
                let barrier = barrier.clone();
                let resets = resets.clone();
                let discovery = discovery.clone();
                let selected = selected.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut samples = samples.lock().unwrap();
                    if observed_generation != new_generation
                        && sync_sample_generation_with_counters(
                            &mut samples,
                            new_generation,
                            &discovery,
                            &selected,
                        )
                    {
                        resets.fetch_add(1, Ordering::Relaxed);
                    }

                    // Each stale callback may continue inspecting its own
                    // portion after taking the lock. The second callback must
                    // preserve the first one's keys and budget increments.
                    for offset in 0..2 {
                        let index = worker_index * 2 + offset;
                        let entity = 90 + index as u32;
                        let key = RenderKey {
                            entity,
                            entity_component_index: 7,
                            slot: -1,
                            model_id: 14,
                        };
                        let dispatch_id = index as u64 + 1;
                        assert_eq!(
                            samples.begin_inspection(
                                new_generation,
                                index,
                                entity,
                                key.entity_component_index,
                                key.slot,
                                dispatch_id,
                            ),
                            PoseInspection::Discover
                        );
                        assert!(reserve_inspection(&discovery, MAX_DISCOVERY_INSPECTIONS));
                        assert!(samples.record_pose(new_generation, key, dispatch_id, true));
                    }
                })
            })
            .collect();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(resets.load(Ordering::Relaxed), 1);
        assert_eq!(discovery.load(Ordering::Relaxed), 4);
        assert_eq!(selected.load(Ordering::Relaxed), 0);
        let mut samples = samples.lock().unwrap();
        for index in 0..4 {
            let entity = 90 + index as u32;
            assert_eq!(
                samples.begin_inspection(new_generation, index, entity, 7, -1, 10 + index as u64),
                PoseInspection::Selected,
                "a stale observer must not clear any selected key"
            );
            assert!(reserve_inspection(&selected, MAX_SELECTED_POSE_INSPECTIONS));
        }
        assert_eq!(selected.load(Ordering::Relaxed), 4);

        // New capture budgets still enforce their limits after the racing
        // initialization; neither stale callback can reset them again.
        discovery.store(MAX_DISCOVERY_INSPECTIONS - 1, Ordering::Relaxed);
        assert!(reserve_inspection(&discovery, MAX_DISCOVERY_INSPECTIONS));
        assert!(!reserve_inspection(&discovery, MAX_DISCOVERY_INSPECTIONS));
        selected.store(MAX_SELECTED_POSE_INSPECTIONS - 1, Ordering::Relaxed);
        assert!(reserve_inspection(&selected, MAX_SELECTED_POSE_INSPECTIONS));
        assert!(!reserve_inspection(
            &selected,
            MAX_SELECTED_POSE_INSPECTIONS
        ));
    }

    #[test]
    fn discovery_selects_at_most_four_moving_keys_and_keeps_them_for_the_capture() {
        let mut samples = SampleSet::new();
        for entity in 1..=4 {
            let key = RenderKey {
                entity,
                entity_component_index: 2,
                slot: -3,
                model_id: 9,
            };
            assert_eq!(
                samples.begin_inspection(1, entity as usize - 1, entity, 2, -3, 1),
                PoseInspection::Discover
            );
            assert!(samples.record_pose(1, key, 1, true));
        }
        assert!(
            samples.begin_inspection(1, 0, 1, 2, -3, 2) == PoseInspection::Selected,
            "selected keys stay eligible"
        );
        assert!(
            samples.begin_inspection(1, 4, 5, 2, -3, 2) == PoseInspection::Discover,
            "discovery remains separate from the four selected-key slots"
        );
        let key = RenderKey {
            entity: 5,
            entity_component_index: 2,
            slot: -3,
            model_id: 9,
        };
        assert!(
            !samples.record_pose(1, key, 2, true),
            "the selected set stays capped"
        );
    }

    #[test]
    fn discovery_and_selected_sampling_have_independent_capture_budgets() {
        let discovery = AtomicU32::new(0);
        let selected = AtomicU32::new(0);
        for _ in 0..MAX_DISCOVERY_INSPECTIONS {
            assert!(reserve_inspection(&discovery, MAX_DISCOVERY_INSPECTIONS));
        }
        assert!(!reserve_inspection(&discovery, MAX_DISCOVERY_INSPECTIONS));
        for _ in 0..MAX_SELECTED_POSE_INSPECTIONS {
            assert!(reserve_inspection(&selected, MAX_SELECTED_POSE_INSPECTIONS));
        }
        assert!(!reserve_inspection(
            &selected,
            MAX_SELECTED_POSE_INSPECTIONS
        ));
        assert_eq!(discovery.load(Ordering::Acquire), MAX_DISCOVERY_INSPECTIONS);
        assert_eq!(
            selected.load(Ordering::Acquire),
            MAX_SELECTED_POSE_INSPECTIONS
        );
    }

    #[test]
    fn road_association_requires_fresh_same_body_child_and_instance() {
        let fresh = FreshInstance {
            present: true,
            entity: 22,
            entity_component_index: 9,
            slot: -1,
            returned: 0x1234,
            capture_generation: 7,
            dispatch_id: 88,
        };
        assert!(association_matches(fresh, 7, 88, 22, 0x1234));
        assert!(!association_matches(fresh, 8, 88, 22, 0x1234));
        assert!(!association_matches(fresh, 7, 89, 22, 0x1234));
        assert!(!association_matches(fresh, 7, 88, 23, 0x1234));
        assert!(!association_matches(fresh, 7, 88, 22, 0x5678));
        assert!(!association_matches(
            FreshInstance::default(),
            7,
            88,
            22,
            0x1234
        ));
        assert!(association_matches(
            FreshInstance { slot: 3, ..fresh },
            7,
            88,
            22,
            0x1234,
        ));
    }

    #[test]
    fn live_body_and_registry_checks_gate_every_native_read() {
        let meta = RenderMeta {
            capture_generation: 7,
            engine: 0x1234,
            binding_valid: true,
            ..RenderMeta::default()
        };
        let expected = BodyContext {
            registry_index: 2,
            key: 0x5678,
            dispatch_id: 90,
            meta,
            active: true,
            ..BodyContext::EMPTY
        };
        assert!(same_live_body(expected, expected, 7, 7, true));
        assert!(!same_live_body(expected, expected, 7, 7, false));
        assert!(!same_live_body(
            expected,
            BodyContext {
                key: 0x9999,
                ..expected
            },
            7,
            7,
            true,
        ));
        assert!(!same_live_body(expected, expected, 8, 7, true));

        let reads = std::cell::Cell::new(0);
        let live = || same_live_body(expected, expected, 7, 7, false);
        assert_eq!(
            read_between_live_checks(live, || {
                reads.set(reads.get() + 1);
                Some(123_u32)
            }),
            None,
        );
        assert_eq!(reads.get(), 0, "an invalid registry slot blocks the read");

        let validations = std::cell::Cell::new(0);
        assert_eq!(
            read_between_live_checks(
                || {
                    let previous = validations.get();
                    validations.set(previous + 1);
                    previous == 0
                },
                || {
                    reads.set(reads.get() + 1);
                    Some(456_u32)
                },
            ),
            None,
            "a body invalidated by the read is rejected before data is used",
        );
        assert_eq!(reads.get(), 1);
        assert_eq!(validations.get(), 2);
    }

    #[test]
    fn road_span_uses_the_timestamp_captured_at_native_return() {
        assert_eq!(road_call_span(100, 450), (100, 350));
        assert_eq!(road_call_span(450, 100), (100, 0));
    }

    #[test]
    fn road_callback_reads_require_the_exact_live_body_engine_and_single_call() {
        let path = RoadObservation {
            active: true,
            meta: RenderMeta {
                capture_generation: 7,
                engine: 0x1234,
                binding_valid: true,
                ..RenderMeta::default()
            },
            owner: 50,
            ..RoadObservation::EMPTY
        };
        assert!(path_read_eligible(path, 7, true, 0x1234, true));
        assert!(!path_read_eligible(path, 7, false, 0x1234, true));
        assert!(!path_read_eligible(path, 7, true, 0x5678, true));
        assert!(!path_read_eligible(path, 8, true, 0x1234, true));
        assert!(!path_read_eligible(path, 7, true, 0x1234, false));
        assert!(!path_read_eligible(
            RoadObservation {
                invalid: true,
                ..path
            },
            7,
            true,
            0x1234,
            true,
        ));

        let finish = RoadObservation {
            path_seen: true,
            ..path
        };
        assert!(finish_read_eligible(finish, 7, true));
        assert!(!finish_read_eligible(
            RoadObservation {
                finish_calls: 1,
                ..finish
            },
            7,
            true,
        ));
        assert!(!finish_read_eligible(finish, 7, false));
        assert!(!finish_read_eligible(finish, 8, true));
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn inactive_road_vf3_forwards_native_arguments_and_return_unchanged() {
        let _serial = ROAD_FORWARD_TEST_LOCK.lock().unwrap();
        assert!(!crate::timeline::active());
        let previous =
            ROAD_VF3_ORIGINAL.swap(capture_road_vf3 as *const () as usize, Ordering::AcqRel);
        let _restore = RestoreOriginal {
            slot: &ROAD_VF3_ORIGINAL,
            value: previous,
        };
        let arguments = [0x1010, 0x2020, 0x3030, 0x4040];
        assert_eq!(
            unsafe { road_vf3_detour(arguments[0], arguments[1], arguments[2], arguments[3]) },
            0xaaaa,
        );
        assert_eq!(
            *ROAD_FORWARD_CAPTURE.lock().unwrap(),
            [arguments[0], arguments[1], arguments[2], arguments[3], 0, 0],
        );
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn inactive_path_helper_forwards_its_six_native_arguments_unchanged() {
        let _serial = ROAD_FORWARD_TEST_LOCK.lock().unwrap();
        assert!(!crate::timeline::active());
        let previous = ROAD_PATH_HELPER_ORIGINAL.swap(
            capture_road_path_helper as *const () as usize,
            Ordering::AcqRel,
        );
        let _restore = RestoreOriginal {
            slot: &ROAD_PATH_HELPER_ORIGINAL,
            value: previous,
        };
        let alpha = f32::from_bits(0x3f12_3456);
        assert_eq!(
            unsafe { road_path_helper_observer(0x10, 0x20, 3, 0x40, 0x50, alpha, 0x60) },
            0xbbbb,
        );
        assert_eq!(
            *ROAD_FORWARD_CAPTURE.lock().unwrap(),
            [0x10, 0x20, 3, 0x40, 0x50, alpha.to_bits() as usize],
        );
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn inactive_user_transform_forwards_input_and_output_arguments_unchanged() {
        let _serial = ROAD_FORWARD_TEST_LOCK.lock().unwrap();
        assert!(!crate::timeline::active());
        let previous = ROAD_USER_TRANSFORMS_ORIGINAL.swap(
            capture_road_user_transforms as *const () as usize,
            Ordering::AcqRel,
        );
        let _restore = RestoreOriginal {
            slot: &ROAD_USER_TRANSFORMS_ORIGINAL,
            value: previous,
        };
        let alpha = f32::from_bits(0x3eab_cdef);
        assert_eq!(
            unsafe { road_user_transforms_observer(0x11, 0x22, alpha, 0x44) },
            0xcccc,
        );
        assert_eq!(
            *ROAD_FORWARD_CAPTURE.lock().unwrap(),
            [0x11, 0x22, alpha.to_bits() as usize, 0x44, 0, 0],
        );
    }

    #[test]
    fn road_scope_restores_nested_transformator_context() {
        let first = RoadObservation {
            active: true,
            context: 0x1000,
            ..RoadObservation::EMPTY
        };
        let second = RoadObservation {
            active: true,
            context: 0x2000,
            ..RoadObservation::EMPTY
        };
        let first_scope = RoadScope::enter(first);
        assert_eq!(RoadScope::current().context, 0x1000);
        let second_scope = RoadScope::enter(second);
        assert_eq!(RoadScope::current().context, 0x2000);
        drop(second_scope);
        assert_eq!(RoadScope::current().context, 0x1000);
        drop(first_scope);
        assert!(!RoadScope::current().active);
    }

    #[test]
    fn road_vehicle_selection_is_allowlist_bounded_and_once_per_dispatch() {
        let mut samples = RoadVehicleSamples::new();
        let key = VehicleKey {
            owner: 1,
            child: 101,
            component: 4,
            slot: -1,
            model_id: 77,
        };
        assert_eq!(
            samples.decide(5, key, 10, true),
            VehicleSampleDecision::Discover
        );
        assert_eq!(
            samples.decide(5, key, 10, true),
            VehicleSampleDecision::Skip
        );
        assert_eq!(
            samples.decide(5, key, 11, false),
            VehicleSampleDecision::Selected,
            "an already selected vehicle remains observable when its path pair collapses"
        );
        let changed_lod = VehicleKey {
            model_id: 78,
            ..key
        };
        assert_eq!(
            samples.decide(5, changed_lod, 11, true),
            VehicleSampleDecision::Skip,
            "the same owner cannot emit twice during one dispatch"
        );
        assert_eq!(
            samples.decide(5, changed_lod, 12, false),
            VehicleSampleDecision::Selected,
            "a selected renderer key may report a later model identifier"
        );
        assert_eq!(samples.entries[0].key.model_id, 78);

        let other = VehicleKey {
            owner: 2,
            child: 102,
            component: 4,
            slot: -1,
            model_id: 80,
        };
        assert_eq!(
            samples.decide(5, other, 2, true),
            VehicleSampleDecision::Discover
        );
        let third = VehicleKey {
            owner: 3,
            child: 103,
            component: 4,
            slot: -1,
            model_id: 81,
        };
        assert_eq!(
            samples.decide(5, third, 20, true),
            VehicleSampleDecision::Skip
        );
    }

    #[test]
    fn stale_road_capture_generation_cannot_reset_current_vehicle_samples() {
        let mut samples = RoadVehicleSamples::new();
        let current = VehicleKey {
            owner: 45,
            child: 450,
            component: 8,
            slot: -1,
            model_id: 91,
        };
        assert_eq!(
            decide_road_sample(&mut samples, 9, 9, current, 1, true),
            Some(VehicleSampleDecision::Discover)
        );
        samples.note_sample();
        samples.note_refused();
        let stale = VehicleKey {
            owner: 46,
            child: 460,
            ..current
        };
        assert_eq!(decide_road_sample(&mut samples, 8, 9, stale, 2, true), None);
        assert_eq!(samples.capture_generation, 9);
        assert_eq!(samples.entries[0].key, current);
        assert_eq!(samples.samples, 1);
        assert_eq!(samples.refused, 1);
    }

    #[test]
    fn world_vector_bounds_reject_bad_ranges_and_cap_records() {
        assert_eq!(vector24_record_count(0, 0, 0), Some(0));
        assert_eq!(vector24_record_count(0x1000, 0x1018, 0x1030), Some(1));
        assert_eq!(vector24_record_count(0x1000, 0x1048, 0x1060), Some(3));
        assert_eq!(vector24_record_count(0x1000, 0x0ff8, 0x1030), None);
        assert_eq!(vector24_record_count(0x1000, 0x1019, 0x1030), None);
        assert_eq!(vector24_record_count(0x1000, 0x1030, 0x1020), None);
        assert_eq!(vector24_record_count(0x1001, 0x1019, 0x1031), None);
        assert_eq!(
            vector24_record_count(usize::MAX - 15, usize::MAX - 15, usize::MAX),
            None
        );
        assert_eq!(
            vector24_record_count(0x1000, 0x1000 + 24 * 4097, 0x1000 + 24 * 4100),
            None
        );
    }

    fn test_world_vector(records: &[[f32; 6]]) -> WorldVectorCopy {
        let mut words = [0; 24];
        for (index, record) in records.iter().enumerate() {
            for (word, value) in record.iter().enumerate() {
                words[index * 6 + word] = value.to_bits();
            }
        }
        WorldVectorCopy {
            total: records.len() as u16,
            copied: records.len() as u8,
            truncated: false,
            words,
        }
    }

    #[test]
    fn road_offset_clones_each_finite_record_and_changes_only_world_x() {
        let original = test_world_vector(&[
            [1.25, -2.5, 3.0, 0.0, 1.0, -0.0],
            [-10.0, 4.0, 5.0, 0.25, -0.5, 0.75],
        ]);
        let translated = translated_world_records(original, ROAD_OFFSET_X_METERS).unwrap();
        assert_eq!(translated.count, 2);
        assert_eq!(f32::from_bits(translated.words[0]), 6.25);
        assert_eq!(f32::from_bits(translated.words[6]), -5.0);
        for index in [1, 2, 3, 4, 5, 7, 8, 9, 10, 11] {
            assert_eq!(translated.words[index], original.words[index]);
        }
        assert_eq!(original.words[0], 1.25_f32.to_bits());
        assert_eq!(original.words[6], (-10.0_f32).to_bits());

        let header = private_world_vector_header(&translated);
        assert_eq!(header[0] % 32, 0, "private records are 32-byte aligned");
        assert_eq!(header[1] - header[0], 2 * 24);
        assert_eq!(header[2], header[1], "native vector capacity is bounded");
    }

    #[test]
    fn road_offset_falls_back_for_empty_truncated_malformed_or_nonfinite_vectors() {
        let empty = test_world_vector(&[]);
        assert!(translated_world_records(empty, ROAD_OFFSET_X_METERS).is_none());

        let four = test_world_vector(&[[0.0; 6]; 4]);
        assert!(translated_world_records(four, ROAD_OFFSET_X_METERS).is_some());
        let marked_truncated = WorldVectorCopy {
            truncated: true,
            ..four
        };
        assert!(translated_world_records(marked_truncated, ROAD_OFFSET_X_METERS).is_none());
        let oversized = WorldVectorCopy {
            total: 5,
            copied: 4,
            truncated: true,
            ..four
        };
        assert!(translated_world_records(oversized, ROAD_OFFSET_X_METERS).is_none());
        let inconsistent = WorldVectorCopy {
            total: 2,
            copied: 1,
            ..four
        };
        assert!(translated_world_records(inconsistent, ROAD_OFFSET_X_METERS).is_none());
        let nan = WorldVectorCopy {
            words: [f32::NAN.to_bits(); 24],
            total: 1,
            copied: 1,
            truncated: false,
        };
        assert!(translated_world_records(nan, ROAD_OFFSET_X_METERS).is_none());
        let overflow = WorldVectorCopy {
            words: [f32::MAX.to_bits(); 24],
            total: 1,
            copied: 1,
            truncated: false,
        };
        assert!(translated_world_records(overflow, f32::MAX).is_none());
        assert!(translated_world_records(four, f32::INFINITY).is_none());
    }

    #[test]
    fn road_offset_requires_active_capture_live_singleton_scope_and_applied_record() {
        let observation = RoadObservation {
            active: true,
            meta: RenderMeta {
                capture_generation: 7,
                ..RenderMeta::default()
            },
            owner: 50,
            path_seen: true,
            ..RoadObservation::EMPTY
        };
        assert!(road_offset_scope_eligible(
            true,
            true,
            7,
            observation,
            true,
            true
        ));
        assert!(!road_offset_scope_eligible(
            false,
            true,
            7,
            observation,
            true,
            true
        ));
        assert!(!road_offset_scope_eligible(
            true,
            false,
            7,
            observation,
            true,
            true
        ));
        assert!(!road_offset_scope_eligible(
            true,
            true,
            8,
            observation,
            true,
            true
        ));
        assert!(!road_offset_scope_eligible(
            true,
            true,
            7,
            observation,
            false,
            true
        ));
        assert!(!road_offset_scope_eligible(
            true,
            true,
            7,
            observation,
            true,
            false
        ));
        assert!(!road_offset_scope_eligible(
            true,
            true,
            7,
            RoadObservation {
                path_seen: false,
                ..observation
            },
            true,
            true,
        ));
        let repeat_finish = RoadObservation {
            finish_calls: 1,
            ..observation
        };
        assert!(!road_offset_scope_eligible(
            true,
            true,
            7,
            repeat_finish,
            true,
            true,
        ));
        assert!(!should_forward_translated(
            road_offset_scope_eligible(true, true, 7, repeat_finish, true, true),
            true,
            true,
        ));
        assert!(!should_forward_translated(true, true, false));
        assert!(!should_forward_translated(true, false, true));
        assert!(!should_forward_translated(false, true, true));
        assert!(should_forward_translated(true, true, true));
    }

    #[test]
    fn road_world_records_keep_first_four_and_report_truncation() {
        let mut observation = RoadObservation::EMPTY;
        let first = WorldVectorCopy {
            total: 3,
            copied: 3,
            truncated: false,
            words: std::array::from_fn(|index| index as u32 + 1),
        };
        append_world_vector(&mut observation, first);
        assert_eq!(observation.world_records_total, 3);
        assert_eq!(observation.world_records_copied, 3);
        assert_eq!(&observation.world_points[..18], &first.words[..18]);

        let second = WorldVectorCopy {
            total: 3,
            copied: 3,
            truncated: false,
            words: [0xfeed_beef; 24],
        };
        append_world_vector(&mut observation, second);
        assert_eq!(observation.world_records_total, 6);
        assert_eq!(observation.world_records_copied, 4);
        assert!(observation.world_records_truncated);
        assert_eq!(&observation.world_points[18..24], &[0xfeed_beef; 6]);

        observation.invalid_reason = ROAD_INVALID_REPEATED_FINISH;
        assert_eq!(observation.invalid_reason, 14);
    }

    #[test]
    fn road_vehicle_sample_budget_is_bounded() {
        let mut samples = RoadVehicleSamples::new();
        samples.samples = MAX_ROAD_VEHICLE_SAMPLES - 1;
        assert!(samples.note_sample());
        assert!(!samples.note_sample());
        assert_eq!(samples.samples, MAX_ROAD_VEHICLE_SAMPLES);
        assert_eq!(samples.skips, 1);
    }

    #[test]
    fn entity_allowlist_is_strict_bounded_and_has_no_fallback() {
        let valid = parse_entity_allowlist(b"101\r\n202\r\n303\r\n");
        assert!(valid.is_valid());
        assert_eq!(valid.len, 3);
        assert_eq!(
            valid.ids[..valid.len].iter().position(|id| *id == 202),
            Some(1)
        );
        assert_eq!(
            valid.ids[..valid.len].iter().position(|id| *id == 404),
            None
        );

        assert_eq!(parse_entity_allowlist(b"").status(), AllowlistStatus::Empty);
        for malformed in [
            &b"101,202"[..],
            &b"101\n\n202"[..],
            &b"0"[..],
            &b"101\n101"[..],
            &b"+101"[..],
            &b"4294967296"[..],
        ] {
            assert_eq!(
                parse_entity_allowlist(malformed).status(),
                AllowlistStatus::Malformed
            );
        }
        let too_many = (1..=MAX_TARGET_ENTITIES + 1)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            parse_entity_allowlist(too_many.as_bytes()).status(),
            AllowlistStatus::Oversized
        );
        assert_eq!(
            parse_entity_allowlist(&vec![b'1'; MAX_ENTITY_FILE_BYTES + 1]).status(),
            AllowlistStatus::Oversized
        );
    }

    fn native_candidate(updates: u32) -> NativeTimeCandidate {
        let key = ClockKey {
            engine: 0x1000,
            entity_ptr: 0x2000,
            entity_id: 44,
        };
        NativeTimeCandidate {
            present: true,
            sample_valid: true,
            capture_generation: 7,
            native_this: 0x3000,
            game_state: 0x4000,
            engine: key.engine,
            updates,
            frame_time_micros: 200_000,
            room: true,
            before: NativeClockSample {
                key,
                time: 10_000,
                tick_count: 20,
                update_count: 30,
            },
            after: NativeClockSample {
                key,
                time: 10_000 + i64::from(updates) * 200,
                tick_count: 20 + updates,
                update_count: 30 + updates,
            },
            writer_stable: true,
            ..NativeTimeCandidate::default()
        }
    }

    fn native_endpoint(engine: usize, game_state: usize, room_step: u64) -> NativeTimeEndpoint {
        NativeTimeEndpoint {
            used: true,
            capture_generation: 7,
            world_epoch: 9,
            game_state,
            engine,
            clock: ClockKey {
                engine,
                entity_ptr: engine + 0x100,
                entity_id: 44,
            },
            time: 10_000 + room_step as i64 * 200,
            tick_count: room_step as u32,
            room_step,
            update_count: room_step as u32,
        }
    }

    #[test]
    fn native_time_opt_in_requires_render_probe_and_timeline() {
        assert!(!native_time_requested_from(None));
        assert!(!native_time_requested_from(Some("true")));
        assert!(native_time_requested_from(Some("1")));
        assert!(!requested_with_native_time(
            None,
            None,
            Some("1"),
            Some("watch")
        ));
        assert!(!requested_with_native_time(
            Some("1"),
            None,
            Some("1"),
            None
        ));
        assert!(requested_with_native_time(
            Some("1"),
            None,
            Some("1"),
            Some("watch")
        ));
        assert!(!road_history_requested_with_features(
            Some("1"),
            None,
            Some("1"),
            Some("watch")
        ));
        assert!(!road_history_requested_with_features(
            Some("1"),
            Some("1"),
            Some("1"),
            None
        ));
        assert!(road_history_requested_with_features(
            Some("1"),
            Some("1"),
            Some("1"),
            Some("watch")
        ));
    }

    #[test]
    fn native_time_mapping_points_cover_exact_one_and_two_update_batches() {
        for updates in [1, 2] {
            let candidate = native_candidate(updates);
            let points = native_batch_points(candidate, 501, 9).unwrap();
            assert_eq!(points.len, updates as usize);
            for (index, endpoint) in points.entries.iter().take(points.len).enumerate() {
                assert!(endpoint.used);
                assert_eq!(endpoint.time, 10_200 + (index as i64) * 200);
                assert_eq!(endpoint.room_step, 501 + index as u64);
                assert_eq!(endpoint.update_count, 31 + index as u32);
                assert_eq!(endpoint.world_epoch, 9);
            }
        }
    }

    #[test]
    fn native_zero_callback_verifies_without_inventing_a_time_endpoint() {
        let candidate = native_candidate(0);
        let points = native_batch_points(candidate, 501, 9).unwrap();
        assert_eq!(points.len, 0);
        assert!(points.entries.iter().all(|endpoint| !endpoint.used));

        let mut changed = candidate;
        changed.after.time += 200;
        assert_eq!(
            native_batch_points(changed, 501, 9).unwrap_err(),
            TIME_REJECT_SAMPLE
        );
    }

    #[test]
    fn map_rebuild_lease_cannot_clear_a_newer_replication_attempt() {
        let state = AtomicU64::new(0);
        let first = mark_map_rebuild_state(&state);
        assert_eq!(state.load(Ordering::Acquire), first);
        assert!(first & 1 != 0);

        let second = mark_map_rebuild_state(&state);
        assert!(second > first);
        assert!(!clear_map_rebuild_state(&state, first));
        assert_eq!(state.load(Ordering::Acquire), second);
        assert!(clear_map_rebuild_state(&state, second));
        assert_eq!(state.load(Ordering::Acquire), second.wrapping_add(1));
    }

    #[test]
    fn replication_writer_bracket_uses_the_production_four_version_span() {
        let engine = 0xE000_0000_0000_0000_usize
            | NEXT_REPLICA_COPY.fetch_add(1, Ordering::Relaxed) as usize;
        let before = native_writer_snapshot(engine);
        let token = native_writer_enter_engine(engine);
        let during = native_writer_snapshot(engine);
        assert!(token.valid);
        assert_eq!(during.version, before.version + 2);
        assert_eq!(during.active, 1);

        let after = native_writer_exit(token);
        assert_eq!(after.version, before.version + 4);
        assert!(completed_native_copy_bracket(before, token, after));
        assert!(!completed_native_copy_bracket(
            before,
            token,
            NativeWriterSnapshot {
                version: before.version + 2,
                ..after
            }
        ));
    }

    #[test]
    fn copied_endpoint_and_local_reports_publish_both_destination_labels() {
        let source = crate::road_history::AcceptedAnchor {
            capture_generation: 7,
            world_epoch: 9,
            game_state: 0x2000,
            engine: 0x1000,
            clock_entity_ptr: 0x1100,
            clock_entity_id: 44,
            clock_kind: crate::road_history::ClockKind::CGameTime,
            native_time: 10_000,
            tick_count: 20,
            update_count: 30,
            room_step: 499,
        };
        let copy = crate::road_history::BoundCopy {
            id: 77,
            source,
            destination_game_state: 0x4000,
            destination_engine: 0x3000,
            destination_clock_entity_id: 44,
            destination_clock_entity_ptr: 0x3100,
            capture_generation: 7,
            world_epoch: 9,
            family_revision: crate::road_history::current_family_revision(),
        };
        let key = ClockKey {
            engine: 0x3000,
            entity_ptr: 0x3100,
            entity_id: 44,
        };
        let candidate = NativeTimeCandidate {
            present: true,
            sample_valid: true,
            capture_generation: 7,
            native_this: 0x5000,
            game_state: 0x4000,
            engine: 0x3000,
            updates: 2,
            frame_time_micros: 200_000,
            room: true,
            before: NativeClockSample {
                key,
                time: 10_000,
                tick_count: 20,
                update_count: 30,
            },
            after: NativeClockSample {
                key,
                time: 10_400,
                tick_count: 22,
                update_count: 32,
            },
            bound_copy: Some(copy),
            writer_stable: true,
            ..NativeTimeCandidate::default()
        };
        let inherited = copied_anchor_endpoint(copy, candidate, 500, 9, true).unwrap();
        let local = native_batch_points(candidate, 500, 9).unwrap();
        let mut map = NativeTimeMap::new();
        map.capture_generation = 7;
        map.world_mark = Some(crate::step::WorldMark::default());
        map.world_epoch = 9;
        assert!(map.insert(inherited));
        for endpoint in local.entries.iter().take(local.len).copied() {
            assert!(map.insert(endpoint));
        }

        assert_eq!(
            map.lookup(7, 9, 0x4000, 0x3000, key, 10_000)
                .map(|endpoint| endpoint.room_step),
            Some(499)
        );
        assert_eq!(
            map.lookup(7, 9, 0x4000, 0x3000, key, 10_200)
                .map(|endpoint| endpoint.room_step),
            Some(500)
        );
        assert_eq!(
            map.lookup(7, 9, 0x4000, 0x3000, key, 10_400)
                .map(|endpoint| endpoint.room_step),
            Some(501)
        );
    }

    #[test]
    fn pending_map_rebuild_blocks_zero_reuse_and_invalidates_only_destination() {
        let mut map = NativeTimeMap::new();
        let destination = native_endpoint(0x1000, 0x2000, 500);
        let other_engine = native_endpoint(0x3000, 0x4000, 500);
        assert!(map.insert(destination));
        assert!(map.insert(other_engine));
        assert_eq!(
            lookup_zero_endpoint(
                &map,
                ZeroEndpointQuery {
                    rebuild_state: Some(0),
                    generation: 7,
                    world_epoch: 9,
                    game_state: destination.game_state,
                    engine: destination.engine,
                    clock: destination.clock,
                    time: destination.time,
                    next_step: 501,
                },
            ),
            Some(destination)
        );

        let rebuild = mark_map_rebuild_state(&AtomicU64::new(0));
        invalidate_rebuilt_engine(&mut map, destination.engine, Some(rebuild));
        assert_eq!(
            lookup_zero_endpoint(
                &map,
                ZeroEndpointQuery {
                    rebuild_state: Some(rebuild),
                    generation: 7,
                    world_epoch: 9,
                    game_state: destination.game_state,
                    engine: destination.engine,
                    clock: destination.clock,
                    time: destination.time,
                    next_step: 501,
                },
            ),
            None
        );
        assert_eq!(
            map.latest_for(7, 9, destination.game_state, destination.engine),
            None
        );
        assert_eq!(
            map.latest_for(7, 9, other_engine.game_state, other_engine.engine),
            Some(other_engine)
        );
    }

    #[test]
    fn native_time_mapping_rejects_count_or_clock_gaps() {
        let mut candidate = native_candidate(2);
        candidate.after.time += 200;
        assert_eq!(
            native_batch_points(candidate, 501, 9).unwrap_err(),
            TIME_REJECT_COUNT_GAP
        );
        let mut candidate = native_candidate(1);
        candidate.after.key.entity_id += 1;
        assert_eq!(
            native_batch_points(candidate, 501, 9).unwrap_err(),
            TIME_REJECT_COUNT_GAP
        );
    }

    #[test]
    fn native_time_mapping_uses_the_actual_step_frame_time_argument() {
        let mut candidate = native_candidate(2);
        candidate.frame_time_micros = 66_667;
        candidate.after.time = candidate.before.time + 132;
        let points = native_batch_points(candidate, 900, 3).unwrap();
        assert_eq!(points.entries[0].time, candidate.before.time + 66);
        assert_eq!(points.entries[1].time, candidate.before.time + 132);

        candidate.frame_time_micros = 67_000;
        assert_eq!(
            native_batch_points(candidate, 900, 3).unwrap_err(),
            TIME_REJECT_COUNT_GAP
        );
    }
}
