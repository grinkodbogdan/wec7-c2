//! The DEX interpreter: values, objects, classes, method resolution,
//! exceptions and the Dalvik instruction set.
//!
//! Classes come from two places: the app's DEX files, and native classes
//! (the Java core library and the Android API surface) implemented in Rust
//! in `natives.rs`. App classes can extend native ones (an Activity), and
//! native code can call back into app code (equals, hashCode, Runnable).

use crate::dex::{self, Code, Dex, Encoded};
use crate::natives;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub type Ref = Rc<Obj>;
/// A method's outcome: a value, or a thrown exception object.
pub type R<T> = Result<T, Ref>;
pub type NativeFn = fn(&mut Vm, &[V]) -> R<V>;

/// A register or field value: primitive bits (32-bit values in the low
/// half; wide values whole) or a reference.
#[derive(Clone, Debug)]
pub enum V {
    P(u64),
    R(Option<Ref>),
}

impl Default for V {
    fn default() -> V {
        V::P(0)
    }
}

impl V {
    pub fn i(&self) -> i32 {
        match self {
            V::P(b) => *b as u32 as i32,
            V::R(_) => 0,
        }
    }
    pub fn j(&self) -> i64 {
        match self {
            V::P(b) => *b as i64,
            V::R(_) => 0,
        }
    }
    pub fn f(&self) -> f32 {
        f32::from_bits(self.i() as u32)
    }
    pub fn d(&self) -> f64 {
        f64::from_bits(self.j() as u64)
    }
    pub fn r(&self) -> Option<Ref> {
        match self {
            V::R(r) => r.clone(),
            V::P(_) => None,
        }
    }
    pub fn int(v: i32) -> V {
        V::P(v as u32 as u64)
    }
    pub fn long(v: i64) -> V {
        V::P(v as u64)
    }
    pub fn float(v: f32) -> V {
        V::P(v.to_bits() as u64)
    }
    pub fn double(v: f64) -> V {
        V::P(v.to_bits())
    }
    pub fn obj(r: Ref) -> V {
        V::R(Some(r))
    }
    pub const NULL: V = V::R(None);
    pub fn bool(b: bool) -> V {
        V::int(b as i32)
    }
    fn is_zero(&self) -> bool {
        match self {
            V::P(b) => *b as u32 == 0,
            V::R(r) => r.is_none(),
        }
    }
}

/// Arrays: primitive elements as bits, or references.
pub struct Arr {
    pub elem: char,
    pub prim: Vec<u64>,
    pub refs: Vec<Option<Ref>>,
}

impl Arr {
    pub fn len(&self) -> usize {
        if self.elem == 'L' || self.elem == '[' {
            self.refs.len()
        } else {
            self.prim.len()
        }
    }
}

/// Native state attached to an object.
pub enum Ext {
    None,
    Str(Rc<str>),
    Arr(Arr),
    List(Vec<V>),
    Map(Vec<(V, V)>),
    Builder(String),
    Class(Rc<Class>),
    Iter(Ref, usize),
    Boxed(V),
}

pub struct Obj {
    pub class: Rc<Class>,
    pub fields: RefCell<Vec<V>>,
    pub ext: RefCell<Ext>,
}

impl std::fmt::Debug for Obj {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "<{}>", self.class.name)
    }
}

pub enum Body {
    Code(Rc<Code>, Rc<Dex>),
    Native(NativeFn),
    /// A JNI method: the app's own native code, which droid cannot load.
    Jni,
    Abstract,
}

pub struct Method {
    pub name: Rc<str>,
    pub desc: Rc<str>,
    pub class: Rc<str>,
    pub is_static: bool,
    pub body: Body,
}

pub struct Class {
    pub name: Rc<str>,
    pub super_: Option<Rc<Class>>,
    pub interfaces: Vec<Rc<Class>>,
    pub is_interface: bool,
    /// All instance field slots, inherited first: (declaring class, name, type).
    pub ifields: Vec<(Rc<str>, Rc<str>, Rc<str>)>,
    pub sfields: RefCell<HashMap<Rc<str>, V>>,
    pub methods: Vec<Rc<Method>>,
    pub init: Cell<u8>,
}

impl Class {
    pub fn java_name(&self) -> String {
        let n = &self.name;
        if let Some(inner) = n.strip_prefix('L').and_then(|s| s.strip_suffix(';')) {
            inner.replace('/', ".")
        } else {
            n.to_string()
        }
    }
    fn own_method(&self, name: &str, desc: &str) -> Option<Rc<Method>> {
        self.methods.iter().find(|m| &*m.name == name && &*m.desc == desc).cloned()
    }
}

/// Parameter slots of a descriptor: for each parameter, whether it is wide.
pub fn params(desc: &str) -> Vec<bool> {
    let inner = &desc[1..desc.find(')').unwrap_or(1)];
    let b = inner.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let wide = b[i] == b'J' || b[i] == b'D';
        while b[i] == b'[' {
            i += 1;
        }
        if b[i] == b'L' {
            while b[i] != b';' {
                i += 1;
            }
        }
        out.push(wide && b[i] != b';' && (b[i] == b'J' || b[i] == b'D') && !inner[..i].ends_with('['));
        i += 1;
    }
    out
}

