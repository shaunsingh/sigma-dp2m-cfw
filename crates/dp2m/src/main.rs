//! dp2m dual-chip fw toolkit (m6m arm926ej-s BE + m5slave fr80 BE)

mod arm_elf;
mod config;
mod m5;
mod m6m;
mod pack;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use config::Config;
use pack::FilmPack;

fn usage() -> ! {
    eprintln!(
        "Usage:\n  \
         dp2m build [DP2MV105.bin] [--config dp2m.fnl] [--out <dir>] [--pack <film_pack.bin>] [--skip-verify]\n  \
         dp2m stage [--config dp2m.fnl] [--from <out_dir>] [--to <sd_mount>]\n  \
         dp2m verify-m5 <Firm.bin>\n  \
         dp2m disasm-fr80 <Firm.bin> <va_start_hex> <va_end_hex>\n  \
         dp2m inspect <file.bin>"
    );
    process::exit(2);
}

fn parse_hex(s: &str) -> Result<u32, String> {
    let t = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(t, 16).map_err(|_| format!("invalid hex '{}'", s))
}

fn repo_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("src/asm/m5").is_dir() {
        return cwd;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or(cwd)
}

fn load_config(explicit: Option<&Path>, root: &Path) -> Result<Config, String> {
    if let Some(p) = explicit {
        return Config::load(p);
    }
    let user_cfg = root.join("dp2m.fnl");
    if user_cfg.is_file() {
        return Config::load(&user_cfg);
    }
    let example_cfg = root.join("dp2m.example.fnl");
    if example_cfg.is_file() {
        return Config::load(&example_cfg);
    }
    Ok(Config::default())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let root = repo_root();

    match args[0].as_str() {
        "build" => {
            let mut raw_arg: Option<PathBuf> = None;
            let mut cfg_arg: Option<PathBuf> = None;
            let mut out_arg: Option<PathBuf> = None;
            let mut pack_arg: Option<PathBuf> = None;
            let mut skip_verify = false;

            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--config" => {
                        i += 1;
                        cfg_arg = Some(PathBuf::from(args.get(i).ok_or("missing --config path")?));
                    }
                    "--out" => {
                        i += 1;
                        out_arg = Some(PathBuf::from(args.get(i).ok_or("missing --out dir")?));
                    }
                    "--pack" => {
                        i += 1;
                        pack_arg = Some(PathBuf::from(args.get(i).ok_or("missing --pack path")?));
                    }
                    "--skip-verify" => skip_verify = true,
                    s if s.starts_with('-') => return Err(format!("unknown flag '{}'", s)),
                    pos => raw_arg = Some(PathBuf::from(pos)),
                }
                i += 1;
            }

            let cfg = load_config(cfg_arg.as_deref(), &root)?;
            let raw_path = raw_arg
                .or_else(|| cfg.firmware.clone())
                .ok_or("provide <DP2MV105.bin> or set :firmware in dp2m.fnl")?;
            let out_dir = out_arg
                .or_else(|| cfg.out_dir.clone())
                .unwrap_or_else(|| root.join("out"));
            let pack_path = pack_arg
                .or_else(|| cfg.pack.clone())
                .unwrap_or_else(|| root.join("data/film_pack.bin"));
            let do_verify = cfg.verify && !skip_verify;

            let fw_raw = fs::read(&raw_path)
                .map_err(|e| format!("read {}: {}", raw_path.display(), e))?;
            let pack = FilmPack::load(&pack_path)?;

            let s2 = m5::build_m5(&fw_raw, &pack, &root.join("src/asm/m5"), &cfg)?;
            if do_verify {
                m5::verify_m5(&s2)?;
            }
            let c78 = m6m::build_m6m(&fw_raw, &pack, &root.join("src/asm/m6"), &root.join("src/c/m6"), &cfg)?;

            fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {}: {}", out_dir.display(), e))?;
            let firm_out = out_dir.join("Firm.bin");
            let c78_out = out_dir.join("C78FRM.bin");
            let comb_out = out_dir.join("C78_M6M5.bin");

            fs::write(&firm_out, &s2).map_err(|e| format!("write Firm.bin: {}", e))?;
            fs::write(&c78_out, &c78).map_err(|e| format!("write C78FRM.bin: {}", e))?;

            let mut combined = Vec::with_capacity(fw_raw.len());
            combined.extend_from_slice(&fw_raw[0..m5::S2_OFF]);
            combined.extend_from_slice(&s2);
            combined.extend_from_slice(&c78);
            if combined.len() != fw_raw.len() {
                return Err(format!("combined size mismatch: {} != {}", combined.len(), fw_raw.len()));
            }
            fs::write(&comb_out, &combined).map_err(|e| format!("write C78_M6M5.bin: {}", e))?;

            println!("Built firmware images in {}:", out_dir.display());
            println!("  Firm.bin     : {:>9} B (checksum {:#010x})", s2.len(), m5::m5_checksum(&s2));
            println!("  C78FRM.bin   : {:>9} B", c78.len());
            println!("  C78_M6M5.bin : {:>9} B", combined.len());
            Ok(())
        }
        "stage" => {
            let mut cfg_arg: Option<PathBuf> = None;
            let mut from_arg: Option<PathBuf> = None;
            let mut to_arg: Option<PathBuf> = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--config" => { i += 1; cfg_arg = Some(PathBuf::from(args.get(i).ok_or("missing --config")?)); }
                    "--from" => { i += 1; from_arg = Some(PathBuf::from(args.get(i).ok_or("missing --from")?)); }
                    "--to" => { i += 1; to_arg = Some(PathBuf::from(args.get(i).ok_or("missing --to")?)); }
                    other => return Err(format!("unknown flag '{}'", other)),
                }
                i += 1;
            }
            let cfg = load_config(cfg_arg.as_deref(), &root)?;
            let from_dir = from_arg.or_else(|| cfg.out_dir.clone()).unwrap_or_else(|| root.join("out"));
            let mount = to_arg.or_else(|| cfg.sd_mount.clone()).ok_or("provide --to <sd_mount> or set :sd {:mount ...} in dp2m.fnl")?;
            if !mount.is_dir() {
                return Err(format!("SD mount path does not exist: {}", mount.display()));
            }
            let mut copied = Vec::new();
            if cfg.sd_copy_firm { copied.push("Firm.bin"); }
            if cfg.sd_copy_c78 { copied.push("C78FRM.bin"); }
            for name in copied {
                let src = from_dir.join(name);
                let dst = mount.join(name);
                let data = fs::read(&src).map_err(|e| format!("read {}: {}", src.display(), e))?;
                fs::write(&dst, &data).map_err(|e| format!("write {}: {}", dst.display(), e))?;
                let verify = fs::read(&dst).map_err(|e| format!("verify {}: {}", dst.display(), e))?;
                if verify != data {
                    return Err(format!("read-back mismatch on {}", dst.display()));
                }
                println!("Staged {} -> {} ({} B, read-back OK)", src.display(), dst.display(), data.len());
            }
            Ok(())
        }
        "verify-m5" => {
            if args.len() != 2 {
                usage();
            }
            let s2 = fs::read(&args[1]).map_err(|e| format!("read {}: {}", args[1], e))?;
            m5::verify_m5(&s2)
        }
        "disasm-fr80" => {
            if args.len() != 4 {
                usage();
            }
            let buf = fs::read(&args[1]).map_err(|e| format!("read {}: {}", args[1], e))?;
            let start = parse_hex(&args[2])?;
            let end = parse_hex(&args[3])?;
            for line in fr80::dis_range(&buf, start, end, m5::DELTA) {
                println!("{}", line);
            }
            Ok(())
        }
        "inspect" => {
            if args.len() != 2 {
                usage();
            }
            let buf = fs::read(&args[1]).map_err(|e| format!("read {}: {}", args[1], e))?;
            println!("File : {} ({} B)", args[1], buf.len());
            if buf.len() == m5::S2_LEN && &buf[0..16] == b"SIGMA.CODSLRSD15" {
                let hdr = u32::from_be_bytes(buf[0x4C..0x50].try_into().unwrap());
                let calc = m5::m5_checksum(&buf);
                let nor8 = buf[0x80..0x80_0080].iter().fold(0u32, |a, &b| a.wrapping_add(b as u32));
                println!("Type : M5Slave (Firm.bin)");
                println!("Hdr  : {:#010x} | Full: {:#010x} | 8MB: {:#010x}", hdr, calc, nor8);
            } else if buf.len() == m6m::C78_LEN && &buf[0..8] == b"FUJITSU!" {
                println!("Type : M6M (C78FRM.bin)");
            } else if buf.len() == m6m::C78_S3_OFF + m6m::C78_LEN {
                println!("Type : Combined dual-chip package (DP2MV105 / C78_M6M5)");
            } else {
                println!("Type : Unknown binary");
            }
            Ok(())
        }
        _ => usage(),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}
