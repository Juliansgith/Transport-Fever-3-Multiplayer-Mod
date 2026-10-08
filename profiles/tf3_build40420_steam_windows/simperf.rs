//! Native data for Steam Windows build 40420's simulation timers and the
//! faster component lookup (investigation/TF3_SIM_COST_2026-10-05.md,
//! sections 6 and 7). See hooks.toml for the targets' bytes.

/// `ecs::EmissionGridSystem::Update(this, engine, nodes, float dt)`,
/// vtable RVA `0x3705e38` slot 11; reached only through that slot.
pub const EMISSION_GRID: &str = "simperf: EmissionGridSystem::Update";
/// `ecs::EmissionEmitterSystem::Update2(this, engine, int, float dt)`,
/// vtable RVA `0x37057b0` slot 12; reached only through that slot.
pub const EMISSION_EMITTERS: &str = "simperf: EmissionEmitterSystem::Update2";
/// `ecs::TownSystem` slot 12 `(this, engine, int)`, the town update that
/// calls `CalculateTownPollution`; reached only through that slot.
pub const TOWNS: &str = "simperf: TownSystem::Update2";

/// The vtable slot (RVA) holding each system's function: the hook wraps
/// the slot, so a detour of the function itself, by another feature,
/// still runs inside the timer.
pub const EMISSION_GRID_SLOT: u64 = 0x3705e90;
pub const EMISSION_EMITTERS_SLOT: u64 = 0x3705810;
pub const TOWNS_SLOT: u64 = 0x3711f38;

/// `parcel_util::UpdateParcelCollision(rcx, rdx, boxes, r9)`: `boxes` a
/// `std::vector` of `{min x, min y, max x, max y}` (16 bytes); the walk's
/// query box is their union widened by [`PARCEL_MARGIN`] metres.
pub const PARCEL_COLLISION: &str = "simperf: UpdateParcelCollision";
/// Its only call (`construction_util_engine.cpp`, `0x1425fcb9a`).
pub const PARCEL_COLLISION_CALL: &str = "simperf: UpdateParcelCollision call";
/// The margin added to every side of the union (`[0x14368c18c]`, 50.0f).
pub const PARCEL_MARGIN: f32 = 50.0;

/// `ecs::Engine::GetComponentDataIndex(engine, entity, type)`: the
/// signature covers the whole hit path, entry to `ret`.
pub const COMPONENT_INDEX: &str = "fast-component-index: Engine::GetComponentDataIndex";
/// `[engine+0x90]`: the per-entity component lists, one
/// `std::vector<{int type, int index}>` (begin, end, capacity) per entity.
pub const ENTITY_LISTS: usize = 0x90;
/// One entity's list header: three pointers.
pub const ENTITY_LIST_STRIDE: usize = 24;
/// One `{type, index}` pair.
pub const PAIR_SIZE: usize = 8;
