//! Opt-in causal timeline for the 40420 smooth-pacing experiment.
//!
//! The watcher does filesystem work only on its own thread. Hook sites append
//! fixed-size scalar records to a preallocated bounded buffer; they never
//! format or write a file. A capture is requested by changing the small nonce
//! file in the hook's data directory while the game is running.

#![allow(unsafe_code)]

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

/// Set to `watch` alongside `TPF3MP_SMOOTH_PACING=1` to arm the request-file
/// watcher. Unset, this module creates no thread and touches no files.
pub const ENV: &str = "TPF3MP_HOOK_PACING_TIMELINE";
/// Replace this file's contents with a new ASCII nonce to request one capture.
pub const REQUEST_FILE: &str = "pacing-timeline.request";

const MAX_NONCE_BYTES: usize = 64;
const EVENT_CAPACITY: usize = 4_096;
const ROAD_EVENT_RESERVE: usize = 512;
const CAPTURE_DURATION: Duration = Duration::from_secs(5);
const FILE_POLL: Duration = Duration::from_millis(500);
const CAPTURE_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    Disabled,
    ObserveOnly,
    Pacing,
}

impl HookMode {
    pub fn from_options(smooth_pacing: bool, timeline_watch: bool) -> Self {
        match (smooth_pacing, timeline_watch) {
            (true, _) => Self::Pacing,
            (false, true) => Self::ObserveOnly,
            (false, false) => Self::Disabled,
        }
    }

    pub fn installs_observers(self) -> bool {
        self != Self::Disabled
    }

    pub fn changes_pacing(self) -> bool {
        self == Self::Pacing
    }
}

static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);
/// Zero means no capture is open; each new capture receives a fresh generation.
static ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);
static CLOCK_EPOCH: OnceLock<Instant> = OnceLock::new();
static EVENTS: OnceLock<Mutex<CaptureBuffer>> = OnceLock::new();

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CaptureClockAnchor {
    /// Same `Instant`-relative nanosecond domain used by event `t_ns`.
    native_start_ns: u64,
    /// Windows QPC units per second; zero where the OS query failed or is
    /// unavailable.
    qpc_frequency: i64,
    /// QPC samples bound the activation of `ACTIVE_GENERATION`.
    qpc_before: i64,
    qpc_after: i64,
}

impl CaptureClockAnchor {
    fn bracket_ticks(self) -> i64 {
        self.qpc_after.saturating_sub(self.qpc_before).max(0)
    }

    fn qpc_valid(self) -> bool {
        self.qpc_frequency > 0 && self.qpc_after >= self.qpc_before
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    CGameStepEntryRaw = 1,
    CGameStepEntryPrepared = 2,
    CGameStepExitRaw = 3,
    SyncEntryRaw = 4,
    SyncExitRaw = 5,
    SyncExitPolicy = 6,
    GameSimStepEntry = 7,
    GameSimStepExit = 8,
    RunStepEntry = 9,
    RunStepExit = 10,
    ComputeFrameTime = 11,
    RoomActionQueued = 12,
    ReplayRequested = 13,
    ReplayGuiPoll = 14,
    ReplayTakeEntered = 15,
    ReplayTaken = 16,
    ReplayAcknowledged = 17,
    ReplayStateAfterZero = 18,
    ReplayStaged = 19,
    NativeCommandAccepted = 20,
    NativeCommandHandedOver = 21,
    RenderUpdateEntry = 22,
    RenderModelListBinding = 23,
    RenderDispatchEntry = 24,
    RenderDispatchExit = 25,
    RenderPoseSample = 26,
    RenderProbeRefused = 27,
    RenderUpdateExit = 28,
    RenderProbeConfig = 29,
    RoadVehicleSample = 30,
    RoadOffsetApplied = 31,
    NativeTimeStep = 32,
    NativeReplicaCopy = 33,
    RoadHistoryApplied = 34,
    RoadHistoryFallback = 35,
    RoadHistorySample = 36,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::CGameStepEntryRaw => "cgame_step_entry_raw",
            Self::CGameStepEntryPrepared => "cgame_step_entry_prepared",
            Self::CGameStepExitRaw => "cgame_step_exit_raw",
            Self::SyncEntryRaw => "sync_entry_raw",
            Self::SyncExitRaw => "sync_exit_raw",
            Self::SyncExitPolicy => "sync_exit_policy",
            Self::GameSimStepEntry => "gamesim_step_entry",
            Self::GameSimStepExit => "gamesim_step_exit",
            Self::RunStepEntry => "run_step_entry",
            Self::RunStepExit => "run_step_exit",
            Self::ComputeFrameTime => "compute_frame_time",
            Self::RoomActionQueued => "room_action_queued",
            Self::ReplayRequested => "replay_requested",
            Self::ReplayGuiPoll => "replay_gui_poll",
            Self::ReplayTakeEntered => "replay_take_entered",
            Self::ReplayTaken => "replay_taken",
            Self::ReplayAcknowledged => "replay_acknowledged",
            Self::ReplayStateAfterZero => "replay_state_after_zero",
            Self::ReplayStaged => "replay_staged",
            Self::NativeCommandAccepted => "native_command_accepted",
            Self::NativeCommandHandedOver => "native_command_handed_over",
            Self::RenderUpdateEntry => "render_update_entry",
            Self::RenderModelListBinding => "render_model_list_binding",
            Self::RenderDispatchEntry => "render_dispatch_entry",
            Self::RenderDispatchExit => "render_dispatch_exit",
            Self::RenderPoseSample => "render_pose_sample",
            Self::RenderProbeRefused => "render_probe_refused",
            Self::RenderUpdateExit => "render_update_exit",
            Self::RenderProbeConfig => "render_probe_config",
            Self::RoadVehicleSample => "road_vehicle_sample",
            Self::RoadOffsetApplied => "road_offset_applied",
            Self::NativeTimeStep => "native_time_step",
            Self::NativeReplicaCopy => "native_replica_copy",
            Self::RoadHistoryApplied => "road_history_applied",
            Self::RoadHistoryFallback => "road_history_fallback",
            Self::RoadHistorySample => "road_history_sample",
        }
    }

    fn from_byte(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::CGameStepEntryRaw,
            2 => Self::CGameStepEntryPrepared,
            3 => Self::CGameStepExitRaw,
            4 => Self::SyncEntryRaw,
            5 => Self::SyncExitRaw,
            6 => Self::SyncExitPolicy,
            7 => Self::GameSimStepEntry,
            8 => Self::GameSimStepExit,
            9 => Self::RunStepEntry,
            10 => Self::RunStepExit,
            11 => Self::ComputeFrameTime,
            12 => Self::RoomActionQueued,
            13 => Self::ReplayRequested,
            14 => Self::ReplayGuiPoll,
            15 => Self::ReplayTakeEntered,
            16 => Self::ReplayTaken,
            17 => Self::ReplayAcknowledged,
            18 => Self::ReplayStateAfterZero,
            19 => Self::ReplayStaged,
            20 => Self::NativeCommandAccepted,
            21 => Self::NativeCommandHandedOver,
            22 => Self::RenderUpdateEntry,
            23 => Self::RenderModelListBinding,
            24 => Self::RenderDispatchEntry,
            25 => Self::RenderDispatchExit,
            26 => Self::RenderPoseSample,
            27 => Self::RenderProbeRefused,
            28 => Self::RenderUpdateExit,
            29 => Self::RenderProbeConfig,
            30 => Self::RoadVehicleSample,
            31 => Self::RoadOffsetApplied,
            32 => Self::NativeTimeStep,
            33 => Self::NativeReplicaCopy,
            34 => Self::RoadHistoryApplied,
            35 => Self::RoadHistoryFallback,
            36 => Self::RoadHistorySample,
            _ => return None,
        })
    }
}

