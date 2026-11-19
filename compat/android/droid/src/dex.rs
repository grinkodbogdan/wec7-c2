//! DEX (Dalvik executable) files: the parts an interpreter needs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const NO_INDEX: u32 = 0xffff_ffff;
pub const ACC_STATIC: u32 = 0x8;
pub const ACC_NATIVE: u32 = 0x100;
pub const ACC_INTERFACE: u32 = 0x200;

pub struct ClassDef {
    pub class_idx: u32,
    pub access: u32,
    pub super_idx: u32,
    pub interfaces_off: u32,
    pub class_data_off: u32,
    pub static_values_off: u32,
}

#[derive(Default)]
pub struct ClassData {
    pub static_fields: Vec<(u32, u32)>,
    pub instance_fields: Vec<(u32, u32)>,
    /// (method index, access flags, code offset)
    pub direct_methods: Vec<(u32, u32, u32)>,
    pub virtual_methods: Vec<(u32, u32, u32)>,
}

pub struct Handler {
    /// (exception type descriptor, handler address); `None` type = catch-all.
    pub catches: Vec<(Option<Rc<str>>, u32)>,
}

pub struct Try {
    pub start: u32,
    pub end: u32,
    pub handler: Rc<Handler>,
}

pub struct Code {
    pub registers: usize,
    pub ins: usize,
    pub insns: Vec<u16>,
    pub tries: Vec<Try>,
}

#[derive(Debug, Clone)]
pub enum Encoded {
    Int(i64),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(u32),
    Type(u32),
    Null,
    Bool(bool),
    Other,
}

pub struct Dex {
    data: Vec<u8>,
    string_offs: Vec<u32>,
    pub types: Vec<u32>,
    protos: Vec<(u32, u32, u32)>,
    fields: Vec<(u16, u16, u32)>,
    methods: Vec<(u16, u16, u32)>,
    pub class_defs: Vec<ClassDef>,
    string_cache: RefCell<HashMap<u32, Rc<str>>>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn uleb(b: &[u8], p: &mut usize) -> u32 {
    let mut r = 0u32;
    let mut shift = 0;
    loop {
        let byte = b[*p];
        *p += 1;
        r |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return r;
        }
        shift += 7;
    }
}

pub fn sleb(b: &[u8], p: &mut usize) -> i32 {
    let mut r = 0i32;
    let mut shift = 0;
    loop {
        let byte = b[*p];
        *p += 1;
        r |= ((byte & 0x7f) as i32) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            if shift < 32 && byte & 0x40 != 0 {
                r |= -1 << shift;
            }
            return r;
        }
    }
}

/// Modified UTF-8 (as in DEX string data) to a Rust string.
fn mutf8(b: &[u8], mut p: usize) -> String {
    let mut units: Vec<u16> = Vec::new();
    loop {
        let a = b[p] as u16;
        if a == 0 {
            break;
        }
        if a < 0x80 {
            units.push(a);
            p += 1;
        } else if a & 0xe0 == 0xc0 {
            units.push(((a & 0x1f) << 6) | (b[p + 1] as u16 & 0x3f));
            p += 2;
        } else {
            units.push(((a & 0x0f) << 12) | ((b[p + 1] as u16 & 0x3f) << 6) | (b[p + 2] as u16 & 0x3f));
            p += 3;
        }
    }
    String::from_utf16_lossy(&units)
}

