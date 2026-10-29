//! Native save settings for Steam Windows 40425, verified 2026-10-28.
//! Port of silver2127's TPF2 Big Maps save_fast, using the existing TF3
//! rewrite implementation. Only the compressor is patched; the shared
//! level constant and decompressor remain unchanged.

pub const LEVEL_LOAD: &str = "save: PushCompressor level load";
pub const BUFFER_SIZE: &str = "save: PushCompressor buffer size";
/// At RVA 0x32dec4: load level 3 from RVA 0x37e4ad8.
pub const LEVEL_LOAD_BYTES: [u8; 6] = [0x8B, 0x05, 0x0E, 0x6C, 0x4B, 0x03];
pub const LEVEL_ONE_BYTES: [u8; 6] = [0xB8, 0x01, 0x00, 0x00, 0x00, 0x90];
/// At RVA 0x32e024: the compressor's stream buffer size.
pub const BUFFER_BYTES: [u8; 6] = [0x41, 0xB8, 0x80, 0x00, 0x00, 0x00];
pub const BUFFER_64K_BYTES: [u8; 6] = [0x41, 0xB8, 0x00, 0x00, 0x01, 0x00];
