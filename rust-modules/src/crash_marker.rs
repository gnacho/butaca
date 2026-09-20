//! crash_marker - the running image's identity, written into the LOCAL crash log.
//!
//! Purely local debugging infrastructure: `main.c` calls [`plx_crash_write_image_marker`] at
//! startup so the append-only crash log names the binary that was running before any signal
//! arrived. Nothing here sends anything anywhere - there is no reporting left in this build.

/// **Fallback `image_addr` for this binary: the lowest `PT_LOAD` vaddr - `0x10000` - and NOT
/// zero.** Measured on armv7 (2026-08-29). [`image_addr`] derives the real number from
/// `/proc/self/exe`; this constant is only the answer when the read cannot be trusted.
pub(crate) const IMAGE_ADDR: &str = "0x10000";

/// The image base this binary actually links at, as a `0x…` string. Falls back to [`IMAGE_ADDR`]
/// whenever the answer cannot be read honestly (no `/proc`, not an ELF, big-endian, `ET_DYN`).
pub(crate) fn image_addr() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        read_head("/proc/self/exe")
            .as_deref()
            .map(image_addr_of)
            .unwrap_or_else(|| IMAGE_ADDR.to_string())
    })
}

/// The pure half of [`image_addr`], so a test grades the expression the app actually uses.
fn image_addr_of(buf: &[u8]) -> String {
    lowest_load_vaddr(buf)
        .map(|v| format!("0x{v:x}"))
        .unwrap_or_else(|| IMAGE_ADDR.to_string())
}

/// The head of a file - enough to hold an ELF header and every program header after it. Bounded
/// rather than a whole read: this runs at boot on a set with 1.68 GB, and the binary is 7 MB.
fn read_head(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// **The GNU build id of the running binary, as lowercase hex** - read from the binary that is
/// actually running rather than baked in at compile time. Empty when it cannot be read.
pub(crate) fn build_id() -> &'static str {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        read_head("/proc/self/exe")
            .as_deref()
            .and_then(gnu_build_id)
            .unwrap_or_default()
    })
}

/// The mapped span of this executable's loadable ELF segments.
pub(crate) fn image_size() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        read_head("/proc/self/exe")
            .as_deref()
            .and_then(load_span)
            .unwrap_or(0)
    })
}

/// Append the running image's identity to the crash log before any signal can arrive.
///
/// The signal handler itself cannot parse ELF or allocate. This normal-startup marker associates
/// every later fallback record with the binary that actually crashed even if another binary is
/// deployed before the log is read.
#[no_mangle]
pub extern "C" fn plx_crash_write_image_marker(fd: std::os::raw::c_int) {
    if fd < 0 {
        return;
    }
    let line = if build_id().is_empty() {
        // Still delimit this process. Otherwise a rare ELF-read failure would let a later crash
        // inherit the preceding process's perfectly valid, but now wrong, build identity.
        "img: unavailable\n".to_string()
    } else {
        format!(
            "img: build_id={} image_addr={} image_size=0x{:x}\n",
            build_id(),
            image_addr(),
            image_size()
        )
    };
    unsafe {
        let _ = libc::write(fd, line.as_ptr().cast(), line.len());
    }
}