pub struct Vm {
    classes: HashMap<Rc<str>, Rc<Class>>,
    dex_classes: HashMap<Rc<str>, (Rc<Dex>, usize)>,
    strings: HashMap<Rc<str>, Ref>,
    pub package: String,
    pub missing: Vec<String>,
    depth: usize,
}

impl Vm {
    pub fn new(dexes: Vec<Dex>, package: &str) -> Vm {
        let mut dex_classes = HashMap::new();
        for d in dexes {
            let d = Rc::new(d);
            for (i, def) in d.class_defs.iter().enumerate() {
                dex_classes.insert(d.type_name(def.class_idx), (d.clone(), i));
            }
        }
        Vm {
            classes: HashMap::new(),
            dex_classes,
            strings: HashMap::new(),
            package: package.to_string(),
            missing: Vec::new(),
            depth: 0,
        }
    }

    // ---------------------------------------------------------------
    // Classes
    // ---------------------------------------------------------------

    pub fn class(&mut self, name: &str) -> R<Rc<Class>> {
        if let Some(c) = self.classes.get(name) {
            return Ok(c.clone());
        }
        let c = if let Some((d, i)) = self.dex_classes.get(name).cloned() {
            self.define_dex(&d, i)?
        } else if name.starts_with('[') || name.len() == 1 {
            let object = self.class("Ljava/lang/Object;")?;
            Rc::new(Class {
                name: name.into(),
                super_: Some(object),
                interfaces: Vec::new(),
                is_interface: false,
                ifields: Vec::new(),
                sfields: RefCell::new(HashMap::new()),
                methods: Vec::new(),
                init: Cell::new(2),
            })
        } else if let Some(def) = natives::lookup(name) {
            let super_ = match def.super_ {
                Some(s) => Some(self.class(s)?),
                None => None,
            };
            let mut interfaces = Vec::new();
            for i in def.interfaces {
                interfaces.push(self.class(i)?);
            }
            let mut ifields = super_.as_ref().map(|s| s.ifields.clone()).unwrap_or_default();
            let cname: Rc<str> = name.into();
            for (n, t) in def.fields {
                ifields.push((cname.clone(), (*n).into(), (*t).into()));
            }
            let methods = def
                .methods
                .iter()
                .map(|(n, d, f, st)| {
                    Rc::new(Method {
                        name: (*n).into(),
                        desc: (*d).into(),
                        class: cname.clone(),
                        is_static: *st,
                        body: Body::Native(*f),
                    })
                })
                .collect();
            let c = Rc::new(Class {
                name: cname,
                super_,
                interfaces,
                is_interface: def.interface,
                ifields,
                sfields: RefCell::new(HashMap::new()),
                methods,
                init: Cell::new(0),
            });
            self.classes.insert(c.name.clone(), c.clone());
            for (n, v) in (def.statics)(self) {
                c.sfields.borrow_mut().insert(n.into(), v);
            }
            c.init.set(2);
            return Ok(c);
        } else {
            let msg = format!("{} (not in the app and not provided by droid)", name);
            self.note_missing(&msg);
            return Err(self.exception("Ljava/lang/NoClassDefFoundError;", &msg));
        };
        self.classes.insert(c.name.clone(), c.clone());
        Ok(c)
    }

    fn define_dex(&mut self, d: &Rc<Dex>, i: usize) -> R<Rc<Class>> {
        let def = &d.class_defs[i];
        let name = d.type_name(def.class_idx);
        let super_ = if def.super_idx == dex::NO_INDEX { None } else { Some(self.class(&d.type_name(def.super_idx))?) };
        let mut interfaces = Vec::new();
        for i in d.interfaces(def) {
            interfaces.push(self.class(&i)?);
        }
        let cd = d.class_data(def);
        let mut ifields = super_.as_ref().map(|s| s.ifields.clone()).unwrap_or_default();
        for &(f, _) in &cd.instance_fields {
            let (_, n, t) = d.field_ref(f);
            ifields.push((name.clone(), n, t));
        }
        let mut sfields = HashMap::new();
        let statics = d.static_values(def);
        for (k, &(f, _)) in cd.static_fields.iter().enumerate() {
            let (_, n, t) = d.field_ref(f);
            let v = match statics.get(k) {
                Some(Encoded::Int(x)) | Some(Encoded::Long(x)) => {
                    V::P(*x as u64 & if t.starts_with('J') { !0 } else { 0xffff_ffff })
                }
                Some(Encoded::Float(x)) => V::float(*x),
                Some(Encoded::Double(x)) => V::double(*x),
                Some(Encoded::Bool(b)) => V::bool(*b),
                Some(Encoded::Str(s)) => V::obj(self.intern(&d.string(*s))),
                Some(Encoded::Type(ty)) => natives::class_value(self, &d.type_name(*ty)),
                _ if t.starts_with('L') || t.starts_with('[') => V::NULL,
                _ => V::P(0),
            };
            sfields.insert(n, v);
        }
        let mut methods = Vec::new();
        for &(m, acc, code) in cd.direct_methods.iter().chain(cd.virtual_methods.iter()) {
            let (_, n, desc) = d.method_ref(m);
            let body = if code != 0 {
                Body::Code(Rc::new(d.code(code)), d.clone())
            } else if acc & dex::ACC_NATIVE != 0 {
                Body::Jni
            } else {
                Body::Abstract
            };
            methods.push(Rc::new(Method {
                name: n,
                desc,
                class: name.clone(),
                is_static: acc & dex::ACC_STATIC != 0,
                body,
            }));
        }
        Ok(Rc::new(Class {
            name,
            super_,
            interfaces,
            is_interface: def.access & dex::ACC_INTERFACE != 0,
            ifields,
            sfields: RefCell::new(sfields),
            methods,
            init: Cell::new(0),
        }))
    }

