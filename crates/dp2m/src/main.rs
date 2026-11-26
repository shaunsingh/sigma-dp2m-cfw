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
  dp2m sim [--from <out_dir>] [--port <port>]
  dp2m sim-trace [DP2MVxxx.bin] [--keys k1,k2,...] [--ppm <out.ppm>] [--watch-text]
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

fn sim(root: &Path, args: &[String]) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let a = Args::parse(args, &["--config", "--from", "--port"], &[])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let from = a
        .path("--from")
        .or_else(|| cfg.out_dir.clone().map(|p| rel(root, p)))
        .unwrap_or_else(|| root.join("out"));
    let port: u16 = a
        .flags
        .iter()
        .rev()
        .find(|(f, _)| f == "--port")
        .map_or(Ok(8080), |(_, v)| v.parse().map_err(|_| format!("invalid port '{v}'")))?;

    let status = process::Command::new("cargo")
        .current_dir(root)
        .args(["build", "--release", "--offline", "--target", "wasm32-unknown-unknown", "-p", "dp2m-web"])
        .status()
        .map_err(|e| format!("failed to run cargo build for dp2m-web: {e}"))?;
    if !status.success() {
        return Err("wasm32-unknown-unknown build of dp2m-web failed".into());
    }

    let html_path = root.join("web/index.html");
    let wasm_path = root.join("target/wasm32-unknown-unknown/release/dp2m_web.wasm");
    let c78_path = from.join("C78FRM.bin");
    let firm_path = from.join("Firm.bin");
    if !c78_path.is_file() || !firm_path.is_file() {
        return Err(format!(
            "missing C78FRM.bin or Firm.bin in {} (run `dp2m build` first)",
            from.display()
        ));
    }

    use std::time::Duration;

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("DP2M simulator listening on http://{addr}");

    for stream in listener.incoming().flatten() {
        let html_p = html_path.clone();
        let wasm_p = wasm_path.clone();
        let c78_p = c78_path.clone();
        let firm_p = firm_path.clone();
        std::thread::spawn(move || {
            let mut s = stream;
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut req = [0u8; 2048];
            let n = s.read(&mut req).unwrap_or(0);
            if n == 0 {
                return;
            }
            let line = String::from_utf8_lossy(&req[..n]);
            let path = line.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/");
            let owned: Vec<u8>;
            let (code, ctype, body): (&str, &str, &[u8]) = match path {
                "/" | "/index.html" => match read(&html_p) {
                    Ok(b) => {
                        owned = b;
                        ("200 OK", "text/html; charset=utf-8", &owned)
                    }
                    Err(_) => ("404 Not Found", "text/plain", b"missing index.html"),
                },
                "/dp2m_web.wasm" => match read(&wasm_p) {
                    Ok(b) => {
                        owned = b;
                        ("200 OK", "application/wasm", &owned)
                    }
                    Err(_) => ("404 Not Found", "text/plain", b"missing wasm"),
                },
                "/C78FRM.bin" => match read(&c78_p) {
                    Ok(b) => {
                        owned = b;
                        ("200 OK", "application/octet-stream", &owned)
                    }
                    Err(_) => ("404 Not Found", "text/plain", b"missing C78FRM.bin"),
                },
                "/Firm.bin" => match read(&firm_p) {
                    Ok(mut b) => {
                        b.truncate(0x007C_4080);
                        owned = b;
                        ("200 OK", "application/octet-stream", &owned)
                    }
                    Err(_) => ("404 Not Found", "text/plain", b"missing Firm.bin"),
                },
                "/favicon.ico" => ("204 No Content", "image/x-icon", b""),
                _ => ("404 Not Found", "text/plain", b"not found"),
            };
            let hdr = format!(
                "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            if !body.is_empty() {
                let _ = s.write_all(body);
            }
        });
    }
    Ok(())
}

fn parse_sim_key(name: &str) -> Result<dp2m_sim::Key, String> {
    use dp2m_sim::Key;
    Ok(match name.to_ascii_lowercase().as_str() {
        "ok" | "enter" => Key::Ok,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "menu" | "m" | "esc" => Key::Menu,
        "disp" | "i" | "v" => Key::Disp,
        "ael" | "l" => Key::Ael,
        "qs" | "q" => Key::Qs,
        "mode" | "o" => Key::Mode,
        "play" | "p" => Key::Play,
        "dialleft" | "[" => Key::DialLeft,
        "dialright" | "]" => Key::DialRight,
        "shutter" | "shoot" | "space" => Key::Shutter,
        other => return Err(format!("unknown sim-trace key '{other}'")),
    })
}