/// Walk an ELF's `PT_NOTE` segments for `NT_GNU_BUILD_ID`. Pure, for the reason
/// [`lowest_load_vaddr`] is.
///
/// Note records are `[namesz][descsz][type]` then the name and the descriptor, each padded to four
/// bytes - and the PADDING is the part that is easy to drop, because `"GNU\0"` is exactly four
/// bytes and so a parser that forgets to round up reads correctly on every ELF anyone would test
/// with and wrongly on the one that has a different note first.
pub(crate) fn gnu_build_id(buf: &[u8]) -> Option<String> {
    const PT_NOTE: u32 = 4;
    const NT_GNU_BUILD_ID: u32 = 3;
    if buf.len() < 64 || &buf[..4] != b"\x7fELF" || buf[5] != 1 {
        return None;
    }
    let is64 = buf[4] == 2;
    let u16at =
        |o: usize| -> Option<u16> { Some(u16::from_le_bytes(buf.get(o..o + 2)?.try_into().ok()?)) };
    let u32at =
        |o: usize| -> Option<u32> { Some(u32::from_le_bytes(buf.get(o..o + 4)?.try_into().ok()?)) };
    let u64at =
        |o: usize| -> Option<u64> { Some(u64::from_le_bytes(buf.get(o..o + 8)?.try_into().ok()?)) };
    let (phoff, phentsize, phnum) = if is64 {
        (
            u64at(32)? as usize,
            u16at(54)? as usize,
            u16at(56)? as usize,
        )
    } else {
        (
            u32at(28)? as usize,
            u16at(42)? as usize,
            u16at(44)? as usize,
        )
    };
    for i in 0..phnum {
        let base = phoff.checked_add(i.checked_mul(phentsize)?)?;
        if u32at(base)? != PT_NOTE {
            continue;
        }
        let (off, size) = if is64 {
            (u64at(base + 8)? as usize, u64at(base + 32)? as usize)
        } else {
            (u32at(base + 4)? as usize, u32at(base + 16)? as usize)
        };
        let mut at = off;
        let end = off.checked_add(size)?;
        while at + 12 <= end && at + 12 <= buf.len() {
            let namesz = u32at(at)? as usize;
            let descsz = u32at(at + 4)? as usize;
            let kind = u32at(at + 8)?;
            let name_at = at + 12;
            let desc_at = name_at.checked_add(namesz.next_multiple_of(4))?;
            let desc_end = desc_at.checked_add(descsz)?;
            if kind == NT_GNU_BUILD_ID && buf.get(name_at..name_at + namesz) == Some(b"GNU\0") {
                let d = buf.get(desc_at..desc_end)?;
                return Some(d.iter().map(|b| format!("{b:02x}")).collect());
            }
            let next = desc_at.checked_add(descsz.next_multiple_of(4))?;
            if next <= at {
                break; // a zero-length note would otherwise spin here forever
            }
            at = next;
        }
    }
    None
}

/// The lowest `PT_LOAD` virtual address in an ELF image. Pure - the caller owns the read, which is
/// what makes every branch here host-testable against a hand-built header.
///
/// `None` for anything this cannot answer honestly: not an ELF, big-endian, `ET_DYN` (see
/// [`image_addr`]), a truncated header, or no `PT_LOAD` at all.
pub(crate) fn lowest_load_vaddr(buf: &[u8]) -> Option<u64> {
    const PT_LOAD: u32 = 1;
    const ET_EXEC: u16 = 2;
    if buf.len() < 64 || &buf[..4] != b"\x7fELF" {
        return None;
    }
    let is64 = match buf[4] {
        1 => false,
        2 => true,
        _ => return None,
    };
    if buf[5] != 1 {
        return None; // big-endian: every field below is read little-endian
    }
    let u16at =
        |o: usize| -> Option<u16> { Some(u16::from_le_bytes(buf.get(o..o + 2)?.try_into().ok()?)) };
    let u32at =
        |o: usize| -> Option<u32> { Some(u32::from_le_bytes(buf.get(o..o + 4)?.try_into().ok()?)) };
    let u64at =
        |o: usize| -> Option<u64> { Some(u64::from_le_bytes(buf.get(o..o + 8)?.try_into().ok()?)) };

    if u16at(16)? != ET_EXEC {
        return None;
    }
    let (phoff, phentsize, phnum) = if is64 {
        (
            u64at(32)? as usize,
            u16at(54)? as usize,
            u16at(56)? as usize,
        )
    } else {
        (
            u32at(28)? as usize,
            u16at(42)? as usize,
            u16at(44)? as usize,
        )
    };
    // `p_vaddr` is the field after `p_offset` in both classes, at different widths and - because
    // ELF64 moves `p_flags` up to second - different offsets. Getting this pair wrong reads a file
    // offset as a load address, which on this binary happens to be a plausible small number.
    let vaddr_at = if is64 { 16 } else { 8 };
    let min_entry = if is64 { 24 } else { 12 };
    if phentsize < min_entry || phnum == 0 {
        return None;
    }
    (0..phnum)
        .filter_map(|i| {
            let base = phoff.checked_add(i.checked_mul(phentsize)?)?;
            if u32at(base)? != PT_LOAD {
                return None;
            }
            if is64 {
                u64at(base + vaddr_at)
            } else {
                u32at(base + vaddr_at).map(u64::from)
            }
        })
        .min()
}

