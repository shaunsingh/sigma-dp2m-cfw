//! dp2m dual-chip fw toolkit (m6m arm926ej-s BE + m5slave fr80 BE)

mod catalog;
mod config;
mod doom;
mod elf;
mod lut;
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
  dp2m build-doom [--doomgeneric <dir>] [--config dp2m.fnl] [--out <dir>] [--warp]
  dp2m verify [out_dir] [--fw <DP2MV105.bin>] [--suite m6m|custom|labels] [--artifacts <dir>] [--quiet]
  dp2m re [--fw <DP2MV105.bin>] [--image <C78FRM.bin>] [--syms <m6m.syms>] <package|dis|xrefs|callers|sid|strings|menus|screens|icons|nvram|liveness|text> ...
  dp2m x3f <file.X3F> [--dump <out.camf>] [--all]
  dp2m lut <input.cube> <LUTn.BIN> [--grain N] [--grain-profile 0|1|2] [--halo N] [--halo-tint 0|1|2] [--tag NAME]
  dp2m lut inspect <LUTn.BIN>
  dp2m stage [--config dp2m.fnl] [--from <out_dir>] [--to <sd_mount>] [--wad <DOOM1.WAD>]
  dp2m sim [--config dp2m.fnl] [--from <out_dir>] [--wad <DOOM1.WAD>] [--port <port>]
  dp2m sim-trace [DP2MVxxx.bin] [--config dp2m.fnl] [--sd NAME=PATH] [--keys k1,k2,...] [--ppm <out.ppm>] [--watch-text]
  dp2m verify-m5 <Firm.bin|out_dir>
  dp2m disasm-fr80 <Firm.bin> <va_start_hex> <va_end_hex>
  dp2m inspect <file.bin>";

fn parse_hex(s: &str) -> Result<u32, String> {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(hex, 16).map_err(|_| format!("invalid hex '{s}'"))
}

fn repo_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("firmware/m6m").is_dir() {
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

#[derive(Default)]
struct Args<'a> {
    flags: Vec<(&'a str, &'a str)>,
    switches: Vec<&'a str>,
    positional: Vec<&'a str>,
}

impl<'a> Args<'a> {
    fn parse(args: &'a [String], valued: &[&str], switches: &[&str]) -> Result<Self, String> {
        let mut a = Self::default();
        let mut it = args.iter().map(String::as_str);
        while let Some(s) = it.next() {
            if valued.contains(&s) {
                let v = it.next().filter(|v| !valued.contains(v) && !switches.contains(v));
                a.flags.push((s, v.ok_or_else(|| format!("missing value for {s}"))?));
            } else if switches.contains(&s) {
                a.switches.push(s);
            } else if s.starts_with('-') {
                return Err(format!("unknown flag '{s}'"));
            } else {
                a.positional.push(s);
            }
        }
        Ok(a)
    }

    fn values<'b>(&'b self, flag: &'b str) -> impl DoubleEndedIterator<Item = &'a str> + 'b {
        self.flags.iter().filter_map(move |&(f, v)| (f == flag).then_some(v))
    }

    fn value(&self, flag: &str) -> Option<&'a str> {
        self.values(flag).next_back()
    }

    fn get<T: std::str::FromStr>(&self, flag: &str, default: T) -> Result<T, String> {
        self.value(flag).map_or(Ok(default), |v| v.parse().map_err(|_| format!("invalid {flag} '{v}'")))
    }

    fn path(&self, flag: &str) -> Option<PathBuf> {
        self.value(flag).map(PathBuf::from)
    }

    fn has(&self, flag: &str) -> bool {
        self.switches.contains(&flag)
    }
}

