//! dp2m dual-chip fw toolkit (m6m arm926ej-s BE + m5slave fr80 BE)

mod catalog;
mod config;
mod elf;
mod m5;
mod m6m;
mod pack;
mod package;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use config::Config;
use pack::FilmPack;

const USAGE: &str = "usage:
  dp2m build [DP2MV105.bin] [--config dp2m.fnl] [--out <dir>] [--pack <film_pack.bin>] [--skip-verify]
  dp2m stage [--config dp2m.fnl] [--from <out_dir>] [--to <sd_mount>]
  dp2m verify-m5 <Firm.bin>
  dp2m disasm-fr80 <Firm.bin> <va_start_hex> <va_end_hex>
  dp2m inspect <file.bin>";

fn parse_hex(s: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).map_err(|_| format!("invalid hex '{s}'"))
}

fn repo_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("src/m6m").is_dir() {
        return cwd;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).map_or(cwd, Path::to_path_buf)
}

fn load_config(explicit: Option<&Path>, root: &Path) -> Result<Config, String> {
    let path = explicit.map(Path::to_path_buf).or_else(|| {
        ["dp2m.fnl", "dp2m.example.fnl"].iter().map(|n| root.join(n)).find(|p| p.is_file())
    });
    path.map_or_else(|| Ok(Config::default()), |p| Config::load(&p))
}

/// `--flag value` pairs plus positionals.
struct Args {
    flags: Vec<(String, String)>,
    switches: Vec<String>,
    positional: Vec<String>,
}

impl Args {
    fn parse(args: &[String], valued: &[&str], switches: &[&str]) -> Result<Self, String> {
        let mut a = Args { flags: vec![], switches: vec![], positional: vec![] };
        let mut it = args.iter();
        while let Some(s) = it.next() {
            if valued.contains(&s.as_str()) {
                a.flags.push((s.clone(), it.next().ok_or_else(|| format!("missing value for {s}"))?.clone()));
            } else if switches.contains(&s.as_str()) {
                a.switches.push(s.clone());
            } else if s.starts_with('-') {
                return Err(format!("unknown flag '{s}'"));
            } else {
                a.positional.push(s.clone());
            }
        }
        Ok(a)
    }

    fn path(&self, flag: &str) -> Option<PathBuf> {
        self.flags.iter().rev().find(|(f, _)| f == flag).map(|(_, v)| PathBuf::from(v))
    }
}

fn rel(root: &Path, p: PathBuf) -> PathBuf {
    if p.is_absolute() { p } else { root.join(p) }
}

fn read(p: &Path) -> Result<Vec<u8>, String> {
    fs::read(p).map_err(|e| format!("read {}: {e}", p.display()))
}

fn write(p: &Path, data: &[u8]) -> Result<(), String> {
    fs::write(p, data).map_err(|e| format!("write {}: {e}", p.display()))
}

fn build(root: &Path, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--config", "--out", "--pack"], &["--skip-verify"])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let raw = a.positional.first().map(PathBuf::from).or_else(|| cfg.firmware.clone().map(|p| rel(root, p))).ok_or("provide <DP2MV105.bin> or set :firmware")?;
    let out = a.path("--out").or_else(|| cfg.out_dir.clone().map(|p| rel(root, p))).unwrap_or_else(|| root.join("out"));
    let pack_path = a.path("--pack").or_else(|| cfg.pack.clone().map(|p| rel(root, p))).unwrap_or_else(|| root.join("data/film_pack.bin"));

    let fw = read(&raw)?;
    let pack = FilmPack::load(&pack_path)?;
    let firm = m5::build_m5(&fw, &pack, &root.join("src/asm/m5"), &cfg)?;
    if cfg.verify && !a.switches.iter().any(|s| s == "--skip-verify") {
        m5::verify_m5(&firm)?;
    }
    let c78 = m6m::build_m6m(&fw, &pack, &root.join("src/m6m"), &out.join("m6m"), &cfg)?;
    let pkg = package::build(&fw, &firm, &c78, &cfg)?;

    let pkg_name = package::file_name(&cfg)?;
    write(&out.join(&pkg_name), &pkg)?;
    write(&out.join("Firm.bin"), &firm)?;
    write(&out.join("C78FRM.bin"), &c78)?;
    println!("Built {} (version {}):", out.display(), cfg.version);
    println!("  {pkg_name:<12} : {:>9} B  stock updater (Setup > Firmware Update)", pkg.len());
    println!("  Firm.bin     : {:>9} B  M5 debug image, checksum {:#010x}", firm.len(), m5::m5_checksum(&firm));
    println!("  C78FRM.bin   : {:>9} B  M6M debug image", c78.len());
    Ok(())
}

fn stage(root: &Path, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--config", "--from", "--to"], &[])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let from = a.path("--from").or_else(|| cfg.out_dir.clone().map(|p| rel(root, p))).unwrap_or_else(|| root.join("out"));
    let mount = a.path("--to").or_else(|| cfg.sd.mount.clone()).ok_or("provide --to <sd_mount> or set :sd {:mount ...}")?;
    if !mount.is_dir() {
        return Err(format!("SD mount path does not exist: {}", mount.display()));
    }
    let pkg = package::file_name(&cfg)?;
    let names = [(cfg.sd.package, pkg.as_str()), (cfg.sd.firm, "Firm.bin"), (cfg.sd.c78, "C78FRM.bin")];
    for (_, name) in names.iter().filter(|(on, _)| *on) {
        let data = read(&from.join(name))?;
        let dst = mount.join(name);
        write(&dst, &data)?;
        if read(&dst)? != data {
            return Err(format!("read-back mismatch on {}", dst.display()));
        }
        println!("Staged {name} -> {} ({} B, read-back OK)", dst.display(), data.len());
    }
    Ok(())
}

fn inspect(path: &str) -> Result<(), String> {
    let b = read(Path::new(path))?;
    println!("File : {path} ({} B)", b.len());
    if b.len() == m5::S2_LEN && b.starts_with(b"SIGMA.CODSLRSD15") {
        let hdr = u32::from_be_bytes(b[0x4C..0x50].try_into().unwrap());
        println!("Type : M5Slave (Firm.bin)\nSum  : header {hdr:#010x}, computed {:#010x}", m5::m5_checksum(&b));
    } else if b.len() == m6m::C78_LEN && b.starts_with(b"FUJITSU!") {
        println!("Type : M6M (C78FRM.bin)");
    } else if b.len() == m6m::C78_S3_OFF + m6m::C78_LEN && b.starts_with(b"SIGMA.CODP2M") {
        let field = |o: usize| String::from_utf8_lossy(&b[o..o + 16]).trim_end_matches('\0').to_owned();
        println!("Type : DP2M updater package\nVer  : {} ({})", field(0x10), field(0x50));
    } else {
        println!("Type : unknown");
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = repo_root();
    match args.first().map(String::as_str) {
        Some("build") => build(&root, &args[1..]),
        Some("stage") => stage(&root, &args[1..]),
        Some("verify-m5") if args.len() == 2 => m5::verify_m5(&read(Path::new(&args[1]))?),
        Some("disasm-fr80") if args.len() == 4 => {
            let buf = read(Path::new(&args[1]))?;
            for line in fr80::dis_range(&buf, parse_hex(&args[2])?, parse_hex(&args[3])?, m5::DELTA) {
                println!("{line}");
            }
            Ok(())
        }
        Some("inspect") if args.len() == 2 => inspect(&args[1]),
        _ => Err(USAGE.into()),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        process::exit(1);
    }
}
