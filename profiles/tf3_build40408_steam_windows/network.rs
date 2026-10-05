//! Owned Luabridge component snapshots, build 40408 only.
//! Constructors 0x17776a0 (BaseEdge) and 0x1754450 (BaseNodeConfig)
//! allocate UserdataValue<T>: vtable, payload pointer, then inline T.
pub const EDGE_VTABLE: usize = 0x374f800;
pub const NODE_CONFIG_VTABLE: usize = 0x374f840;
pub const PAYLOAD: usize = 0x10;
pub const EDGE_SIZE: usize = 0x118;
pub const LANE_VECTOR: usize = 0x70;
pub const LANE_SIZE: usize = 0x18;
pub const LANE_SPEED: usize = 0;
pub const LANE_WIDTH: usize = 4;
pub const LANE_HEIGHT: usize = 8;
pub const LANE_FORWARD: usize = 0xc;
pub const LANE_MODES: usize = 0x10;
pub const LANE_OFFSET: usize = 0x14;
pub const NODE_CONFIG_SIZE: usize = 0x78;
