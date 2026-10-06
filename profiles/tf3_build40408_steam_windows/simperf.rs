//! Native data for Steam Windows build 40408's simulation timers and the
//! faster component lookup (investigation/TF3_BIGMAP_SIM_COST_2026-10-05.md,
//! sections 6 and 7). See hooks.toml for the targets' bytes.

/// `ecs::EmissionGridSystem::Update(this, engine, nodes, float dt)`,
/// vtable `0x1436fcab8` slot 11; reached only through that slot.
pub const EMISSION_GRID: &str = "simperf: EmissionGridSystem::Update";
/// `ecs::EmissionEmitterSystem::Update2(this, engine, int, float dt)`,
/// vtable `0x1436fc430` slot 12; reached only through that slot.
pub const EMISSION_EMITTERS: &str = "simperf: EmissionEmitterSystem::Update2";
/// `ecs::TownSystem` slot 12 `(this, engine, int)`, the town update that
/// calls `CalculateTownPollution`; reached only through that slot.
pub const TOWNS: &str = "simperf: TownSystem::Update2";

/// The vtable slot (RVA) holding each system's function: the hook wraps
/// the slot, so a detour of the function itself, by another feature,
/// still runs inside the timer.
pub const EMISSION_GRID_SLOT: u64 = 0x36fcb10;
pub const EMISSION_EMITTERS_SLOT: u64 = 0x36fc490;
pub const TOWNS_SLOT: u64 = 0x3708bb8;

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

/// The parcel walk's probe (`crate::parcelprobe`): four calls inside the
/// walk, each the only call of its site, and the callee each must reach
/// (RVAs). See hooks.toml for their bytes.
pub const PROBE_NODE_CALL: &str = "parcel-probe: visitor node call";
/// The octree node iterator's dereference: `rcx` the iterator, returns
/// the node.
pub const PROBE_NODE_CALLEE: u64 = 0x2fe150;
pub const PROBE_INDEX_CALL: &str = "parcel-probe: visitor bounding-volume index call";
/// `ecs::Engine::GetComponentDataIndex(engine, entity, type)`.
pub const PROBE_INDEX_CALLEE: u64 = 0xa4b90;
pub const PROBE_STREET_CALL: &str = "parcel-probe: visitor street call";
/// The ParcelSystem's visit of one street's parcels: `rcx` the system,
/// `edx` the street, `r8` a `std::function` called with each parcel.
pub const PROBE_STREET_CALLEE: u64 = 0xae7110;
pub const PROBE_PARCEL_CALL: &str = "parcel-probe: parcel element test call";
/// The element test of one parcel: `rcx` the collision context, `rdx`
/// the Parcel, `r8d`, `r9` the boxes, the fifth argument the result
/// vector of 8-byte `{element, flag}` (begin, end, capacity).
pub const PROBE_PARCEL_CALLEE: u64 = 0x931a70;

/// The octree node component (`EcsOctreeNode`, 0xc0 bytes): its loose box
/// `{min x, y, z, max x, y, z}` and its entities (`std::vector<int>`).
pub const NODE_BOX: usize = 0x8;
pub const NODE_ENTITIES: usize = 0x20;
/// `[engine+0x78]`: one storage per component type, by type index.
pub const ENGINE_STORAGES: usize = 0x78;
/// A storage's dense array, used below [`STORAGE_PAGED_FROM`].
pub const STORAGE_DENSE: usize = 0x68;
/// A storage's pages (16-byte entries, the page's pointer first), of
/// [`STORAGE_PAGE_LEN`] records, for indices from [`STORAGE_PAGED_FROM`].
pub const STORAGE_PAGES: usize = 0x80;
pub const STORAGE_PAGED_FROM: i32 = 0x4000_0000;
pub const STORAGE_PAGE_LEN: i32 = 32;
pub const STORAGE_PAGE_ENTRY: usize = 16;
/// One `BoundingVolume`: `{min x, y, z, max x, y, z}`.
pub const BOUNDING_VOLUME_STRIDE: usize = 24;
