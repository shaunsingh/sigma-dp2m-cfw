//! two-pass fujitsu fr80 BE assembler w/ .include, .equ, .macro/.endm, .rept, & *.far branches

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AsmError {
    pub loc: String,
    pub msg: String,
}

impl fmt::Display for AsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.loc, self.msg)
    }
}

impl std::error::Error for AsmError {}

fn err(loc: &str, msg: impl Into<String>) -> AsmError {
    AsmError { loc: loc.to_string(), msg: msg.into() }
}

const BRA: u8 = 0xE0;
const BEQ: u8 = 0xE2;
const BNE: u8 = 0xE3;
const BLO: u8 = 0xE4;
const BHS: u8 = 0xE5;
const BLT: u8 = 0xEA;
const BGE: u8 = 0xEB;
const BLE: u8 = 0xEC;
const BGT: u8 = 0xED;
const BLS: u8 = 0xEE;
const BHI: u8 = 0xEF;

fn cc_from_name(name: &str) -> Option<u8> {
    match name {
        "bra" => Some(BRA),
        "beq" => Some(BEQ),
        "bne" => Some(BNE),
        "blo" | "bc" => Some(BLO),
        "bhs" | "bnc" => Some(BHS),
        "blt" => Some(BLT),
        "bge" => Some(BGE),
        "ble" => Some(BLE),
        "bgt" => Some(BGT),
        "bls" => Some(BLS),
        "bhi" => Some(BHI),
        _ => None,
    }
}

fn invert_cc(cc: u8) -> Option<u8> {
    Some(match cc {
        BEQ => BNE,
        BNE => BEQ,
        BLO => BHS,
        BHS => BLO,
        BLT => BGE,
        BGE => BLT,
        BLE => BGT,
        BGT => BLE,
        BLS => BHI,
        BHI => BLS,
        _ => return None,
    })
}

#[derive(Clone)]
struct MacroDef {
    params: Vec<String>,
    body: Vec<(String, String)>,
}

struct Preproc {
    consts: HashMap<String, i64>,
    macros: HashMap<String, MacroDef>,
    lines: Vec<(String, String)>,
    macro_uid: usize,
}

impl Preproc {
    fn new(extra_consts: &HashMap<String, i64>) -> Self {
        Self {
            consts: extra_consts.clone(),
            macros: HashMap::new(),
            lines: Vec::new(),
            macro_uid: 0,
        }
    }

    fn feed(&mut self, src: &str, name: &str, base_dir: Option<&Path>) -> Result<(), AsmError> {
        let raw_lines: Vec<&str> = src.lines().collect();
        let mut idx = 0;
        while idx < raw_lines.len() {
            let loc = format!("{}:{}", name, idx + 1);
            let line = strip_comment(raw_lines[idx]).trim();
            idx += 1;
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix(".include") {
                let inc = rest.trim().trim_matches('"');
                let dir = base_dir.ok_or_else(|| err(&loc, ".include requires file context"))?;
                let path = dir.join(inc);
                let text = fs::read_to_string(&path)
                    .map_err(|e| err(&loc, format!("cannot read include '{}': {}", path.display(), e)))?;
                self.feed(&text, inc, path.parent())?;
                continue;
            }
            if let Some(rest) = line.strip_prefix(".equ") {
                let parts: Vec<&str> = rest.splitn(2, ',').collect();
                if parts.len() != 2 {
                    return Err(err(&loc, "expected `.equ NAME, EXPR`"));
                }
                let sym = parts[0].trim().to_string();
                let val = eval_expr(parts[1].trim(), &self.consts)
                    .map_err(|m| err(&loc, m))?;
                self.consts.insert(sym, val);
                continue;
            }
            if let Some(rest) = line.strip_prefix(".macro") {
                let mut toks = rest.split_whitespace();
                let mname = toks.next().ok_or_else(|| err(&loc, "missing macro name"))?.to_string();
                let param_str = rest.trim()[mname.len()..].trim();
                let params: Vec<String> = if param_str.is_empty() {
                    Vec::new()
                } else {
                    param_str.split(',').map(|s| s.trim().to_string()).collect()
                };
                let mut body = Vec::new();
                let mut closed = false;
                while idx < raw_lines.len() {
                    let bloc = format!("{}:{}", name, idx + 1);
                    let bline = strip_comment(raw_lines[idx]).trim();
                    idx += 1;
                    if bline == ".endm" {
                        closed = true;
                        break;
                    }
                    if !bline.is_empty() {
                        body.push((bloc, bline.to_string()));
                    }
                }
                if !closed {
                    return Err(err(&loc, format!("unterminated .macro {}", mname)));
                }
                self.macros.insert(mname, MacroDef { params, body });
                continue;
            }
            if let Some(rest) = line.strip_prefix(".rept") {
                let mut stream = Vec::with_capacity(raw_lines.len() - idx);
                for (j, rl) in raw_lines[idx..].iter().enumerate() {
                    stream.push((format!("{}:{}", name, idx + j + 1), strip_comment(rl).trim().to_string()));
                }
                let consumed = self.expand_rept(&loc, rest.trim(), &stream)?;
                idx += consumed;
                continue;
            }
            self.expand_line(&loc, line)?;
        }
        Ok(())
    }