impl Dex {
    pub fn parse(data: Vec<u8>) -> Result<Dex, String> {
        if data.len() < 0x70 || &data[0..4] != b"dex\n" {
            return Err("not a DEX file".into());
        }
        let sect = |o: usize| (u32_at(&data, o) as usize, u32_at(&data, o + 4) as usize);
        let (ns, os) = sect(0x38);
        let (nt, ot) = sect(0x40);
        let (np, op) = sect(0x48);
        let (nf, of) = sect(0x50);
        let (nm, om) = sect(0x58);
        let (nc, oc) = sect(0x60);
        let string_offs = (0..ns).map(|i| u32_at(&data, os + i * 4)).collect();
        let types = (0..nt).map(|i| u32_at(&data, ot + i * 4)).collect();
        let protos = (0..np)
            .map(|i| (u32_at(&data, op + i * 12), u32_at(&data, op + i * 12 + 4), u32_at(&data, op + i * 12 + 8)))
            .collect();
        let fields = (0..nf)
            .map(|i| (u16_at(&data, of + i * 8), u16_at(&data, of + i * 8 + 2), u32_at(&data, of + i * 8 + 4)))
            .collect();
        let methods = (0..nm)
            .map(|i| (u16_at(&data, om + i * 8), u16_at(&data, om + i * 8 + 2), u32_at(&data, om + i * 8 + 4)))
            .collect();
        let class_defs = (0..nc)
            .map(|i| {
                let o = oc + i * 32;
                ClassDef {
                    class_idx: u32_at(&data, o),
                    access: u32_at(&data, o + 4),
                    super_idx: u32_at(&data, o + 8),
                    interfaces_off: u32_at(&data, o + 12),
                    class_data_off: u32_at(&data, o + 24),
                    static_values_off: u32_at(&data, o + 28),
                }
            })
            .collect();
        Ok(Dex {
            data,
            string_offs,
            types,
            protos,
            fields,
            methods,
            class_defs,
            string_cache: RefCell::new(HashMap::new()),
        })
    }

    pub fn string(&self, idx: u32) -> Rc<str> {
        if let Some(s) = self.string_cache.borrow().get(&idx) {
            return s.clone();
        }
        let mut p = self.string_offs[idx as usize] as usize;
        uleb(&self.data, &mut p);
        let s: Rc<str> = mutf8(&self.data, p).into();
        self.string_cache.borrow_mut().insert(idx, s.clone());
        s
    }

    pub fn type_name(&self, idx: u32) -> Rc<str> {
        self.string(self.types[idx as usize])
    }

    fn type_list(&self, off: u32) -> Vec<Rc<str>> {
        if off == 0 {
            return Vec::new();
        }
        let o = off as usize;
        let n = u32_at(&self.data, o) as usize;
        (0..n).map(|i| self.type_name(u16_at(&self.data, o + 4 + i * 2) as u32)).collect()
    }

    pub fn proto_desc(&self, idx: u32) -> String {
        let (_, ret, params) = self.protos[idx as usize];
        let mut s = String::from("(");
        for p in self.type_list(params) {
            s.push_str(&p);
        }
        s.push(')');
        s.push_str(&self.type_name(ret));
        s
    }

    /// (class descriptor, name, descriptor)
    pub fn method_ref(&self, idx: u32) -> (Rc<str>, Rc<str>, Rc<str>) {
        let (c, p, n) = self.methods[idx as usize];
        (self.type_name(c as u32), self.string(n), self.proto_desc(p as u32).into())
    }

    /// (class descriptor, name, type descriptor)
    pub fn field_ref(&self, idx: u32) -> (Rc<str>, Rc<str>, Rc<str>) {
        let (c, t, n) = self.fields[idx as usize];
        (self.type_name(c as u32), self.string(n), self.type_name(t as u32))
    }

    pub fn interfaces(&self, def: &ClassDef) -> Vec<Rc<str>> {
        self.type_list(def.interfaces_off)
    }

    pub fn class_data(&self, def: &ClassDef) -> ClassData {
        let mut cd = ClassData::default();
        if def.class_data_off == 0 {
            return cd;
        }
        let b = &self.data;
        let mut p = def.class_data_off as usize;
        let counts = [uleb(b, &mut p), uleb(b, &mut p), uleb(b, &mut p), uleb(b, &mut p)];
        for (k, &n) in counts.iter().enumerate().take(2) {
            let mut idx = 0;
            for _ in 0..n {
                idx += uleb(b, &mut p);
                let acc = uleb(b, &mut p);
                if k == 0 {
                    cd.static_fields.push((idx, acc));
                } else {
                    cd.instance_fields.push((idx, acc));
                }
            }
        }
        for (k, &n) in counts.iter().enumerate().skip(2) {
            let mut idx = 0;
            for _ in 0..n {
                idx += uleb(b, &mut p);
                let acc = uleb(b, &mut p);
                let code = uleb(b, &mut p);
                if k == 2 {
                    cd.direct_methods.push((idx, acc, code));
                } else {
                    cd.virtual_methods.push((idx, acc, code));
                }
            }
        }
        cd
    }