/// Size of the address range covered by all `PT_LOAD` segments in one `ET_EXEC` ELF.
fn load_span(buf: &[u8]) -> Option<u64> {
    const PT_LOAD: u32 = 1;
    const ET_EXEC: u16 = 2;
    if buf.len() < 64 || &buf[..4] != b"\x7fELF" || buf[5] != 1 {
        return None;
    }
    let is64 = match buf[4] {
        1 => false,
        2 => true,
        _ => return None,
    };
    let u16at =
        |o: usize| -> Option<u16> { Some(u16::from_le_bytes(buf.get(o..o + 2)?.try_into().ok()?)) };
    let u32at =
        |o: usize| -> Option<u32> { Some(u32::from_le_bytes(buf.get(o..o + 4)?.try_into().ok()?)) };
    let u64at =
        |o: usize| -> Option<u64> { Some(u64::from_le_bytes(buf.get(o..o + 8)?.try_into().ok()?)) };
    if u16at(16)? != ET_EXEC {
        return None;
    }
    let (phoff, phentsize, phnum) = if is64 {
        (
            u64at(32)? as usize,
            u16at(54)? as usize,
            u16at(56)? as usize,
        )
    } else {
        (
            u32at(28)? as usize,
            u16at(42)? as usize,
            u16at(44)? as usize,
        )
    };
    let (offset_at, filesz_at, min_entry) = if is64 { (8, 32, 24) } else { (4, 16, 12) };
    if phentsize < min_entry || phnum == 0 {
        return None;
    }
    let mut span: Option<u64> = None;
    for i in 0..phnum {
        let base = phoff.checked_add(i.checked_mul(phentsize)?)?;
        if u32at(base)? != PT_LOAD {
            continue;
        }
        let (off, filesz) = if is64 {
            (u64at(base + offset_at)?, u64at(base + filesz_at)?)
        } else {
            (
                u64::from(u32at(base + offset_at)?),
                u64::from(u32at(base + filesz_at)?),
            )
        };
        let end = off.checked_add(filesz)?;
        span = Some(span.map_or(end, |s: u64| s.max(end)));
    }
    span
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 32-bit ET_EXEC ELF header plus one PT_LOAD, hand-assembled - the pure parsers
    /// must answer it without any file I/O.
    fn elf32(loads: &[(u32, u32, u32)]) -> Vec<u8> {
        let phnum = loads.len() as u16;
        let phentsize = 32u16;
        let phoff = 52usize;
        let mut buf = vec![0u8; phoff];
        buf[..4].copy_from_slice(b"\x7fELF");
        buf[4] = 1; // ELFCLASS32
        buf[5] = 1; // little-endian
        buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        buf[28..32].copy_from_slice(&(phoff as u32).to_le_bytes());
        buf[42..44].copy_from_slice(&phentsize.to_le_bytes());
        buf[44..46].copy_from_slice(&phnum.to_le_bytes());
        for (off, vaddr, filesz) in loads {
            let mut ph = vec![0u8; phentsize as usize];
            ph[0..4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
            ph[4..8].copy_from_slice(&off.to_le_bytes());
            ph[8..12].copy_from_slice(&vaddr.to_le_bytes());
            ph[16..20].copy_from_slice(&filesz.to_le_bytes());
            buf.extend_from_slice(&ph);
        }
        buf
    }

    #[test]
    fn lowest_load_vaddr_reads_the_lowest_pt_load() {
        let e = elf32(&[(0, 0x10000, 0x500), (0x600, 0x20000, 0x300)]);
        assert_eq!(lowest_load_vaddr(&e), Some(0x10000));
        let single = elf32(&[(0, 0x8000, 0x1000)]);
        assert_eq!(lowest_load_vaddr(&single), Some(0x8000));
        assert_eq!(lowest_load_vaddr(&[0u8; 64]), None);
        let mut bad = e.clone();
        bad[5] = 2; // big-endian
        assert_eq!(lowest_load_vaddr(&bad), None);
    }

    #[test]
    fn load_span_covers_every_pt_load() {
        let e = elf32(&[(0, 0x10000, 0x500), (0x600, 0x20000, 0x900)]);
        assert_eq!(load_span(&e), Some(0xf00));
    }

    #[test]
    fn image_addr_of_falls_back_to_the_measured_constant() {
        assert_eq!(image_addr_of(&[0u8; 64]), IMAGE_ADDR);
        let e = elf32(&[(0, 0x10000, 0x500)]);
        assert_eq!(image_addr_of(&e), "0x10000");
    }
}