    fn expand_rept(&mut self, loc: &str, count_expr: &str, rest: &[(String, String)]) -> Result<usize, AsmError> {
        let count = eval_expr(count_expr, &self.consts).map_err(|m| err(loc, m))?;
        let mut body = Vec::new();
        let mut depth = 1usize;
        let mut consumed = 0usize;
        for (bloc, bline) in rest {
            consumed += 1;
            if bline.starts_with(".rept") {
                depth += 1;
            } else if bline == ".endr" {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            if !bline.is_empty() {
                body.push((bloc.clone(), bline.clone()));
            }
        }
        if depth != 0 {
            return Err(err(loc, "unterminated .rept"));
        }
        for rep_i in 0..count {
            let i_str = rep_i.to_string();
            for (bloc, bline) in &body {
                let expanded = bline.replace("\\i", &i_str);
                self.expand_line(bloc, &expanded)?;
            }
        }
        Ok(consumed)
    }

    fn expand_line(&mut self, loc: &str, line: &str) -> Result<(), AsmError> {
        if line.contains(';') {
            for part in line.split(';') {
                let p = part.trim();
                if !p.is_empty() {
                    self.expand_line(loc, p)?;
                }
            }
            return Ok(());
        }
        let first = line.split_whitespace().next().unwrap_or("");
        if let Some(mdef) = self.macros.get(first).cloned() {
            let arg_str = line[first.len()..].trim();
            let args: Vec<&str> = if arg_str.is_empty() {
                Vec::new()
            } else {
                arg_str.split(',').map(|s| s.trim()).collect()
            };
            if args.len() != mdef.params.len() {
                return Err(err(
                    loc,
                    format!("macro {} expects {} args, got {}", first, mdef.params.len(), args.len()),
                ));
            }
            self.macro_uid += 1;
            let uid = self.macro_uid.to_string();
            let subst: Vec<(String, String)> = mdef
                .body
                .iter()
                .map(|(bloc, bline)| {
                    let mut s = bline.replace("\\@", &uid);
                    for (p, a) in mdef.params.iter().zip(args.iter()) {
                        s = s.replace(&format!("\\{}", p), a);
                    }
                    (format!("{} -> {}", loc, bloc), s)
                })
                .collect();
            let mut bi = 0;
            while bi < subst.len() {
                let (bloc, s) = &subst[bi];
                bi += 1;
                if let Some(rest) = s.strip_prefix(".rept") {
                    bi += self.expand_rept(bloc, rest.trim(), &subst[bi..])?;
                    continue;
                }
                self.expand_line(bloc, s)?;
            }
            return Ok(());
        }
        self.lines.push((loc.to_string(), line.to_string()));
        Ok(())
    }
}

fn strip_comment(line: &str) -> &str {
    // `#` only strips at line start since `#imm` is immediate syntax
    let mut s = line;
    if let Some(i) = s.find("//") {
        s = &s[..i];
    }
    if s.trim_start().starts_with('#') {
        return "";
    }
    s
}

fn eval_expr(expr: &str, syms: &HashMap<String, i64>) -> Result<i64, String> {
    let s = expr.trim().trim_start_matches('#').trim();
    if s.is_empty() {
        return Err("empty expression".to_string());
    }
    let bytes = s.as_bytes();
    let mut terms: Vec<(i64, &str)> = Vec::new();
    let mut start = 0usize;
    let mut sign = 1i64;
    let mut i = 0usize;
    while i < bytes.len() {
        if i == start && (bytes[i] == b'+' || bytes[i] == b'-') {
            if bytes[i] == b'-' {
                sign = -sign;
            }
            i += 1;
            start = i;
            continue;
        }
        if bytes[i] == b'+' || bytes[i] == b'-' {
            terms.push((sign, s[start..i].trim()));
            sign = if bytes[i] == b'-' { -1 } else { 1 };
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    terms.push((sign, s[start..].trim()));

    let mut acc = 0i64;
    for (sgn, tok) in terms {
        let v = parse_atom(tok, syms)?;
        acc += sgn * v;
    }
    Ok(acc)
}

fn parse_atom(tok: &str, syms: &HashMap<String, i64>) -> Result<i64, String> {
    if tok.starts_with('\'') && tok.ends_with('\'') && tok.len() == 3 {
        return Ok(tok.as_bytes()[1] as i64);
    }
    if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).map_err(|_| format!("bad hex '{}'", tok));
    }
    if let Ok(v) = tok.parse::<i64>() {
        return Ok(v);
    }
    if let Some(&v) = syms.get(tok) {
        return Ok(v);
    }
    Err(format!("unknown symbol or immediate '{}'", tok))
}

fn parse_reg(tok: &str) -> Result<u8, String> {
    let t = tok.trim();
    if t == "sp" {
        return Ok(15);
    }
    if t == "fp" {
        return Ok(14);
    }
    let rest = t.strip_prefix('r').ok_or_else(|| format!("expected register, got '{}'", tok))?;
    let n: u8 = rest.parse().map_err(|_| format!("invalid register '{}'", tok))?;
    if n > 15 {
        return Err(format!("register out of range '{}'", tok));
    }
    Ok(n)
}

fn split_operands(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

enum MemAddr {
    AtReg(u8),
    IdxR13(u8),
    FpDisp(i64),
    SpDisp(i64),
    PushSp,
    PopSp,
}

fn parse_mem(op: &str, syms: &HashMap<String, i64>) -> Result<MemAddr, String> {
    let s = op.trim();
    if s == "@-r15" || s == "@-sp" {
        return Ok(MemAddr::PushSp);
    }
    if s == "@r15+" || s == "@sp+" {
        return Ok(MemAddr::PopSp);
    }
    let inner = s.strip_prefix('@').ok_or_else(|| format!("expected @mem, got '{}'", op))?;
    if let Some(paren) = inner.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        let parts: Vec<&str> = paren.split(',').map(|x| x.trim()).collect();
        if parts.len() != 2 {
            return Err(format!("bad memory operand '{}'", op));
        }
        if parts[0] == "r13" {
            return Ok(MemAddr::IdxR13(parse_reg(parts[1])?));
        }
        if parts[0] == "r14" || parts[0] == "fp" {
            return Ok(MemAddr::FpDisp(eval_expr(parts[1], syms)?));
        }
        if parts[0] == "r15" || parts[0] == "sp" {
            return Ok(MemAddr::SpDisp(eval_expr(parts[1], syms)?));
        }
        return Err(format!("unsupported indexed memory '{}'", op));
    }
    Ok(MemAddr::AtReg(parse_reg(inner)?))
}

enum Item {
    Label { loc: String, name: String },
    Bytes(Vec<u8>),
    Branch { loc: String, cc: u8, target: String },
    BranchFar { loc: String, cc: u8, target: String },
    JmpFar { loc: String, target: String },
    CallFar { loc: String, target: String },
}

pub fn assemble_file(
    path: &Path,
    base_va: u32,
    extra_consts: &HashMap<String, i64>,
) -> Result<Vec<u8>, AsmError> {
    assemble_file_labels(path, base_va, extra_consts).map(|(b, _)| b)
}

/// Assembled bytes plus every label's VA, in address order.
pub type Labelled = (Vec<u8>, Vec<(u32, String)>);

/// [`assemble_file`] plus every label's VA, in address order
pub fn assemble_file_labels(
    path: &Path,
    base_va: u32,
    extra_consts: &HashMap<String, i64>,
) -> Result<Labelled, AsmError> {
    let src = fs::read_to_string(path)
        .map_err(|e| err(&path.display().to_string(), format!("read error: {}", e)))?;
    let mut pp = Preproc::new(extra_consts);
    pp.feed(&src, &path.display().to_string(), path.parent())?;
    let (bytes, labels) = assemble_lines(&pp.lines, base_va, &pp.consts)?;
    let mut labels: Vec<_> = labels.into_iter().map(|(l, va)| (va, l)).collect();
    labels.sort();
    Ok((bytes, labels))
}

pub fn assemble_sections(
    path: &Path,
    section_vas: &HashMap<String, u32>,
    extra_consts: &HashMap<String, i64>,
) -> Result<HashMap<String, Vec<u8>>, AsmError> {
    let src = fs::read_to_string(path)
        .map_err(|e| err(&path.display().to_string(), format!("read error: {}", e)))?;
    let mut pp = Preproc::new(extra_consts);
    pp.feed(&src, &path.display().to_string(), path.parent())?;

    let mut buckets: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut cur_sec: Option<String> = None;
    for (loc, line) in pp.lines {
        if let Some(rest) = line.strip_prefix(".section") {
            let name = rest.trim().to_string();
            if !section_vas.contains_key(&name) {
                return Err(err(&loc, format!("unknown section '{}'", name)));
            }
            cur_sec = Some(name);
            continue;
        }
        let sec = cur_sec
            .as_ref()
            .ok_or_else(|| err(&loc, "instruction outside `.section` block"))?;
        buckets.entry(sec.clone()).or_default().push((loc, line));
    }

    let mut out = HashMap::new();
    for (name, &va) in section_vas {
        let lines = buckets.remove(name).unwrap_or_default();
        let bytes = assemble_lines(&lines, va, &pp.consts)?.0;
        out.insert(name.clone(), bytes);
    }
    Ok(out)
}

pub fn assemble_str(
    src: &str,
    base_va: u32,
    extra_consts: &HashMap<String, i64>,
) -> Result<Vec<u8>, AsmError> {
    let mut pp = Preproc::new(extra_consts);
    pp.feed(src, "<inline>", None)?;
    Ok(assemble_lines(&pp.lines, base_va, &pp.consts)?.0)
}

fn assemble_lines(
    lines: &[(String, String)],
    base_va: u32,
    consts: &HashMap<String, i64>,
) -> Result<(Vec<u8>, HashMap<String, u32>), AsmError> {
    let mut items: Vec<Item> = Vec::new();
    for (loc, raw_line) in lines {
        let mut line = raw_line.as_str();
        while let Some(colon) = line.find(':') {
            let candidate = line[..colon].trim();
            if !candidate.is_empty() && !candidate.contains(char::is_whitespace) && candidate != "ldi" {
                items.push(Item::Label { loc: loc.clone(), name: candidate.to_string() });
                line = line[colon + 1..].trim();
            } else {
                break;
            }
        }
        if line.is_empty() {
            continue;
        }
        items.push(encode_stmt(loc, line, consts)?);
    }

    let mut labels: HashMap<String, u32> = HashMap::new();
    let mut pos = base_va;
    for it in &items {
        match it {
            Item::Label { loc, name } => {
                if labels.insert(name.clone(), pos).is_some() {
                    return Err(err(loc, format!("duplicate label '{}'", name)));
                }
            }
            Item::Bytes(b) => pos += b.len() as u32,
            Item::Branch { .. } => pos += 2,
            Item::BranchFar { .. } => pos += 10,
            Item::JmpFar { .. } | Item::CallFar { .. } => pos += 8,
        }
    }

    let mut out = Vec::with_capacity((pos - base_va) as usize);
    let mut cur = base_va;
    for it in &items {
        match it {
            Item::Label { .. } => {}
            Item::Bytes(b) => {
                out.extend_from_slice(b);
                cur += b.len() as u32;
            }
            Item::Branch { loc, cc, target } => {
                let tgt = *labels
                    .get(target)
                    .ok_or_else(|| err(loc, format!("undefined label '{}'", target)))?;
                let disp = (tgt as i64) - ((cur as i64) + 2);
                if disp % 2 != 0 || !(-256..=254).contains(&disp) {
                    return Err(err(
                        loc,
                        format!("short branch to '{}' out of range (disp={}); use .far", target, disp),
                    ));
                }
                out.push(*cc);
                out.push(((disp / 2) as i8) as u8);
                cur += 2;
            }
            Item::BranchFar { loc, cc, target } => {
                let tgt = *labels
                    .get(target)
                    .ok_or_else(|| err(loc, format!("undefined label '{}'", target)))?;
                let inv = invert_cc(*cc).ok_or_else(|| err(loc, "cannot invert branch cc"))?;
                out.push(inv);
                out.push(4);
                out.extend_from_slice(&[0x9F, 0x8C]);
                out.extend_from_slice(&tgt.to_be_bytes());
                out.extend_from_slice(&[0x97, 0x0C]);
                cur += 10;
            }
            Item::JmpFar { loc, target } => {
                let tgt = *labels
                    .get(target)
                    .ok_or_else(|| err(loc, format!("undefined label '{}'", target)))?;
                out.extend_from_slice(&[0x9F, 0x8C]);
                out.extend_from_slice(&tgt.to_be_bytes());
                out.extend_from_slice(&[0x97, 0x0C]);
                cur += 8;
            }
            Item::CallFar { loc, target } => {
                let tgt = *labels
                    .get(target)
                    .ok_or_else(|| err(loc, format!("undefined label '{}'", target)))?;
                out.extend_from_slice(&[0x9F, 0x8C]);
                out.extend_from_slice(&tgt.to_be_bytes());
                out.extend_from_slice(&[0x97, 0x1C]);
                cur += 8;
            }
        }
    }
    Ok((out, labels))
}

fn encode_stmt(loc: &str, line: &str, syms: &HashMap<String, i64>) -> Result<Item, AsmError> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let mnem = parts.next().unwrap().to_lowercase();
    let rest = parts.next().unwrap_or("").trim();
    let ops = split_operands(rest);
    let e = |m: String| err(loc, m);