fn sim_trace(root: &Path, args: &[String]) -> Result<(), String> {
    use dp2m_sim::{Camera, LCD_H, LCD_W};

    let a = Args::parse(args, &["--config", "--keys", "--ppm"], &["--watch-text"])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let out = cfg.out_dir.clone().map(|p| rel(root, p)).unwrap_or_else(|| root.join("out"));
    let pkg_path = a
        .positional
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join(package::file_name(&cfg).unwrap_or_else(|_| "DP2MV200.bin".into())));

    let pkg = read(&pkg_path)?;
    let mut cam = Camera::from_package(&pkg)?;
    let watch_text = a.switches.iter().any(|s| s == "--watch-text");
    if watch_text {
        cam.cpu_mut().set_write_watch(Some((0xA000_0000, 0xA017_9000)));
    }

    let report = |label: &str, insns: u64, cam: &mut Camera| {
        let fb = cam.render_lcd();
        let mut sum = [0u64; 3];
        for px in fb.as_chunks::<4>().0 {
            sum[0] += px[0] as u64;
            sum[1] += px[1] as u64;
            sum[2] += px[2] as u64;
        }
        let n = (LCD_W * LCD_H) as f64;
        let osd_nz = cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H]
            .iter()
            .filter(|&&b| b != 0)
            .count();
        let av_idx = cam.cpu().r16_be(0xA07D_D6E0);
        let tv_idx = cam.cpu().r16_be(0xA07D_D6E4);
        let sv_q16 = cam.cpu().r32_be(0xA080_0908) as i32;
        println!(
            "{label:<16} scr={:#04x} pb={} expo={} cm={:<2} av={:<2} tv={:<2} sv={:+.2} caps={} osd_nz={:<6} rgb=({:.1},{:.1},{:.1}) insns={insns}",
            cam.screen_id(),
            u8::from(cam.in_playback()),
            cam.expo_mode(),
            cam.color_mode(),
            av_idx,
            tv_idx,
            (sv_q16 as f64) / 65536.0,
            cam.capture_count(),
            osd_nz,
            sum[0] as f64 / n,
            sum[1] as f64 / n,
            sum[2] as f64 / n,
        );
    };

    report("boot", cam.cpu().total_steps, &mut cam);

    if let Some((_, keys_str)) = a.flags.iter().rev().find(|(f, _)| f == "--keys") {
        for raw_tok in keys_str.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(rest) = raw_tok.strip_prefix("cm=") {
                let cm: u32 = rest.parse().map_err(|_| format!("invalid cm '{rest}'"))?;
                let before = cam.cpu().total_steps;
                cam.set_color_mode(cm);
                report(raw_tok, cam.cpu().total_steps - before, &mut cam);
            } else if let Some(rest) = raw_tok.strip_prefix("nv:") {
                let (va_s, v_s) = rest.split_once('=').ok_or_else(|| format!("expected nv:<hex>=<u8> in '{raw_tok}'"))?;
                let va = parse_hex(va_s)?;
                let val: u8 = if v_s.starts_with("0x") { parse_hex(v_s)? as u8 } else { v_s.parse().map_err(|_| format!("invalid u8 '{v_s}'"))? };
                let before = cam.cpu().total_steps;
                cam.write_u8(va, val);
                report(raw_tok, cam.cpu().total_steps - before, &mut cam);
            } else {
                let (reps, key_s) = match raw_tok.split_once('*') {
                    Some((n_s, k_s)) => (n_s.parse::<usize>().map_err(|_| format!("invalid repeat '{n_s}'"))?, k_s),
                    None => (1, raw_tok),
                };
                let key = parse_sim_key(key_s)?;
                let mut insns = 0u64;
                for _ in 0..reps {
                    insns += cam.press(key);
                    if cam.cpu().last_error.is_some() || !cam.cpu().watch_hits.is_empty() {
                        break;
                    }
                }
                report(raw_tok, insns, &mut cam);
            }
            if !cam.cpu().watch_hits.is_empty() {
                for &(pc, lr, va) in &cam.cpu().watch_hits {
                    eprintln!("WATCH-TEXT hit: pc={pc:#010x} lr={lr:#010x} write_va={va:#010x}");
                }
                return Err(format!(".text write watchpoint triggered ({} hits)", cam.cpu().watch_hits.len()));
            }
            if let Some(ref err) = cam.cpu().last_error {
                return Err(format!("CPU error after '{raw_tok}': {err}"));
            }
        }
    }

    if let Some(ppm_path) = a.path("--ppm") {
        let fb = cam.render_lcd();
        let mut ppm = format!("P6\n{LCD_W} {LCD_H}\n255\n").into_bytes();
        ppm.reserve(LCD_W * LCD_H * 3);
        for px in fb.as_chunks::<4>().0 {
            ppm.extend_from_slice(&px[..3]);
        }
        write(&ppm_path, &ppm)?;
        println!("Wrote {} ({}x{} PPM)", ppm_path.display(), LCD_W, LCD_H);
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = repo_root();
    match args.first().map(String::as_str) {
        Some("build") => build(&root, &args[1..]),
        Some("stage") => stage(&root, &args[1..]),
        Some("sim") => sim(&root, &args[1..]),
        Some("sim-trace") => sim_trace(&root, &args[1..]),
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
