//! The lanes' text hash (`hashStr` in the mod's `scripts/tpf3mp/lanes.lua`)
//! in Rust, for the mod to call as `tpf3mp_native.hash(text)`: the same
//! two multiplicative congruential hashes over the text's bytes, with the
//! same output, but without a Lua loop over every byte. A checkpoint's
//! lanes are megabytes of text on a mid-sized map, and the Lua loop alone
//! took about 150 ms of the game's step there.
//!
//! The Lua computes in doubles; every intermediate value here stays below
//! 2^53 (`h < 2^31`, `h * 48271 + 255 < 2^47`), so integer arithmetic gives
//! exactly the same numbers.

const M1: u64 = 2_147_483_647;
const A1: u64 = 48_271;
const M2: u64 = 2_147_483_629;
const A2: u64 = 40_692;
const SEED: u64 = 2_166_136_261;

/// `hashStr(text)`: `"%010d-%010d"` of the two hashes.
pub fn hash(text: &[u8]) -> String {
    let (mut h1, mut h2) = (SEED % M1, SEED % M2);
    for &b in text {
        h1 = (h1 * A1 + u64::from(b)) % M1;
        h2 = (h2 * A2 + u64::from(b)) % M2;
    }
    format!("{h1:010}-{h2:010}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mod's own function, as lanes.lua has it.
    const LUA: &str = r#"
local M1, A1 = 2147483647, 48271
local M2, A2 = 2147483629, 40692
local function hashStr(s)
	local h1, h2 = 2166136261 % M1, 2166136261 % M2
	for i = 1, #s do
		local b = string.byte(s, i)
		h1 = (h1 * A1 + b) % M1
		h2 = (h2 * A2 + b) % M2
	end
	return string.format("%010d-%010d", h1, h2)
end
return hashStr
"#;

    #[test]
    fn the_mod_still_hashes_with_this_function() {
        let lanes = include_str!("../../../mod/tpf3mp_1/content/scripts/tpf3mp/lanes.lua")
            .replace("\r\n", "\n");
        let body = LUA.trim_start().trim_end_matches("return hashStr\n");
        assert!(
            lanes.contains(body),
            "lanes.lua's hashStr changed: change lanehash.rs with it"
        );
    }

    #[test]
    fn it_hashes_as_the_mods_lua_does() {
        let lua = mlua::Lua::new();
        let lua_hash: mlua::Function = lua.load(LUA).eval().unwrap();
        let mut samples: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"a".to_vec(),
            b"0:0000000000-0000000000".to_vec(),
            (0..=255u8).collect(),
            vec![255; 4096],
        ];
        // Rows as a lane joins them, with the record separator.
        let mut rows = Vec::new();
        for i in 0..2000u32 {
            rows.extend_from_slice(format!("{i}.5,-{i},0>1,2,3:7|lanes:1.000/2.000").as_bytes());
            rows.push(30);
        }
        samples.push(rows);
        for sample in samples {
            let text = lua.create_string(&sample).unwrap();
            let expected: String = lua_hash.call(text).unwrap();
            assert_eq!(hash(&sample), expected, "{} bytes", sample.len());
        }
    }
}