    /// Run static initialisers (superclass first).
    pub fn init_class(&mut self, c: &Rc<Class>) -> R<()> {
        if c.init.get() != 0 {
            return Ok(());
        }
        c.init.set(1);
        if let Some(s) = &c.super_ {
            self.init_class(&s.clone())?;
        }
        if let Some(m) = c.own_method("<clinit>", "()V") {
            self.invoke(&m, Vec::new())?;
        }
        c.init.set(2);
        Ok(())
    }

    pub fn is_subclass(&self, c: &Class, target: &str) -> bool {
        if &*c.name == target || target == "Ljava/lang/Object;" {
            return true;
        }
        if c.interfaces.iter().any(|i| self.is_subclass(i, target)) {
            return true;
        }
        if let Some(s) = &c.super_ {
            return self.is_subclass(s, target);
        }
        // Arrays: covariance for reference arrays.
        if let (Some(a), Some(b)) = (c.name.strip_prefix('['), target.strip_prefix('[')) {
            if a == b {
                return true;
            }
        }
        false
    }

    /// Find a method by name and descriptor: the class, its superclasses,
    /// then (default methods) its interfaces.
    pub fn find_method(&self, c: &Class, name: &str, desc: &str) -> Option<Rc<Method>> {
        let mut cur = Some(c);
        while let Some(k) = cur {
            if let Some(m) = k.own_method(name, desc) {
                if !matches!(m.body, Body::Abstract) || k.is_interface {
                    return Some(m);
                }
            }
            cur = k.super_.as_deref();
        }
        let mut cur = Some(c);
        while let Some(k) = cur {
            for i in &k.interfaces {
                if let Some(m) = self.find_method(i, name, desc) {
                    if !matches!(m.body, Body::Abstract) {
                        return Some(m);
                    }
                }
            }
            cur = k.super_.as_deref();
        }
        None
    }

    pub fn note_missing(&mut self, what: &str) {
        if !self.missing.iter().any(|m| m == what) {
            eprintln!("droid: missing {}", what);
            self.missing.push(what.to_string());
        }
    }

    // ---------------------------------------------------------------
    // Objects
    // ---------------------------------------------------------------

    pub fn new_object(&mut self, c: &Rc<Class>) -> Ref {
        let fields = c
            .ifields
            .iter()
            .map(|(_, _, t)| if t.starts_with('L') || t.starts_with('[') { V::NULL } else { V::P(0) })
            .collect();
        Rc::new(Obj { class: c.clone(), fields: RefCell::new(fields), ext: RefCell::new(Ext::None) })
    }

    pub fn new_with(&mut self, class: &str, ext: Ext) -> R<Ref> {
        let c = self.class(class)?;
        let o = self.new_object(&c);
        *o.ext.borrow_mut() = ext;
        Ok(o)
    }

    pub fn intern(&mut self, s: &str) -> Ref {
        if let Some(r) = self.strings.get(s) {
            return r.clone();
        }
        let r = self.new_string(s);
        self.strings.insert(s.into(), r.clone());
        r
    }

    pub fn new_string(&mut self, s: &str) -> Ref {
        let c = self.class("Ljava/lang/String;").expect("java.lang.String");
        Rc::new(Obj { class: c, fields: RefCell::new(Vec::new()), ext: RefCell::new(Ext::Str(s.into())) })
    }

    pub fn str_value(&mut self, s: &str) -> V {
        V::obj(self.new_string(s))
    }

    pub fn new_array(&mut self, desc: &str, len: usize) -> R<Ref> {
        let elem = desc[1..].chars().next().unwrap_or('I');
        let arr = if elem == 'L' || elem == '[' {
            Arr { elem, prim: Vec::new(), refs: vec![None; len] }
        } else {
            Arr { elem, prim: vec![0; len], refs: Vec::new() }
        };
        self.new_with(desc, Ext::Arr(arr))
    }

    /// Create and initialise an exception with a message.
    pub fn exception(&mut self, class: &str, msg: &str) -> Ref {
        let c = match self.class(class) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let o = self.new_object(&c);
        let m = self.str_value(msg);
        natives::set_field(&o, "detailMessage", m);
        o
    }

    pub fn field_slot(&self, o: &Obj, decl: &str, name: &str) -> Option<usize> {
        let f = &o.class.ifields;
        (0..f.len()).rev().find(|&i| &*f[i].1 == name && (&*f[i].0 == decl || self.decl_matches(&f[i].0, decl)))
    }

    fn decl_matches(&self, field_class: &str, ref_class: &str) -> bool {
        // A field referenced through a subclass is declared in an ancestor.
        match self.classes.get(ref_class) {
            Some(c) => self.is_subclass(c, field_class),
            None => false,
        }
    }

    // ---------------------------------------------------------------
    // Calls
    // ---------------------------------------------------------------

