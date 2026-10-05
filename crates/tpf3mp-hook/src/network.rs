//! Bulk reads of owned Lua component snapshots. No live ECS pointers or caches
//! of world data: the userdata and its vectors stay owned by Lua on its stack.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::build_data::native::network as layout;
use crate::modules::{Memory, i32_at, read, u64_at, vector};

static BASE: AtomicUsize = AtomicUsize::new(0);

/// Called only after the executable has matched the compiled ABI identity.
pub fn install(base: usize) {
    BASE.store(base, Ordering::Release);
}

/// Read permission caching is scoped to an engine update (image::invalidate).
/// The caller additionally keeps the owned userdata alive throughout decoding.
pub(crate) struct SnapshotMemory;

impl Memory for SnapshotMemory {
    fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
        if len == 0 {
            return Some(Vec::new());
        }
        if address == 0 || !crate::image::readable_cached(address, len) {
            return None;
        }
        let mut bytes = vec![0; len];
        // SAFETY: permission checked above; callers retain the immutable owned
        // component snapshot on Lua's stack, including its vector allocations.
        unsafe { std::ptr::copy_nonoverlapping(address as *const u8, bytes.as_mut_ptr(), len) };
        Some(bytes)
    }
}

fn payload(
    memory: &dyn Memory,
    base: usize,
    userdata: usize,
    vtable: usize,
) -> Result<usize, String> {
    if base == 0 || userdata == 0 {
        return Err("native snapshot reader unavailable".into());
    }
    let raw = read(memory, userdata, layout::PAYLOAD, "component userdata")?;
    let expected = base.checked_add(vtable).ok_or("vtable overflow")?;
    let data = userdata
        .checked_add(layout::PAYLOAD)
        .ok_or("payload overflow")?;
    if u64_at(&raw, 0) != expected as u64 || u64_at(&raw, 8) != data as u64 {
        return Err("not an owned component snapshot of this build".into());
    }
    Ok(data)
}

fn lane_rows(
    memory: &dyn Memory,
    address: usize,
    reversed: bool,
) -> Result<(String, usize), String> {
    let raw = read(memory, address, layout::EDGE_SIZE, "edge snapshot")?;
    let (begin, count) = vector(
        &raw,
        layout::LANE_VECTOR,
        layout::LANE_SIZE,
        256,
        "lane configs",
    )?;
    let configs = read(memory, begin, count * layout::LANE_SIZE, "lane configs")?;
    let mut rows = Vec::with_capacity(count);
    for lane in configs.as_chunks::<{ layout::LANE_SIZE }>().0 {
        let number = |offset| -> Result<f64, String> {
            let value = f32::from_bits(i32_at(lane, offset) as u32);
            if !value.is_finite() {
                return Err("non-finite lane setting".into());
            }
            Ok(f64::from(value))
        };
        let forward = match lane[layout::LANE_FORWARD] {
            0 => false,
            1 => true,
            _ => return Err("invalid lane direction".into()),
        };
        let bits = i32_at(lane, layout::LANE_MODES) as u32;
        let modes: String = (0..16)
            .map(|i| if bits & (1 << i) != 0 { '1' } else { '0' })
            .collect();
        rows.push(format!(
            "{:.3}/{:.3}/{:.3}/{:.3}/{}/{}",
            number(layout::LANE_SPEED)?,
            number(layout::LANE_WIDTH)?,
            number(layout::LANE_HEIGHT)?,
            number(layout::LANE_OFFSET)? * if reversed { -1.0 } else { 1.0 },
            forward != reversed,
            modes
        ));
    }
    rows.sort_unstable();
    Ok((rows.join(";"), count))
}

pub(crate) fn lanes(userdata: usize, reversed: bool) -> Result<(String, usize), String> {
    let memory = SnapshotMemory;
    let address = payload(
        &memory,
        BASE.load(Ordering::Acquire),
        userdata,
        layout::EDGE_VTABLE,
    )?;
    lane_rows(&memory, address, reversed)
}