    if mnem == "jmp.far" || mnem == "bra.far" {
        if ops.len() != 1 {
            return Err(e("expected 1 target label".into()));
        }
        return Ok(Item::JmpFar { loc: loc.to_string(), target: ops[0].clone() });
    }
    if mnem == "call.far" {
        if ops.len() != 1 {
            return Err(e("expected 1 target label".into()));
        }
        return Ok(Item::CallFar { loc: loc.to_string(), target: ops[0].clone() });
    }
    if let Some(base) = mnem.strip_suffix(".far")
        && let Some(cc) = cc_from_name(base)
    {
        if ops.len() != 1 {
            return Err(e("expected 1 target label".into()));
        }
        return Ok(Item::BranchFar { loc: loc.to_string(), cc, target: ops[0].clone() });
    }
    if let Some(cc) = cc_from_name(&mnem) {
        if ops.len() != 1 {
            return Err(e("expected 1 target label".into()));
        }
        return Ok(Item::Branch { loc: loc.to_string(), cc, target: ops[0].clone() });
    }

    let bytes: Vec<u8> = match mnem.as_str() {
        "nop" => vec![0x9F, 0xA0],
        "ret" => vec![0x97, 0x20],
        "leave" => vec![0x9F, 0x90],
        "enter" => {
            let sz = eval_expr(&ops[0], syms).map_err(e)?;
            if sz % 4 != 0 || !(4..=1020).contains(&sz) {
                return Err(e(format!("invalid enter size {}", sz)));
            }
            vec![0x0F, (sz / 4) as u8]
        }
        "addsp" => {
            let d = eval_expr(&ops[0], syms).map_err(e)?;
            if d % 4 != 0 || !(-512..=508).contains(&d) {
                return Err(e(format!("invalid addsp {}", d)));
            }
            vec![0xA3, ((d / 4) as i8) as u8]
        }
        "ldi:8" => {
            let imm = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(0..=255).contains(&imm) {
                return Err(e(format!("ldi:8 immediate {} out of [0..255]", imm)));
            }
            let u = imm as u8;
            vec![0xC0 | (u >> 4), ((u & 0xF) << 4) | dst]
        }
        "ldi:20" => {
            let imm = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(0..=0xF_FFFF).contains(&imm) {
                return Err(e(format!("ldi:20 immediate {} out of 20-bit range", imm)));
            }
            let u = imm as u32;
            vec![0x9B, (((u >> 16) as u8) << 4) | dst, ((u >> 8) & 0xFF) as u8, (u & 0xFF) as u8]
        }
        "ldi:32" => {
            let imm = eval_expr(&ops[0], syms).map_err(e)? as u32;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            let mut v = vec![0x9F, 0x80 | dst];
            v.extend_from_slice(&imm.to_be_bytes());
            v
        }
        "mov" => {
            if ops[0].eq_ignore_ascii_case("mdl") {
                let dst = parse_reg(&ops[1]).map_err(e)?;
                vec![0xB7, 0x50 | dst]
            } else {
                let src = parse_reg(&ops[0]).map_err(e)?;
                let dst = parse_reg(&ops[1]).map_err(e)?;
                vec![0x8B, (src << 4) | dst]
            }
        }
        "add" => {
            if ops[0].starts_with('#') {
                let u4 = eval_expr(&ops[0], syms).map_err(e)?;
                let dst = parse_reg(&ops[1]).map_err(e)?;
                if !(0..=15).contains(&u4) {
                    return Err(e(format!("add #u4 out of range: {}", u4)));
                }
                vec![0xA4, ((u4 as u8) << 4) | dst]
            } else {
                let src = parse_reg(&ops[0]).map_err(e)?;
                let dst = parse_reg(&ops[1]).map_err(e)?;
                vec![0xA6, (src << 4) | dst]
            }
        }
        "add2" => {
            let neg = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(-16..=-1).contains(&neg) {
                return Err(e(format!("add2 immediate {} out of [-16..-1]", neg)));
            }
            vec![0xA5, (((neg + 16) as u8) << 4) | dst]
        }
        "sub" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            vec![0xAC, (src << 4) | dst]
        }
        "mul" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            vec![0xAF, (src << 4) | dst]
        }
        "and" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            vec![0x82, (src << 4) | dst]
        }
        "or" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            vec![0x92, (src << 4) | dst]
        }
        "eor" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            vec![0x9A, (src << 4) | dst]
        }
        "cmp" => {
            if ops[0].starts_with('#') {
                let u4 = eval_expr(&ops[0], syms).map_err(e)?;
                let dst = parse_reg(&ops[1]).map_err(e)?;
                if !(0..=15).contains(&u4) {
                    return Err(e(format!("cmp #u4 out of range: {}", u4)));
                }
                vec![0xA8, ((u4 as u8) << 4) | dst]
            } else {
                let src = parse_reg(&ops[0]).map_err(e)?;
                let dst = parse_reg(&ops[1]).map_err(e)?;
                vec![0xAA, (src << 4) | dst]
            }
        }
        // fr80 shifts (silicon-verified @ 0x4D3F72): 0xB0=LSR 0xB4=LSL 0xB8=ASR
        "lsr" => {
            let u4 = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(0..=15).contains(&u4) {
                return Err(e(format!("lsr #u4 out of range: {}", u4)));
            }
            vec![0xB0, ((u4 as u8) << 4) | dst]
        }
        "lsl" => {
            let u4 = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(0..=15).contains(&u4) {
                return Err(e(format!("lsl #u4 out of range: {}", u4)));
            }
            vec![0xB4, ((u4 as u8) << 4) | dst]
        }
        "asr" => {
            let u4 = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(0..=15).contains(&u4) {
                return Err(e(format!("asr #u4 out of range: {}", u4)));
            }
            vec![0xB8, ((u4 as u8) << 4) | dst]
        }
        // *2 forms shift by 16..31 (stock `b1f1 lsr2 #31` @ 0x4D3F6E)
        "lsr2" | "lsl2" | "asr2" => {
            let n = eval_expr(&ops[0], syms).map_err(e)?;
            let dst = parse_reg(&ops[1]).map_err(e)?;
            if !(16..=31).contains(&n) {
                return Err(e(format!("{} #n out of [16..31]: {}", mnem, n)));
            }
            let op = match mnem.as_str() {
                "lsr2" => 0xB1,
                "lsl2" => 0xB5,
                _ => 0xB9,
            };
            vec![op, (((n - 16) as u8) << 4) | dst]
        }
        "extsb" => vec![0x97, 0x80 | parse_reg(&ops[0]).map_err(e)?],
        "extub" => vec![0x97, 0x90 | parse_reg(&ops[0]).map_err(e)?],
        "extsh" => vec![0x97, 0xA0 | parse_reg(&ops[0]).map_err(e)?],
        "extuh" => vec![0x97, 0xB0 | parse_reg(&ops[0]).map_err(e)?],
        "jmp" => {
            let r = parse_reg(ops[0].trim_start_matches('@')).map_err(e)?;
            vec![0x97, r]
        }
        "call" => {
            let r = parse_reg(ops[0].trim_start_matches('@')).map_err(e)?;
            vec![0x97, 0x10 | r]
        }
        "ld" => {
            let mem = parse_mem(&ops[0], syms).map_err(e)?;
            if ops[1].eq_ignore_ascii_case("rp") {
                match mem {
                    MemAddr::PopSp => vec![0x07, 0x81],
                    _ => return Err(e("ld into RP only supports @r15+".into())),
                }
            } else {
                let dst = parse_reg(&ops[1]).map_err(e)?;
                match mem {
                    MemAddr::AtReg(b) => vec![0x04, (b << 4) | dst],
                    MemAddr::IdxR13(b) => vec![0x00, (b << 4) | dst],
                    MemAddr::FpDisp(d) => {
                        if d % 4 != 0 || !(-512..=508).contains(&d) {
                            return Err(e(format!("fp disp {} out of range", d)));
                        }
                        let s8 = ((d / 4) as i8) as u8;
                        vec![0x20 | (s8 >> 4), ((s8 & 0xF) << 4) | dst]
                    }
                    MemAddr::SpDisp(d) => {
                        if d % 4 != 0 || !(0..=60).contains(&d) {
                            return Err(e(format!("sp disp {} out of range", d)));
                        }
                        vec![0x03, (((d / 4) as u8) << 4) | dst]
                    }
                    MemAddr::PopSp => vec![0x07, dst],
                    MemAddr::PushSp => return Err(e("cannot ld from @-r15".into())),
                }
            }
        }
        "lduh" => {
            let dst = parse_reg(&ops[1]).map_err(e)?;
            match parse_mem(&ops[0], syms).map_err(e)? {
                MemAddr::AtReg(b) => vec![0x05, (b << 4) | dst],
                MemAddr::IdxR13(b) => vec![0x01, (b << 4) | dst],
                _ => return Err(e("unsupported lduh addressing mode".into())),
            }
        }
        "ldub" => {
            let dst = parse_reg(&ops[1]).map_err(e)?;
            match parse_mem(&ops[0], syms).map_err(e)? {
                MemAddr::AtReg(b) => vec![0x06, (b << 4) | dst],
                MemAddr::IdxR13(b) => {
                    if dst == 13 {
                        return Err(e("avoid `ldub @(r13, Rx), r13` (dst == index register)".into()));
                    }
                    vec![0x02, (b << 4) | dst]
                }
                _ => return Err(e("unsupported ldub addressing mode".into())),
            }
        }
        "st" => {
            let mem = parse_mem(&ops[1], syms).map_err(e)?;
            if ops[0].eq_ignore_ascii_case("rp") {
                match mem {
                    MemAddr::PushSp => vec![0x17, 0x81],
                    _ => return Err(e("st RP only supports @-r15".into())),
                }
            } else {
                let src = parse_reg(&ops[0]).map_err(e)?;
                match mem {
                    MemAddr::AtReg(b) => vec![0x14, (b << 4) | src],
                    MemAddr::IdxR13(b) => vec![0x10, (b << 4) | src],
                    MemAddr::FpDisp(d) => {
                        if d % 4 != 0 || !(-512..=508).contains(&d) {
                            return Err(e(format!("fp disp {} out of range", d)));
                        }
                        let s8 = ((d / 4) as i8) as u8;
                        vec![0x30 | (s8 >> 4), ((s8 & 0xF) << 4) | src]
                    }
                    MemAddr::SpDisp(d) => {
                        if d % 4 != 0 || !(0..=60).contains(&d) {
                            return Err(e(format!("sp disp {} out of range", d)));
                        }
                        vec![0x13, (((d / 4) as u8) << 4) | src]
                    }
                    MemAddr::PushSp => vec![0x17, src],
                    MemAddr::PopSp => return Err(e("cannot st to @r15+".into())),
                }
            }
        }
        "sth" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            match parse_mem(&ops[1], syms).map_err(e)? {
                MemAddr::AtReg(b) => vec![0x15, (b << 4) | src],
                MemAddr::IdxR13(b) => vec![0x11, (b << 4) | src],
                _ => return Err(e("unsupported sth addressing mode".into())),
            }
        }
        "stb" => {
            let src = parse_reg(&ops[0]).map_err(e)?;
            match parse_mem(&ops[1], syms).map_err(e)? {
                MemAddr::AtReg(b) => vec![0x16, (b << 4) | src],
                MemAddr::IdxR13(b) => vec![0x12, (b << 4) | src],
                _ => return Err(e("unsupported stb addressing mode".into())),
            }
        }
        _ => return Err(e(format!("unknown mnemonic '{}'", mnem))),
    };
    Ok(Item::Bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_encodings_match_fr80_silicon() {
        let empty = HashMap::new();
        assert_eq!(assemble_str("asr #1, r0", 0, &empty).unwrap(), vec![0xB8, 0x10]);
        assert_eq!(assemble_str("lsr #1, r0", 0, &empty).unwrap(), vec![0xB0, 0x10]);
        assert_eq!(assemble_str("lsl #1, r0", 0, &empty).unwrap(), vec![0xB4, 0x10]);
        assert_eq!(assemble_str("lsr2 #31, r1", 0, &empty).unwrap(), vec![0xB1, 0xF1]);
        assert_eq!(assemble_str("lsl2 #24, r2", 0, &empty).unwrap(), vec![0xB5, 0x82]);
        assert!(assemble_str("lsr2 #15, r0", 0, &empty).is_err());
    }

    #[test]
    fn rejects_ldub_indexed_into_r13() {
        let empty = HashMap::new();
        assert!(assemble_str("ldub @(r13, r3), r13", 0, &empty).is_err());
    }
}