    /// Call a virtual method from native code.
    pub fn call(&mut self, obj: &Ref, name: &str, desc: &str, args: &[V]) -> R<V> {
        let m = match self.find_method(&obj.class.clone(), name, desc) {
            Some(m) => m,
            None => {
                let what = format!("{}.{}{}", obj.class.java_name(), name, desc);
                self.note_missing(&what);
                return Err(self.exception("Ljava/lang/NoSuchMethodError;", &what));
            }
        };
        let mut a = vec![V::obj(obj.clone())];
        a.extend_from_slice(args);
        self.invoke(&m, a)
    }

    pub fn call_static(&mut self, class: &str, name: &str, desc: &str, args: Vec<V>) -> R<V> {
        let c = self.class(class)?;
        self.init_class(&c)?;
        match self.find_method(&c, name, desc) {
            Some(m) => self.invoke(&m, args),
            None => Err(self.exception("Ljava/lang/NoSuchMethodError;", &format!("{}.{}{}", class, name, desc))),
        }
    }

    /// Invoke with one value per parameter (wide values as one value).
    pub fn invoke(&mut self, m: &Rc<Method>, args: Vec<V>) -> R<V> {
        match &m.body {
            Body::Native(f) => f(self, &args),
            Body::Jni => {
                let what = format!("JNI method {}.{}{} (app native code)", m.class, m.name, m.desc);
                self.note_missing(&what);
                Err(self.exception("Ljava/lang/UnsatisfiedLinkError;", &what))
            }
            Body::Abstract => {
                Err(self.exception("Ljava/lang/AbstractMethodError;", &format!("{}.{}{}", m.class, m.name, m.desc)))
            }
            Body::Code(code, dex) => {
                let (code, dex) = (code.clone(), dex.clone());
                // Lay the arguments out in the last `ins` registers.
                let mut regs = vec![V::default(); code.registers];
                let mut r = code.registers - code.ins;
                let mut it = args.into_iter();
                if !m.is_static {
                    regs[r] = it.next().unwrap_or(V::NULL);
                    r += 1;
                }
                for wide in params(&m.desc) {
                    let v = it.next().unwrap_or_default();
                    regs[r] = v;
                    r += if wide { 2 } else { 1 };
                }
                self.depth += 1;
                if self.depth > 2000 {
                    self.depth -= 1;
                    return Err(self.exception("Ljava/lang/StackOverflowError;", "stack depth"));
                }
                let r = self.run(m, &code, &dex, regs);
                self.depth -= 1;
                r
            }
        }
    }

    fn resolve_static(&mut self, class: &str, name: &str, desc: &str) -> R<Rc<Method>> {
        let c = self.class(class)?;
        self.init_class(&c)?;
        match self.find_method(&c, name, desc) {
            Some(m) => Ok(m),
            None => {
                let what = format!("{}.{}{}", c.java_name(), name, desc);
                self.note_missing(&what);
                Err(self.exception("Ljava/lang/NoSuchMethodError;", &what))
            }
        }
    }

    fn static_owner(&mut self, class: &str, name: &str) -> R<Rc<Class>> {
        let mut c = Some(self.class(class)?);
        while let Some(k) = c {
            self.init_class(&k)?;
            if k.sfields.borrow().contains_key(name) {
                return Ok(k);
            }
            // Interfaces can hold constants too.
            for i in &k.interfaces {
                if i.sfields.borrow().contains_key(name) {
                    return Ok(i.clone());
                }
            }
            c = k.super_.clone();
        }
        let what = format!("static field {}.{}", class, name);
        self.note_missing(&what);
        Err(self.exception("Ljava/lang/NoSuchFieldError;", &what))
    }

    fn npe(&mut self, what: &str) -> Ref {
        self.exception("Ljava/lang/NullPointerException;", what)
    }

    // ---------------------------------------------------------------
    // The interpreter
    // ---------------------------------------------------------------