/// Numeric fields shared by every event. Zero is used where a value does not
/// apply; `selected_known` distinguishes no selection from a selected zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fields {
    pub this: usize,
    pub data: usize,
    pub now: i64,
    pub last: i64,
    pub interval_micros: i32,
    pub alpha_milli: i64,
    pub arg_a: usize,
    pub arg_b: usize,
    pub arg_c: usize,
    pub result_byte: u8,
    pub legacy_return: i32,
    pub applied_return: i32,
    pub selected_updates: u64,
    pub selected_known: bool,
    pub room: bool,
    pub pacing_active: bool,
    pub room_speed_percent: u16,
    pub previous_room_speed_percent: u16,
    pub room_steps_per_second: u16,
    pub sealed_ahead: u16,
    pub checkpoint_limited: bool,
    pub completed_updates: u64,
    pub completed_step_wall_nanos: u64,
    pub active_worker_calls: u32,
    pub elapsed_nanos: u64,
    pub scheduler_debt: u64,
    /// Room event identity and action type. Empty for non-command events.
    pub event_seq: u64,
    pub event_step: u64,
    pub action_class: [u8; 32],
    /// Numeric replay token and safe aggregate replay state.
    pub replay_token: u64,
    pub replay_step: u64,
    pub replay_action_count: u32,
    /// 0 none, 1 pending wake, 2 taken, 3 completed successfully, 4 failed,
    /// 5 means the diagnostic replay snapshot skipped a contended lock.
    pub replay_state: u8,
    /// Whether native_replayed accepted the token; its script result is
    /// recorded separately in `result_byte`.
    pub replay_ack_accepted: bool,
    /// Whether `arg_a` holds the local client's command sequence number.
    pub command_number_known: bool,
    /// Renderer-probe identifiers and native scalar snapshots. Pointer values
    /// are diagnostic scalars only; no pointed-to object is retained.
    pub render_epoch: u64,
    pub render_context: usize,
    pub render_game_state: usize,
    pub render_engine: usize,
    pub render_type_index: i32,
    pub render_task_count: i32,
    pub render_worker_count: i32,
    pub render_body_started: u32,
    pub render_body_completed: u32,
    pub render_body_max_active: u32,
    pub render_thread_ids: [u32; 4],
    pub render_invalid: bool,
    pub render_entity: u32,
    pub render_entity_component_index: u32,
    pub render_instance_slot: i32,
    pub render_model_id: u32,
    pub render_current_pose_hash: u64,
    pub render_previous_pose_hash: u64,
    /// Raw IEEE-754 bit patterns, in the native matrix's 16-word order.
    /// No column/row or translation convention is inferred by the probe.
    pub render_current_pose_bits: [u32; 16],
    pub render_previous_pose_bits: [u32; 16],
    pub render_instance_valid: u8,
    pub render_allowlist_status: u8,
    pub render_allowlist_valid: bool,
    pub render_allowlist_count: u16,
    pub render_discovery_inspections: u32,
    pub render_selected_pose_inspections: u32,
    pub render_sample_skips: u64,
    pub render_context_misses: u64,
    pub render_pose_inspections: u32,
    /// Road-vehicle transformator observation. Pointer-sized values are
    /// call-local scalar identifiers only; no native pointer is retained.
    pub render_vehicle_owner: u32,
    pub render_vehicle_child: u32,
    pub render_vehicle_component_index: u32,
    pub render_vehicle_slot: i32,
    pub render_vehicle_model_id: u32,
    pub render_vehicle_get_instance: usize,
    pub render_vehicle_started_nanos: u64,
    pub render_vehicle_duration_nanos: u64,
    pub render_vehicle_alpha_bits: u32,
    pub render_vehicle_path_valid_before: u8,
    pub render_vehicle_path_valid_after: u8,
    pub render_vehicle_path_current_before: [u32; 10],
    pub render_vehicle_path_previous_before: [u32; 10],
    pub render_vehicle_path_current_after: [u32; 10],
    pub render_vehicle_path_previous_after: [u32; 10],
    pub render_vehicle_path_result_pointer: usize,
    pub render_vehicle_path_result_word0: u64,
    pub render_vehicle_path_result_word1: u32,
    pub render_vehicle_helper_engine: usize,
    pub render_vehicle_helper_out: usize,
    pub render_vehicle_helper_type_index: u32,
    pub render_vehicle_helper_network: usize,
    pub render_vehicle_helper_move_path: usize,
    pub render_vehicle_helper_alpha_bits: u32,
    pub render_vehicle_helper_calls: u8,
    pub render_vehicle_finish_definition: usize,
    pub render_vehicle_finish_output: usize,
    pub render_vehicle_finish_alpha_bits: u32,
    pub render_vehicle_finish_return: usize,
    pub render_vehicle_finish_calls: u8,
    pub render_vehicle_world_records_total: u16,
    pub render_vehicle_world_records_copied: u8,
    pub render_vehicle_world_records_truncated: bool,
    /// Applied-only road-vector experiment marker and fixed world-X offset.
    pub render_vehicle_offset_x_bits: u32,
    /// Up to four records, each six raw IEEE-754 words (position then
    /// direction); preserves native order without claiming a matrix layout.
    pub render_vehicle_world_points: [u32; 24],
    pub render_vehicle_invalid_reason: u8,
    pub render_vehicle_samples_total: u64,
    pub render_vehicle_sample_skips: u64,
    pub render_vehicle_refused_total: u64,
    /// Exact-build native GameTime observations. These values are labels and
    /// identities only; they never alter simulation or renderer state.
    pub native_time_before: i64,
    pub native_time_after: i64,
    pub native_time_delta: i64,
    pub native_time_interp_return: i64,
    pub native_time_alpha_bits: u32,
    pub native_time_prior_valid: u8,
    pub native_time_current_calls: u8,
    pub native_time_previous_calls: u8,
    pub native_time_rejection: u8,
    pub native_time_label_valid: bool,
    pub native_time_update_before: u32,
    pub native_time_update_after: u32,
    pub native_time_tick_before: u32,
    pub native_time_tick_after: u32,
    pub native_time_selected_updates: u32,
    pub native_time_first_step: u64,
    pub native_time_last_step: u64,
    pub native_time_previous_step: u64,
    pub native_time_current_step: u64,
    pub native_time_world_epoch: u64,
    pub native_time_sample_nanos: u64,
    pub native_time_current_engine: usize,
    pub native_time_previous_engine: usize,
    pub native_time_current_entity_ptr: usize,
    pub native_time_previous_entity_ptr: usize,
    pub native_time_current_entity_id: u32,
    pub native_time_previous_entity_id: u32,
    pub native_time_helper_engine: usize,
    pub native_time_writer_version_start: u64,
    pub native_time_writer_version_end: u64,
    pub native_time_writer_active_start: u32,
    pub native_time_writer_active_end: u32,
    /// GameState and Engine seen by the simulation worker, for safe
    /// writer/renderer correlation without following entity-generation data.
    pub native_game_state: usize,
    pub native_engine: usize,
    /// Source/destination lineage observed at GameState::Replicate. These
    /// pointer values are identities only; the row contains no native data.
    pub native_replica_source_state: usize,
    pub native_replica_source_engine: usize,
    pub native_replica_destination_state: usize,
    pub native_replica_destination_engine: usize,
    pub native_replica_room_step: u64,
    pub native_replica_family_revision: u64,
    pub native_replica_accepted: bool,
    /// Bounded room-step coordinates used by the private-pose experiment.
    pub road_history_target_step_bits: u64,
    pub road_history_lower_step_bits: u64,
    pub road_history_upper_step_bits: u64,
    pub road_history_incarnation: u64,
    pub road_history_record_count: u8,
    pub road_history_rejection: u8,
}