fn output_dir(root: &Path, explicit: Option<PathBuf>, cfg: &Config) -> PathBuf {
    explicit.or_else(|| cfg.out_dir.clone().map(|p| rel(root, p))).unwrap_or_else(|| root.join("out"))
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
    let out = output_dir(root, a.path("--out"), &cfg);
    let pack_path = a.path("--pack").or_else(|| cfg.pack.clone().map(|p| rel(root, p))).unwrap_or_else(|| root.join("data/film_pack.bin"));

    let fw = read(&raw)?;
    let pack = FilmPack::load(&pack_path)?;
    let firm = m5::build_m5(&fw, &pack, &root.join("firmware/m5"), &cfg)?;
    if cfg.verify && !a.has("--skip-verify") {
        m5::verify_m5(&firm)?;
    }
    let c78 = m6m::build_m6m(&fw, &pack, &root.join("firmware/m6m"), &out.join("m6m"), &cfg)?;
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

fn build_doom_cmd(root: &Path, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--doomgeneric", "--config", "--out"], &["--warp"])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let out = output_dir(root, a.path("--out"), &cfg);
    let dg = doom::resolve_doomgeneric_dir(root, a.path("--doomgeneric").as_deref())?;
    doom::build_doom(root, &dg, &out, a.has("--warp")).map(|_| ())
}

fn stage_checked(src: &Path, dst: &Path, label: &str) -> Result<(), String> {
    let data = read(src)?;
    write(dst, &data)?;
    if read(dst)? != data {
        return Err(format!("read-back mismatch on {}", dst.display()));
    }
    println!("Staged {label} -> {} ({} B, read-back OK)", dst.display(), data.len());
    Ok(())
}

fn stage(root: &Path, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args, &["--config", "--from", "--to", "--wad"], &[])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let from = output_dir(root, a.path("--from"), &cfg);
    let mount = a.path("--to").or_else(|| cfg.sd.mount.clone()).ok_or("provide --to <sd_mount> or set :sd {:mount ...}")?;
    if !mount.is_dir() {
        return Err(format!("SD mount path does not exist: {}", mount.display()));
    }
    let pkg = package::file_name(&cfg)?;
    let names = [(cfg.sd.package, pkg.as_str()), (cfg.sd.firm, "Firm.bin"), (cfg.sd.c78, "C78FRM.bin")];
    for (_, name) in names.iter().filter(|(on, _)| *on) {
        stage_checked(&from.join(name), &mount.join(name), name)?;
    }
    let doom_bin = from.join("DOOM.BIN");
    if cfg.m6m.doom && doom_bin.is_file() {
        stage_checked(&doom_bin, &mount.join("DOOM.BIN"), "DOOM.BIN")?;
    }
    let wad_src = a.path("--wad").or_else(|| {
        let p = from.join("DOOM1.WAD");
        p.is_file().then_some(p)
    });
    if let Some(wad) = wad_src {
        let fname = wad.file_name().and_then(|s| s.to_str()).unwrap_or("DOOM1.WAD").to_ascii_uppercase();
        stage_checked(&wad, &mount.join(&fname), &fname)?;
    }
    Ok(())
}

fn inspect(path: &str) -> Result<(), String> {
    let b = read(Path::new(path))?;
    println!("File : {path} ({} B)", b.len());
    if b.len() == m5::S2_LEN && b.starts_with(b"SIGMA.CODSLRSD15") {
        let hdr = m5::stored_checksum(&b);
        println!("Type : M5Slave (Firm.bin)\nSum  : header {hdr:#010x}, computed {:#010x}", m5::m5_checksum(&b));
    } else if b.len() == m6m::C78_LEN && b.starts_with(b"FUJITSU!") {
        println!("Type : M6M (C78FRM.bin)");
    } else if b.len() == m6m::C78_S3_OFF + m6m::C78_LEN && b.starts_with(b"SIGMA.CODP2M") {
        let field = |o: usize| String::from_utf8_lossy(&b[o..o + 16]).trim_end_matches('\0').to_owned();
        println!("Type : DP2M updater package\nVer  : {} ({})", field(0x10), field(0x50));
    } else if b.len() >= 32 && b.starts_with(b"D2AP") {
        let u32be = |o: usize| u32::from_be_bytes(b[o..o + 4].try_into().unwrap());
        println!(
            "Type : DP2M standalone applet (D2AP v{})\nLoad : {:#010x}..{:#010x} (entry {:#010x}, bss_end {:#010x}, flags {:#x})",
            u32be(4), u32be(8), u32be(8).wrapping_add(u32be(16)), u32be(12), u32be(20), u32be(24)
        );
    } else {
        println!("Type : unknown");
    }
    Ok(())
}