    pub fn code(&self, off: u32) -> Code {
        let b = &self.data;
        let o = off as usize;
        let registers = u16_at(b, o) as usize;
        let ins = u16_at(b, o + 2) as usize;
        let tries_size = u16_at(b, o + 6) as usize;
        let n = u32_at(b, o + 12) as usize;
        let insns: Vec<u16> = (0..n).map(|i| u16_at(b, o + 16 + i * 2)).collect();
        let mut tries = Vec::new();
        if tries_size > 0 {
            let mut t = o + 16 + n * 2;
            if n % 2 == 1 {
                t += 2;
            }
            let handlers_base = t + tries_size * 8;
            let mut cache: HashMap<usize, Rc<Handler>> = HashMap::new();
            for i in 0..tries_size {
                let start = u32_at(b, t + i * 8);
                let count = u16_at(b, t + i * 8 + 4) as u32;
                let hoff = handlers_base + u16_at(b, t + i * 8 + 6) as usize;
                let handler = cache
                    .entry(hoff)
                    .or_insert_with(|| {
                        let mut p = hoff;
                        let size = sleb(b, &mut p);
                        let mut catches = Vec::new();
                        for _ in 0..size.unsigned_abs() {
                            let ty = uleb(b, &mut p);
                            let addr = uleb(b, &mut p);
                            catches.push((Some(self.type_name(ty)), addr));
                        }
                        if size <= 0 {
                            catches.push((None, uleb(b, &mut p)));
                        }
                        Rc::new(Handler { catches })
                    })
                    .clone();
                tries.push(Try { start, end: start + count, handler });
            }
        }
        Code { registers, ins, insns, tries }
    }

    pub fn static_values(&self, def: &ClassDef) -> Vec<Encoded> {
        if def.static_values_off == 0 {
            return Vec::new();
        }
        let mut p = def.static_values_off as usize;
        let n = uleb(&self.data, &mut p);
        (0..n).map(|_| self.encoded_value(&mut p)).collect()
    }

    fn encoded_value(&self, p: &mut usize) -> Encoded {
        let b = &self.data;
        let head = b[*p];
        *p += 1;
        let (ty, arg) = (head & 0x1f, (head >> 5) as usize);
        let mut raw = |signed: bool| -> u64 {
            let n = arg + 1;
            let mut v = 0u64;
            for i in 0..n {
                v |= (b[*p + i] as u64) << (8 * i);
            }
            *p += n;
            if signed && n < 8 && (v >> (8 * n - 1)) & 1 == 1 {
                v |= !0u64 << (8 * n);
            }
            v
        };
        match ty {
            0x00 | 0x02 | 0x04 => Encoded::Int(raw(true) as i64),
            0x03 => Encoded::Int(raw(false) as i64),
            0x06 => Encoded::Long(raw(true) as i64),
            0x10 => Encoded::Float(f32::from_bits((raw(false) << (8 * (3 - arg))) as u32)),
            0x11 => Encoded::Double(f64::from_bits(raw(false) << (8 * (7 - arg)))),
            0x17 => Encoded::Str(raw(false) as u32),
            0x18 => Encoded::Type(raw(false) as u32),
            0x1e => Encoded::Null,
            0x1f => Encoded::Bool(arg != 0),
            0x1c => {
                let n = uleb(b, p);
                for _ in 0..n {
                    self.encoded_value(p);
                }
                Encoded::Other
            }
            0x1d => {
                uleb(b, p);
                let n = uleb(b, p);
                for _ in 0..n {
                    uleb(b, p);
                    self.encoded_value(p);
                }
                Encoded::Other
            }
            _ => {
                raw(false);
                Encoded::Other
            }
        }
    }
}