pub(crate) fn junction(userdata: usize) -> Result<tpf3mp_proto::lua::LuaValue, String> {
    let memory = SnapshotMemory;
    let address = payload(
        &memory,
        BASE.load(Ordering::Acquire),
        userdata,
        layout::NODE_CONFIG_VTABLE,
    )?;
    crate::junctions::snapshot(&memory, address)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bytes(Vec<u8>);
    impl Memory for Bytes {
        fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
            if len == 0 {
                return Some(vec![]);
            }
            let at = address.checked_sub(4096)?;
            self.0.get(at..at.checked_add(len)?).map(<[u8]>::to_vec)
        }
    }
    fn word(bytes: &mut [u8], at: usize, value: usize) {
        bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
    fn edge() -> Bytes {
        let mut raw = vec![0; layout::EDGE_SIZE + 2 * layout::LANE_SIZE];
        let begin = 4096 + layout::EDGE_SIZE;
        word(&mut raw, layout::LANE_VECTOR, begin);
        word(
            &mut raw,
            layout::LANE_VECTOR + 8,
            begin + 2 * layout::LANE_SIZE,
        );
        word(
            &mut raw,
            layout::LANE_VECTOR + 16,
            begin + 2 * layout::LANE_SIZE,
        );
        for (i, lane) in raw[layout::EDGE_SIZE..]
            .as_chunks_mut::<{ layout::LANE_SIZE }>()
            .0
            .iter_mut()
            .enumerate()
        {
            for (at, value) in [
                (layout::LANE_SPEED, 13.888889f32),
                (layout::LANE_WIDTH, 3.5),
                (layout::LANE_HEIGHT, 0.125),
                (layout::LANE_OFFSET, if i == 0 { -0.0 } else { 1.25 }),
            ] {
                lane[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            lane[layout::LANE_FORWARD] = i as u8;
            lane[layout::LANE_MODES..layout::LANE_MODES + 4]
                .copy_from_slice(&0x8005u32.to_le_bytes());
        }
        Bytes(raw)
    }

    #[test]
    fn snapshot_lanes_match_lua_including_reverse_and_negative_zero() {
        let lua = mlua::Lua::new();
        let format: mlua::Function = lua.load("return function(s,w,h,o,f,m) return string.format('%.3f/%.3f/%.3f/%.3f/%s/%s',s,w,h,o,tostring(f),m) end").eval().unwrap();
        for reversed in [false, true] {
            let mut expected = Vec::new();
            for (i, offset) in [-0.0f64, 1.25].into_iter().enumerate() {
                expected.push(
                    format
                        .call::<String>((
                            f64::from(13.888889f32),
                            3.5,
                            0.125,
                            offset * if reversed { -1.0 } else { 1.0 },
                            (i == 1) != reversed,
                            "1010000000000001",
                        ))
                        .unwrap(),
                );
            }
            expected.sort();
            assert_eq!(
                lane_rows(&edge(), 4096, reversed).unwrap(),
                (expected.join(";"), 2)
            );
        }
    }

    #[test]
    fn snapshot_float_format_matches_lua_rounding() {
        let lua = mlua::Lua::new();
        let format: mlua::Function = lua
            .load("return function(n) return string.format('%.3f',n) end")
            .eval()
            .unwrap();
        // Exactly representable ties, signed zero, very small/large values,
        // and deterministic f32 bit patterns (the engine stores floats).
        let mut bits = 1234567u32;
        let mut values = vec![0.0f32, -0.0, 0.0625, 0.1875, -0.0625, 1e-30, 1e30];
        for _ in 0..2000 {
            bits = bits.wrapping_mul(1664525).wrapping_add(1013904223);
            values.push(f32::from_bits(bits));
        }
        for value in values.into_iter().filter(|v| v.is_finite()) {
            let value = f64::from(value);
            assert_eq!(format!("{value:.3}"), format.call::<String>(value).unwrap());
        }
    }

    #[test]
    fn snapshot_refuses_foreign_and_borrowed_userdata() {
        let mut memory = Bytes(vec![0; 16]);
        word(&mut memory.0, 0, 0x100000 + layout::EDGE_VTABLE);
        word(&mut memory.0, 8, 4096 + 16);
        assert_eq!(
            payload(&memory, 0x100000, 4096, layout::EDGE_VTABLE).unwrap(),
            4112
        );
        assert!(payload(&memory, 0, 4096, layout::EDGE_VTABLE).is_err());
        assert!(payload(&memory, 0x100000, 4096, layout::NODE_CONFIG_VTABLE).is_err());
        word(&mut memory.0, 8, 1234);
        assert!(payload(&memory, 0x100000, 4096, layout::EDGE_VTABLE).is_err());
    }

    #[test]
    fn snapshot_refuses_malformed_vectors_flags_and_floats() {
        for end in [
            1,
            4096 + layout::EDGE_SIZE + 1,
            4096 + layout::EDGE_SIZE + 257 * layout::LANE_SIZE,
        ] {
            let mut memory = edge();
            word(&mut memory.0, layout::LANE_VECTOR + 8, end);
            assert!(lane_rows(&memory, 4096, false).is_err());
        }
        let mut memory = edge();
        memory.0[layout::EDGE_SIZE + layout::LANE_FORWARD] = 2;
        assert!(lane_rows(&memory, 4096, false).is_err());
        let mut memory = edge();
        memory.0[layout::EDGE_SIZE..layout::EDGE_SIZE + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(lane_rows(&memory, 4096, false).is_err());
        let empty = Bytes(vec![0; layout::EDGE_SIZE]);
        assert_eq!(lane_rows(&empty, 4096, false).unwrap(), (String::new(), 0));
    }

    #[test]
    fn snapshot_junction_needs_no_entity_suffix() {
        use tpf3mp_proto::lua::LuaValue;
        let mut memory = Bytes(vec![0; layout::NODE_CONFIG_SIZE]);
        memory.0[0x68..0x6c].copy_from_slice(&(-1i32).to_le_bytes());
        let value = crate::junctions::snapshot(&memory, 4096).unwrap();
        let LuaValue::Table(fields) = value else {
            panic!("expected table")
        };
        assert!(fields.contains(&(
            LuaValue::string("doubleSlipSwitch"),
            LuaValue::Boolean(false)
        )));
        assert!(
            !fields
                .iter()
                .any(|(key, _)| *key == LuaValue::string("entity"))
        );
        memory.0[0x70] = 2;
        assert!(crate::junctions::snapshot(&memory, 4096).is_err());
    }
}