fn sim(root: &Path, args: &[String]) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let a = Args::parse(args, &["--config", "--from", "--wad", "--port"], &[])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let from = output_dir(root, a.path("--from"), &cfg);
    let port = a.get("--port", 8080u16)?;

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
    let doom_path = from.join("DOOM.BIN");
    let wad_path = a.path("--wad").unwrap_or_else(|| from.join("DOOM1.WAD"));
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
        let doom_p = doom_path.clone();
        let wad_p = wad_path.clone();
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
            let serve = |p: &Path, ctype, missing: &[u8]| match fs::read(p) {
                Ok(mut body) => {
                    if path == "/Firm.bin" {
                        body.truncate(m5::NOR_END_OFF);
                    }
                    ("200 OK", ctype, body)
                }
                Err(_) => ("404 Not Found", "text/plain", missing.to_vec()),
            };
            let (code, ctype, body) = match path {
                "/" | "/index.html" => serve(&html_p, "text/html; charset=utf-8", b"missing index.html"),
                "/dp2m_web.wasm" => serve(&wasm_p, "application/wasm", b"missing wasm"),
                "/C78FRM.bin" => serve(&c78_p, "application/octet-stream", b"missing C78FRM.bin"),
                "/Firm.bin" => serve(&firm_p, "application/octet-stream", b"missing Firm.bin"),
                "/DOOM.BIN" => serve(&doom_p, "application/octet-stream", b"missing DOOM.BIN"),
                "/DOOM1.WAD" => serve(&wad_p, "application/octet-stream", b"missing DOOM1.WAD"),
                "/favicon.ico" => ("204 No Content", "image/x-icon", Vec::new()),
                _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
            };
            let hdr = format!(
                "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(hdr.as_bytes());
            if !body.is_empty() {
                let _ = s.write_all(&body);
            }
        });
    }
    Ok(())
}

