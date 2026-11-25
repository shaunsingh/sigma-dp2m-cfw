//! minimal ELF32 big-endian reader: sections & symbols of a linked image

pub struct Section<'a> {
    pub name: String,
    pub kind: u32,
    pub flags: u32,
    pub addr: u32,
    pub data: &'a [u8],
    pub size: u32,
}

pub struct Symbol {
    pub name: String,
    pub value: u32,
    pub size: u32,
}

pub const SHT_NOBITS: u32 = 8;
pub const SHF_ALLOC: u32 = 2;

pub struct Elf<'a> {
    pub sections: Vec<Section<'a>>,
    pub symbols: Vec<Symbol>,
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}

fn be16(b: &[u8], o: usize) -> usize {
    u16::from_be_bytes(b[o..o + 2].try_into().unwrap()) as usize
}

fn cstr(b: &[u8], o: usize) -> String {
    let end = b[o..].iter().position(|&c| c == 0).map_or(b.len(), |p| o + p);
    String::from_utf8_lossy(&b[o..end]).into_owned()
}

impl<'a> Elf<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Self, String> {
        if b.len() < 52 || &b[0..4] != b"\x7fELF" || b[4] != 1 || b[5] != 2 {
            return Err("expected ELF32 big-endian".into());
        }
        let (shoff, shentsize, shnum, shstrndx) = (be32(b, 32) as usize, be16(b, 46), be16(b, 48), be16(b, 50));
        let hdr = |i: usize| shoff + i * shentsize;
        let shstr = be32(b, hdr(shstrndx) + 16) as usize;

        let mut sections = Vec::with_capacity(shnum);
        let mut symtab = None;
        for i in 0..shnum {
            let h = hdr(i);
            let (kind, off, size) = (be32(b, h + 4), be32(b, h + 16) as usize, be32(b, h + 20));
            let data = if kind == SHT_NOBITS { &b[0..0] } else { &b[off..off + size as usize] };
            if kind == 2 {
                symtab = Some((off, size as usize, be32(b, h + 24) as usize));
            }
            sections.push(Section {
                name: cstr(b, shstr + be32(b, h) as usize),
                kind,
                flags: be32(b, h + 8),
                addr: be32(b, h + 12),
                data,
                size,
            });
        }

        let mut symbols = Vec::new();
        if let Some((off, size, link)) = symtab {
            let strtab = be32(b, hdr(link) + 16) as usize;
            for s in (off..off + size).step_by(16).skip(1) {
                symbols.push(Symbol { name: cstr(b, strtab + be32(b, s) as usize), value: be32(b, s + 4), size: be32(b, s + 8) });
            }
        }
        Ok(Self { sections, symbols })
    }

    pub fn section(&self, name: &str) -> Option<&Section<'a>> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn symbol(&self, name: &str) -> Option<&Symbol> {
        self.symbols.iter().find(|s| s.name == name)
    }
}
