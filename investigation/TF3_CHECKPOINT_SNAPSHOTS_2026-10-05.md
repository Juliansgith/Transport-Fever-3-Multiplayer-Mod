# Bulk checkpoint snapshots, build 40408

The checkpoint accelerator reads **owned Lua snapshots**, not the live ECS
registry. `api.engine.getComponent` chooses a borrowed component reference;
`api.type.BaseEdge.new(component)` / `BaseNodeConfig.new(component)` copy it
at the checkpoint. Both copy constructors were exercised in the real game.
The hook only replaces the expensive nested Luabridge
property reads. Every checkpoint still reads the whole network.

The executable remains pinned by the compiled profile's SHA-256. All offsets
below are RVAs in that executable, observed with the read-only tpfre database
`investigation/dayone-2026-09-29/TransportFever3.tpfdb`.

| Evidence | RVA | Observation |
|---|---:|---|
| `lua_touserdata` | `0x2fbef50` | Calls index2addr, accepts tag 2 or 7; adds `0x28` to a full userdata's GC address |
| BaseEdge userdata constructor | `0x17776a0` | Allocates `0x128`, stores vtable `0x374f800`, constructs BaseEdge at `+0x10`, sets its pointer at `+8` to that inline payload |
| BaseNodeConfig userdata constructor | `0x1754450` | Allocates `0x88`, stores vtable `0x374f840`, constructs BaseNodeConfig at `+0x10` |

The hook accepts only full userdata with the exact expected vtable and a
payload pointer equal to its own address plus `0x10`. It keeps that userdata
on the Lua stack while reading. No pointer or decoded world data survives the
callback. Vector bounds, readable memory, finite floats and boolean fields
are checked. Unsupported types/layouts return nil and use the existing Lua
reader; they never substitute an empty successful checkpoint.

BaseEdge's `laneConfigs` vector is at payload `+0x70`, stride `0x18`:
speed, width and height are f32 at `+0`, `+4`, `+8`; forward is a byte at
`+0xc`; the transport-mode bits are u32 at `+0x10`; offset is f32 at `+0x14`.
The hook emits the same sorted lane strings, including reverse direction,
signed zero, three-decimal formatting and mode bits 0 through 15. Tests compare
float formatting against a real Lua for exact ties and 2,000 deterministic
f32 bit patterns.

BaseNodeConfig is `0x78` bytes. Its decoder is shared with the already-tested
junction tool decoder, which handles connections, crosswalk hash-set iteration
and traffic-light phases. A standalone snapshot has no entity suffix; the
proposal wrapper alone reads that suffix at `+0x78`. Lua receives an ordinary
table and keeps the existing canonical junction-row construction, phase-index
mapping and resource-name resolution.

The optional `lua_touserdata` profile target is address-only, never detoured.
An older external profile without it retains the Lua path. This adds no wire
fields and changes neither checkpoint cadence nor the hash algorithm.

Live inspection also confirmed that the original reference uses the common
`UserdataPtr` vtable (`0x36ee848`) and an external payload pointer. It is
deliberately rejected: that shared vtable alone cannot prove the component's
type or ownership. Only the typed, inline copy reaches the native decoder.