fn parse_sim_key(name: &str) -> Result<dp2m_emu::sim::Key, String> {
    use dp2m_emu::sim::Key;
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
    use dp2m_emu::sim::{Camera, LCD_H, LCD_W};

    let a = Args::parse(args, &["--config", "--sd", "--keys", "--ppm"], &["--watch-text"])?;
    let cfg = load_config(a.path("--config").as_deref(), root)?;
    let pkg_path = match a.positional.first() {
        Some(p) => PathBuf::from(p),
        None => output_dir(root, None, &cfg).join(package::file_name(&cfg)?),
    };

    let pkg = read(&pkg_path)?;
    let mut cam = Camera::from_package(&pkg)?;
    for spec in a.values("--sd") {
        let (name, path_s) = spec.split_once('=').ok_or_else(|| format!("expected NAME=PATH for --sd '{spec}'"))?;
        let data = read(Path::new(path_s))?;
        cam.mount_sd_file(name, data);
    }
    let watch_text = a.has("--watch-text");
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

    if let Some(keys_str) = a.value("--keys") {
        for raw_tok in keys_str.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(rest) = raw_tok.strip_prefix("cm=") {
                let cm: u32 = rest.parse().map_err(|_| format!("invalid cm '{rest}'"))?;
                let before = cam.cpu().total_steps;
                cam.set_color_mode(cm);
                report(raw_tok, cam.cpu().total_steps - before, &mut cam);
            } else if let Some(rest) = raw_tok.strip_prefix("nv:") {
                let (va_s, v_s) = rest.split_once('=').ok_or_else(|| format!("expected nv:<hex>=<u8> in '{raw_tok}'"))?;
                let va = parse_hex(va_s)?;
                let val = u8::try_from(parse_u32_auto(v_s)?).map_err(|_| format!("invalid u8 '{v_s}'"))?;
                let before = cam.cpu().total_steps;
                cam.write_u8(va, val);
                report(raw_tok, cam.cpu().total_steps - before, &mut cam);
            } else {
                let (reps, key_s) = match raw_tok.split_once('*') {
                    Some((n_s, k_s)) => (n_s.parse::<usize>().map_err(|_| format!("invalid repeat '{n_s}'"))?, k_s),
                    None => (1, raw_tok),
                };
                if key_s.eq_ignore_ascii_case("tick") {
                    let before = cam.cpu().total_steps;
                    for _ in 0..reps {
                        let _ = cam.tick();
                        if cam.cpu().last_error.is_some() || !cam.cpu().watch_hits.is_empty() {
                            break;
                        }
                    }
                    report(raw_tok, cam.cpu().total_steps - before, &mut cam);
                } else {
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

fn parse_u32_auto(s: &str) -> Result<u32, String> {
    if s.starts_with("0x") || s.starts_with("0X") {
        parse_hex(s)
    } else {
        s.parse::<u32>().map_err(|_| format!("invalid int '{s}'"))
    }
}

fn verify_cmd(root: &Path, args: &[String]) -> Result<(), String> {
    use dp2m_emu::verify::{Build, run_suites_with_artifacts};
    use std::time::Instant;

    let a = Args::parse(args, &["--fw", "--suite", "--artifacts"], &["--quiet"])?;
    let out = a.positional.first().map(PathBuf::from).unwrap_or_else(|| root.join("out"));
    let fw = a.path("--fw").unwrap_or_else(|| root.join("data/DP2MV105.bin"));
    let verbose = !a.has("--quiet");
    let suites: Vec<&str> = a.values("--suite").collect();

    let t0 = Instant::now();
    let b = Build::load(&out, &fw)?;
    let results = run_suites_with_artifacts(&b, &suites, verbose, a.path("--artifacts").as_deref());
    println!("== summary ({:.2}s)", t0.elapsed().as_secs_f64());
    for s in &results {
        let status = if s.skipped { "SKIP" } else if s.failed > 0 { "FAIL" } else { "PASS" };
        println!("  {:<7} {status}  {} pass, {} fail", s.name, s.passed, s.failed);
        for f in &s.fails {
            println!("    FAIL {f}");
        }
    }
    if results.iter().any(|s| s.failed > 0) {
        return Err("verification failed".into());
    }
    let skipped: Vec<_> = results.iter().filter(|s| s.skipped).map(|s| s.name).collect();
    if !skipped.is_empty() {
        return Err(format!("verification incomplete: skipped suites: {}", skipped.join(", ")));
    }
    Ok(())
}

fn re_cmd(root: &Path, args: &[String]) -> Result<(), String> {
    use dp2m_emu::re::{self, ReCtx, dis, xref::{self, RefKind, XrefIndex}};
    use dp2m_emu::stock;

    let opts = ["--fw", "--image", "--syms"];
    let mut prefix = 0;
    while args.get(prefix).is_some_and(|f| opts.contains(&f.as_str())) {
        prefix = (prefix + 2).min(args.len());
    }
    let a = Args::parse(&args[..prefix], &opts, &[])?;
    let (sub, rest) = args[prefix..].split_first().ok_or(USAGE)?;
    let fw = a.path("--fw").unwrap_or_else(|| root.join("data/DP2MV105.bin"));
    let img_path = a.path("--image").unwrap_or_else(|| fw.clone());
    let syms = a.path("--syms");

    match sub.as_str() {
        "package" => {
            let p = rest.first().map(PathBuf::from).unwrap_or(img_path);
            for line in re::cmd_package(&p, Some(&fw))? {
                println!("{line}");
            }
        }
        "dis" => {
            let a = Args::parse(rest, &["-n"], &["--arm"])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let va = parse_u32_auto(a.positional.first().ok_or("dis <va> [end]")?)?;
            let end = a.positional.get(1).map(|s| parse_u32_auto(s)).transpose()?;
            let n: usize = a.get("-n", 40)?;
            let arm = a.has("--arm");
            let (stop, lim) = match end {
                Some(e) => (e, None),
                None => (va.saturating_add(u32::try_from(n.saturating_mul(4)).unwrap_or(u32::MAX)), Some(n)),
            };
            for line in dis::dis_range(&ctx.img, &ctx.names, va, stop, lim, arm) {
                println!("{line}");
            }
        }
        "xrefs" | "callers" => {
            let a = Args::parse(rest, &["--kinds"], &[])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let val = parse_u32_auto(a.positional.first().ok_or("expected hex address")?)?;
            let kinds: Option<Vec<RefKind>> = if sub == "callers" {
                Some(vec![RefKind::Tbl, RefKind::Tblx, RefKind::Abl, RefKind::Ablx])
            } else if let Some(ks) = a.value("--kinds") {
                Some(ks.split(',').map(|k| RefKind::parse(k).ok_or_else(|| format!("unknown kind '{k}'"))).collect::<Result<_, _>>()?)
            } else {
                None
            };
            let idx = XrefIndex::build(&ctx.img);
            for r in idx.refs(val, kinds.as_deref()) {
                let slot = if r.slot != r.src { format!(" slot {:#x}", r.slot) } else { String::new() };
                println!("{:08x} {:<5}{slot}  {}", r.src, r.kind.as_str(), ctx.where_va(r.src));
            }
        }
        "sid" => {
            let a = Args::parse(rest, &["--find", "--lang"], &[])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let find = a.value("--find");
            let lang = a.value("--lang").map_or(Ok(0), parse_u32_auto)?;
            let sids: Vec<u32> = a.positional.iter().copied().map(parse_u32_auto).collect::<Result<_, _>>()?;
            for line in re::cmd_sid(&ctx, &sids, find, lang) {
                println!("{line}");
            }
        }
        "strings" => {
            let a = Args::parse(rest, &["--min"], &["--refs"])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let min: usize = a.get("--min", 4)?;
            let with_refs = a.has("--refs");
            for line in re::cmd_strings(&ctx, a.positional.first().copied(), min, with_refs) {
                println!("{line}");
            }
        }
        "menus" => {
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let ids: Vec<u32> = rest.iter().map(|s| parse_u32_auto(s)).collect::<Result<_, _>>()?;
            for line in re::cmd_menus(&ctx, &ids) {
                println!("{line}");
            }
        }
        "screens" => {
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            for line in re::cmd_screens(&ctx) {
                println!("{line}");
            }
        }
        "icons" => {
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let ids: Vec<u32> = rest.iter().map(|s| parse_u32_auto(s)).collect::<Result<_, _>>()?;
            for line in re::cmd_icons(&ctx, &ids) {
                println!("{line}");
            }
        }
        "nvram" => {
            let a = Args::parse(rest, &["--window", "--block", "--len", "--offset"], &[])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let (lo, hi) = if let Some(w) = a.value("--window") {
                let (a_s, b_s) = w.split_once(':').ok_or("expected LO:HI for --window")?;
                (parse_u32_auto(a_s)?, parse_u32_auto(b_s)?)
            } else {
                (0xA093_9600, 0xA093_9B00)
            };
            let block = a.value("--block").map_or(Ok(stock::NV_BLOCK), parse_u32_auto)?;
            let len = a.value("--len").map_or(Ok(stock::NV_LEN), |v| parse_u32_auto(v).map(|x| x as usize))?;
            let offsets: Vec<u32> = a.values("--offset").map(parse_u32_auto).collect::<Result<_, _>>()?;
            let idx = XrefIndex::build(&ctx.img);
            let (acc, calls) = xref::nvram_scan(&ctx.img, lo, hi, &idx, 48);
            eprintln!("{} accesses, {} calls", acc.len(), calls.len());
            for line in xref::nvram_report(&ctx.img, &acc, &calls, block, len, stock::NV_DEFAULTS, &offsets) {
                println!("{line}");
            }
        }
        "liveness" => {
            let a = Args::parse(rest, &["--min-size", "--stock"], &["--tsv"])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let min_size = a.value("--min-size").map_or(Ok(512), parse_u32_auto)?;
            let tsv = a.has("--tsv");
            let t0 = std::time::Instant::now();
            let lv = re::liveness::analyse(&ctx.img, &[]);
            let stock_lv = if let Some(sp) = a.path("--stock") {
                let simg = dp2m_emu::image::Image::load(&sp)?;
                Some(re::liveness::analyse(&simg, &[]))
            } else {
                None
            };
            let cands = re::liveness::candidates(&ctx.img, &lv, stock_lv.as_ref(), min_size, &[], &[]);
            eprintln!(
                "liveness: {} functions ({} live, {} roots), {} candidates >= {min_size} B ({:.0} ms)",
                lv.ents.len(),
                lv.live.len(),
                lv.roots.len(),
                cands.len(),
                t0.elapsed().as_secs_f64() * 1000.0
            );
            if tsv {
                println!("{}", re::liveness::TSV_HEADER);
                for c in &cands {
                    println!("{}", c.tsv());
                }
            } else {
                for c in &cands {
                    println!("{:#010x}..{:#010x} {:7} B  {:<18} {:3} fns  {}", c.start, c.end, c.size(), c.kind, c.fns, c.hint);
                }
            }
        }
        "text" => {
            let a = Args::parse(rest, &["--px", "--style", "--lang"], &[])?;
            let ctx = ReCtx::load(&img_path, syms.as_deref())?;
            let s = a.positional.first().ok_or("usage: dp2m re text <string> [--px N] [--style H] [--lang N]")?;
            let px = a.value("--px").map_or(Ok(18), parse_u32_auto)?;
            let style = a.value("--style").map_or(Ok(0x22), parse_u32_auto)?;
            let lang = a.value("--lang").map_or(Ok(0), parse_u32_auto)?;
            for line in re::cmd_text(&ctx, s, px, style, lang)? {
                println!("{line}");
            }
        }
        _ => return Err(format!("unknown re subcommand '{sub}'")),
    }
    Ok(())
}

fn x3f_cmd(args: &[String]) -> Result<(), String> {
    use dp2m_emu::x3f;
    let a = Args::parse(args, &["--dump"], &["--all"])?;
    let path = PathBuf::from(a.positional.first().ok_or("usage: dp2m x3f <file.X3F> [--dump out.camf] [--all]")?);
    let raw = read(&path)?;
    let camf = x3f::extract_x3f_camf(&raw)?;
    if let Some(dp) = a.path("--dump") {
        write(&dp, &camf)?;
    }
    let entries = x3f::parse_camf(&camf);
    let show_all = a.has("--all");
    println!("Decoded {} B CAMF, {} entries from {}", camf.len(), entries.len(), path.display());
    const FILTER: &[&str] = &[
        "Color", "CMCM", "CMCC", "MCAS", "CCMatrix", "WBGain", "TC", "Gamma", "Sat",
        "CP1", "CP2", "Camsettings", "Model", "Firmware", "Sharp", "Contrast", "Noise",
        "Mode", "Capture", "WhiteBalance",
    ];
    for e in &entries {
        if show_all || FILTER.iter().any(|w| e.name.contains(w)) {
            let line = x3f::format_entry(e);
            if line.len() > 180 {
                println!("  {}...", &line[..line.floor_char_boundary(177)]);
            } else {
                println!("  {line}");
            }
        }
    }
    Ok(())
}

fn lut_cmd(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) == Some("inspect") {
        let p = args.get(1).ok_or("usage: dp2m lut inspect <LUTn.BIN>")?;
        return lut::inspect_d2ml(Path::new(p));
    }
    let a = Args::parse(args, &["--grain", "--grain-profile", "--halo", "--halo-tint", "--tag"], &[])?;
    let input = PathBuf::from(a.positional.first().ok_or("usage: dp2m lut <in.cube> <out.bin>")?);
    let output = PathBuf::from(a.positional.get(1).ok_or("usage: dp2m lut <in.cube> <out.bin>")?);
    lut::compile_file(&input, &output, a.get("--grain", 28)?, a.get("--grain-profile", 0)?, a.get("--halo", 64)?, a.get("--halo-tint", 0)?, a.value("--tag"))
}

fn run() -> Result<(), String> {
    let args = env::args_os().skip(1).map(|a| a.into_string().map_err(|_| "argument is not valid UTF-8"))
        .collect::<Result<Vec<_>, _>>()?;
    let root = repo_root();
    match args.first().map(String::as_str) {
        Some("build") => build(&root, &args[1..]),
        Some("build-doom") => build_doom_cmd(&root, &args[1..]),
        Some("verify") => verify_cmd(&root, &args[1..]),
        Some("re") => re_cmd(&root, &args[1..]),
        Some("x3f") => x3f_cmd(&args[1..]),
        Some("lut") => lut_cmd(&args[1..]),
        Some("stage") => stage(&root, &args[1..]),
        Some("sim") => sim(&root, &args[1..]),
        Some("sim-trace") => sim_trace(&root, &args[1..]),
        Some("verify-m5") if args.len() == 2 => {
            let path = Path::new(&args[1]);
            let firm = if path.is_dir() { path.join("Firm.bin") } else { path.to_path_buf() };
            m5::verify_m5(&read(&firm)?)
        }
        Some("disasm-fr80") if args.len() == 4 => {
            let buf = read(Path::new(&args[1]))?;
            for line in dp2m_emu::fr80::dis_range(&buf, parse_hex(&args[2])?, parse_hex(&args[3])?, m5::DELTA) {
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
