//! Read-only PE32+ import discovery for the Windows entitlement adapter.
//! This parses an image mapped in memory, not arbitrary disk section offsets.
//! Import names narrow interception to QByteArray rather than shared Qt code.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportSlot {
    pub offset: usize,
    pub symbol: String,
}

pub fn byte_array_imports(image: &[u8]) -> Result<Vec<ImportSlot>, &'static str> {
    fn u16_at(b: &[u8], i: usize) -> Result<u16, &'static str> {
        Ok(u16::from_le_bytes(
            b.get(i..i.checked_add(2).ok_or("PE overflow")?)
                .ok_or("Truncated PE")?
                .try_into()
                .unwrap(),
        ))
    }
    fn u32_at(b: &[u8], i: usize) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            b.get(i..i.checked_add(4).ok_or("PE overflow")?)
                .ok_or("Truncated PE")?
                .try_into()
                .unwrap(),
        ))
    }
    fn u64_at(b: &[u8], i: usize) -> Result<u64, &'static str> {
        Ok(u64::from_le_bytes(
            b.get(i..i.checked_add(8).ok_or("PE overflow")?)
                .ok_or("Truncated PE")?
                .try_into()
                .unwrap(),
        ))
    }
    fn name(b: &[u8], i: usize) -> Result<&str, &'static str> {
        let tail = b.get(i..).ok_or("Invalid import name RVA")?;
        let end = tail
            .iter()
            .take(1024)
            .position(|c| *c == 0)
            .ok_or("Unterminated import name")?;
        std::str::from_utf8(&tail[..end]).map_err(|_| "Invalid import name")
    }
    if image.get(..2) != Some(b"MZ") {
        return Err("Not a PE image");
    }
    let pe = u32_at(image, 0x3c)? as usize;
    if image.get(pe..pe.checked_add(4).ok_or("PE overflow")?) != Some(b"PE\0\0")
        || u16_at(image, pe + 4)? != 0x8664
        || u16_at(image, pe + 24)? != 0x20b
    {
        return Err("Unsupported PE architecture");
    }
    let optional_size = u16_at(image, pe + 20)? as usize;
    if optional_size < 128 || u32_at(image, pe + 24 + 108)? < 2 {
        return Err("Missing PE import directory");
    }
    let image_size = u32_at(image, pe + 24 + 56)? as usize;
    if image_size > image.len() || image_size == 0 {
        return Err("Incomplete mapped image");
    }
    let image = &image[..image_size];
    let imports = u32_at(image, pe + 24 + 120)? as usize;
    let size = u32_at(image, pe + 24 + 124)? as usize;
    if imports == 0 && size == 0 {
        return Ok(vec![]);
    }
    let end = imports
        .checked_add(size)
        .filter(|e| *e <= image.len())
        .ok_or("Invalid import directory")?;
    let mut result = Vec::new();
    let mut terminated = false;
    for descriptor in (imports..end).step_by(20).take(4096) {
        if descriptor + 20 > end {
            return Err("Truncated import descriptor");
        }
        if image[descriptor..descriptor + 20].iter().all(|b| *b == 0) {
            terminated = true;
            break;
        }
        let names = u32_at(image, descriptor)? as usize;
        let dll = u32_at(image, descriptor + 12)? as usize;
        let iat = u32_at(image, descriptor + 16)? as usize;
        if !name(image, dll)?.eq_ignore_ascii_case("Qt5Core.dll") {
            continue;
        }
        if names == 0 || iat == 0 || !iat.is_multiple_of(8) {
            return Err("Unsupported Qt import table");
        }
        let mut thunk_terminated = false;
        for index in 0..65536usize {
            let entry = u64_at(image, names + index * 8)?;
            if entry == 0 {
                thunk_terminated = true;
                break;
            }
            let offset = iat + index * 8;
            u64_at(image, offset)?;
            if entry >> 63 != 0 {
                continue;
            }
            let rva = usize::try_from(entry).map_err(|_| "Import RVA overflow")?;
            let symbol = name(image, rva.checked_add(2).ok_or("Import RVA overflow")?)?;
            if matches!(
                symbol,
                "?constData@QByteArray@@QEBAPEBDXZ" | "?data@QByteArray@@QEBAPEBDXZ"
            ) {
                result.push(ImportSlot {
                    offset,
                    symbol: symbol.to_owned(),
                });
            }
        }
        if !thunk_terminated {
            return Err("Unterminated Qt import table");
        }
    }
    if !terminated {
        return Err("Unterminated import directory");
    }
    Ok(result)
}
