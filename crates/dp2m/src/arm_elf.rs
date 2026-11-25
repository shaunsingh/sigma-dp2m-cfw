//! arm926ej-s BE thumb-1 build helper: invokes clang on .S/.c, extracts .text* from elf32-be, encodes BL/BLX

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn compile_thumb_c(src_path: &Path, include_dir: &Path, defines: &[String]) -> Result<Vec<u8>, String> {
    let obj_path = temp_obj_path(src_path);
    let mut cmd = Command::new("clang");
    cmd.args([
        "-target", "armeb-none-eabi",
        "-mcpu=arm926ej-s",
        "-mthumb",
        "-Os",
        "-ffreestanding",
        "-fno-builtin",
        "-fno-stack-protector",
        "-I",
    ])
    .arg(include_dir);
    for d in defines {
        cmd.arg(format!("-D{}", d));
    }
    let out = cmd
        .arg("-c")
        .arg(src_path)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .map_err(|e| format!("failed to invoke clang: {}", e))?;

    if !out.status.success() {
        let _ = fs::remove_file(&obj_path);
        return Err(format!(
            "clang failed on {}:\n{}",
            src_path.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let elf = fs::read(&obj_path).map_err(|e| format!("read {}: {}", obj_path.display(), e))?;
    let _ = fs::remove_file(&obj_path);
    let secs = extract_elf32_be_sections(&elf)?;

    for (name, data) in &secs {
        if name.starts_with(".rodata") && !data.is_empty() {
            return Err(format!(
                "{} emitted non-empty {} ({} B); only .text is patched into ROM caves",
                src_path.display(),
                name,
                data.len()
            ));
        }
    }
    secs.get(".text")
        .cloned()
        .ok_or_else(|| format!("no .text section in {}", src_path.display()))
}

pub fn assemble_thumb_sections(
    src_path: &Path,
    include_dir: &Path,
) -> Result<HashMap<String, Vec<u8>>, String> {
    let obj_path = temp_obj_path(src_path);
    let out = Command::new("clang")
        .args([
            "-target", "armeb-none-eabi",
            "-mcpu=arm926ej-s",
            "-mthumb",
            "-I",
        ])
        .arg(include_dir)
        .arg("-c")
        .arg(src_path)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .map_err(|e| format!("failed to invoke clang: {}", e))?;

    if !out.status.success() {
        let _ = fs::remove_file(&obj_path);
        return Err(format!(
            "clang asm failed on {}:\n{}",
            src_path.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let elf = fs::read(&obj_path).map_err(|e| format!("read {}: {}", obj_path.display(), e))?;
    let _ = fs::remove_file(&obj_path);
    extract_elf32_be_sections(&elf)
}

fn temp_obj_path(src: &Path) -> std::path::PathBuf {
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("unit");
    std::env::temp_dir().join(format!("dp2m_{}_{}.o", stem, std::process::id()))
}

fn extract_elf32_be_sections(elf: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    if elf.len() < 52 || &elf[0..4] != b"\x7fELF" || elf[4] != 1 || elf[5] != 2 {
        return Err("expected ELF32 Big-Endian object".into());
    }
    let e_shoff = u32::from_be_bytes(elf[32..36].try_into().unwrap()) as usize;
    let e_shentsize = u16::from_be_bytes(elf[46..48].try_into().unwrap()) as usize;
    let e_shnum = u16::from_be_bytes(elf[48..50].try_into().unwrap()) as usize;
    let e_shstrndx = u16::from_be_bytes(elf[50..52].try_into().unwrap()) as usize;

    let shstr_hdr = e_shoff + e_shstrndx * e_shentsize;
    let shstr_off = u32::from_be_bytes(elf[shstr_hdr + 16..shstr_hdr + 20].try_into().unwrap()) as usize;

    let mut map = HashMap::new();
    for i in 0..e_shnum {
        let sh = e_shoff + i * e_shentsize;
        let sh_name = u32::from_be_bytes(elf[sh..sh + 4].try_into().unwrap()) as usize;
        let sh_off = u32::from_be_bytes(elf[sh + 16..sh + 20].try_into().unwrap()) as usize;
        let sh_size = u32::from_be_bytes(elf[sh + 20..sh + 24].try_into().unwrap()) as usize;
        let name_start = shstr_off + sh_name;
        let name_end = elf[name_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| name_start + p)
            .unwrap_or(elf.len());
        let name = String::from_utf8_lossy(&elf[name_start..name_end]).to_string();
        if sh_off + sh_size <= elf.len() {
            map.insert(name, elf[sh_off..sh_off + sh_size].to_vec());
        }
    }
    Ok(map)
}

pub fn thumb_bl(src_va: u32, dst_va: u32) -> [u8; 4] {
    let diff = (dst_va as i64) - ((src_va as i64) + 4);
    assert!((-(1 << 22)..(1 << 22)).contains(&diff) && (diff & 1) == 0, "BL out of range");
    let hi = (0xF000 | ((diff >> 12) & 0x7FF)) as u16;
    let lo = (0xF800 | ((diff >> 1) & 0x7FF)) as u16;
    let mut b = [0u8; 4];
    b[0..2].copy_from_slice(&hi.to_be_bytes());
    b[2..4].copy_from_slice(&lo.to_be_bytes());
    b
}

pub fn thumb_blx_arm(src_va: u32, dst_arm_va: u32) -> [u8; 4] {
    let pc_aligned = ((src_va + 4) & !3) as i64;
    let diff = (dst_arm_va as i64) - pc_aligned;
    assert!((-(1 << 22)..(1 << 22)).contains(&diff) && (diff & 3) == 0, "BLX out of range");
    let hi = (0xF000 | ((diff >> 12) & 0x7FF)) as u16;
    let lo = (0xE800 | ((diff >> 1) & 0x7FE)) as u16;
    let mut b = [0u8; 4];
    b[0..2].copy_from_slice(&hi.to_be_bytes());
    b[2..4].copy_from_slice(&lo.to_be_bytes());
    b
}