impl Fields {
    /// Encodes an action variant's public kind name without allocating.
    pub fn action_class_name(name: &str) -> [u8; 32] {
        let mut bytes = [0; 32];
        let count = name.len().min(bytes.len());
        bytes[..count].copy_from_slice(&name.as_bytes()[..count]);
        bytes
    }

    /// Samples only atomically published room/worker data. Call this from a
    /// record builder, which runs only while a capture is active.
    pub fn pacing_snapshot() -> Self {
        let snapshot = crate::pacing::current();
        let completed = crate::pacing::completion_totals();
        Self {
            pacing_active: snapshot.active,
            room_speed_percent: snapshot.speed.0,
            previous_room_speed_percent: snapshot.previous_speed.0,
            room_steps_per_second: snapshot.steps_per_second,
            sealed_ahead: snapshot.sealed_ahead,
            checkpoint_limited: snapshot.checkpoint_limited,
            completed_updates: completed.updates,
            completed_step_wall_nanos: completed.step_wall_nanos,
            active_worker_calls: crate::pacing::active_worker_calls(),
            scheduler_debt: crate::pacing::scheduler_debt_pending(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event {
    sequence: u64,
    timestamp_nanos: u64,
    thread_id: u32,
    kind: u8,
    call_id: u64,
    parent_id: u64,
    fields: Fields,
}

#[derive(Debug)]
struct CaptureBuffer {
    capacity: usize,
    events: Vec<Event>,
}

impl CaptureBuffer {
    fn new() -> Self {
        Self {
            capacity: EVENT_CAPACITY,
            events: Vec::with_capacity(EVENT_CAPACITY),
        }
    }

    fn clear(&mut self) {
        self.events.clear();
    }

    fn push(&mut self, event: Event) -> bool {
        if self.events.len() >= self.capacity {
            return false;
        }
        self.events.push(event);
        true
    }
}

fn events() -> &'static Mutex<CaptureBuffer> {
    EVENTS.get_or_init(|| Mutex::new(CaptureBuffer::new()))
}

/// Whether a fixed-duration capture is currently accepting events.
#[inline]
pub fn active() -> bool {
    ACTIVE_GENERATION.load(Ordering::Acquire) != 0
}

/// Capture generation, or zero when the fixed-duration timeline is inactive.
#[inline]
pub fn current_generation() -> u64 {
    ACTIVE_GENERATION.load(Ordering::Acquire)
}

/// Stable native thread identifier used by every timeline record.
#[inline]
pub fn thread_id() -> u32 {
    trace_thread_id()
}

/// Samples the simulation worker's GameState and Engine pointers only while a
/// capture is active. The returned pointers are values for correlation, not
/// references and never escape into a cache.
#[inline]
pub fn native_world_fields(this: usize) -> (usize, usize) {
    let Some(game_state_address) = this.checked_add(8) else {
        return (0, 0);
    };
    let Some(game_state) = crate::image::guarded::read::<usize>(game_state_address) else {
        return (0, 0);
    };
    let Some(engine_address) = game_state.checked_add(0x18) else {
        return (game_state, 0);
    };
    let engine = crate::image::guarded::read::<usize>(engine_address).unwrap_or(0);
    (game_state, engine)
}

/// Encodes all native matrix words without choosing a translation convention.
fn pose_bits_hex(bits: &[u32; 16]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(16 * 8);
    for word in bits {
        let _ = write!(encoded, "{word:08x}");
    }
    encoded
}

fn words_hex<const N: usize>(bits: &[u32; N]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(N * 8);
    for word in bits {
        let _ = write!(encoded, "{word:08x}");
    }
    encoded
}

fn epoch() -> Instant {
    *CLOCK_EPOCH.get_or_init(Instant::now)
}

#[inline]
fn timestamp_nanos() -> u64 {
    u64::try_from(epoch().elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "QueryPerformanceFrequency"]
    fn query_performance_frequency_native(frequency: *mut i64) -> i32;
    #[link_name = "QueryPerformanceCounter"]
    fn query_performance_counter_native(counter: *mut i64) -> i32;
}

#[cfg(windows)]
fn query_performance_frequency() -> Option<i64> {
    let mut frequency = 0_i64;
    // SAFETY: the Windows API writes one LARGE_INTEGER to this live pointer.
    (unsafe { query_performance_frequency_native(&mut frequency) } != 0 && frequency > 0)
        .then_some(frequency)
}

#[cfg(not(windows))]
fn query_performance_frequency() -> Option<i64> {
    None
}

#[cfg(windows)]
fn query_performance_counter() -> Option<i64> {
    let mut counter = 0_i64;
    // SAFETY: the Windows API writes one LARGE_INTEGER to this live pointer.
    (unsafe { query_performance_counter_native(&mut counter) } != 0).then_some(counter)
}

#[cfg(not(windows))]
fn query_performance_counter() -> Option<i64> {
    None
}

fn permits_event_at_count(kind: Kind, count: usize) -> bool {
    !matches!(
        kind,
        Kind::RoadVehicleSample
            | Kind::RoadOffsetApplied
            | Kind::RoadHistoryApplied
            | Kind::RoadHistoryFallback
            | Kind::RoadHistorySample
    ) || count < EVENT_CAPACITY - ROAD_EVENT_RESERVE
}

/// Returns the timeline's monotonic timestamp domain. It does not arm the
/// watcher or read any native data.
#[inline]
pub fn monotonic_nanos() -> u64 {
    timestamp_nanos()
}

#[cfg(windows)]
fn trace_thread_id() -> u32 {
    // SAFETY: GetCurrentThreadId has no preconditions and is diagnostic-only.
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(windows))]
thread_local! {
    static FALLBACK_THREAD_ID: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(not(windows))]
fn trace_thread_id() -> u32 {
    static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
    FALLBACK_THREAD_ID.with(|id| {
        let current = id.get();
        if current != 0 {
            current
        } else {
            let fresh = u32::try_from(NEXT_THREAD.fetch_add(1, Ordering::Relaxed))
                .unwrap_or(u32::MAX)
                .max(1);
            id.set(fresh);
            fresh
        }
    })
}

thread_local! {
    static OUTER_CALL_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static WORKER_CALL_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SYNC_CALL_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Restores the prior outer call even if the original engine function exits
/// unusually. This scope is created only after an entry was captured.
pub struct OuterCallScope(u64);

impl OuterCallScope {
    pub fn enter(call_id: Option<u64>) -> Option<Self> {
        let call_id = call_id?;
        let previous = OUTER_CALL_ID.with(|slot| slot.replace(call_id));
        Some(Self(previous))
    }
}

impl Drop for OuterCallScope {
    fn drop(&mut self) {
        OUTER_CALL_ID.with(|slot| slot.set(self.0));
    }
}

/// Restores the prior simulation callback when the gate returns.
pub struct WorkerCallScope(u64);

impl WorkerCallScope {
    pub fn enter(call_id: Option<u64>) -> Option<Self> {
        let call_id = call_id?;
        let previous = WORKER_CALL_ID.with(|slot| slot.replace(call_id));
        Some(Self(previous))
    }
}

impl Drop for WorkerCallScope {
    fn drop(&mut self) {
        WORKER_CALL_ID.with(|slot| slot.set(self.0));
    }
}

/// Associates ComputeFrameTime with the Sync call that invokes it.
pub struct SyncCallScope(u64);

impl SyncCallScope {
    pub fn enter(call_id: Option<u64>) -> Option<Self> {
        let call_id = call_id?;
        let previous = SYNC_CALL_ID.with(|slot| slot.replace(call_id));
        Some(Self(previous))
    }
}

impl Drop for SyncCallScope {
    fn drop(&mut self) {
        SYNC_CALL_ID.with(|slot| slot.set(self.0));
    }
}

/// Appends one fixed-size record without waiting on another hook thread.
/// Builders are lazy so an unarmed/inactive call costs one atomic branch and
/// does not read native data or allocate.
#[inline]
pub fn record_with(kind: Kind, call_id: u64, build: impl FnOnce() -> Fields) -> Option<u64> {
    record_with_guard(kind, call_id, || true, build)
}

/// Appends a fixed record only if `guard` still accepts the captured state
/// after the buffer slot is available. This lets a diagnostic decision be
/// committed immediately before its corresponding native call.
#[inline]
pub fn record_with_guard(
    kind: Kind,
    call_id: u64,
    guard: impl FnOnce() -> bool,
    build: impl FnOnce() -> Fields,
) -> Option<u64> {
    let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
    if generation == 0 {
        return None;
    }
    let timestamp_nanos = timestamp_nanos();
    let thread_id = trace_thread_id();
    let fields = build();
    let parent_id = match kind {
        Kind::SyncEntryRaw | Kind::SyncExitRaw | Kind::SyncExitPolicy => {
            OUTER_CALL_ID.with(std::cell::Cell::get)
        }
        Kind::ComputeFrameTime => SYNC_CALL_ID.with(std::cell::Cell::get),
        Kind::RunStepEntry | Kind::RunStepExit => WORKER_CALL_ID.with(std::cell::Cell::get),
        _ => 0,
    };
    let Ok(mut buffer) = events().try_lock() else {
        DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        return None;
    };
    if !permits_event_at_count(kind, buffer.events.len()) {
        DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    if ACTIVE_GENERATION.load(Ordering::Acquire) != generation || !guard() {
        return None;
    }
    if ACTIVE_GENERATION.load(Ordering::Acquire) != generation {
        return None;
    }
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let call_id = if call_id == 0
        && matches!(
            kind,
            Kind::CGameStepEntryRaw
                | Kind::SyncEntryRaw
                | Kind::GameSimStepEntry
                | Kind::RunStepEntry
        ) {
        sequence
    } else {
        call_id
    };
    if !buffer.push(Event {
        sequence,
        timestamp_nanos,
        thread_id,
        kind: kind as u8,
        call_id,
        parent_id,
        fields,
    }) {
        DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(sequence)
}

/// Starts the watcher only when explicitly requested and only after the exact
/// smooth-pacing hooks have installed. The returned bool says it created a
/// watcher thread.
pub fn arm(data_dir: &Path) -> Result<bool, String> {
    if !watch_requested(std::env::var(ENV).ok().as_deref()) {
        return Ok(false);
    }
    if WATCHER_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(false);
    }
    let request = data_dir.join(REQUEST_FILE);
    let initial_stamp = file_stamp(&request);
    let initial_nonce = read_nonce(&request).ok().flatten();
    let _ = epoch();
    let _ = events();
    let thread_request = request.clone();
    let spawned = thread::Builder::new()
        .name("tpf3mp-pacing-timeline".into())
        .spawn(move || watch(thread_request, initial_stamp, initial_nonce));
    match spawned {
        Ok(_handle) => Ok(true),
        Err(error) => {
            WATCHER_STARTED.store(false, Ordering::Release);
            Err(format!("could not start the timeline watcher: {error}"))
        }
    }
}

/// `watch` is deliberately the only accepted mode. Capture length and paths
/// are fixed in code rather than accepted from the file or environment.
pub fn watch_requested(value: Option<&str>) -> bool {
    value == Some("watch")
}

fn watch(request: PathBuf, mut observed: Option<FileStamp>, mut last_nonce: Option<String>) {
    crate::log::line(&format!(
        "smooth timeline watcher armed; change {} with a fresh ASCII nonce to capture five seconds",
        request.display()
    ));
    let mut pending_nonce = None;
    let mut capture: Option<(Instant, String, u64, CaptureClockAnchor)> = None;
    let mut capture_number = 0_u64;
    let mut next_file_poll = Instant::now();
    loop {
        let now = Instant::now();
        if now >= next_file_poll {
            next_file_poll = now + FILE_POLL;
            let stamp = file_stamp(&request);
            if stamp != observed {
                observed = stamp;
                if stamp.is_some() {
                    match read_nonce(&request) {
                        Ok(Some(nonce)) if last_nonce.as_deref() != Some(&nonce) => {
                            last_nonce = Some(nonce.clone());
                            pending_nonce = Some(nonce);
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => crate::log::line(
                            "smooth timeline request ignored: expected a 1..64 byte ASCII nonce",
                        ),
                        Err(error) => crate::log::line(&format!(
                            "smooth timeline request could not be read: {error}"
                        )),
                    }
                }
            }
        }

        if let Some((started, nonce, capture_id, clock_anchor)) = capture.as_ref()
            && started.elapsed() >= CAPTURE_DURATION
        {
            let nonce = nonce.clone();
            let capture_id = *capture_id;
            let clock_anchor = *clock_anchor;
            stop_capture();
            write_capture(&request, &nonce, capture_id, clock_anchor);
            capture = None;
        }
        if capture.is_none()
            && let Some(nonce) = pending_nonce.take()
        {
            capture_number = capture_number.saturating_add(1);
            let capture_id = capture_number;
            let clock_anchor = start_capture(&request);
            capture = Some((Instant::now(), nonce, capture_id, clock_anchor));
            crate::log::line(&format!(
                "smooth timeline capture {capture_id} started; fixed five-second window"
            ));
        }

        thread::sleep(if capture.is_some() {
            CAPTURE_POLL
        } else {
            FILE_POLL
        });
    }
}

fn start_capture(request: &Path) -> CaptureClockAnchor {
    let mut buffer = events().lock().unwrap_or_else(|poison| poison.into_inner());
    buffer.clear();
    DROPPED_EVENTS.store(0, Ordering::Release);
    NEXT_SEQUENCE.store(1, Ordering::Release);
    drop(buffer);
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel).saturating_add(1);

    #[cfg(all(windows, target_arch = "x86_64"))]
    if crate::renderprobe::requested() {
        let allowlist_path = request.with_file_name(crate::renderprobe::ENTITY_FILE);
        let allowlist = crate::renderprobe::load_entity_allowlist(&allowlist_path);
        crate::renderprobe::begin_capture(generation.max(1), allowlist);
        let clock_anchor = activate_capture(generation.max(1));
        let (status, count, valid) = crate::renderprobe::allowlist_info(generation.max(1));
        record_with(Kind::RenderProbeConfig, generation, || Fields {
            result_byte: status as u8,
            render_allowlist_status: status as u8,
            render_allowlist_valid: valid,
            render_allowlist_count: count as u16,
            render_invalid: !valid,
            ..Fields::default()
        });
        return clock_anchor;
    }

    activate_capture(generation.max(1))
}

fn activate_capture(generation: u64) -> CaptureClockAnchor {
    let qpc_frequency = query_performance_frequency().unwrap_or(0);
    let qpc_before = query_performance_counter().unwrap_or(0);
    let native_start_ns = timestamp_nanos();
    ACTIVE_GENERATION.store(generation, Ordering::Release);
    let qpc_after = query_performance_counter().unwrap_or(0);
    CaptureClockAnchor {
        native_start_ns,
        qpc_frequency,
        qpc_before,
        qpc_after,
    }
}

fn stop_capture() {
    // Producers that got the old generation recheck it under the same mutex.
    ACTIVE_GENERATION.store(0, Ordering::Release);
    crate::renderprobe::end_capture();
}

fn write_capture(request: &Path, nonce: &str, capture_id: u64, clock_anchor: CaptureClockAnchor) {
    let (mut events, dropped) = {
        let mut buffer = events().lock().unwrap_or_else(|poison| poison.into_inner());
        let copy = buffer.events.clone();
        buffer.clear();
        (copy, DROPPED_EVENTS.swap(0, Ordering::AcqRel))
    };
    events.sort_unstable_by_key(|event| (event.timestamp_nanos, event.sequence));
    let Some(dir) = request.parent() else {
        crate::log::line("smooth timeline capture not written: data directory has no parent");
        return;
    };
    let path = dir.join(format!(
        "pacing-timeline-{}-{capture_id}.tsv",
        std::process::id()
    ));
    let result = write_capture_file(&path, nonce, clock_anchor, dropped, &events);
    match result {
        Ok(()) => crate::log::line(&format!(
            "smooth timeline capture {capture_id} written to {}; {} events, {dropped} dropped",
            path.display(),
            events.len()
        )),
        Err(error) => crate::log::line(&format!(
            "smooth timeline capture {capture_id} could not be written: {error}"
        )),
    }
}

fn write_capture_file(
    path: &Path,
    nonce: &str,
    clock_anchor: CaptureClockAnchor,
    dropped: u64,
    events: &[Event],
) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut output = BufWriter::new(file);
    writeln!(output, "capture_nonce\t{nonce}")?;
    writeln!(output, "capture_start_ns\t{}", clock_anchor.native_start_ns)?;
    writeln!(
        output,
        "capture_duration_ms\t{}",
        CAPTURE_DURATION.as_millis()
    )?;
    writeln!(output, "events_dropped\t{dropped}")?;
    writeln!(
        output,
        "capture_qpc_frequency\t{}",
        clock_anchor.qpc_frequency
    )?;
    writeln!(output, "capture_qpc_before\t{}", clock_anchor.qpc_before)?;
    writeln!(output, "capture_qpc_after\t{}", clock_anchor.qpc_after)?;
    writeln!(
        output,
        "capture_qpc_bracket_ticks\t{}",
        clock_anchor.bracket_ticks()
    )?;
    writeln!(output, "capture_qpc_valid\t{}", clock_anchor.qpc_valid())?;
    writeln!(
        output,
        "t_ns\tseq\tthread\tevent\tcall_id\tparent_id\tthis\tdata\tnow\tlast\tinterval_us\talpha_milli\targ_a_raw\targ_b_raw\targ_c_raw\tresult_byte\tlegacy_return\tapplied_return\tselected_updates\tselected_known\troom\tpacing_active\troom_speed_pct\tprevious_room_speed_pct\troom_sps\tsealed_ahead\tcheckpoint_limited\tcompleted_updates\tcompleted_step_wall_ns\tactive_worker_calls\telapsed_ns\tscheduler_debt\tevent_seq\tevent_step\taction_class\treplay_token\treplay_step\treplay_action_count\treplay_state\treplay_ack_accepted\tcommand_number_known\trender_epoch\trender_context\trender_game_state\trender_engine\trender_type_index\trender_task_count\trender_worker_count\trender_body_started\trender_body_completed\trender_body_max_active\trender_thread_0\trender_thread_1\trender_thread_2\trender_thread_3\trender_invalid\trender_entity\trender_entity_component_index\trender_instance_slot\trender_model_id\trender_current_pose_hash\trender_previous_pose_hash\trender_current_pose_bits\trender_previous_pose_bits\trender_instance_valid\trender_allowlist_status\trender_allowlist_valid\trender_allowlist_count\trender_discovery_inspections\trender_selected_pose_inspections\trender_sample_skips_total\trender_context_misses_total\trender_pose_inspections\tnative_game_state\tnative_engine\trender_vehicle_owner\trender_vehicle_child\trender_vehicle_component_index\trender_vehicle_slot\trender_vehicle_model_id\trender_vehicle_get_instance\trender_vehicle_started_ns\trender_vehicle_duration_ns\trender_vehicle_alpha_bits\trender_vehicle_path_valid_before\trender_vehicle_path_valid_after\trender_vehicle_path_current_before\trender_vehicle_path_previous_before\trender_vehicle_path_current_after\trender_vehicle_path_previous_after\trender_vehicle_path_result_pointer\trender_vehicle_path_result_word0\trender_vehicle_path_result_word1\trender_vehicle_helper_engine\trender_vehicle_helper_out\trender_vehicle_helper_type_index\trender_vehicle_helper_network\trender_vehicle_helper_move_path\trender_vehicle_helper_alpha_bits\trender_vehicle_helper_calls\trender_vehicle_finish_definition\trender_vehicle_finish_output\trender_vehicle_finish_alpha_bits\trender_vehicle_finish_return\trender_vehicle_finish_calls\trender_vehicle_world_records_total\trender_vehicle_world_records_copied\trender_vehicle_world_records_truncated\trender_vehicle_offset_x_bits\trender_vehicle_world_points\trender_vehicle_invalid_reason\trender_vehicle_samples_total\trender_vehicle_sample_skips_total\trender_vehicle_refused_total\tnative_time_before\tnative_time_after\tnative_time_delta\tnative_time_interp_return\tnative_time_alpha_bits\tnative_time_prior_valid\tnative_time_current_calls\tnative_time_previous_calls\tnative_time_rejection\tnative_time_label_valid\tnative_time_update_before\tnative_time_update_after\tnative_time_tick_before\tnative_time_tick_after\tnative_time_selected_updates\tnative_time_first_step\tnative_time_last_step\tnative_time_previous_step\tnative_time_current_step\tnative_time_world_epoch\tnative_time_sample_ns\tnative_time_current_engine\tnative_time_previous_engine\tnative_time_current_entity_ptr\tnative_time_previous_entity_ptr\tnative_time_current_entity_id\tnative_time_previous_entity_id\tnative_time_helper_engine\tnative_time_writer_version_start\tnative_time_writer_version_end\tnative_time_writer_active_start\tnative_time_writer_active_end\tnative_replica_source_state\tnative_replica_source_engine\tnative_replica_destination_state\tnative_replica_destination_engine\tnative_replica_room_step\tnative_replica_family_revision\tnative_replica_accepted\troad_history_target_step_bits\troad_history_lower_step_bits\troad_history_upper_step_bits\troad_history_incarnation\troad_history_record_count\troad_history_rejection"
    )?;
    for event in events {
        let Some(kind) = Kind::from_byte(event.kind) else {
            continue;
        };
        let fields = event.fields;
        writeln!(
            output,
            "{t}\t{seq}\t{thread}\t{kind}\t{call}\t{parent}\t{this:#x}\t{data:#x}\t{now}\t{last}\t{interval}\t{alpha}\t{arg_a:#x}\t{arg_b:#x}\t{arg_c:#x}\t{result}\t{legacy}\t{applied}\t{updates}\t{known}\t{room}\t{active}\t{speed}\t{previous_speed}\t{sps}\t{ahead}\t{checkpoint}\t{completed}\t{worker_wall}\t{workers}\t{elapsed}\t{debt}\t{event_seq}\t{event_step}\t{action_class}\t{replay_token}\t{replay_step}\t{replay_action_count}\t{replay_state}\t{replay_ack_accepted}\t{command_number_known}\t{render_epoch}\t{render_context:#x}\t{render_game_state:#x}\t{render_engine:#x}\t{render_type_index}\t{render_task_count}\t{render_worker_count}\t{render_body_started}\t{render_body_completed}\t{render_body_max_active}\t{render_thread_0}\t{render_thread_1}\t{render_thread_2}\t{render_thread_3}\t{render_invalid}\t{render_entity}\t{render_entity_component_index}\t{render_instance_slot}\t{render_model_id}\t{render_current_pose_hash}\t{render_previous_pose_hash}\t{render_current_pose_bits}\t{render_previous_pose_bits}\t{render_instance_valid}\t{render_allowlist_status}\t{render_allowlist_valid}\t{render_allowlist_count}\t{render_discovery_inspections}\t{render_selected_pose_inspections}\t{render_sample_skips_total}\t{render_context_misses_total}\t{render_pose_inspections}\t{native_game_state:#x}\t{native_engine:#x}\t{vehicle_owner}\t{vehicle_child}\t{vehicle_component}\t{vehicle_slot}\t{vehicle_model}\t{vehicle_get_instance:#x}\t{vehicle_started}\t{vehicle_duration}\t{vehicle_alpha_bits:08x}\t{path_valid_before}\t{path_valid_after}\t{path_current_before}\t{path_previous_before}\t{path_current_after}\t{path_previous_after}\t{path_result_pointer:#x}\t{path_result_word0:016x}\t{path_result_word1:08x}\t{helper_engine:#x}\t{helper_out:#x}\t{helper_type_index}\t{helper_network:#x}\t{helper_move_path:#x}\t{helper_alpha_bits:08x}\t{helper_calls}\t{finish_definition:#x}\t{finish_output:#x}\t{finish_alpha_bits:08x}\t{finish_return:#x}\t{finish_calls}\t{world_records_total}\t{world_records_copied}\t{world_records_truncated}\t{offset_x_bits:08x}\t{world_points}\t{vehicle_invalid_reason}\t{vehicle_samples_total}\t{vehicle_sample_skips}\t{vehicle_refused}\t{native_time_before}\t{native_time_after}\t{native_time_delta}\t{native_time_interp_return}\t{native_time_alpha_bits:08x}\t{native_time_prior_valid}\t{native_time_current_calls}\t{native_time_previous_calls}\t{native_time_rejection}\t{native_time_label_valid}\t{native_time_update_before}\t{native_time_update_after}\t{native_time_tick_before}\t{native_time_tick_after}\t{native_time_selected_updates}\t{native_time_first_step}\t{native_time_last_step}\t{native_time_previous_step}\t{native_time_current_step}\t{native_time_world_epoch}\t{native_time_sample_nanos}\t{native_time_current_engine:#x}\t{native_time_previous_engine:#x}\t{native_time_current_entity_ptr:#x}\t{native_time_previous_entity_ptr:#x}\t{native_time_current_entity_id}\t{native_time_previous_entity_id}\t{native_time_helper_engine:#x}\t{native_time_writer_version_start}\t{native_time_writer_version_end}\t{native_time_writer_active_start}\t{native_time_writer_active_end}\t{replica_source_state:#x}\t{replica_source_engine:#x}\t{replica_destination_state:#x}\t{replica_destination_engine:#x}\t{replica_room_step}\t{replica_family_revision}\t{replica_accepted}\t{road_history_target_step_bits:016x}\t{road_history_lower_step_bits:016x}\t{road_history_upper_step_bits:016x}\t{road_history_incarnation}\t{road_history_record_count}\t{road_history_rejection}",
            t = event.timestamp_nanos,
            seq = event.sequence,
            thread = event.thread_id,
            kind = kind.as_str(),
            call = event.call_id,
            parent = event.parent_id,
            this = fields.this,
            data = fields.data,
            now = fields.now,
            last = fields.last,
            interval = fields.interval_micros,
            alpha = fields.alpha_milli,
            arg_a = fields.arg_a,
            arg_b = fields.arg_b,
            arg_c = fields.arg_c,
            result = fields.result_byte,
            legacy = fields.legacy_return,
            applied = fields.applied_return,
            updates = fields.selected_updates,
            known = fields.selected_known,
            room = fields.room,
            active = fields.pacing_active,
            speed = fields.room_speed_percent,
            previous_speed = fields.previous_room_speed_percent,
            sps = fields.room_steps_per_second,
            ahead = fields.sealed_ahead,
            checkpoint = fields.checkpoint_limited,
            completed = fields.completed_updates,
            worker_wall = fields.completed_step_wall_nanos,
            workers = fields.active_worker_calls,
            elapsed = fields.elapsed_nanos,
            debt = fields.scheduler_debt,
            event_seq = fields.event_seq,
            event_step = fields.event_step,
            action_class = std::str::from_utf8(&fields.action_class)
                .unwrap_or("")
                .trim_end_matches('\0'),
            replay_token = fields.replay_token,
            replay_step = fields.replay_step,
            replay_action_count = fields.replay_action_count,
            replay_state = fields.replay_state,
            replay_ack_accepted = fields.replay_ack_accepted,
            command_number_known = fields.command_number_known,
            render_epoch = fields.render_epoch,
            render_context = fields.render_context,
            render_game_state = fields.render_game_state,
            render_engine = fields.render_engine,
            render_type_index = fields.render_type_index,
            render_task_count = fields.render_task_count,
            render_worker_count = fields.render_worker_count,
            render_body_started = fields.render_body_started,
            render_body_completed = fields.render_body_completed,
            render_body_max_active = fields.render_body_max_active,
            render_thread_0 = fields.render_thread_ids[0],
            render_thread_1 = fields.render_thread_ids[1],
            render_thread_2 = fields.render_thread_ids[2],
            render_thread_3 = fields.render_thread_ids[3],
            render_invalid = fields.render_invalid,
            render_entity = fields.render_entity,
            render_entity_component_index = fields.render_entity_component_index,
            render_instance_slot = fields.render_instance_slot,
            render_model_id = fields.render_model_id,
            render_current_pose_hash = fields.render_current_pose_hash,
            render_previous_pose_hash = fields.render_previous_pose_hash,
            render_current_pose_bits = pose_bits_hex(&fields.render_current_pose_bits),
            render_previous_pose_bits = pose_bits_hex(&fields.render_previous_pose_bits),
            render_instance_valid = fields.render_instance_valid,
            render_allowlist_status = fields.render_allowlist_status,
            render_allowlist_valid = fields.render_allowlist_valid,
            render_allowlist_count = fields.render_allowlist_count,
            render_discovery_inspections = fields.render_discovery_inspections,
            render_selected_pose_inspections = fields.render_selected_pose_inspections,
            render_sample_skips_total = fields.render_sample_skips,
            render_context_misses_total = fields.render_context_misses,
            render_pose_inspections = fields.render_pose_inspections,
            native_game_state = fields.native_game_state,
            native_engine = fields.native_engine,
            vehicle_owner = fields.render_vehicle_owner,
            vehicle_child = fields.render_vehicle_child,
            vehicle_component = fields.render_vehicle_component_index,
            vehicle_slot = fields.render_vehicle_slot,
            vehicle_model = fields.render_vehicle_model_id,
            vehicle_get_instance = fields.render_vehicle_get_instance,
            vehicle_started = fields.render_vehicle_started_nanos,
            vehicle_duration = fields.render_vehicle_duration_nanos,
            vehicle_alpha_bits = fields.render_vehicle_alpha_bits,
            path_valid_before = fields.render_vehicle_path_valid_before,
            path_valid_after = fields.render_vehicle_path_valid_after,
            path_current_before = words_hex(&fields.render_vehicle_path_current_before),
            path_previous_before = words_hex(&fields.render_vehicle_path_previous_before),
            path_current_after = words_hex(&fields.render_vehicle_path_current_after),
            path_previous_after = words_hex(&fields.render_vehicle_path_previous_after),
            path_result_pointer = fields.render_vehicle_path_result_pointer,
            path_result_word0 = fields.render_vehicle_path_result_word0,
            path_result_word1 = fields.render_vehicle_path_result_word1,
            helper_engine = fields.render_vehicle_helper_engine,
            helper_out = fields.render_vehicle_helper_out,
            helper_type_index = fields.render_vehicle_helper_type_index,
            helper_network = fields.render_vehicle_helper_network,
            helper_move_path = fields.render_vehicle_helper_move_path,
            helper_alpha_bits = fields.render_vehicle_helper_alpha_bits,
            helper_calls = fields.render_vehicle_helper_calls,
            finish_definition = fields.render_vehicle_finish_definition,
            finish_output = fields.render_vehicle_finish_output,
            finish_alpha_bits = fields.render_vehicle_finish_alpha_bits,
            finish_return = fields.render_vehicle_finish_return,
            finish_calls = fields.render_vehicle_finish_calls,
            world_records_total = fields.render_vehicle_world_records_total,
            world_records_copied = fields.render_vehicle_world_records_copied,
            world_records_truncated = fields.render_vehicle_world_records_truncated,
            offset_x_bits = fields.render_vehicle_offset_x_bits,
            world_points = words_hex(&fields.render_vehicle_world_points),
            vehicle_invalid_reason = fields.render_vehicle_invalid_reason,
            vehicle_samples_total = fields.render_vehicle_samples_total,
            vehicle_sample_skips = fields.render_vehicle_sample_skips,
            vehicle_refused = fields.render_vehicle_refused_total,
            native_time_before = fields.native_time_before,
            native_time_after = fields.native_time_after,
            native_time_delta = fields.native_time_delta,
            native_time_interp_return = fields.native_time_interp_return,
            native_time_alpha_bits = fields.native_time_alpha_bits,
            native_time_prior_valid = fields.native_time_prior_valid,
            native_time_current_calls = fields.native_time_current_calls,
            native_time_previous_calls = fields.native_time_previous_calls,
            native_time_rejection = fields.native_time_rejection,
            native_time_label_valid = fields.native_time_label_valid,
            native_time_update_before = fields.native_time_update_before,
            native_time_update_after = fields.native_time_update_after,
            native_time_tick_before = fields.native_time_tick_before,
            native_time_tick_after = fields.native_time_tick_after,
            native_time_selected_updates = fields.native_time_selected_updates,
            native_time_first_step = fields.native_time_first_step,
            native_time_last_step = fields.native_time_last_step,
            native_time_previous_step = fields.native_time_previous_step,
            native_time_current_step = fields.native_time_current_step,
            native_time_world_epoch = fields.native_time_world_epoch,
            native_time_sample_nanos = fields.native_time_sample_nanos,
            native_time_current_engine = fields.native_time_current_engine,
            native_time_previous_engine = fields.native_time_previous_engine,
            native_time_current_entity_ptr = fields.native_time_current_entity_ptr,
            native_time_previous_entity_ptr = fields.native_time_previous_entity_ptr,
            native_time_current_entity_id = fields.native_time_current_entity_id,
            native_time_previous_entity_id = fields.native_time_previous_entity_id,
            native_time_helper_engine = fields.native_time_helper_engine,
            native_time_writer_version_start = fields.native_time_writer_version_start,
            native_time_writer_version_end = fields.native_time_writer_version_end,
            native_time_writer_active_start = fields.native_time_writer_active_start,
            native_time_writer_active_end = fields.native_time_writer_active_end,
            replica_source_state = fields.native_replica_source_state,
            replica_source_engine = fields.native_replica_source_engine,
            replica_destination_state = fields.native_replica_destination_state,
            replica_destination_engine = fields.native_replica_destination_engine,
            replica_room_step = fields.native_replica_room_step,
            replica_family_revision = fields.native_replica_family_revision,
            replica_accepted = fields.native_replica_accepted,
            road_history_target_step_bits = fields.road_history_target_step_bits,
            road_history_lower_step_bits = fields.road_history_lower_step_bits,
            road_history_upper_step_bits = fields.road_history_upper_step_bits,
            road_history_incarnation = fields.road_history_incarnation,
            road_history_record_count = fields.road_history_record_count,
            road_history_rejection = fields.road_history_rejection,
        )?;
    }
    output.flush()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    length: u64,
    modified_nanos: u128,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileStamp {
        length: metadata.len(),
        modified_nanos,
    })
}

fn read_nonce(path: &Path) -> std::io::Result<Option<String>> {
    let mut bytes = [0_u8; MAX_NONCE_BYTES + 3];
    let file = File::open(path)?;
    let mut limited = file.take(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    let mut count = 0;
    while count < bytes.len() {
        let read = limited.read(&mut bytes[count..])?;
        if read == 0 {
            break;
        }
        count += read;
    }
    if count > MAX_NONCE_BYTES + 2 {
        return Ok(None);
    }
    Ok(parse_nonce(&bytes[..count]).map(str::to_owned))
}

fn parse_nonce(bytes: &[u8]) -> Option<&str> {
    let token = bytes
        .strip_suffix(b"\r\n")
        .or_else(|| bytes.strip_suffix(b"\n"))
        .unwrap_or(bytes);
    if token.is_empty() || token.len() > MAX_NONCE_BYTES {
        return None;
    }
    if !token
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    std::str::from_utf8(token).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn watcher_requires_the_explicit_watch_value() {
        assert!(watch_requested(Some("watch")));
        assert!(!watch_requested(None));
        assert!(!watch_requested(Some("1")));
        assert!(!watch_requested(Some("true")));
        assert!(!watch_requested(Some("Watch")));
    }

    #[test]
    fn observe_only_mode_installs_readers_without_enabling_the_pacing_policy() {
        let mode = HookMode::from_options(false, true);
        assert_eq!(mode, HookMode::ObserveOnly);
        assert!(mode.installs_observers());
        assert!(!mode.changes_pacing());
        assert_eq!(HookMode::from_options(false, false), HookMode::Disabled);
        assert_eq!(HookMode::from_options(true, false), HookMode::Pacing);
    }

    #[test]
    fn nonce_is_short_ascii_and_contains_no_path_or_log_delimiters() {
        assert_eq!(parse_nonce(b"run-2026_10\r\n"), Some("run-2026_10"));
        assert!(parse_nonce(b"").is_none());
        assert!(parse_nonce(b"bad/nonce").is_none());
        assert!(parse_nonce(b"bad\tnonce").is_none());
        assert!(parse_nonce(&[b'a'; MAX_NONCE_BYTES + 1]).is_none());
    }

    #[test]
    fn event_buffer_never_grows_past_its_preallocated_bound() {
        let mut buffer = CaptureBuffer {
            capacity: 2,
            events: Vec::with_capacity(2),
        };
        let event = Event {
            sequence: 1,
            timestamp_nanos: 1,
            thread_id: 1,
            kind: Kind::SyncEntryRaw as u8,
            call_id: 0,
            parent_id: 0,
            fields: Fields::default(),
        };
        assert!(buffer.push(event));
        assert!(buffer.push(event));
        assert!(!buffer.push(event));
        assert_eq!(buffer.events.len(), 2);
        assert_eq!(buffer.events.capacity(), 2);
    }

    #[test]
    fn road_pose_rows_leave_bounded_capacity_for_later_timeline_markers() {
        assert!(permits_event_at_count(Kind::RoadVehicleSample, 0));
        assert!(permits_event_at_count(
            Kind::RoadVehicleSample,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE - 1,
        ));
        assert!(!permits_event_at_count(
            Kind::RoadVehicleSample,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE,
        ));
        assert!(permits_event_at_count(
            Kind::ReplayAcknowledged,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE,
        ));
        assert!(permits_event_at_count(
            Kind::RoadOffsetApplied,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE - 1,
        ));
        assert!(!permits_event_at_count(
            Kind::RoadOffsetApplied,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE,
        ));
    }

    #[test]
    fn disabled_record_path_does_not_construct_fields() {
        assert_eq!(ACTIVE_GENERATION.load(Ordering::Acquire), 0);
        let built = AtomicBool::new(false);
        let record = record_with(Kind::ReplayStaged, 0, || {
            built.store(true, Ordering::Relaxed);
            Fields::default()
        });
        assert_eq!(record, None);
        assert!(!built.load(Ordering::Relaxed));
    }

    #[test]
    fn action_class_name_is_fixed_size_ascii() {
        let class = Fields::action_class_name("NotificationSeen");
        assert_eq!(class.len(), 32);
        assert_eq!(
            std::str::from_utf8(&class).unwrap().trim_end_matches('\0'),
            "NotificationSeen"
        );
    }

    #[test]
    fn pose_bits_keep_all_sixteen_native_words_in_order() {
        let bits = std::array::from_fn(|index| index as u32);
        let encoded = pose_bits_hex(&bits);
        assert_eq!(encoded.len(), 16 * 8);
        assert!(encoded.starts_with("000000000000000100000002"));
        assert!(encoded.ends_with("0000000e0000000f"));
    }

    #[test]
    fn renderer_event_kinds_round_trip_to_their_output_names() {
        for (kind, name) in [
            (Kind::RenderUpdateEntry, "render_update_entry"),
            (Kind::RenderModelListBinding, "render_model_list_binding"),
            (Kind::RenderDispatchEntry, "render_dispatch_entry"),
            (Kind::RenderDispatchExit, "render_dispatch_exit"),
            (Kind::RenderPoseSample, "render_pose_sample"),
            (Kind::RenderProbeRefused, "render_probe_refused"),
            (Kind::RenderUpdateExit, "render_update_exit"),
            (Kind::RenderProbeConfig, "render_probe_config"),
            (Kind::RoadVehicleSample, "road_vehicle_sample"),
            (Kind::RoadOffsetApplied, "road_offset_applied"),
        ] {
            assert_eq!(Kind::from_byte(kind as u8), Some(kind));
            assert_eq!(kind.as_str(), name);
        }
    }

    #[test]
    fn renderer_tsv_rows_match_the_bounded_schema() {
        let unique = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tpf3mp-render-timeline-{}-{unique}.tsv",
            std::process::id()
        ));
        let event = Event {
            sequence: 1,
            timestamp_nanos: 2,
            thread_id: 3,
            kind: Kind::RoadVehicleSample as u8,
            call_id: 4,
            parent_id: 0,
            fields: Fields {
                render_current_pose_bits: std::array::from_fn(|index| index as u32),
                render_previous_pose_bits: [0x3f80_0000; 16],
                render_vehicle_owner: 77,
                render_vehicle_child: 88,
                render_vehicle_path_current_before: [0x3f80_0000; 10],
                render_vehicle_world_points: [0x4000_0000; 24],
                render_vehicle_offset_x_bits: 0x40a0_0000,
                ..Fields::default()
            },
        };
        let applied = Event {
            sequence: 2,
            timestamp_nanos: 4,
            thread_id: 3,
            kind: Kind::RoadOffsetApplied as u8,
            call_id: 5,
            parent_id: 0,
            fields: Fields {
                render_vehicle_owner: 77,
                render_vehicle_child: 88,
                render_vehicle_offset_x_bits: 0x40a0_0000,
                render_vehicle_world_points: [0x4000_0000; 24],
                ..Fields::default()
            },
        };
        write_capture_file(
            &path,
            "schema",
            CaptureClockAnchor {
                native_start_ns: 1,
                qpc_frequency: 10_000_000,
                qpc_before: 50_000,
                qpc_after: 50_010,
            },
            0,
            &[event, applied],
        )
        .unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        let mut lines = contents.lines();
        let header = lines
            .find(|line| line.starts_with("t_ns\tseq\t"))
            .unwrap()
            .split('\t')
            .count();
        let row = lines.next().unwrap().split('\t').count();
        let applied_row = lines.next().unwrap().split('\t').count();
        let _ = fs::remove_file(path);
        assert_eq!(row, header);
        assert_eq!(applied_row, header);
        assert!(header > 60);
        assert!(contents.contains("road_vehicle_sample"));
        assert!(contents.contains("road_offset_applied"));
        assert!(contents.contains("\t77\t88\t"));
        assert!(contents.contains("40000000"));
        assert!(contents.contains("40a00000"));
        assert!(contents.contains("capture_start_ns\t1\n"));
        assert!(contents.contains("capture_qpc_frequency\t10000000\n"));
        assert!(contents.contains("capture_qpc_before\t50000\n"));
        assert!(contents.contains("capture_qpc_after\t50010\n"));
        assert!(contents.contains("capture_qpc_bracket_ticks\t10\n"));
        assert!(contents.contains("capture_qpc_valid\ttrue\n"));
    }

    #[test]
    fn capture_clock_anchor_reports_invalid_qpc_order_without_guessing() {
        let anchor = CaptureClockAnchor {
            native_start_ns: 123,
            qpc_frequency: 10_000_000,
            qpc_before: 900,
            qpc_after: 899,
        };
        assert_eq!(anchor.bracket_ticks(), 0);
        assert!(!anchor.qpc_valid());
    }

    #[test]
    fn road_offset_marker_is_bounded_and_disabled_without_a_capture() {
        assert_eq!(ACTIVE_GENERATION.load(Ordering::Acquire), 0);
        let built = AtomicBool::new(false);
        assert_eq!(
            record_with_guard(
                Kind::RoadOffsetApplied,
                1,
                || true,
                || {
                    built.store(true, Ordering::Relaxed);
                    Fields::default()
                }
            ),
            None
        );
        assert!(!built.load(Ordering::Relaxed));
        assert!(permits_event_at_count(
            Kind::RoadOffsetApplied,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE - 1,
        ));
        assert!(!permits_event_at_count(
            Kind::RoadOffsetApplied,
            EVENT_CAPACITY - ROAD_EVENT_RESERVE,
        ));
    }
}