    fn run(&mut self, m: &Rc<Method>, code: &Code, dex: &Rc<Dex>, mut regs: Vec<V>) -> R<V> {
        let ins = &code.insns;
        let mut pc: usize = 0;
        let mut result = V::default();
        let mut exception: Option<Ref> = None;
        loop {
            let step = self.step(m, code, dex, &mut regs, &mut pc, &mut result, &mut exception);
            match step {
                Ok(Some(ret)) => return Ok(ret),
                Ok(None) => {}
                Err(exc) => {
                    // Find a handler for this pc.
                    let mut target = None;
                    'outer: for t in &code.tries {
                        if (pc as u32) >= t.start && (pc as u32) < t.end {
                            for (ty, addr) in &t.handler.catches {
                                let ok = match ty {
                                    None => true,
                                    Some(ty) => match self.class(ty) {
                                        Ok(_) => self.is_subclass(&exc.class, ty),
                                        Err(_) => false,
                                    },
                                };
                                if ok {
                                    target = Some(*addr as usize);
                                    break 'outer;
                                }
                            }
                        }
                    }
                    match target {
                        Some(t) => {
                            exception = Some(exc);
                            pc = t;
                        }
                        None => return Err(exc),
                    }
                }
            }
            if pc >= ins.len() {
                return Ok(V::default());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        m: &Rc<Method>,
        code: &Code,
        dex: &Rc<Dex>,
        regs: &mut [V],
        pc: &mut usize,
        result: &mut V,
        exception: &mut Option<Ref>,
    ) -> R<Option<V>> {
        let ins = &code.insns;
        let w = ins[*pc];
        let op = (w & 0xff) as u8;
        let a8 = (w >> 8) as usize; // vAA
        let a4 = ((w >> 8) & 0xf) as usize; // vA
        let b4 = (w >> 12) as usize; // vB
        let u1 = || ins[*pc + 1] as usize;
        let u2 = || ins[*pc + 2] as usize;
        let s32 = |o: usize| (ins[*pc + o] as u32 | (ins[*pc + o + 1] as u32) << 16) as i32;
        macro_rules! next {
            ($n:expr) => {{
                *pc += $n;
                return Ok(None);
            }};
        }
        macro_rules! jump {
            ($off:expr) => {{
                *pc = (*pc as i64 + $off as i64) as usize;
                return Ok(None);
            }};
        }
        match op {
            0x00 => next!(1),
            0x01 | 0x04 | 0x07 => {
                regs[a4] = regs[b4].clone();
                next!(1)
            }
            0x02 | 0x05 | 0x08 => {
                regs[a8] = regs[u1()].clone();
                next!(2)
            }
            0x03 | 0x06 | 0x09 => {
                regs[u1()] = regs[u2()].clone();
                next!(3)
            }
            0x0a..=0x0c => {
                regs[a8] = core::mem::take(result);
                next!(1)
            }
            0x0d => {
                regs[a8] = V::R(exception.take());
                next!(1)
            }
            0x0e => Ok(Some(V::default())),
            0x0f..=0x11 => Ok(Some(regs[a8].clone())),
            0x12 => {
                regs[a4] = V::int(((w as i16) >> 12) as i32);
                next!(1)
            }
            0x13 => {
                regs[a8] = V::int(ins[*pc + 1] as i16 as i32);
                next!(2)
            }
            0x14 => {
                regs[a8] = V::int(s32(1));
                next!(3)
            }
            0x15 => {
                regs[a8] = V::int((u1() as i32) << 16);
                next!(2)
            }
            0x16 => {
                regs[a8] = V::long(ins[*pc + 1] as i16 as i64);
                next!(2)
            }
            0x17 => {
                regs[a8] = V::long(s32(1) as i64);
                next!(3)
            }
            0x18 => {
                let v = (0..4).fold(0u64, |acc, k| acc | (ins[*pc + 1 + k] as u64) << (16 * k));
                regs[a8] = V::P(v);
                next!(5)
            }
            0x19 => {
                regs[a8] = V::P((u1() as u64) << 48);
                next!(2)
            }
            0x1a | 0x1b => {
                let idx = if op == 0x1a { u1() as u32 } else { s32(1) as u32 };
                let s = dex.string(idx);
                regs[a8] = V::obj(self.intern(&s));
                next!(if op == 0x1a { 2 } else { 3 })
            }
            0x1c => {
                let c = self.class(&dex.type_name(u1() as u32))?;
                regs[a8] = V::obj(self.new_with("Ljava/lang/Class;", Ext::Class(c))?);
                next!(2)
            }
            0x1d | 0x1e => next!(1),
            0x1f => {
                if let Some(o) = regs[a8].r() {
                    let ty = dex.type_name(u1() as u32);
                    self.class(&ty)?;
                    if !self.is_subclass(&o.class, &ty) {
                        let msg = format!("{} cannot be cast to {}", o.class.java_name(), ty);
                        return Err(self.exception("Ljava/lang/ClassCastException;", &msg));
                    }
                }
                next!(2)
            }
            0x20 => {
                let ty = dex.type_name(u1() as u32);
                let r = match regs[b4].r() {
                    Some(o) => self.class(&ty).is_ok() && self.is_subclass(&o.class, &ty),
                    None => false,
                };
                regs[a4] = V::bool(r);
                next!(2)
            }
            0x21 => {
                let o = regs[b4].r().ok_or_else(|| self.npe("array-length"))?;
                let n = match &*o.ext.borrow() {
                    Ext::Arr(a) => a.len(),
                    _ => 0,
                };
                regs[a4] = V::int(n as i32);
                next!(1)
            }
            0x22 => {
                let c = self.class(&dex.type_name(u1() as u32))?;
                self.init_class(&c)?;
                regs[a8] = V::obj(self.new_object(&c));
                next!(2)
            }
            0x23 => {
                let n = regs[b4].i();
                if n < 0 {
                    return Err(self.exception("Ljava/lang/NegativeArraySizeException;", &n.to_string()));
                }
                let ty = dex.type_name(u1() as u32);
                regs[a4] = V::obj(self.new_array(&ty, n as usize)?);
                next!(2)
            }
            0x24 | 0x25 => {
                let ty = dex.type_name(u1() as u32);
                let regs_list = arg_regs(ins, *pc, op == 0x25);
                let arr = self.new_array(&ty, regs_list.len())?;
                if let Ext::Arr(a) = &mut *arr.ext.borrow_mut() {
                    for (k, &r) in regs_list.iter().enumerate() {
                        if a.elem == 'L' || a.elem == '[' {
                            a.refs[k] = regs[r].r();
                        } else {
                            a.prim[k] = regs[r].i() as u32 as u64;
                        }
                    }
                }
                *result = V::obj(arr);
                next!(3)
            }
            0x26 => {
                let p = (*pc as i64 + s32(1) as i64) as usize;
                let width = ins[p + 1] as usize;
                let size = (ins[p + 2] as usize) | (ins[p + 3] as usize) << 16;
                let bytes: Vec<u8> = ins[p + 4..].iter().flat_map(|u| u.to_le_bytes()).take(width * size).collect();
                let o = regs[a8].r().ok_or_else(|| self.npe("fill-array-data"))?;
                if let Ext::Arr(a) = &mut *o.ext.borrow_mut() {
                    for k in 0..size.min(a.prim.len()) {
                        let mut v = 0u64;
                        for byte in 0..width {
                            v |= (bytes[k * width + byte] as u64) << (8 * byte);
                        }
                        if width < 8 && (v >> (8 * width - 1)) & 1 == 1 && a.elem != 'C' && a.elem != 'Z' {
                            v |= !0u64 << (8 * width);
                            if width < 8 && a.elem != 'J' && a.elem != 'D' {
                                v &= 0xffff_ffff;
                            }
                        }
                        a.prim[k] = v;
                    }
                }
                next!(3)
            }
            0x27 => {
                let e = regs[a8].r().ok_or_else(|| self.npe("throw null"))?;
                Err(e)
            }
            0x28 => jump!((w as i16) >> 8),
            0x29 => jump!(ins[*pc + 1] as i16),
            0x2a => jump!(s32(1)),
            0x2b | 0x2c => {
                let p = (*pc as i64 + s32(1) as i64) as usize;
                let key = regs[a8].i();
                let size = ins[p + 1] as usize;
                let rd = |o: usize| (ins[o] as u32 | (ins[o + 1] as u32) << 16) as i32;
                let target = if op == 0x2b {
                    let first = rd(p + 2);
                    let idx = key.wrapping_sub(first);
                    if idx >= 0 && (idx as usize) < size {
                        Some(rd(p + 4 + idx as usize * 2))
                    } else {
                        None
                    }
                } else {
                    (0..size).find(|&k| rd(p + 2 + k * 2) == key).map(|k| rd(p + 2 + size * 2 + k * 2))
                };
                match target {
                    Some(t) => jump!(t),
                    None => next!(3),
                }
            }
            0x2d..=0x31 => {
                let (b, c) = (regs[u1() & 0xff].clone(), regs[u1() >> 8].clone());
                let r = match op {
                    0x2d | 0x2e => cmp_float(b.f() as f64, c.f() as f64, op == 0x2e),
                    0x2f | 0x30 => cmp_float(b.d(), c.d(), op == 0x30),
                    _ => (b.j().cmp(&c.j())) as i32,
                };
                regs[a8] = V::int(r);
                next!(2)
            }
            0x32..=0x37 => {
                let (x, y) = (&regs[a4], &regs[b4]);
                let take = match (x, y) {
                    (V::R(p), V::R(q)) => {
                        let same = match (p, q) {
                            (Some(p), Some(q)) => Rc::ptr_eq(p, q),
                            (None, None) => true,
                            _ => false,
                        };
                        match op {
                            0x32 => same,
                            0x33 => !same,
                            _ => false,
                        }
                    }
                    _ => {
                        let (p, q) = (x.i(), y.i());
                        match op {
                            0x32 => p == q,
                            0x33 => p != q,
                            0x34 => p < q,
                            0x35 => p >= q,
                            0x36 => p > q,
                            _ => p <= q,
                        }
                    }
                };
                if take {
                    jump!(ins[*pc + 1] as i16)
                }
                next!(2)
            }
            0x38..=0x3d => {
                let x = &regs[a8];
                let take = match op {
                    0x38 => x.is_zero(),
                    0x39 => !x.is_zero(),
                    0x3a => x.i() < 0,
                    0x3b => x.i() >= 0,
                    0x3c => x.i() > 0,
                    _ => x.i() <= 0,
                };
                if take {
                    jump!(ins[*pc + 1] as i16)
                }
                next!(2)
            }
            0x44..=0x51 => {
                let (ar, ix) = (u1() & 0xff, u1() >> 8);
                let o = regs[ar].r().ok_or_else(|| self.npe("array access"))?;
                let i = regs[ix].i();
                let len = match &*o.ext.borrow() {
                    Ext::Arr(a) => a.len(),
                    _ => 0,
                };
                if i < 0 || i as usize >= len {
                    let msg = format!("length={}; index={}", len, i);
                    return Err(self.exception("Ljava/lang/ArrayIndexOutOfBoundsException;", &msg));
                }
                let i = i as usize;
                if op <= 0x4a {
                    let v = match &*o.ext.borrow() {
                        Ext::Arr(a) if op == 0x46 => V::R(a.refs[i].clone()),
                        Ext::Arr(a) => V::P(a.prim[i]),
                        _ => V::default(),
                    };
                    regs[a8] = v;
                } else {
                    if op == 0x4d {
                        let v = regs[a8].r();
                        if let (Some(v), Some(elem)) = (&v, o.class.name.strip_prefix('[')) {
                            if self.class(elem).is_ok() && !self.is_subclass(&v.class, elem) {
                                return Err(self.exception("Ljava/lang/ArrayStoreException;", &v.class.java_name()));
                            }
                        }
                    }
                    let v = regs[a8].clone();
                    if let Ext::Arr(a) = &mut *o.ext.borrow_mut() {
                        match op {
                            0x4d => a.refs[i] = v.r(),
                            0x4c => a.prim[i] = v.j() as u64,
                            0x4e => a.prim[i] = (v.i() & 1) as u64,
                            0x4f => a.prim[i] = v.i() as i8 as i32 as u32 as u64,
                            0x50 => a.prim[i] = v.i() as u16 as u64,
                            0x51 => a.prim[i] = v.i() as i16 as i32 as u32 as u64,
                            _ => a.prim[i] = v.i() as u32 as u64,
                        }
                    }
                }
                next!(2)
            }
            0x52..=0x5f => {
                let (decl, name, _) = dex.field_ref(u1() as u32);
                let o = regs[b4].r().ok_or_else(|| self.npe(&format!("field {}", name)))?;
                let slot = match self.field_slot(&o, &decl, &name) {
                    Some(s) => s,
                    None => {
                        let what = format!("field {}.{}", decl, name);
                        self.note_missing(&what);
                        return Err(self.exception("Ljava/lang/NoSuchFieldError;", &what));
                    }
                };
                if op <= 0x58 {
                    regs[a4] = o.fields.borrow()[slot].clone();
                } else {
                    let v = match op {
                        0x5c => V::int(regs[a4].i() & 1),
                        0x5d => V::int(regs[a4].i() as i8 as i32),
                        0x5e => V::int(regs[a4].i() as u16 as i32),
                        0x5f => V::int(regs[a4].i() as i16 as i32),
                        _ => regs[a4].clone(),
                    };
                    o.fields.borrow_mut()[slot] = v;
                }
                next!(2)
            }
            0x60..=0x6d => {
                let (decl, name, _) = dex.field_ref(u1() as u32);
                let owner = self.static_owner(&decl, &name)?;
                if op <= 0x66 {
                    regs[a8] = owner.sfields.borrow().get(&*name).cloned().unwrap_or_default();
                } else {
                    owner.sfields.borrow_mut().insert(name, regs[a8].clone());
                }
                next!(2)
            }
            0x6e..=0x72 | 0x74..=0x78 => {
                let range = op >= 0x74;
                let kind = if range { op - 0x74 } else { op - 0x6e };
                let (class, name, desc) = dex.method_ref(u1() as u32);
                let rlist = arg_regs(ins, *pc, range);
                let is_static = kind == 3;
                // Marshal arguments: one value per parameter.
                let mut args = Vec::with_capacity(rlist.len());
                let mut k = 0;
                if !is_static {
                    args.push(regs[rlist[0]].clone());
                    k = 1;
                }
                for wide in params(&desc) {
                    args.push(regs.get(*rlist.get(k).unwrap_or(&0)).cloned().unwrap_or_default());
                    k += if wide { 2 } else { 1 };
                }
                let target = match kind {
                    3 => self.resolve_static(&class, &name, &desc)?,
                    _ => {
                        let this =
                            args[0].r().ok_or_else(|| self.npe(&format!("{}.{}", class.trim_matches(';'), name)))?;
                        let found = match kind {
                            1 => {
                                // invoke-super: from the caller's superclass.
                                let caller = self.class(&m.class)?;
                                let start = caller.super_.clone().unwrap_or(caller);
                                self.find_method(&start, &name, &desc)
                            }
                            2 => {
                                let c = self.class(&class)?;
                                self.find_method(&c, &name, &desc)
                            }
                            _ => self.find_method(&this.class.clone(), &name, &desc),
                        };
                        match found {
                            Some(t) => t,
                            None if &*name == "<init>" => {
                                // A framework class without an explicit constructor.
                                next!(3)
                            }
                            None => {
                                let what = format!("{}.{}{}", this.class.java_name(), name, desc);
                                self.note_missing(&what);
                                return Err(self.exception("Ljava/lang/NoSuchMethodError;", &what));
                            }
                        }
                    }
                };
                *result = self.invoke(&target, args)?;
                next!(3)
            }
            0x7b..=0x8f => {
                let s = regs[b4].clone();
                regs[a4] = match op {
                    0x7b => V::int(s.i().wrapping_neg()),
                    0x7c => V::int(!s.i()),
                    0x7d => V::long(s.j().wrapping_neg()),
                    0x7e => V::long(!s.j()),
                    0x7f => V::float(-s.f()),
                    0x80 => V::double(-s.d()),
                    0x81 => V::long(s.i() as i64),
                    0x82 => V::float(s.i() as f32),
                    0x83 => V::double(s.i() as f64),
                    0x84 => V::int(s.j() as i32),
                    0x85 => V::float(s.j() as f32),
                    0x86 => V::double(s.j() as f64),
                    0x87 => V::int(s.f() as i32),
                    0x88 => V::long(s.f() as i64),
                    0x89 => V::double(s.f() as f64),
                    0x8a => V::int(s.d() as i32),
                    0x8b => V::long(s.d() as i64),
                    0x8c => V::float(s.d() as f32),
                    0x8d => V::int(s.i() as i8 as i32),
                    0x8e => V::int(s.i() as u16 as i32),
                    _ => V::int(s.i() as i16 as i32),
                };
                next!(1)
            }
            0x90..=0xcf => {
                let (dst, x, y, len, base) = if op < 0xb0 {
                    (a8, regs[u1() & 0xff].clone(), regs[u1() >> 8].clone(), 2, op - 0x90)
                } else {
                    (a4, regs[a4].clone(), regs[b4].clone(), 1, op - 0xb0)
                };
                regs[dst] = self.binop(base, &x, &y)?;
                next!(len)
            }
            0xd0..=0xe2 => {
                let (dst, x, lit, len, base) = if op < 0xd8 {
                    (a4, regs[b4].clone(), ins[*pc + 1] as i16 as i32, 2, op - 0xd0)
                } else {
                    (a8, regs[u1() & 0xff].clone(), (u1() >> 8) as u8 as i8 as i32, 2, op - 0xd8)
                };
                let (p, q) = (x.i(), lit);
                let v = match base {
                    0 => p.wrapping_add(q),
                    1 => q.wrapping_sub(p),
                    2 => p.wrapping_mul(q),
                    3 | 4 => {
                        if q == 0 {
                            return Err(self.exception("Ljava/lang/ArithmeticException;", "divide by zero"));
                        }
                        if base == 3 {
                            p.wrapping_div(q)
                        } else {
                            p.wrapping_rem(q)
                        }
                    }
                    5 => p & q,
                    6 => p | q,
                    7 => p ^ q,
                    8 => p.wrapping_shl(q as u32 & 31),
                    9 => p.wrapping_shr(q as u32 & 31),
                    _ => ((p as u32) >> (q as u32 & 31)) as i32,
                };
                regs[dst] = V::int(v);
                next!(len)
            }
            _ => {
                let what = format!("opcode {:#04x} in {}.{}", op, m.class, m.name);
                self.note_missing(&what);
                Err(self.exception("Ljava/lang/VerifyError;", &what))
            }
        }
    }

    /// Binary operations 0x90..0xaf (and the /2addr forms) by index.
    fn binop(&mut self, k: u8, x: &V, y: &V) -> R<V> {
        let div0 = |vm: &mut Vm| vm.exception("Ljava/lang/ArithmeticException;", "divide by zero");
        Ok(match k {
            0..=10 => {
                let (p, q) = (x.i(), y.i());
                V::int(match k {
                    0 => p.wrapping_add(q),
                    1 => p.wrapping_sub(q),
                    2 => p.wrapping_mul(q),
                    3 => {
                        if q == 0 {
                            return Err(div0(self));
                        }
                        p.wrapping_div(q)
                    }
                    4 => {
                        if q == 0 {
                            return Err(div0(self));
                        }
                        p.wrapping_rem(q)
                    }
                    5 => p & q,
                    6 => p | q,
                    7 => p ^ q,
                    8 => p.wrapping_shl(q as u32 & 31),
                    9 => p.wrapping_shr(q as u32 & 31),
                    _ => ((p as u32) >> (q as u32 & 31)) as i32,
                })
            }
            11..=21 => {
                let (p, q) = (x.j(), y.j());
                let sh = y.i() as u32 & 63;
                V::long(match k - 11 {
                    0 => p.wrapping_add(q),
                    1 => p.wrapping_sub(q),
                    2 => p.wrapping_mul(q),
                    3 => {
                        if q == 0 {
                            return Err(div0(self));
                        }
                        p.wrapping_div(q)
                    }
                    4 => {
                        if q == 0 {
                            return Err(div0(self));
                        }
                        p.wrapping_rem(q)
                    }
                    5 => p & q,
                    6 => p | q,
                    7 => p ^ q,
                    8 => p.wrapping_shl(sh),
                    9 => p.wrapping_shr(sh),
                    _ => ((p as u64) >> sh) as i64,
                })
            }
            22..=26 => {
                let (p, q) = (x.f(), y.f());
                V::float(match k - 22 {
                    0 => p + q,
                    1 => p - q,
                    2 => p * q,
                    3 => p / q,
                    _ => p % q,
                })
            }
            _ => {
                let (p, q) = (x.d(), y.d());
                V::double(match k - 27 {
                    0 => p + q,
                    1 => p - q,
                    2 => p * q,
                    3 => p / q,
                    _ => p % q,
                })
            }
        })
    }
}

fn cmp_float(a: f64, b: f64, g_bias: bool) -> i32 {
    if a.is_nan() || b.is_nan() {
        return if g_bias { 1 } else { -1 };
    }
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}

/// Registers named by a 35c or 3rc instruction.
fn arg_regs(ins: &[u16], pc: usize, range: bool) -> Vec<usize> {
    let w = ins[pc];
    if range {
        let n = (w >> 8) as usize;
        let first = ins[pc + 2] as usize;
        (first..first + n).collect()
    } else {
        let n = (w >> 12) as usize;
        let g = ((w >> 8) & 0xf) as usize;
        let cdef = ins[pc + 2] as usize;
        let all = [cdef & 0xf, (cdef >> 4) & 0xf, (cdef >> 8) & 0xf, (cdef >> 12) & 0xf, g];
        all[..n].to_vec()
    }
}
