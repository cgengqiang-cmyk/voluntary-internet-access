use sha2::{Digest, Sha256};

const MH_MAGIC_64_LE: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;

/// Hash the immutable Mach-O content while excluding the ad-hoc code-signature
/// blob and the __LINKEDIT size fields that `codesign --force` rewrites.
pub fn normalized_macho_sha256(bytes: &[u8]) -> Result<String, &'static str> {
    if bytes.len() < 32 || bytes[..4] != MH_MAGIC_64_LE {
        return Err("expected a thin little-endian 64-bit Mach-O executable");
    }
    let command_count = read_u32(bytes, 16)? as usize;
    let command_bytes = read_u32(bytes, 20)? as usize;
    let command_end = 32_usize
        .checked_add(command_bytes)
        .filter(|end| *end <= bytes.len())
        .ok_or("invalid Mach-O load-command size")?;

    let mut normalized = bytes.to_vec();
    let mut cursor = 32_usize;
    let mut signature_offset = None;
    for _ in 0..command_count {
        if cursor.checked_add(8).is_none_or(|end| end > command_end) {
            return Err("invalid Mach-O load-command table");
        }
        let command = read_u32(bytes, cursor)?;
        let size = read_u32(bytes, cursor + 4)? as usize;
        let next = cursor
            .checked_add(size)
            .filter(|end| size >= 8 && *end <= command_end)
            .ok_or("invalid Mach-O load command")?;

        if command == LC_CODE_SIGNATURE {
            if size < 16 {
                return Err("invalid LC_CODE_SIGNATURE command");
            }
            let offset = read_u32(bytes, cursor + 8)? as usize;
            if offset < command_end || offset > bytes.len() {
                return Err("invalid Mach-O code-signature offset");
            }
            signature_offset = Some(offset);
            normalized[cursor + 8..cursor + 16].fill(0);
        } else if command == LC_SEGMENT_64 && size >= 72 {
            let name = &bytes[cursor + 8..cursor + 24];
            if name.starts_with(b"__LINKEDIT") {
                // vmaddr/vmsize/fileoff/filesize can be adjusted when the
                // signature blob is replaced. They do not describe code.
                normalized[cursor + 24..cursor + 56].fill(0);
            }
        }
        cursor = next;
    }
    if cursor != command_end {
        return Err("Mach-O load-command count does not match its size");
    }
    let signature_offset = signature_offset.ok_or("Mach-O has no code signature")?;
    Ok(hex::encode(Sha256::digest(&normalized[..signature_offset])))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, &'static str> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or("truncated Mach-O integer")?
        .try_into()
        .map_err(|_| "invalid Mach-O integer")?;
    Ok(u32::from_le_bytes(value))
}
