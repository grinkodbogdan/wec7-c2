//! Native classes: the parts of the Java core library and the Android API
//! that apps call, implemented directly in Rust instead of running
//! Android's framework. Each class is a table of (name, descriptor, fn).
//!
//! Only what apps need goes here; anything else an app touches is reported
//! as missing (NoClassDefFoundError / NoSuchMethodError) so it is obvious
//! what to add next.

#![allow(unused_variables)]

use crate::vm::{Ext, NativeFn, Ref, Vm, R, V};
use std::rc::Rc;

pub type Meth = (&'static str, &'static str, NativeFn, bool);

#[derive(Clone, Copy)]
pub struct NClass {
    pub super_: Option<&'static str>,
    pub interfaces: &'static [&'static str],
    pub interface: bool,
    pub fields: &'static [(&'static str, &'static str)],
    pub methods: &'static [Meth],
    pub statics: fn(&mut Vm) -> Vec<(&'static str, V)>,
}

fn no_statics(_: &mut Vm) -> Vec<(&'static str, V)> {
    Vec::new()
}

const OBJ: &str = "Ljava/lang/Object;";
const BASE: NClass =
    NClass { super_: Some(OBJ), interfaces: &[], interface: false, fields: &[], methods: &[], statics: no_statics };
const IFACE: NClass = NClass { interface: true, ..BASE };

/// An instance method.
macro_rules! m {
    ($n:expr, $d:expr, |$vm:ident, $a:ident| $body:expr) => {{
        fn f($vm: &mut Vm, $a: &[V]) -> R<V> {
            $body
        }
        ($n, $d, f as NativeFn, false)
    }};
}
/// A static method.
macro_rules! s {
    ($n:expr, $d:expr, |$vm:ident, $a:ident| $body:expr) => {{
        fn f($vm: &mut Vm, $a: &[V]) -> R<V> {
            $body
        }
        ($n, $d, f as NativeFn, true)
    }};
}

const NPE: &str = "Ljava/lang/NullPointerException;";

// -------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------

pub fn this(a: &[V]) -> Ref {
    a[0].r().expect("native method called without this")
}

fn str_of(o: &Ref) -> Option<Rc<str>> {
    match &*o.ext.borrow() {
        Ext::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// The receiver's string value.
fn me(a: &[V]) -> Rc<str> {
    str_of(&this(a)).unwrap_or_else(|| "".into())
}

/// A string argument; null is a NullPointerException.
fn sarg(vm: &mut Vm, v: &V) -> R<Rc<str>> {
    match v.r() {
        Some(o) => match str_of(&o) {
            Some(s) => Ok(s),
            None => Ok(jstr(vm, v)?.into()),
        },
        None => Err(vm.exception(NPE, "null string")),
    }
}

pub fn set_field(o: &Ref, name: &str, v: V) {
    let f = &o.class.ifields;
    if let Some(i) = (0..f.len()).rev().find(|&i| &*f[i].1 == name) {
        o.fields.borrow_mut()[i] = v;
    }
}

pub fn get_field(o: &Ref, name: &str) -> V {
    let f = &o.class.ifields;
    match (0..f.len()).rev().find(|&i| &*f[i].1 == name) {
        Some(i) => o.fields.borrow()[i].clone(),
        None => V::default(),
    }
}

pub fn jdouble(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let a = d.abs();
    if (1e-3..1e7).contains(&a) {
        let s = format!("{}", d);
        if s.contains('.') {
            s
        } else {
            s + ".0"
        }
    } else {
        let s = format!("{:e}", d);
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = if m.contains('.') { m.to_string() } else { format!("{}.0", m) };
        format!("{}E{}", m, e)
    }
}

pub fn jfloat(f: f32) -> String {
    if f.is_finite() && f != 0.0 && (1e-3..1e7).contains(&f.abs()) {
        let s = format!("{}", f);
        return if s.contains('.') { s } else { s + ".0" };
    }
    jdouble(f as f64)
}

fn jchar(c: i32) -> String {
    char::from_u32(c as u32 & 0xffff).map(|c| c.to_string()).unwrap_or_else(|| "\u{fffd}".into())
}

fn boxed_str(class: &str, x: &V) -> String {
    match class {
        "Ljava/lang/Long;" => x.j().to_string(),
        "Ljava/lang/Double;" => jdouble(x.d()),
        "Ljava/lang/Float;" => jfloat(x.f()),
        "Ljava/lang/Boolean;" => (x.i() != 0).to_string(),
        "Ljava/lang/Character;" => jchar(x.i()),
        _ => x.i().to_string(),
    }
}

/// `String.valueOf(Object)`: the object's `toString()`, or "null".
pub fn jstr(vm: &mut Vm, v: &V) -> R<String> {
    let o = match v.r() {
        Some(o) => o,
        None => return Ok("null".into()),
    };
    {
        let e = o.ext.borrow();
        match &*e {
            Ext::Str(s) => return Ok(s.to_string()),
            Ext::Builder(b) => return Ok(b.clone()),
            Ext::Boxed(x) => return Ok(boxed_str(&o.class.name, x)),
            Ext::Class(c) => {
                return Ok(format!("{} {}", if c.is_interface { "interface" } else { "class" }, c.java_name()))
            }
            _ => {}
        }
    }
    let r = vm.call(&o, "toString", "()Ljava/lang/String;", &[])?;
    Ok(match r.r() {
        Some(s) => str_of(&s).map(|x| x.to_string()).unwrap_or_default(),
        None => "null".into(),
    })
}

/// `a.equals(b)` with value semantics for strings and boxes.
pub fn jequals(vm: &mut Vm, a: &V, b: &V) -> R<bool> {
    let (a, b) = match (a.r(), b.r()) {
        (None, None) => return Ok(true),
        (Some(a), Some(b)) => (a, b),
        _ => return Ok(false),
    };
    if Rc::ptr_eq(&a, &b) {
        return Ok(true);
    }
    match (&*a.ext.borrow(), &*b.ext.borrow()) {
        (Ext::Str(x), Ext::Str(y)) => return Ok(x == y),
        (Ext::Boxed(x), Ext::Boxed(y)) => return Ok(a.class.name == b.class.name && x.j() == y.j()),
        (Ext::Str(_), _) | (Ext::Boxed(_), _) => return Ok(false),
        _ => {}
    }
    let r = vm.call(&a, "equals", "(Ljava/lang/Object;)Z", &[V::obj(b)])?;
    Ok(r.i() != 0)
}

fn identity_hash(o: &Ref) -> i32 {
    (Rc::as_ptr(o) as usize >> 4) as i32
}

fn string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

pub fn jhash(vm: &mut Vm, v: &V) -> R<i32> {
    let o = match v.r() {
        Some(o) => o,
        None => return Ok(0),
    };
    match &*o.ext.borrow() {
        Ext::Str(s) => return Ok(string_hash(s)),
        Ext::Boxed(x) => return Ok(x.i() ^ (x.j() >> 32) as i32),
        _ => {}
    }
    Ok(vm.call(&o, "hashCode", "()I", &[])?.i())
}

fn new_str(vm: &mut Vm, s: &str) -> R<V> {
    Ok(vm.str_value(s))
}

fn boxed(vm: &mut Vm, class: &str, v: V) -> R<V> {
    Ok(V::obj(vm.new_with(class, Ext::Boxed(v))?))
}

fn unbox(a: &V) -> V {
    match a.r() {
        Some(o) => match &*o.ext.borrow() {
            Ext::Boxed(x) => x.clone(),
            _ => V::default(),
        },
        None => V::default(),
    }
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn sioobe(vm: &mut Vm, i: i32, len: usize) -> Ref {
    vm.exception("Ljava/lang/StringIndexOutOfBoundsException;", &format!("length={}; index={}", len, i))
}

fn string_array(vm: &mut Vm, items: &[String]) -> R<V> {
    let arr = vm.new_array("[Ljava/lang/String;", items.len())?;
    let refs: Vec<_> = items.iter().map(|s| Some(vm.new_string(s))).collect();
    if let Ext::Arr(a) = &mut *arr.ext.borrow_mut() {
        a.refs = refs;
    }
    Ok(V::obj(arr))
}

/// `String.split` for the patterns apps actually use: literals, escaped
/// single characters, and whitespace classes.
fn split(vm: &mut Vm, s: &str, re: &str) -> Vec<String> {
    let mut parts: Vec<String> = match re {
        "\\s+" | " +" => {
            let mut v: Vec<String> = s.split_whitespace().map(String::from).collect();
            if s.starts_with(char::is_whitespace) {
                v.insert(0, String::new());
            }
            v
        }
        "\\s" => s.split(char::is_whitespace).map(String::from).collect(),
        _ => {
            let lit = if re.len() == 2 && re.starts_with('\\') {
                re[1..].to_string()
            } else {
                if re.chars().any(|c| "[]()*+?{}|^$.\\".contains(c)) {
                    vm.note_missing(&format!("regex '{}' in String.split (treated literally)", re));
                }
                re.to_string()
            };
            if lit.is_empty() {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.split(lit.as_str()).map(String::from).collect()
            }
        }
    };
    while parts.len() > 1 && parts.last().map(|p| p.is_empty()).unwrap_or(false) {
        parts.pop();
    }
    if s.is_empty() {
        return vec![String::new()];
    }
    parts
}

/// `String.format` / `printf`: %d %s %f %e %x %c %b %% %n with flags,
/// width and precision.
pub fn jformat(vm: &mut Vm, fmt: &str, args: &[V]) -> R<String> {
    let mut out = String::new();
    let b: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut next = 0;
    while i < b.len() {
        if b[i] != '%' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        let mut flags = String::new();
        while i < b.len() && "-0+ ,#".contains(b[i]) {
            flags.push(b[i]);
            i += 1;
        }
        let mut width = 0usize;
        while i < b.len() && b[i].is_ascii_digit() {
            width = width * 10 + b[i] as usize - '0' as usize;
            i += 1;
        }
        let mut prec: Option<usize> = None;
        if i < b.len() && b[i] == '.' {
            i += 1;
            let mut p = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                p = p * 10 + b[i] as usize - '0' as usize;
                i += 1;
            }
            prec = Some(p);
        }
        let conv = match b.get(i) {
            Some(c) => *c,
            None => break,
        };
        i += 1;
        if conv == '%' {
            out.push('%');
            continue;
        }
        if conv == 'n' {
            out.push('\n');
            continue;
        }
        let arg = args.get(next).cloned().unwrap_or(V::NULL);
        next += 1;
        let class = arg.r().map(|o| o.class.name.to_string()).unwrap_or_default();
        let x = unbox(&arg);
        let as_f64 = || match class.as_str() {
            "Ljava/lang/Double;" => x.d(),
            "Ljava/lang/Float;" => x.f() as f64,
            "Ljava/lang/Long;" => x.j() as f64,
            _ => x.i() as f64,
        };
        let as_i64 = || match class.as_str() {
            "Ljava/lang/Long;" => x.j(),
            "Ljava/lang/Double;" => x.d() as i64,
            _ => x.i() as i64,
        };
        let mut numeric = true;
        let mut s = match conv {
            'd' => {
                let n = as_i64();
                let mut s = n.abs().to_string();
                if flags.contains(',') {
                    let d: Vec<char> = s.chars().collect();
                    s = d
                        .iter()
                        .enumerate()
                        .flat_map(|(k, c)| {
                            let sep = k > 0 && (d.len() - k).is_multiple_of(3);
                            sep.then_some(',').into_iter().chain(Some(*c))
                        })
                        .collect();
                }
                if n < 0 {
                    format!("-{}", s)
                } else if flags.contains('+') {
                    format!("+{}", s)
                } else {
                    s
                }
            }
            'f' => {
                let v = format!("{:.*}", prec.unwrap_or(6), as_f64());
                if flags.contains('+') && !v.starts_with('-') {
                    format!("+{}", v)
                } else {
                    v
                }
            }
            'e' | 'E' => {
                let v = format!("{:.*e}", prec.unwrap_or(6), as_f64());
                let (m, e) = v.split_once('e').unwrap_or((&v, "0"));
                let e: i32 = e.parse().unwrap_or(0);
                let v = format!("{}e{}{:02}", m, if e < 0 { '-' } else { '+' }, e.abs());
                if conv == 'E' {
                    v.to_uppercase()
                } else {
                    v
                }
            }
            'x' | 'X' => {
                let v = if class == "Ljava/lang/Long;" { format!("{:x}", x.j()) } else { format!("{:x}", x.i()) };
                if conv == 'X' {
                    v.to_uppercase()
                } else {
                    v
                }
            }
            'c' => {
                numeric = false;
                jchar(x.i())
            }
            'b' | 'B' => {
                numeric = false;
                (arg.r().is_some() && (class != "Ljava/lang/Boolean;" || x.i() != 0)).to_string()
            }
            _ => {
                numeric = false;
                let mut v = jstr(vm, &arg)?;
                if let Some(p) = prec {
                    v = v.chars().take(p).collect();
                }
                if conv == 'S' {
                    v.to_uppercase()
                } else {
                    v
                }
            }
        };
        let len = s.chars().count();
        if len < width {
            let pad = width - len;
            if flags.contains('-') {
                s.push_str(&" ".repeat(pad));
            } else if flags.contains('0') && numeric {
                let sign = s.starts_with(['-', '+']);
                let at = sign as usize;
                s.insert_str(at, &"0".repeat(pad));
            } else {
                s.insert_str(0, &" ".repeat(pad));
            }
        }
        out.push_str(&s);
    }
    Ok(out)
}

fn varargs(a: &V) -> Vec<V> {
    match a.r() {
        Some(o) => match &*o.ext.borrow() {
            Ext::Arr(arr) => arr.refs.iter().map(|r| V::R(r.clone())).collect(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

// -------------------------------------------------------------------
// Collections
// -------------------------------------------------------------------

fn list(o: &Ref) -> Vec<V> {
    match &*o.ext.borrow() {
        Ext::List(v) => v.clone(),
        _ => Vec::new(),
    }
}

fn with_list<T>(o: &Ref, f: impl FnOnce(&mut Vec<V>) -> T) -> T {
    let mut e = o.ext.borrow_mut();
    if !matches!(&*e, Ext::List(_)) {
        *e = Ext::List(Vec::new());
    }
    match &mut *e {
        Ext::List(v) => f(v),
        _ => unreachable!(),
    }
}

fn index_of(vm: &mut Vm, items: &[V], x: &V) -> R<Option<usize>> {
    for (k, it) in items.iter().enumerate() {
        if jequals(vm, it, x)? {
            return Ok(Some(k));
        }
    }
    Ok(None)
}

fn ioobe(vm: &mut Vm, i: i32, len: usize) -> Ref {
    vm.exception("Ljava/lang/IndexOutOfBoundsException;", &format!("Index {} out of bounds for length {}", i, len))
}

fn check_index(vm: &mut Vm, o: &Ref, i: i32, extra: usize) -> R<usize> {
    let n = with_list(o, |v| v.len());
    if i < 0 || i as usize >= n + extra {
        return Err(ioobe(vm, i, n));
    }
    Ok(i as usize)
}

fn join(vm: &mut Vm, items: &[V], open: &str, close: &str) -> R<V> {
    let mut s = String::from(open);
    for (k, it) in items.iter().enumerate() {
        if k > 0 {
            s.push_str(", ");
        }
        s.push_str(&jstr(vm, it)?);
    }
    s.push_str(close);
    new_str(vm, &s)
}

fn iterator(vm: &mut Vm, o: &Ref) -> R<V> {
    Ok(V::obj(vm.new_with("Ljava/util/Iterator$Impl;", Ext::Iter(o.clone(), 0))?))
}

fn add_all(vm: &mut Vm, dst: &Ref, src: &V) -> R<()> {
    let src = src.r().ok_or_else(|| vm.exception(NPE, "addAll(null)"))?;
    let items = elements(vm, &src)?;
    for it in items {
        vm.call(dst, "add", "(Ljava/lang/Object;)Z", &[it])?;
    }
    Ok(())
}

/// The elements of any collection: native ones directly, app ones through
/// their iterator.
fn elements(vm: &mut Vm, c: &Ref) -> R<Vec<V>> {
    if let Ext::List(v) = &*c.ext.borrow() {
        return Ok(v.clone());
    }
    let it = vm.call(c, "iterator", "()Ljava/util/Iterator;", &[])?.r().ok_or_else(|| vm.exception(NPE, "iterator"))?;
    let mut out = Vec::new();
    while vm.call(&it, "hasNext", "()Z", &[])?.i() != 0 {
        out.push(vm.call(&it, "next", "()Ljava/lang/Object;", &[])?);
    }
    Ok(out)
}

fn map_entries(o: &Ref) -> Vec<(V, V)> {
    match &*o.ext.borrow() {
        Ext::Map(m) => m.clone(),
        _ => Vec::new(),
    }
}

fn with_map<T>(o: &Ref, f: impl FnOnce(&mut Vec<(V, V)>) -> T) -> T {
    let mut e = o.ext.borrow_mut();
    if !matches!(&*e, Ext::Map(_)) {
        *e = Ext::Map(Vec::new());
    }
    match &mut *e {
        Ext::Map(v) => f(v),
        _ => unreachable!(),
    }
}

fn map_find(vm: &mut Vm, o: &Ref, k: &V) -> R<Option<usize>> {
    let keys: Vec<V> = map_entries(o).into_iter().map(|(k, _)| k).collect();
    index_of(vm, &keys, k)
}

fn map_get(vm: &mut Vm, o: &Ref, k: &V) -> R<Option<V>> {
    Ok(map_find(vm, o, k)?.map(|i| map_entries(o)[i].1.clone()))
}

fn map_put(vm: &mut Vm, o: &Ref, k: V, v: V) -> R<V> {
    match map_find(vm, o, &k)? {
        Some(i) => Ok(with_map(o, |m| core::mem::replace(&mut m[i].1, v))),
        None => {
            with_map(o, |m| m.push((k, v)));
            Ok(V::NULL)
        }
    }
}

fn collection(vm: &mut Vm, class: &str, items: Vec<V>) -> R<V> {
    Ok(V::obj(vm.new_with(class, Ext::List(items))?))
}

// -------------------------------------------------------------------
// java.lang
// -------------------------------------------------------------------

const OBJECT: NClass = NClass {
    super_: None,
    methods: &[
        m!("<init>", "()V", |vm, a| Ok(V::default())),
        m!("getClass", "()Ljava/lang/Class;", |vm, a| {
            let c = this(a).class.clone();
            Ok(V::obj(vm.new_with("Ljava/lang/Class;", Ext::Class(c))?))
        }),
        m!("hashCode", "()I", |vm, a| Ok(V::int(identity_hash(&this(a))))),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(match a[1].r() {
            Some(b) => Rc::ptr_eq(&this(a), &b),
            None => false,
        }))),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let o = this(a);
            let s = format!("{}@{:x}", o.class.java_name(), identity_hash(&o));
            new_str(vm, &s)
        }),
        m!("clone", "()Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            let ext = match &*o.ext.borrow() {
                Ext::Arr(x) => Ext::Arr(crate::vm::Arr { elem: x.elem, prim: x.prim.clone(), refs: x.refs.clone() }),
                Ext::None => Ext::None,
                _ => return Err(vm.exception("Ljava/lang/CloneNotSupportedException;", &o.class.java_name())),
            };
            let c = vm.new_object(&o.class);
            *c.fields.borrow_mut() = o.fields.borrow().clone();
            *c.ext.borrow_mut() = ext;
            Ok(V::obj(c))
        }),
        m!("notify", "()V", |vm, a| Ok(V::default())),
        m!("notifyAll", "()V", |vm, a| Ok(V::default())),
    ],
    ..BASE
};

const CLASS: NClass = NClass {
    methods: &[
        m!("getName", "()Ljava/lang/String;", |vm, a| {
            let n = class_of(a).map(|c| c.java_name()).unwrap_or_default();
            new_str(vm, &n)
        }),
        m!("getSimpleName", "()Ljava/lang/String;", |vm, a| {
            let n = class_of(a).map(|c| c.java_name()).unwrap_or_default();
            let n = n.rsplit(['.', '$']).next().unwrap_or("").to_string();
            new_str(vm, &n)
        }),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &a[0])?;
            new_str(vm, &s)
        }),
        m!("isInterface", "()Z", |vm, a| Ok(V::bool(class_of(a).map(|c| c.is_interface).unwrap_or(false)))),
        m!("isInstance", "(Ljava/lang/Object;)Z", |vm, a| {
            let c = class_of(a);
            Ok(V::bool(match (c, a[1].r()) {
                (Some(c), Some(o)) => vm.is_subclass(&o.class, &c.name),
                _ => false,
            }))
        }),
    ],
    ..BASE
};

/// A `java.lang.Class` object for a descriptor.
pub fn class_value(vm: &mut Vm, desc: &str) -> V {
    match vm.class(desc) {
        Ok(c) => V::R(vm.new_with("Ljava/lang/Class;", Ext::Class(c)).ok()),
        Err(_) => V::NULL,
    }
}

fn class_of(a: &[V]) -> Option<Rc<crate::vm::Class>> {
    match &*this(a).ext.borrow() {
        Ext::Class(c) => Some(c.clone()),
        _ => None,
    }
}

const STRING: NClass = NClass {
    interfaces: &["Ljava/lang/CharSequence;", "Ljava/lang/Comparable;"],
    methods: &[
        m!("<init>", "()V", |vm, a| {
            *this(a).ext.borrow_mut() = Ext::Str("".into());
            Ok(V::default())
        }),
        m!("<init>", "(Ljava/lang/String;)V", |vm, a| {
            let s = sarg(vm, &a[1])?;
            *this(a).ext.borrow_mut() = Ext::Str(s);
            Ok(V::default())
        }),
        m!("<init>", "([C)V", |vm, a| {
            let s = chars_of(&a[1]);
            *this(a).ext.borrow_mut() = Ext::Str(s.into());
            Ok(V::default())
        }),
        m!("length", "()I", |vm, a| Ok(V::int(me(a).encode_utf16().count() as i32))),
        m!("isEmpty", "()Z", |vm, a| Ok(V::bool(me(a).is_empty()))),
        m!("charAt", "(I)C", |vm, a| {
            let u = utf16(&me(a));
            let i = a[1].i();
            match u.get(i as usize) {
                Some(c) if i >= 0 => Ok(V::int(*c as i32)),
                _ => Err(sioobe(vm, i, u.len())),
            }
        }),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(
            a[1].r().and_then(|o| str_of(&o)).map(|s| s == me(a)).unwrap_or(false)
        ))),
        m!("equalsIgnoreCase", "(Ljava/lang/String;)Z", |vm, a| Ok(V::bool(
            a[1].r().and_then(|o| str_of(&o)).map(|s| s.to_lowercase() == me(a).to_lowercase()).unwrap_or(false)
        ))),
        m!("hashCode", "()I", |vm, a| Ok(V::int(string_hash(&me(a))))),
        m!("toString", "()Ljava/lang/String;", |vm, a| Ok(a[0].clone())),
        m!("intern", "()Ljava/lang/String;", |vm, a| Ok(V::obj(vm.intern(&me(a))))),
        m!("compareTo", "(Ljava/lang/String;)I", |vm, a| {
            let o = sarg(vm, &a[1])?;
            let (x, y) = (utf16(&me(a)), utf16(&o));
            for (p, q) in x.iter().zip(y.iter()) {
                if p != q {
                    return Ok(V::int(*p as i32 - *q as i32));
                }
            }
            Ok(V::int(x.len() as i32 - y.len() as i32))
        }),
        m!("compareTo", "(Ljava/lang/Object;)I", |vm, a| {
            let o = sarg(vm, &a[1])?;
            Ok(V::int(match me(a).cmp(&o) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        }),
        m!("concat", "(Ljava/lang/String;)Ljava/lang/String;", |vm, a| {
            let o = sarg(vm, &a[1])?;
            new_str(vm, &format!("{}{}", me(a), o))
        }),
        m!("contains", "(Ljava/lang/CharSequence;)Z", |vm, a| {
            let o = sarg(vm, &a[1])?;
            Ok(V::bool(me(a).contains(&*o)))
        }),
        m!("startsWith", "(Ljava/lang/String;)Z", |vm, a| {
            let o = sarg(vm, &a[1])?;
            Ok(V::bool(me(a).starts_with(&*o)))
        }),
        m!("endsWith", "(Ljava/lang/String;)Z", |vm, a| {
            let o = sarg(vm, &a[1])?;
            Ok(V::bool(me(a).ends_with(&*o)))
        }),
        m!("indexOf", "(Ljava/lang/String;)I", |vm, a| {
            let o = sarg(vm, &a[1])?;
            let s = me(a);
            Ok(V::int(s.find(&*o).map(|b| s[..b].encode_utf16().count() as i32).unwrap_or(-1)))
        }),
        m!("indexOf", "(Ljava/lang/String;I)I", |vm, a| {
            let o = utf16(&sarg(vm, &a[1])?);
            Ok(V::int(find16(&utf16(&me(a)), &o, a[2].i().max(0) as usize)))
        }),
        m!("indexOf", "(II)I", |vm, a| {
            let c = a[1].i() as u16;
            let from = a[2].i().max(0) as usize;
            let u = utf16(&me(a));
            Ok(V::int((from..u.len()).find(|&k| u[k] == c).map(|p| p as i32).unwrap_or(-1)))
        }),
        m!("lastIndexOf", "(Ljava/lang/String;)I", |vm, a| {
            let o = utf16(&sarg(vm, &a[1])?);
            let u = utf16(&me(a));
            Ok(V::int(rfind16(&u, &o, u.len())))
        }),
        m!("lastIndexOf", "(Ljava/lang/String;I)I", |vm, a| {
            let o = utf16(&sarg(vm, &a[1])?);
            let u = utf16(&me(a));
            if a[2].i() < 0 {
                return Ok(V::int(-1));
            }
            Ok(V::int(rfind16(&u, &o, a[2].i() as usize)))
        }),
        m!("regionMatches", "(ILjava/lang/String;II)Z", |vm, a| {
            let o = utf16(&sarg(vm, &a[2])?);
            Ok(V::bool(region(&utf16(&me(a)), a[1].i(), &o, a[3].i(), a[4].i(), false)))
        }),
        m!("regionMatches", "(ZILjava/lang/String;II)Z", |vm, a| {
            let o = utf16(&sarg(vm, &a[3])?);
            Ok(V::bool(region(&utf16(&me(a)), a[2].i(), &o, a[4].i(), a[5].i(), a[1].i() != 0)))
        }),
        m!("subSequence", "(II)Ljava/lang/CharSequence;", |vm, a| {
            let u = utf16(&me(a));
            let (b, e) = (a[1].i(), a[2].i());
            if b < 0 || e < b || e as usize > u.len() {
                return Err(sioobe(vm, e, u.len()));
            }
            new_str(vm, &String::from_utf16_lossy(&u[b as usize..e as usize]))
        }),
        m!("contentEquals", "(Ljava/lang/CharSequence;)Z", |vm, a| {
            let o = jstr(vm, &a[1])?;
            Ok(V::bool(*me(a) == *o))
        }),
        m!("repeat", "(I)Ljava/lang/String;", |vm, a| new_str(vm, &me(a).repeat(a[1].i().max(0) as usize))),
        m!("isBlank", "()Z", |vm, a| Ok(V::bool(me(a).trim().is_empty()))),
        m!("strip", "()Ljava/lang/String;", |vm, a| new_str(vm, me(a).trim())),
        m!("codePointAt", "(I)I", |vm, a| {
            let u = utf16(&me(a));
            let i = a[1].i();
            match u.get(i as usize) {
                Some(c) if i >= 0 => Ok(V::int(*c as i32)),
                _ => Err(sioobe(vm, i, u.len())),
            }
        }),
        m!("getBytes", "()[B", |vm, a| {
            let b = me(a).as_bytes().to_vec();
            let arr = vm.new_array("[B", b.len())?;
            if let Ext::Arr(x) = &mut *arr.ext.borrow_mut() {
                x.prim = b.iter().map(|&c| c as i8 as i32 as u32 as u64).collect();
            }
            Ok(V::obj(arr))
        }),
        m!("indexOf", "(I)I", |vm, a| {
            let c = a[1].i() as u16;
            Ok(V::int(utf16(&me(a)).iter().position(|&x| x == c).map(|p| p as i32).unwrap_or(-1)))
        }),
        m!("lastIndexOf", "(I)I", |vm, a| {
            let c = a[1].i() as u16;
            Ok(V::int(utf16(&me(a)).iter().rposition(|&x| x == c).map(|p| p as i32).unwrap_or(-1)))
        }),
        m!("substring", "(I)Ljava/lang/String;", |vm, a| {
            let u = utf16(&me(a));
            let b = a[1].i();
            if b < 0 || b as usize > u.len() {
                return Err(sioobe(vm, b, u.len()));
            }
            new_str(vm, &String::from_utf16_lossy(&u[b as usize..]))
        }),
        m!("substring", "(II)Ljava/lang/String;", |vm, a| {
            let u = utf16(&me(a));
            let (b, e) = (a[1].i(), a[2].i());
            if b < 0 || e < b || e as usize > u.len() {
                return Err(sioobe(vm, e, u.len()));
            }
            new_str(vm, &String::from_utf16_lossy(&u[b as usize..e as usize]))
        }),
        m!("trim", "()Ljava/lang/String;", |vm, a| new_str(vm, me(a).trim())),
        m!("toUpperCase", "(Ljava/util/Locale;)Ljava/lang/String;", |vm, a| new_str(vm, &me(a).to_uppercase())),
        m!("toLowerCase", "(Ljava/util/Locale;)Ljava/lang/String;", |vm, a| new_str(vm, &me(a).to_lowercase())),
        m!("toUpperCase", "()Ljava/lang/String;", |vm, a| new_str(vm, &me(a).to_uppercase())),
        m!("toLowerCase", "()Ljava/lang/String;", |vm, a| new_str(vm, &me(a).to_lowercase())),
        m!("replace", "(CC)Ljava/lang/String;", |vm, a| {
            let (f, t) = (a[1].i() as u16, a[2].i() as u16);
            let u: Vec<u16> = utf16(&me(a)).into_iter().map(|c| if c == f { t } else { c }).collect();
            new_str(vm, &String::from_utf16_lossy(&u))
        }),
        m!("replace", "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Ljava/lang/String;", |vm, a| {
            let (f, t) = (sarg(vm, &a[1])?, sarg(vm, &a[2])?);
            new_str(vm, &me(a).replace(&*f, &t))
        }),
        m!("split", "(Ljava/lang/String;)[Ljava/lang/String;", |vm, a| {
            let re = sarg(vm, &a[1])?;
            let parts = split(vm, &me(a), &re);
            string_array(vm, &parts)
        }),
        m!("toCharArray", "()[C", |vm, a| {
            let u = utf16(&me(a));
            let arr = vm.new_array("[C", u.len())?;
            if let Ext::Arr(x) = &mut *arr.ext.borrow_mut() {
                x.prim = u.iter().map(|&c| c as u64).collect();
            }
            Ok(V::obj(arr))
        }),
        m!("chars", "()Ljava/util/stream/IntStream;", |vm, a| {
            vm.note_missing("java.util.stream");
            Err(vm.exception("Ljava/lang/UnsupportedOperationException;", "streams"))
        }),
        s!("valueOf", "(Ljava/lang/Object;)Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &a[0])?;
            new_str(vm, &s)
        }),
        s!("valueOf", "(I)Ljava/lang/String;", |vm, a| new_str(vm, &a[0].i().to_string())),
        s!("valueOf", "(J)Ljava/lang/String;", |vm, a| new_str(vm, &a[0].j().to_string())),
        s!("valueOf", "(Z)Ljava/lang/String;", |vm, a| new_str(vm, &(a[0].i() != 0).to_string())),
        s!("valueOf", "(C)Ljava/lang/String;", |vm, a| new_str(vm, &jchar(a[0].i()))),
        s!("valueOf", "(D)Ljava/lang/String;", |vm, a| new_str(vm, &jdouble(a[0].d()))),
        s!("valueOf", "(F)Ljava/lang/String;", |vm, a| new_str(vm, &jfloat(a[0].f()))),
        s!("valueOf", "([C)Ljava/lang/String;", |vm, a| new_str(vm, &chars_of(&a[0]))),
        s!("format", "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;", |vm, a| {
            let f = sarg(vm, &a[0])?;
            let s = jformat(vm, &f, &varargs(&a[1]))?;
            new_str(vm, &s)
        }),
        s!("format", "(Ljava/util/Locale;Ljava/lang/String;[Ljava/lang/Object;)Ljava/lang/String;", |vm, a| {
            let f = sarg(vm, &a[1])?;
            let s = jformat(vm, &f, &varargs(&a[2]))?;
            new_str(vm, &s)
        }),
        s!("join", "(Ljava/lang/CharSequence;[Ljava/lang/CharSequence;)Ljava/lang/String;", |vm, a| {
            let sep = sarg(vm, &a[0])?;
            let mut parts = Vec::new();
            for p in varargs(&a[1]) {
                parts.push(jstr(vm, &p)?);
            }
            new_str(vm, &parts.join(&sep))
        }),
        s!("join", "(Ljava/lang/CharSequence;Ljava/lang/Iterable;)Ljava/lang/String;", |vm, a| {
            let sep = sarg(vm, &a[0])?;
            let c = a[1].r().ok_or_else(|| vm.exception(NPE, "join"))?;
            let mut parts = Vec::new();
            for p in elements(vm, &c)? {
                parts.push(jstr(vm, &p)?);
            }
            new_str(vm, &parts.join(&sep))
        }),
    ],
    ..BASE
};

fn find16(h: &[u16], n: &[u16], from: usize) -> i32 {
    if n.is_empty() {
        return from.min(h.len()) as i32;
    }
    (from..h.len().saturating_sub(n.len() - 1)).find(|&k| h[k..k + n.len()] == *n).map(|k| k as i32).unwrap_or(-1)
}

fn rfind16(h: &[u16], n: &[u16], from: usize) -> i32 {
    if n.len() > h.len() {
        return -1;
    }
    let start = from.min(h.len() - n.len());
    (0..=start).rev().find(|&k| h[k..k + n.len()] == *n).map(|k| k as i32).unwrap_or(-1)
}

fn region(a: &[u16], ao: i32, b: &[u16], bo: i32, len: i32, ignore_case: bool) -> bool {
    if ao < 0 || bo < 0 || len < 0 || (ao + len) as usize > a.len() || (bo + len) as usize > b.len() {
        return false;
    }
    let (x, y) = (&a[ao as usize..(ao + len) as usize], &b[bo as usize..(bo + len) as usize]);
    if ignore_case {
        String::from_utf16_lossy(x).to_lowercase() == String::from_utf16_lossy(y).to_lowercase()
    } else {
        x == y
    }
}

fn chars_of(v: &V) -> String {
    match v.r() {
        Some(o) => match &*o.ext.borrow() {
            Ext::Arr(x) => String::from_utf16_lossy(&x.prim.iter().map(|&c| c as u16).collect::<Vec<_>>()),
            _ => String::new(),
        },
        None => String::new(),
    }
}

fn builder<T>(o: &Ref, f: impl FnOnce(&mut String) -> T) -> T {
    let mut e = o.ext.borrow_mut();
    if !matches!(&*e, Ext::Builder(_)) {
        *e = Ext::Builder(String::new());
    }
    match &mut *e {
        Ext::Builder(s) => f(s),
        _ => unreachable!(),
    }
}

fn append(a: &[V], s: &str) -> R<V> {
    builder(&this(a), |b| b.push_str(s));
    Ok(a[0].clone())
}

fn append_range(vm: &mut Vm, a: &[V]) -> R<V> {
    let s = utf16(&jstr(vm, &a[1])?);
    let (b, e) = (a[2].i(), a[3].i());
    if b < 0 || e < b || e as usize > s.len() {
        return Err(ioobe(vm, e, s.len()));
    }
    append(a, &String::from_utf16_lossy(&s[b as usize..e as usize]))
}

macro_rules! builder_methods {
    ($ret:literal) => {
        &[
            m!("<init>", "()V", |vm, a| {
                builder(&this(a), |_| ());
                Ok(V::default())
            }),
            m!("<init>", "(I)V", |vm, a| {
                builder(&this(a), |_| ());
                Ok(V::default())
            }),
            m!("<init>", "(Ljava/lang/String;)V", |vm, a| {
                let s = sarg(vm, &a[1])?;
                builder(&this(a), |b| b.push_str(&s));
                Ok(V::default())
            }),
            m!("<init>", "(Ljava/lang/CharSequence;)V", |vm, a| {
                let s = jstr(vm, &a[1])?;
                builder(&this(a), |b| b.push_str(&s));
                Ok(V::default())
            }),
            m!("append", concat!("(Ljava/lang/String;)", $ret), |vm, a| {
                let s = jstr(vm, &a[1])?;
                append(a, &s)
            }),
            m!("append", concat!("(Ljava/lang/Object;)", $ret), |vm, a| {
                let s = jstr(vm, &a[1])?;
                append(a, &s)
            }),
            m!("append", concat!("(Ljava/lang/CharSequence;)", $ret), |vm, a| {
                let s = jstr(vm, &a[1])?;
                append(a, &s)
            }),
            m!("append", "(Ljava/lang/CharSequence;)Ljava/lang/Appendable;", |vm, a| {
                let s = jstr(vm, &a[1])?;
                append(a, &s)
            }),
            m!("append", "(C)Ljava/lang/Appendable;", |vm, a| append(a, &jchar(a[1].i()))),
            m!("append", concat!("(Ljava/lang/CharSequence;II)", $ret), |vm, a| append_range(vm, a)),
            m!("append", "(Ljava/lang/CharSequence;II)Ljava/lang/Appendable;", |vm, a| append_range(vm, a)),
            m!("append", concat!("(I)", $ret), |vm, a| append(a, &a[1].i().to_string())),
            m!("append", concat!("(J)", $ret), |vm, a| append(a, &a[1].j().to_string())),
            m!("append", concat!("(Z)", $ret), |vm, a| append(a, &(a[1].i() != 0).to_string())),
            m!("append", concat!("(C)", $ret), |vm, a| append(a, &jchar(a[1].i()))),
            m!("append", concat!("(D)", $ret), |vm, a| append(a, &jdouble(a[1].d()))),
            m!("append", concat!("(F)", $ret), |vm, a| append(a, &jfloat(a[1].f()))),
            m!("append", concat!("([C)", $ret), |vm, a| append(a, &chars_of(&a[1]))),
            m!("length", "()I", |vm, a| Ok(V::int(builder(&this(a), |b| b.encode_utf16().count()) as i32))),
            m!("charAt", "(I)C", |vm, a| {
                let u = builder(&this(a), |b| utf16(b));
                let i = a[1].i();
                match u.get(i as usize) {
                    Some(c) if i >= 0 => Ok(V::int(*c as i32)),
                    _ => Err(sioobe(vm, i, u.len())),
                }
            }),
            m!("setLength", "(I)V", |vm, a| {
                let n = a[1].i().max(0) as usize;
                builder(&this(a), |b| {
                    let mut u = utf16(b);
                    u.resize(n, 0);
                    *b = String::from_utf16_lossy(&u);
                });
                Ok(V::default())
            }),
            m!("reverse", concat!("()", $ret), |vm, a| {
                builder(&this(a), |b| *b = b.chars().rev().collect());
                Ok(a[0].clone())
            }),
            m!("insert", concat!("(ILjava/lang/String;)", $ret), |vm, a| {
                let s = jstr(vm, &a[2])?;
                let at = a[1].i().max(0) as usize;
                builder(&this(a), |b| {
                    let mut u = utf16(b);
                    let at = at.min(u.len());
                    u.splice(at..at, s.encode_utf16());
                    *b = String::from_utf16_lossy(&u);
                });
                Ok(a[0].clone())
            }),
            m!("deleteCharAt", concat!("(I)", $ret), |vm, a| {
                let at = a[1].i();
                let len = builder(&this(a), |b| b.encode_utf16().count());
                if at < 0 || at as usize >= len {
                    return Err(sioobe(vm, at, len));
                }
                builder(&this(a), |b| {
                    let mut u = utf16(b);
                    u.remove(at as usize);
                    *b = String::from_utf16_lossy(&u);
                });
                Ok(a[0].clone())
            }),
            m!("toString", "()Ljava/lang/String;", |vm, a| {
                let s = builder(&this(a), |b| b.clone());
                new_str(vm, &s)
            }),
        ]
    };
}

const STRING_BUILDER: NClass = NClass {
    interfaces: &["Ljava/lang/CharSequence;", "Ljava/lang/Appendable;"],
    methods: builder_methods!("Ljava/lang/StringBuilder;"),
    ..BASE
};
const STRING_BUFFER: NClass =
    NClass { interfaces: &["Ljava/lang/CharSequence;"], methods: builder_methods!("Ljava/lang/StringBuffer;"), ..BASE };

const THROWABLE: NClass = NClass {
    fields: &[("detailMessage", "Ljava/lang/String;"), ("cause", "Ljava/lang/Throwable;")],
    methods: &[
        m!("<init>", "()V", |vm, a| Ok(V::default())),
        m!("<init>", "(Ljava/lang/String;)V", |vm, a| {
            set_field(&this(a), "detailMessage", a[1].clone());
            Ok(V::default())
        }),
        m!("<init>", "(Ljava/lang/String;Ljava/lang/Throwable;)V", |vm, a| {
            set_field(&this(a), "detailMessage", a[1].clone());
            set_field(&this(a), "cause", a[2].clone());
            Ok(V::default())
        }),
        m!("<init>", "(Ljava/lang/Throwable;)V", |vm, a| {
            if let Some(c) = a[1].r() {
                let s = jstr(vm, &a[1])?;
                let s = vm.str_value(&s);
                set_field(&this(a), "detailMessage", s);
                set_field(&this(a), "cause", V::obj(c));
            }
            Ok(V::default())
        }),
        m!("getMessage", "()Ljava/lang/String;", |vm, a| Ok(get_field(&this(a), "detailMessage"))),
        m!("getLocalizedMessage", "()Ljava/lang/String;", |vm, a| {
            vm.call(&this(a), "getMessage", "()Ljava/lang/String;", &[])
        }),
        m!("getCause", "()Ljava/lang/Throwable;", |vm, a| Ok(get_field(&this(a), "cause"))),
        m!("initCause", "(Ljava/lang/Throwable;)Ljava/lang/Throwable;", |vm, a| {
            set_field(&this(a), "cause", a[1].clone());
            Ok(a[0].clone())
        }),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let o = this(a);
            let msg = vm.call(&o, "getLocalizedMessage", "()Ljava/lang/String;", &[])?;
            let s = match msg.r() {
                Some(_) => format!("{}: {}", o.class.java_name(), jstr(vm, &msg)?),
                None => o.class.java_name(),
            };
            new_str(vm, &s)
        }),
        m!("printStackTrace", "()V", |vm, a| {
            let s = jstr(vm, &a[0])?;
            eprintln!("{}\n\tat <droid interpreter>", s);
            Ok(V::default())
        }),
        m!("getStackTrace", "()[Ljava/lang/StackTraceElement;", |vm, a| {
            Ok(V::obj(vm.new_array("[Ljava/lang/StackTraceElement;", 0)?))
        }),
        m!("addSuppressed", "(Ljava/lang/Throwable;)V", |vm, a| Ok(V::default())),
    ],
    ..BASE
};

/// Exception classes and their superclasses; they inherit everything from
/// Throwable.
const EXCEPTIONS: &[(&str, &str)] = &[
    ("Ljava/lang/Exception;", "Ljava/lang/Throwable;"),
    ("Ljava/lang/Error;", "Ljava/lang/Throwable;"),
    ("Ljava/lang/RuntimeException;", "Ljava/lang/Exception;"),
    ("Ljava/lang/IllegalStateException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/IllegalArgumentException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/NumberFormatException;", "Ljava/lang/IllegalArgumentException;"),
    ("Ljava/lang/IndexOutOfBoundsException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/ArrayIndexOutOfBoundsException;", "Ljava/lang/IndexOutOfBoundsException;"),
    ("Ljava/lang/StringIndexOutOfBoundsException;", "Ljava/lang/IndexOutOfBoundsException;"),
    ("Ljava/lang/NullPointerException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/ArithmeticException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/ClassCastException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/NegativeArraySizeException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/ArrayStoreException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/UnsupportedOperationException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/SecurityException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/util/NoSuchElementException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/util/ConcurrentModificationException;", "Ljava/lang/RuntimeException;"),
    ("Ljava/lang/InterruptedException;", "Ljava/lang/Exception;"),
    ("Ljava/lang/CloneNotSupportedException;", "Ljava/lang/Exception;"),
    ("Ljava/lang/ReflectiveOperationException;", "Ljava/lang/Exception;"),
    ("Ljava/lang/ClassNotFoundException;", "Ljava/lang/ReflectiveOperationException;"),
    ("Ljava/io/IOException;", "Ljava/lang/Exception;"),
    ("Ljava/io/FileNotFoundException;", "Ljava/io/IOException;"),
    ("Ljava/lang/LinkageError;", "Ljava/lang/Error;"),
    ("Ljava/lang/NoClassDefFoundError;", "Ljava/lang/LinkageError;"),
    ("Ljava/lang/IncompatibleClassChangeError;", "Ljava/lang/LinkageError;"),
    ("Ljava/lang/NoSuchMethodError;", "Ljava/lang/IncompatibleClassChangeError;"),
    ("Ljava/lang/NoSuchFieldError;", "Ljava/lang/IncompatibleClassChangeError;"),
    ("Ljava/lang/AbstractMethodError;", "Ljava/lang/IncompatibleClassChangeError;"),
    ("Ljava/lang/VerifyError;", "Ljava/lang/LinkageError;"),
    ("Ljava/lang/UnsatisfiedLinkError;", "Ljava/lang/LinkageError;"),
    ("Ljava/lang/VirtualMachineError;", "Ljava/lang/Error;"),
    ("Ljava/lang/StackOverflowError;", "Ljava/lang/VirtualMachineError;"),
    ("Ljava/lang/OutOfMemoryError;", "Ljava/lang/VirtualMachineError;"),
    ("Ljava/lang/AssertionError;", "Ljava/lang/Error;"),
];

macro_rules! boxed_class {
    ($cls:literal, $prim:literal, $getter:literal) => {
        NClass {
            super_: Some("Ljava/lang/Number;"),
            interfaces: &["Ljava/lang/Comparable;"],
            methods: &[
                s!("valueOf", concat!("(", $prim, ")", $cls), |vm, a| boxed(vm, $cls, a[0].clone())),
                m!("<init>", concat!("(", $prim, ")V"), |vm, a| {
                    *this(a).ext.borrow_mut() = Ext::Boxed(a[1].clone());
                    Ok(V::default())
                }),
                m!($getter, concat!("()", $prim), |vm, a| Ok(unbox(&a[0]))),
                m!("intValue", "()I", |vm, a| Ok(V::int(num(&a[0]) as i32))),
                m!("longValue", "()J", |vm, a| Ok(V::long(num(&a[0]) as i64))),
                m!("doubleValue", "()D", |vm, a| Ok(V::double(num(&a[0])))),
                m!("floatValue", "()F", |vm, a| Ok(V::float(num(&a[0]) as f32))),
                m!("equals", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(jequals(vm, &a[0], &a[1])?))),
                m!("hashCode", "()I", |vm, a| Ok(V::int(jhash(vm, &a[0])?))),
                m!("compareTo", "(Ljava/lang/Object;)I", |vm, a| {
                    let (x, y) = (num(&a[0]), num(&a[1]));
                    Ok(V::int(if x < y {
                        -1
                    } else if x > y {
                        1
                    } else {
                        0
                    }))
                }),
                m!("toString", "()Ljava/lang/String;", |vm, a| {
                    let s = jstr(vm, &a[0])?;
                    new_str(vm, &s)
                }),
                s!("toString", concat!("(", $prim, ")Ljava/lang/String;"), |vm, a| {
                    let s = boxed_str($cls, &a[0]);
                    new_str(vm, &s)
                }),
            ],
            statics: |vm| vec![("TYPE", class_value(vm, $prim))],
            ..BASE
        }
    };
}

/// A boxed number as f64 (for widening conversions and comparisons).
fn num(v: &V) -> f64 {
    let class = v.r().map(|o| o.class.name.to_string()).unwrap_or_default();
    let x = unbox(v);
    match class.as_str() {
        "Ljava/lang/Long;" => x.j() as f64,
        "Ljava/lang/Double;" => x.d(),
        "Ljava/lang/Float;" => x.f() as f64,
        _ => x.i() as f64,
    }
}

fn parse_int(vm: &mut Vm, v: &V, radix: u32) -> R<i64> {
    let s = sarg(vm, v)?;
    i64::from_str_radix(&s, radix)
        .map_err(|_| vm.exception("Ljava/lang/NumberFormatException;", &format!("For input string: \"{}\"", s)))
}

const INTEGER: NClass = boxed_class!("Ljava/lang/Integer;", "I", "intValue");
const LONG: NClass = boxed_class!("Ljava/lang/Long;", "J", "longValue");
const DOUBLE: NClass = boxed_class!("Ljava/lang/Double;", "D", "doubleValue");
const FLOAT: NClass = boxed_class!("Ljava/lang/Float;", "F", "floatValue");
const SHORT: NClass = boxed_class!("Ljava/lang/Short;", "S", "shortValue");
const BYTE: NClass = boxed_class!("Ljava/lang/Byte;", "B", "byteValue");

const INTEGER_STATICS: NClass = NClass {
    super_: Some("Ljava/lang/Integer$Base;"),
    methods: &[
        s!("parseInt", "(Ljava/lang/String;)I", |vm, a| {
            let n = parse_int(vm, &a[0], 10)?;
            if n < i32::MIN as i64 || n > i32::MAX as i64 {
                return Err(vm.exception("Ljava/lang/NumberFormatException;", "out of range"));
            }
            Ok(V::int(n as i32))
        }),
        s!("parseInt", "(Ljava/lang/String;I)I", |vm, a| {
            let n = parse_int(vm, &a[0], a[1].i() as u32)?;
            Ok(V::int(n as i32))
        }),
        s!("valueOf", "(Ljava/lang/String;)Ljava/lang/Integer;", |vm, a| {
            let n = parse_int(vm, &a[0], 10)?;
            boxed(vm, "Ljava/lang/Integer;", V::int(n as i32))
        }),
        s!("toHexString", "(I)Ljava/lang/String;", |vm, a| new_str(vm, &format!("{:x}", a[0].i()))),
        s!("toBinaryString", "(I)Ljava/lang/String;", |vm, a| new_str(vm, &format!("{:b}", a[0].i()))),
        s!("max", "(II)I", |vm, a| Ok(V::int(a[0].i().max(a[1].i())))),
        s!("min", "(II)I", |vm, a| Ok(V::int(a[0].i().min(a[1].i())))),
        s!("compare", "(II)I", |vm, a| Ok(V::int(a[0].i().cmp(&a[1].i()) as i32))),
        s!("sum", "(II)I", |vm, a| Ok(V::int(a[0].i().wrapping_add(a[1].i())))),
        s!("bitCount", "(I)I", |vm, a| Ok(V::int(a[0].i().count_ones() as i32))),
    ],
    ..INTEGER
};

const LONG_STATICS: NClass = NClass {
    super_: Some("Ljava/lang/Long$Base;"),
    methods: &[
        s!("parseLong", "(Ljava/lang/String;)J", |vm, a| Ok(V::long(parse_int(vm, &a[0], 10)?))),
        s!("compare", "(JJ)I", |vm, a| Ok(V::int(a[0].j().cmp(&a[1].j()) as i32))),
    ],
    ..LONG
};

const DOUBLE_STATICS: NClass = NClass {
    super_: Some("Ljava/lang/Double$Base;"),
    methods: &[
        s!("parseDouble", "(Ljava/lang/String;)D", |vm, a| {
            let s = sarg(vm, &a[0])?;
            match s.trim().parse::<f64>() {
                Ok(d) => Ok(V::double(d)),
                Err(_) => {
                    Err(vm.exception("Ljava/lang/NumberFormatException;", &format!("For input string: \"{}\"", s)))
                }
            }
        }),
        s!("isNaN", "(D)Z", |vm, a| Ok(V::bool(a[0].d().is_nan()))),
        s!("compare", "(DD)I", |vm, a| Ok(V::int(a[0].d().partial_cmp(&a[1].d()).map(|o| o as i32).unwrap_or(0)))),
        s!("doubleToLongBits", "(D)J", |vm, a| Ok(V::long(a[0].d().to_bits() as i64))),
        s!("longBitsToDouble", "(J)D", |vm, a| Ok(V::double(f64::from_bits(a[0].j() as u64)))),
    ],
    ..DOUBLE
};

const BOOLEAN: NClass = NClass {
    methods: &[
        s!("valueOf", "(Z)Ljava/lang/Boolean;", |vm, a| boxed(vm, "Ljava/lang/Boolean;", a[0].clone())),
        s!("parseBoolean", "(Ljava/lang/String;)Z", |vm, a| {
            let s = jstr(vm, &a[0])?;
            Ok(V::bool(s.eq_ignore_ascii_case("true")))
        }),
        s!("toString", "(Z)Ljava/lang/String;", |vm, a| new_str(vm, &(a[0].i() != 0).to_string())),
        m!("booleanValue", "()Z", |vm, a| Ok(unbox(&a[0]))),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(jequals(vm, &a[0], &a[1])?))),
        m!("hashCode", "()I", |vm, a| Ok(V::int(if unbox(&a[0]).i() != 0 { 1231 } else { 1237 }))),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &a[0])?;
            new_str(vm, &s)
        }),
    ],
    statics: |vm| {
        let ty = class_value(vm, "Z");
        let t = V::obj(vm.new_with("Ljava/lang/Boolean;", Ext::Boxed(V::int(1))).expect("Boolean"));
        let f = V::obj(vm.new_with("Ljava/lang/Boolean;", Ext::Boxed(V::int(0))).expect("Boolean"));
        vec![("TRUE", t), ("FALSE", f), ("TYPE", ty)]
    },
    ..BASE
};

const CHARACTER: NClass = NClass {
    statics: |vm| vec![("TYPE", class_value(vm, "C"))],
    methods: &[
        s!("valueOf", "(C)Ljava/lang/Character;", |vm, a| boxed(vm, "Ljava/lang/Character;", a[0].clone())),
        m!("charValue", "()C", |vm, a| Ok(unbox(&a[0]))),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(jequals(vm, &a[0], &a[1])?))),
        m!("hashCode", "()I", |vm, a| Ok(V::int(unbox(&a[0]).i()))),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &a[0])?;
            new_str(vm, &s)
        }),
        s!("toString", "(C)Ljava/lang/String;", |vm, a| new_str(vm, &jchar(a[0].i()))),
        s!("isDigit", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_numeric()))),
        s!("isLetter", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_alphabetic()))),
        s!("isLetterOrDigit", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_alphanumeric()))),
        s!("isWhitespace", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_whitespace()))),
        s!("isUpperCase", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_uppercase()))),
        s!("isLowerCase", "(C)Z", |vm, a| Ok(V::bool(ch(&a[0]).is_lowercase()))),
        s!("toUpperCase", "(C)C", |vm, a| Ok(V::int(ch(&a[0]).to_uppercase().next().unwrap_or('\0') as i32))),
        s!("toLowerCase", "(C)C", |vm, a| Ok(V::int(ch(&a[0]).to_lowercase().next().unwrap_or('\0') as i32))),
        s!("getNumericValue", "(C)I", |vm, a| Ok(V::int(ch(&a[0]).to_digit(36).map(|d| d as i32).unwrap_or(-1)))),
    ],
    ..BASE
};

fn ch(v: &V) -> char {
    char::from_u32(v.i() as u32 & 0xffff).unwrap_or('\0')
}

const MATH: NClass = NClass {
    methods: &[
        s!("abs", "(I)I", |vm, a| Ok(V::int(a[0].i().wrapping_abs()))),
        s!("abs", "(J)J", |vm, a| Ok(V::long(a[0].j().wrapping_abs()))),
        s!("abs", "(D)D", |vm, a| Ok(V::double(a[0].d().abs()))),
        s!("abs", "(F)F", |vm, a| Ok(V::float(a[0].f().abs()))),
        s!("max", "(II)I", |vm, a| Ok(V::int(a[0].i().max(a[1].i())))),
        s!("min", "(II)I", |vm, a| Ok(V::int(a[0].i().min(a[1].i())))),
        s!("max", "(JJ)J", |vm, a| Ok(V::long(a[0].j().max(a[1].j())))),
        s!("min", "(JJ)J", |vm, a| Ok(V::long(a[0].j().min(a[1].j())))),
        s!("max", "(DD)D", |vm, a| Ok(V::double(a[0].d().max(a[1].d())))),
        s!("min", "(DD)D", |vm, a| Ok(V::double(a[0].d().min(a[1].d())))),
        s!("max", "(FF)F", |vm, a| Ok(V::float(a[0].f().max(a[1].f())))),
        s!("min", "(FF)F", |vm, a| Ok(V::float(a[0].f().min(a[1].f())))),
        s!("sqrt", "(D)D", |vm, a| Ok(V::double(a[0].d().sqrt()))),
        s!("cbrt", "(D)D", |vm, a| Ok(V::double(a[0].d().cbrt()))),
        s!("pow", "(DD)D", |vm, a| Ok(V::double(a[0].d().powf(a[1].d())))),
        s!("exp", "(D)D", |vm, a| Ok(V::double(a[0].d().exp()))),
        s!("log", "(D)D", |vm, a| Ok(V::double(a[0].d().ln()))),
        s!("log10", "(D)D", |vm, a| Ok(V::double(a[0].d().log10()))),
        s!("sin", "(D)D", |vm, a| Ok(V::double(a[0].d().sin()))),
        s!("cos", "(D)D", |vm, a| Ok(V::double(a[0].d().cos()))),
        s!("tan", "(D)D", |vm, a| Ok(V::double(a[0].d().tan()))),
        s!("atan", "(D)D", |vm, a| Ok(V::double(a[0].d().atan()))),
        s!("atan2", "(DD)D", |vm, a| Ok(V::double(a[0].d().atan2(a[1].d())))),
        s!("hypot", "(DD)D", |vm, a| Ok(V::double(a[0].d().hypot(a[1].d())))),
        s!("floor", "(D)D", |vm, a| Ok(V::double(a[0].d().floor()))),
        s!("ceil", "(D)D", |vm, a| Ok(V::double(a[0].d().ceil()))),
        s!("rint", "(D)D", |vm, a| Ok(V::double(a[0].d().round_ties_even()))),
        s!("round", "(D)J", |vm, a| Ok(V::long((a[0].d() + 0.5).floor() as i64))),
        s!("round", "(F)I", |vm, a| Ok(V::int((a[0].f() + 0.5).floor() as i32))),
        s!("toRadians", "(D)D", |vm, a| Ok(V::double(a[0].d().to_radians()))),
        s!("toDegrees", "(D)D", |vm, a| Ok(V::double(a[0].d().to_degrees()))),
        s!("floorMod", "(II)I", |vm, a| {
            if a[1].i() == 0 {
                return Err(vm.exception("Ljava/lang/ArithmeticException;", "/ by zero"));
            }
            Ok(V::int(a[0].i().rem_euclid(a[1].i())))
        }),
        s!("random", "()D", |vm, a| Ok(V::double(random() as f64 / u64::MAX as f64))),
    ],
    ..BASE
};

fn random() -> u64 {
    use std::cell::Cell;
    thread_local!(static SEED: Cell<u64> = Cell::new(0x2545_f491_4f6c_dd1d ^ now_nanos()));
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

fn now_nanos() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

fn monotonic_nanos() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

const SYSTEM: NClass = NClass {
    methods: &[
        s!("currentTimeMillis", "()J", |vm, a| Ok(V::long((now_nanos() / 1_000_000) as i64))),
        s!("nanoTime", "()J", |vm, a| Ok(V::long(monotonic_nanos() as i64))),
        s!("arraycopy", "(Ljava/lang/Object;ILjava/lang/Object;II)V", |vm, a| {
            let (src, dst) = match (a[0].r(), a[2].r()) {
                (Some(s), Some(d)) => (s, d),
                _ => return Err(vm.exception(NPE, "arraycopy")),
            };
            let (sp, dp, n) = (a[1].i(), a[3].i(), a[4].i());
            let (sl, dl) = (arr_len(&src), arr_len(&dst));
            if sp < 0 || dp < 0 || n < 0 || (sp + n) as usize > sl || (dp + n) as usize > dl {
                return Err(vm.exception(
                    "Ljava/lang/ArrayIndexOutOfBoundsException;",
                    "arraycopy: last source index out of bounds",
                ));
            }
            let (sp, dp, n) = (sp as usize, dp as usize, n as usize);
            let (prim, refs) = match &*src.ext.borrow() {
                Ext::Arr(x) => (
                    x.prim.get(sp..sp + n).map(|s| s.to_vec()).unwrap_or_default(),
                    x.refs.get(sp..sp + n).map(|s| s.to_vec()).unwrap_or_default(),
                ),
                _ => (Vec::new(), Vec::new()),
            };
            if let Ext::Arr(x) = &mut *dst.ext.borrow_mut() {
                if !prim.is_empty() {
                    x.prim[dp..dp + n].copy_from_slice(&prim);
                }
                if !refs.is_empty() {
                    x.refs[dp..dp + n].clone_from_slice(&refs);
                }
            }
            Ok(V::default())
        }),
        s!("identityHashCode", "(Ljava/lang/Object;)I", |vm, a| Ok(V::int(
            a[0].r().map(|o| identity_hash(&o)).unwrap_or(0)
        ))),
        s!("getProperty", "(Ljava/lang/String;)Ljava/lang/String;", |vm, a| {
            let k = sarg(vm, &a[0])?;
            Ok(match &*k {
                "os.name" => vm.str_value("Linux"),
                "java.vm.name" => vm.str_value("droid (Flare)"),
                "line.separator" => vm.str_value("\n"),
                "os.arch" => vm.str_value("aarch64"),
                _ => V::NULL,
            })
        }),
        s!("lineSeparator", "()Ljava/lang/String;", |vm, a| new_str(vm, "\n")),
        s!("exit", "(I)V", |vm, a| std::process::exit(a[0].i())),
        s!("gc", "()V", |vm, a| Ok(V::default())),
    ],
    statics: |vm| {
        let out = V::obj(vm.new_with("Ljava/io/PrintStream;", Ext::Boxed(V::int(1))).expect("PrintStream"));
        let err = V::obj(vm.new_with("Ljava/io/PrintStream;", Ext::Boxed(V::int(2))).expect("PrintStream"));
        vec![("out", out), ("err", err)]
    },
    ..BASE
};

fn arr_len(o: &Ref) -> usize {
    match &*o.ext.borrow() {
        Ext::Arr(a) => a.len(),
        _ => 0,
    }
}

fn emit(a: &[V], s: &str, newline: bool) -> R<V> {
    use std::io::Write;
    let nl = if newline { "\n" } else { "" };
    if unbox(&a[0]).i() == 2 {
        eprint!("{}{}", s, nl);
    } else {
        print!("{}{}", s, nl);
        let _ = std::io::stdout().flush();
    }
    Ok(V::default())
}

const PRINT_STREAM: NClass = NClass {
    methods: &[
        m!("println", "()V", |vm, a| emit(a, "", true)),
        m!("println", "(Ljava/lang/String;)V", |vm, a| {
            let s = jstr(vm, &a[1])?;
            emit(a, &s, true)
        }),
        m!("println", "(Ljava/lang/Object;)V", |vm, a| {
            let s = jstr(vm, &a[1])?;
            emit(a, &s, true)
        }),
        m!("println", "(I)V", |vm, a| emit(a, &a[1].i().to_string(), true)),
        m!("println", "(J)V", |vm, a| emit(a, &a[1].j().to_string(), true)),
        m!("println", "(Z)V", |vm, a| emit(a, &(a[1].i() != 0).to_string(), true)),
        m!("println", "(C)V", |vm, a| emit(a, &jchar(a[1].i()), true)),
        m!("println", "(D)V", |vm, a| emit(a, &jdouble(a[1].d()), true)),
        m!("println", "(F)V", |vm, a| emit(a, &jfloat(a[1].f()), true)),
        m!("print", "(Ljava/lang/String;)V", |vm, a| {
            let s = jstr(vm, &a[1])?;
            emit(a, &s, false)
        }),
        m!("print", "(Ljava/lang/Object;)V", |vm, a| {
            let s = jstr(vm, &a[1])?;
            emit(a, &s, false)
        }),
        m!("print", "(I)V", |vm, a| emit(a, &a[1].i().to_string(), false)),
        m!("print", "(J)V", |vm, a| emit(a, &a[1].j().to_string(), false)),
        m!("print", "(C)V", |vm, a| emit(a, &jchar(a[1].i()), false)),
        m!("print", "(D)V", |vm, a| emit(a, &jdouble(a[1].d()), false)),
        m!("printf", "(Ljava/lang/String;[Ljava/lang/Object;)Ljava/io/PrintStream;", |vm, a| {
            let f = sarg(vm, &a[1])?;
            let s = jformat(vm, &f, &varargs(&a[2]))?;
            emit(a, &s, false)?;
            Ok(a[0].clone())
        }),
        m!("flush", "()V", |vm, a| Ok(V::default())),
    ],
    ..BASE
};

const OBJECTS: NClass = NClass {
    methods: &[
        s!("requireNonNull", "(Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| match a[0].r() {
            Some(_) => Ok(a[0].clone()),
            None => Err(vm.exception(NPE, "requireNonNull")),
        }),
        s!("requireNonNull", "(Ljava/lang/Object;Ljava/lang/String;)Ljava/lang/Object;", |vm, a| match a[0].r() {
            Some(_) => Ok(a[0].clone()),
            None => {
                let m = jstr(vm, &a[1])?;
                Err(vm.exception(NPE, &m))
            }
        }),
        s!("equals", "(Ljava/lang/Object;Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(jequals(vm, &a[0], &a[1])?))),
        s!("hashCode", "(Ljava/lang/Object;)I", |vm, a| Ok(V::int(jhash(vm, &a[0])?))),
        s!("hash", "([Ljava/lang/Object;)I", |vm, a| {
            let mut h = 1i32;
            for v in varargs(&a[0]) {
                h = h.wrapping_mul(31).wrapping_add(jhash(vm, &v)?);
            }
            Ok(V::int(h))
        }),
        s!("toString", "(Ljava/lang/Object;)Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &a[0])?;
            new_str(vm, &s)
        }),
        s!("isNull", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(a[0].r().is_none()))),
        s!("nonNull", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(a[0].r().is_some()))),
    ],
    ..BASE
};

const THREAD: NClass = NClass {
    interfaces: &["Ljava/lang/Runnable;"],
    fields: &[("target", "Ljava/lang/Runnable;")],
    methods: &[
        m!("<init>", "()V", |vm, a| Ok(V::default())),
        m!("<init>", "(Ljava/lang/Runnable;)V", |vm, a| {
            set_field(&this(a), "target", a[1].clone());
            Ok(V::default())
        }),
        // Threads run to completion when started: the runtime is
        // single-threaded, like an app's main looper.
        m!("start", "()V", |vm, a| {
            let o = this(a);
            vm.call(&o, "run", "()V", &[])?;
            Ok(V::default())
        }),
        m!("run", "()V", |vm, a| {
            if let Some(t) = get_field(&this(a), "target").r() {
                vm.call(&t, "run", "()V", &[])?;
            }
            Ok(V::default())
        }),
        m!("join", "()V", |vm, a| Ok(V::default())),
        m!("getName", "()Ljava/lang/String;", |vm, a| new_str(vm, "main")),
        s!("currentThread", "()Ljava/lang/Thread;", |vm, a| Ok(V::obj(vm.new_with("Ljava/lang/Thread;", Ext::None)?))),
        s!("sleep", "(J)V", |vm, a| {
            std::thread::sleep(std::time::Duration::from_millis(a[0].j().max(0) as u64));
            Ok(V::default())
        }),
    ],
    ..BASE
};

const ENUM: NClass = NClass {
    interfaces: &["Ljava/lang/Comparable;", "Ljava/io/Serializable;"],
    fields: &[("name", "Ljava/lang/String;"), ("ordinal", "I")],
    methods: &[
        m!("<init>", "(Ljava/lang/String;I)V", |vm, a| {
            let o = this(a);
            set_field(&o, "name", a[1].clone());
            set_field(&o, "ordinal", a[2].clone());
            Ok(V::default())
        }),
        m!("name", "()Ljava/lang/String;", |vm, a| Ok(get_field(&this(a), "name"))),
        m!("toString", "()Ljava/lang/String;", |vm, a| Ok(get_field(&this(a), "name"))),
        m!("ordinal", "()I", |vm, a| Ok(get_field(&this(a), "ordinal"))),
        m!("compareTo", "(Ljava/lang/Enum;)I", |vm, a| {
            let o = a[1].r().ok_or_else(|| vm.exception(NPE, "compareTo"))?;
            Ok(V::int(get_field(&this(a), "ordinal").i() - get_field(&o, "ordinal").i()))
        }),
        m!("compareTo", "(Ljava/lang/Object;)I", |vm, a| {
            let o = a[1].r().ok_or_else(|| vm.exception(NPE, "compareTo"))?;
            Ok(V::int(get_field(&this(a), "ordinal").i() - get_field(&o, "ordinal").i()))
        }),
        m!("getDeclaringClass", "()Ljava/lang/Class;", |vm, a| {
            let n = this(a).class.name.clone();
            Ok(class_value(vm, &n))
        }),
        s!("valueOf", "(Ljava/lang/Class;Ljava/lang/String;)Ljava/lang/Enum;", |vm, a| {
            let c = a[0].r().and_then(|o| match &*o.ext.borrow() {
                Ext::Class(c) => Some(c.clone()),
                _ => None,
            });
            let c = c.ok_or_else(|| vm.exception(NPE, "Enum.valueOf"))?;
            let want = sarg(vm, &a[1])?;
            let values = vm.call_static(&c.name, "values", &format!("()[{}", c.name), Vec::new())?;
            for v in varargs(&values) {
                if let Some(o) = v.r() {
                    if get_field(&o, "name").r().and_then(|s| str_of(&s)).as_deref() == Some(&*want) {
                        return Ok(v);
                    }
                }
            }
            Err(vm.exception(
                "Ljava/lang/IllegalArgumentException;",
                &format!("No enum constant {}.{}", c.java_name(), want),
            ))
        }),
    ],
    ..BASE
};

fn new_multi(vm: &mut Vm, desc: &str, dims: &[i32]) -> R<Ref> {
    let n = dims[0];
    if n < 0 {
        return Err(vm.exception("Ljava/lang/NegativeArraySizeException;", &n.to_string()));
    }
    let arr = vm.new_array(desc, n as usize)?;
    if dims.len() > 1 {
        let mut subs = Vec::new();
        for _ in 0..n {
            subs.push(Some(new_multi(vm, &desc[1..], &dims[1..])?));
        }
        if let Ext::Arr(x) = &mut *arr.ext.borrow_mut() {
            x.refs = subs;
        }
    }
    Ok(arr)
}

fn component(vm: &mut Vm, v: &V) -> R<Rc<str>> {
    let c = v.r().and_then(|o| match &*o.ext.borrow() {
        Ext::Class(c) => Some(c.name.clone()),
        _ => None,
    });
    c.ok_or_else(|| vm.exception(NPE, "Array.newInstance"))
}

const REFLECT_ARRAY: NClass = NClass {
    methods: &[
        s!("newInstance", "(Ljava/lang/Class;[I)Ljava/lang/Object;", |vm, a| {
            let comp = component(vm, &a[0])?;
            let dims: Vec<i32> = match a[1].r() {
                Some(o) => match &*o.ext.borrow() {
                    Ext::Arr(x) => x.prim.iter().map(|&b| b as u32 as i32).collect(),
                    _ => Vec::new(),
                },
                None => Vec::new(),
            };
            if dims.is_empty() {
                return Err(vm.exception("Ljava/lang/IllegalArgumentException;", "no dimensions"));
            }
            let desc = format!("{}{}", "[".repeat(dims.len()), comp);
            Ok(V::obj(new_multi(vm, &desc, &dims)?))
        }),
        s!("newInstance", "(Ljava/lang/Class;I)Ljava/lang/Object;", |vm, a| {
            let comp = component(vm, &a[0])?;
            Ok(V::obj(new_multi(vm, &format!("[{}", comp), &[a[1].i()])?))
        }),
    ],
    ..BASE
};

// -------------------------------------------------------------------
// java.util
// -------------------------------------------------------------------

const ABSTRACT_COLLECTION: NClass = NClass {
    interfaces: &["Ljava/util/Collection;"],
    methods: &[
        m!("<init>", "()V", |vm, a| {
            with_list(&this(a), |_| ());
            Ok(V::default())
        }),
        m!("<init>", "(I)V", |vm, a| {
            with_list(&this(a), |_| ());
            Ok(V::default())
        }),
        m!("<init>", "(Ljava/util/Collection;)V", |vm, a| {
            let o = this(a);
            with_list(&o, |_| ());
            add_all(vm, &o, &a[1])?;
            Ok(V::default())
        }),
        m!("size", "()I", |vm, a| Ok(V::int(with_list(&this(a), |v| v.len()) as i32))),
        m!("isEmpty", "()Z", |vm, a| Ok(V::bool(with_list(&this(a), |v| v.is_empty())))),
        m!("clear", "()V", |vm, a| {
            with_list(&this(a), |v| v.clear());
            Ok(V::default())
        }),
        m!("contains", "(Ljava/lang/Object;)Z", |vm, a| {
            let items = list(&this(a));
            Ok(V::bool(index_of(vm, &items, &a[1])?.is_some()))
        }),
        m!("iterator", "()Ljava/util/Iterator;", |vm, a| iterator(vm, &this(a))),
        m!("addAll", "(Ljava/util/Collection;)Z", |vm, a| {
            add_all(vm, &this(a), &a[1])?;
            Ok(V::bool(true))
        }),
        m!("remove", "(Ljava/lang/Object;)Z", |vm, a| {
            let o = this(a);
            let items = list(&o);
            match index_of(vm, &items, &a[1])? {
                Some(i) => {
                    with_list(&o, |v| v.remove(i));
                    Ok(V::bool(true))
                }
                None => Ok(V::bool(false)),
            }
        }),
        m!("toArray", "()[Ljava/lang/Object;", |vm, a| {
            let items = list(&this(a));
            let arr = vm.new_array("[Ljava/lang/Object;", items.len())?;
            if let Ext::Arr(x) = &mut *arr.ext.borrow_mut() {
                x.refs = items.iter().map(|v| v.r()).collect();
            }
            Ok(V::obj(arr))
        }),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let items = list(&this(a));
            join(vm, &items, "[", "]")
        }),
        m!("hashCode", "()I", |vm, a| {
            let mut h = 1i32;
            for v in list(&this(a)) {
                h = h.wrapping_mul(31).wrapping_add(jhash(vm, &v)?);
            }
            Ok(V::int(h))
        }),
        m!("forEach", "(Ljava/util/function/Consumer;)V", |vm, a| {
            let f = a[1].r().ok_or_else(|| vm.exception(NPE, "forEach"))?;
            for v in list(&this(a)) {
                vm.call(&f, "accept", "(Ljava/lang/Object;)V", &[v])?;
            }
            Ok(V::default())
        }),
    ],
    ..BASE
};

const ARRAY_LIST: NClass = NClass {
    super_: Some("Ljava/util/AbstractCollection;"),
    interfaces: &["Ljava/util/List;", "Ljava/util/RandomAccess;"],
    methods: &[
        m!("add", "(Ljava/lang/Object;)Z", |vm, a| {
            with_list(&this(a), |v| v.push(a[1].clone()));
            Ok(V::bool(true))
        }),
        m!("add", "(ILjava/lang/Object;)V", |vm, a| {
            let o = this(a);
            let i = check_index(vm, &o, a[1].i(), 1)?;
            with_list(&o, |v| v.insert(i, a[2].clone()));
            Ok(V::default())
        }),
        m!("get", "(I)Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            let i = check_index(vm, &o, a[1].i(), 0)?;
            Ok(with_list(&o, |v| v[i].clone()))
        }),
        m!("set", "(ILjava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            let i = check_index(vm, &o, a[1].i(), 0)?;
            Ok(with_list(&o, |v| core::mem::replace(&mut v[i], a[2].clone())))
        }),
        m!("remove", "(I)Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            let i = check_index(vm, &o, a[1].i(), 0)?;
            Ok(with_list(&o, |v| v.remove(i)))
        }),
        m!("indexOf", "(Ljava/lang/Object;)I", |vm, a| {
            let items = list(&this(a));
            Ok(V::int(index_of(vm, &items, &a[1])?.map(|i| i as i32).unwrap_or(-1)))
        }),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| {
            let other = match a[1].r() {
                Some(o) if vm.is_subclass(&o.class, "Ljava/util/List;") => o,
                _ => return Ok(V::bool(false)),
            };
            let (x, y) = (list(&this(a)), elements(vm, &other)?);
            if x.len() != y.len() {
                return Ok(V::bool(false));
            }
            for (p, q) in x.iter().zip(y.iter()) {
                if !jequals(vm, p, q)? {
                    return Ok(V::bool(false));
                }
            }
            Ok(V::bool(true))
        }),
        m!("subList", "(II)Ljava/util/List;", |vm, a| {
            let items = list(&this(a));
            let (b, e) = (a[1].i().max(0) as usize, (a[2].i().max(0) as usize).min(items.len()));
            collection(vm, "Ljava/util/ArrayList;", items[b.min(e)..e].to_vec())
        }),
        m!("sort", "(Ljava/util/Comparator;)V", |vm, a| {
            let o = this(a);
            let mut items = list(&o);
            sort(vm, &mut items, a[1].r())?;
            with_list(&o, |v| *v = items);
            Ok(V::default())
        }),
    ],
    ..BASE
};

/// Insertion sort with Java comparison semantics (natural order or a
/// Comparator); stable, like Java's.
fn sort(vm: &mut Vm, items: &mut [V], cmp: Option<Ref>) -> R<()> {
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 {
            let c = compare(vm, &items[j - 1], &items[j], &cmp)?;
            if c <= 0 {
                break;
            }
            items.swap(j - 1, j);
            j -= 1;
        }
    }
    Ok(())
}

fn compare(vm: &mut Vm, x: &V, y: &V, cmp: &Option<Ref>) -> R<i32> {
    if let Some(c) = cmp {
        return Ok(vm.call(c, "compare", "(Ljava/lang/Object;Ljava/lang/Object;)I", &[x.clone(), y.clone()])?.i());
    }
    let o = x.r().ok_or_else(|| vm.exception(NPE, "compare"))?;
    if let (Some(a), Some(b)) = (str_of(&o), y.r().and_then(|r| str_of(&r))) {
        return Ok(a.cmp(&b) as i32);
    }
    if matches!(&*o.ext.borrow(), Ext::Boxed(_)) {
        let (a, b) = (num(x), num(y));
        return Ok(if a < b {
            -1
        } else if a > b {
            1
        } else {
            0
        });
    }
    Ok(vm.call(&o, "compareTo", "(Ljava/lang/Object;)I", std::slice::from_ref(y))?.i())
}

const HASH_SET: NClass = NClass {
    super_: Some("Ljava/util/AbstractCollection;"),
    interfaces: &["Ljava/util/Set;"],
    methods: &[m!("add", "(Ljava/lang/Object;)Z", |vm, a| {
        let o = this(a);
        let items = list(&o);
        if index_of(vm, &items, &a[1])?.is_some() {
            return Ok(V::bool(false));
        }
        with_list(&o, |v| v.push(a[1].clone()));
        Ok(V::bool(true))
    })],
    ..BASE
};

const ITERATOR: NClass = NClass {
    interfaces: &["Ljava/util/Iterator;"],
    methods: &[
        m!("hasNext", "()Z", |vm, a| {
            let o = this(a);
            let r = match &*o.ext.borrow() {
                Ext::Iter(c, i) => *i < arr_or_list_len(c),
                _ => false,
            };
            Ok(V::bool(r))
        }),
        m!("next", "()Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            let item = match &mut *o.ext.borrow_mut() {
                Ext::Iter(c, i) => {
                    let v = match &*c.ext.borrow() {
                        Ext::List(v) => v.get(*i).cloned(),
                        _ => None,
                    };
                    *i += 1;
                    v
                }
                _ => None,
            };
            item.ok_or_else(|| vm.exception("Ljava/util/NoSuchElementException;", ""))
        }),
        m!("remove", "()V", |vm, a| {
            let o = this(a);
            if let Ext::Iter(c, i) = &mut *o.ext.borrow_mut() {
                if *i > 0 {
                    *i -= 1;
                    let at = *i;
                    with_list(c, |v| {
                        if at < v.len() {
                            v.remove(at);
                        }
                    });
                }
            }
            Ok(V::default())
        }),
    ],
    ..BASE
};

fn arr_or_list_len(c: &Ref) -> usize {
    match &*c.ext.borrow() {
        Ext::List(v) => v.len(),
        _ => 0,
    }
}

const HASH_MAP: NClass = NClass {
    interfaces: &["Ljava/util/Map;"],
    methods: &[
        m!("<init>", "()V", |vm, a| {
            with_map(&this(a), |_| ());
            Ok(V::default())
        }),
        m!("<init>", "(I)V", |vm, a| {
            with_map(&this(a), |_| ());
            Ok(V::default())
        }),
        m!("<init>", "(Ljava/util/Map;)V", |vm, a| {
            let o = this(a);
            let src = a[1].r().map(|s| map_entries(&s)).unwrap_or_default();
            with_map(&o, |m| *m = src);
            Ok(V::default())
        }),
        m!("put", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            map_put(vm, &this(a), a[1].clone(), a[2].clone())
        }),
        m!("get", "(Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| Ok(
            map_get(vm, &this(a), &a[1])?.unwrap_or(V::NULL)
        )),
        m!("getOrDefault", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            Ok(map_get(vm, &this(a), &a[1])?.unwrap_or_else(|| a[2].clone()))
        }),
        m!("putIfAbsent", "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            match map_get(vm, &o, &a[1])? {
                Some(v) if v.r().is_some() => Ok(v),
                _ => map_put(vm, &o, a[1].clone(), a[2].clone()),
            }
        }),
        m!("containsKey", "(Ljava/lang/Object;)Z", |vm, a| Ok(V::bool(map_find(vm, &this(a), &a[1])?.is_some()))),
        m!("containsValue", "(Ljava/lang/Object;)Z", |vm, a| {
            let vals: Vec<V> = map_entries(&this(a)).into_iter().map(|(_, v)| v).collect();
            Ok(V::bool(index_of(vm, &vals, &a[1])?.is_some()))
        }),
        m!("remove", "(Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            let o = this(a);
            match map_find(vm, &o, &a[1])? {
                Some(i) => Ok(with_map(&o, |m| m.remove(i).1)),
                None => Ok(V::NULL),
            }
        }),
        m!("size", "()I", |vm, a| Ok(V::int(with_map(&this(a), |m| m.len()) as i32))),
        m!("isEmpty", "()Z", |vm, a| Ok(V::bool(with_map(&this(a), |m| m.is_empty())))),
        m!("clear", "()V", |vm, a| {
            with_map(&this(a), |m| m.clear());
            Ok(V::default())
        }),
        m!("keySet", "()Ljava/util/Set;", |vm, a| {
            let keys = map_entries(&this(a)).into_iter().map(|(k, _)| k).collect();
            collection(vm, "Ljava/util/HashSet;", keys)
        }),
        m!("values", "()Ljava/util/Collection;", |vm, a| {
            let vals = map_entries(&this(a)).into_iter().map(|(_, v)| v).collect();
            collection(vm, "Ljava/util/ArrayList;", vals)
        }),
        m!("entrySet", "()Ljava/util/Set;", |vm, a| {
            let mut entries = Vec::new();
            let map = this(a);
            for (k, v) in map_entries(&map) {
                // Entries write through to the map (setValue).
                let node = Ext::List(vec![k, v, V::obj(map.clone())]);
                entries.push(V::obj(vm.new_with("Ljava/util/HashMap$Node;", node)?));
            }
            collection(vm, "Ljava/util/HashSet;", entries)
        }),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let mut s = String::from("{");
            for (n, (k, v)) in map_entries(&this(a)).iter().enumerate() {
                if n > 0 {
                    s.push_str(", ");
                }
                s.push_str(&jstr(vm, k)?);
                s.push('=');
                s.push_str(&jstr(vm, v)?);
            }
            s.push('}');
            new_str(vm, &s)
        }),
    ],
    ..BASE
};

const MAP_NODE: NClass = NClass {
    interfaces: &["Ljava/util/Map$Entry;"],
    methods: &[
        m!("getKey", "()Ljava/lang/Object;", |vm, a| Ok(list(&this(a)).first().cloned().unwrap_or(V::NULL))),
        m!("getValue", "()Ljava/lang/Object;", |vm, a| Ok(list(&this(a)).get(1).cloned().unwrap_or(V::NULL))),
        m!("setValue", "(Ljava/lang/Object;)Ljava/lang/Object;", |vm, a| {
            let node = this(a);
            let l = list(&node);
            if let Some(map) = l.get(2).and_then(|m| m.r()) {
                map_put(vm, &map, l[0].clone(), a[1].clone())?;
            }
            Ok(with_list(&node, |v| core::mem::replace(&mut v[1], a[1].clone())))
        }),
        m!("equals", "(Ljava/lang/Object;)Z", |vm, a| {
            let (x, y) = (list(&this(a)), a[1].r().map(|o| list(&o)).unwrap_or_default());
            Ok(V::bool(y.len() >= 2 && jequals(vm, &x[0], &y[0])? && jequals(vm, &x[1], &y[1])?))
        }),
        m!("toString", "()Ljava/lang/String;", |vm, a| {
            let l = list(&this(a));
            let s = format!("{}={}", jstr(vm, &l[0])?, jstr(vm, &l[1])?);
            new_str(vm, &s)
        }),
    ],
    ..BASE
};

/// `Arrays.copyOf` / `copyOfRange` for any array type.
fn copy_range(vm: &mut Vm, v: &V, from: i32, to: i32) -> R<V> {
    let o = v.r().ok_or_else(|| vm.exception(NPE, "copyOf"))?;
    let len = arr_len(&o);
    if from < 0 || from as usize > len || to < from {
        return Err(vm.exception("Ljava/lang/ArrayIndexOutOfBoundsException;", &format!("{}", from)));
    }
    let (from, to) = (from as usize, to as usize);
    let n = to - from;
    let out = vm.new_array(&o.class.name, n)?;
    if let (Ext::Arr(src), Ext::Arr(dst)) = (&*o.ext.borrow(), &mut *out.ext.borrow_mut()) {
        let end = to.min(len);
        if !src.prim.is_empty() || dst.refs.is_empty() {
            dst.prim[..end - from].copy_from_slice(&src.prim[from..end]);
        } else {
            dst.refs[..end - from].clone_from_slice(&src.refs[from..end]);
        }
    }
    Ok(V::obj(out))
}

const ARRAYS: NClass = NClass {
    methods: &[
        s!("copyOf", "([Ljava/lang/Object;I)[Ljava/lang/Object;", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOf", "([II)[I", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOf", "([JI)[J", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOf", "([CI)[C", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOf", "([BI)[B", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOf", "([DI)[D", |vm, a| copy_range(vm, &a[0], 0, a[1].i())),
        s!("copyOfRange", "([Ljava/lang/Object;II)[Ljava/lang/Object;", |vm, a| copy_range(
            vm,
            &a[0],
            a[1].i(),
            a[2].i()
        )),
        s!("copyOfRange", "([III)[I", |vm, a| copy_range(vm, &a[0], a[1].i(), a[2].i())),
        s!("copyOfRange", "([BII)[B", |vm, a| copy_range(vm, &a[0], a[1].i(), a[2].i())),
        s!("copyOfRange", "([CII)[C", |vm, a| copy_range(vm, &a[0], a[1].i(), a[2].i())),
        s!("asList", "([Ljava/lang/Object;)Ljava/util/List;", |vm, a| {
            let items = varargs(&a[0]);
            collection(vm, "Ljava/util/ArrayList;", items)
        }),
        s!("toString", "([I)Ljava/lang/String;", |vm, a| prim_array_string(vm, &a[0], |b| (b as u32 as i32)
            .to_string())),
        s!("toString", "([J)Ljava/lang/String;", |vm, a| prim_array_string(vm, &a[0], |b| (b as i64).to_string())),
        s!("toString", "([D)Ljava/lang/String;", |vm, a| prim_array_string(vm, &a[0], |b| jdouble(f64::from_bits(b)))),
        s!("toString", "([Ljava/lang/Object;)Ljava/lang/String;", |vm, a| {
            if a[0].r().is_none() {
                return new_str(vm, "null");
            }
            let items = varargs(&a[0]);
            join(vm, &items, "[", "]")
        }),
        s!("sort", "([I)V", |vm, a| {
            if let Some(o) = a[0].r() {
                if let Ext::Arr(x) = &mut *o.ext.borrow_mut() {
                    x.prim.sort_by_key(|&b| b as u32 as i32);
                }
            }
            Ok(V::default())
        }),
        s!("sort", "([Ljava/lang/Object;)V", |vm, a| sort_array(vm, &a[0], None)),
        s!("sort", "([Ljava/lang/Object;Ljava/util/Comparator;)V", |vm, a| sort_array(vm, &a[0], a[1].r())),
        s!("fill", "([II)V", |vm, a| {
            if let Some(o) = a[0].r() {
                if let Ext::Arr(x) = &mut *o.ext.borrow_mut() {
                    x.prim.iter_mut().for_each(|p| *p = a[1].i() as u32 as u64);
                }
            }
            Ok(V::default())
        }),
    ],
    ..BASE
};

fn sort_array(vm: &mut Vm, arr: &V, cmp: Option<Ref>) -> R<V> {
    let o = arr.r().ok_or_else(|| vm.exception(NPE, "sort"))?;
    let mut items = varargs(arr);
    sort(vm, &mut items, cmp)?;
    if let Ext::Arr(x) = &mut *o.ext.borrow_mut() {
        x.refs = items.iter().map(|v| v.r()).collect();
    }
    Ok(V::default())
}

fn prim_array_string(vm: &mut Vm, v: &V, f: fn(u64) -> String) -> R<V> {
    let s = match v.r() {
        Some(o) => match &*o.ext.borrow() {
            Ext::Arr(x) => format!("[{}]", x.prim.iter().map(|&b| f(b)).collect::<Vec<_>>().join(", ")),
            _ => "[]".into(),
        },
        None => "null".into(),
    };
    new_str(vm, &s)
}

const COLLECTIONS: NClass = NClass {
    methods: &[
        s!("sort", "(Ljava/util/List;)V", |vm, a| sort_list(vm, &a[0], None)),
        s!("sort", "(Ljava/util/List;Ljava/util/Comparator;)V", |vm, a| sort_list(vm, &a[0], a[1].r())),
        s!("reverse", "(Ljava/util/List;)V", |vm, a| {
            if let Some(o) = a[0].r() {
                with_list(&o, |v| v.reverse());
            }
            Ok(V::default())
        }),
        s!("emptyList", "()Ljava/util/List;", |vm, a| collection(vm, "Ljava/util/ArrayList;", Vec::new())),
        s!("unmodifiableList", "(Ljava/util/List;)Ljava/util/List;", |vm, a| Ok(a[0].clone())),
        s!("singletonList", "(Ljava/lang/Object;)Ljava/util/List;", |vm, a| {
            collection(vm, "Ljava/util/ArrayList;", vec![a[0].clone()])
        }),
    ],
    ..BASE
};

fn sort_list(vm: &mut Vm, l: &V, cmp: Option<Ref>) -> R<V> {
    let o = l.r().ok_or_else(|| vm.exception(NPE, "sort"))?;
    let mut items = list(&o);
    sort(vm, &mut items, cmp)?;
    with_list(&o, |v| *v = items);
    Ok(V::default())
}

// -------------------------------------------------------------------
// android.*
// -------------------------------------------------------------------

fn log(vm: &mut Vm, level: char, a: &[V]) -> R<V> {
    let tag = jstr(vm, &a[0])?;
    let msg = jstr(vm, &a[1])?;
    println!("{}/{}: {}", level, tag, msg);
    if let Some(t) = a.get(2) {
        if t.r().is_some() {
            println!("{}/{}: {}", level, tag, jstr(vm, t)?);
        }
    }
    Ok(V::int(0))
}

/// Locales: the runtime formats everything the way the root locale does.
const LOCALE: NClass = NClass {
    methods: &[
        s!("getDefault", "()Ljava/util/Locale;", |vm, a| locale(vm, "en_US")),
        m!("toString", "()Ljava/lang/String;", |vm, a| Ok(unbox(&a[0]))),
        m!("getLanguage", "()Ljava/lang/String;", |vm, a| {
            let s = jstr(vm, &unbox(&a[0]))?;
            new_str(vm, s.split('_').next().unwrap_or(""))
        }),
    ],
    statics: |vm| {
        let mut v = Vec::new();
        for (n, tag) in [("ROOT", ""), ("US", "en_US"), ("ENGLISH", "en"), ("UK", "en_GB")] {
            v.push((n, locale(vm, tag).unwrap_or(V::NULL)));
        }
        v
    },
    ..BASE
};

fn locale(vm: &mut Vm, tag: &str) -> R<V> {
    let s = vm.str_value(tag);
    Ok(V::obj(vm.new_with("Ljava/util/Locale;", Ext::Boxed(s))?))
}

const LOG: NClass = NClass {
    methods: &[
        s!("v", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'V', a)),
        s!("d", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'D', a)),
        s!("i", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'I', a)),
        s!("w", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'W', a)),
        s!("e", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'E', a)),
        s!("wtf", "(Ljava/lang/String;Ljava/lang/String;)I", |vm, a| log(vm, 'F', a)),
        s!("v", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I", |vm, a| log(vm, 'V', a)),
        s!("d", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I", |vm, a| log(vm, 'D', a)),
        s!("i", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I", |vm, a| log(vm, 'I', a)),
        s!("w", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I", |vm, a| log(vm, 'W', a)),
        s!("e", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I", |vm, a| log(vm, 'E', a)),
        s!("isLoggable", "(Ljava/lang/String;I)Z", |vm, a| Ok(V::bool(true))),
    ],
    ..BASE
};

const BUILD: NClass = NClass {
    statics: |vm| {
        vec![
            ("MANUFACTURER", vm.str_value("Flare")),
            ("BRAND", vm.str_value("flare")),
            ("MODEL", vm.str_value("virt-phone")),
            ("DEVICE", vm.str_value("qemu-virt")),
            ("PRODUCT", vm.str_value("flare_virt")),
            ("HARDWARE", vm.str_value("qemu")),
            ("BOARD", vm.str_value("virt")),
            ("FINGERPRINT", vm.str_value("flare/virt/qemu:15/FLARE/0:user/release-keys")),
            ("SUPPORTED_ABIS", V::NULL),
        ]
    },
    ..BASE
};

/// The API level apps see.
pub const SDK_INT: i32 = 35;

const BUILD_VERSION: NClass = NClass {
    statics: |vm| {
        vec![
            ("SDK_INT", V::int(SDK_INT)),
            ("RELEASE", vm.str_value("15")),
            ("CODENAME", vm.str_value("REL")),
            ("INCREMENTAL", vm.str_value("flare")),
        ]
    },
    ..BASE
};

fn bundle_put(vm: &mut Vm, a: &[V]) -> R<V> {
    map_put(vm, &this(a), a[1].clone(), a[2].clone())?;
    Ok(V::default())
}

fn bundle_get(vm: &mut Vm, a: &[V]) -> R<V> {
    Ok(map_get(vm, &this(a), &a[1])?.or_else(|| a.get(2).cloned()).unwrap_or_default())
}

const BUNDLE: NClass = NClass {
    methods: &[
        m!("<init>", "()V", |vm, a| {
            with_map(&this(a), |_| ());
            Ok(V::default())
        }),
        m!("putString", "(Ljava/lang/String;Ljava/lang/String;)V", |vm, a| bundle_put(vm, a)),
        m!("putInt", "(Ljava/lang/String;I)V", |vm, a| bundle_put(vm, a)),
        m!("putLong", "(Ljava/lang/String;J)V", |vm, a| bundle_put(vm, a)),
        m!("putBoolean", "(Ljava/lang/String;Z)V", |vm, a| bundle_put(vm, a)),
        m!("getString", "(Ljava/lang/String;)Ljava/lang/String;", |vm, a| {
            Ok(map_get(vm, &this(a), &a[1])?.unwrap_or(V::NULL))
        }),
        m!("getString", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;", |vm, a| bundle_get(vm, a)),
        m!("getInt", "(Ljava/lang/String;)I", |vm, a| bundle_get(vm, a)),
        m!("getInt", "(Ljava/lang/String;I)I", |vm, a| bundle_get(vm, a)),
        m!("getLong", "(Ljava/lang/String;)J", |vm, a| bundle_get(vm, a)),
        m!("getBoolean", "(Ljava/lang/String;)Z", |vm, a| bundle_get(vm, a)),
        m!("getBoolean", "(Ljava/lang/String;Z)Z", |vm, a| bundle_get(vm, a)),
        m!("containsKey", "(Ljava/lang/String;)Z", |vm, a| Ok(V::bool(map_find(vm, &this(a), &a[1])?.is_some()))),
        m!("size", "()I", |vm, a| Ok(V::int(with_map(&this(a), |m| m.len()) as i32))),
        m!("isEmpty", "()Z", |vm, a| Ok(V::bool(with_map(&this(a), |m| m.is_empty())))),
    ],
    ..BASE
};

const CONTEXT: NClass = NClass {
    methods: &[
        m!("<init>", "()V", |vm, a| Ok(V::default())),
        m!("getPackageName", "()Ljava/lang/String;", |vm, a| {
            let p = vm.package.clone();
            new_str(vm, &p)
        }),
        m!("getApplicationContext", "()Landroid/content/Context;", |vm, a| Ok(a[0].clone())),
        m!("getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", |vm, a| {
            let s = jstr(vm, &a[1])?;
            vm.note_missing(&format!("system service '{}'", s));
            Ok(V::NULL)
        }),
    ],
    ..BASE
};

const ACTIVITY: NClass = NClass {
    super_: Some("Landroid/view/ContextThemeWrapper;"),
    fields: &[("mFinished", "Z"), ("mTitle", "Ljava/lang/CharSequence;")],
    methods: &[
        m!("<init>", "()V", |vm, a| Ok(V::default())),
        m!("onCreate", "(Landroid/os/Bundle;)V", |vm, a| Ok(V::default())),
        m!("onPostCreate", "(Landroid/os/Bundle;)V", |vm, a| Ok(V::default())),
        m!("onStart", "()V", |vm, a| Ok(V::default())),
        m!("onRestart", "()V", |vm, a| Ok(V::default())),
        m!("onResume", "()V", |vm, a| Ok(V::default())),
        m!("onPostResume", "()V", |vm, a| Ok(V::default())),
        m!("onPause", "()V", |vm, a| Ok(V::default())),
        m!("onStop", "()V", |vm, a| Ok(V::default())),
        m!("onDestroy", "()V", |vm, a| Ok(V::default())),
        m!("onSaveInstanceState", "(Landroid/os/Bundle;)V", |vm, a| Ok(V::default())),
        m!("onBackPressed", "()V", |vm, a| {
            set_field(&this(a), "mFinished", V::bool(true));
            Ok(V::default())
        }),
        m!("finish", "()V", |vm, a| {
            set_field(&this(a), "mFinished", V::bool(true));
            Ok(V::default())
        }),
        m!("isFinishing", "()Z", |vm, a| Ok(get_field(&this(a), "mFinished"))),
        m!("setTitle", "(Ljava/lang/CharSequence;)V", |vm, a| {
            set_field(&this(a), "mTitle", a[1].clone());
            Ok(V::default())
        }),
        m!("getTitle", "()Ljava/lang/CharSequence;", |vm, a| Ok(get_field(&this(a), "mTitle"))),
        m!("setContentView", "(I)V", |vm, a| {
            vm.note_missing("layouts (setContentView with a resource id): no display yet");
            Ok(V::default())
        }),
        m!("setContentView", "(Landroid/view/View;)V", |vm, a| {
            vm.note_missing("views (setContentView): no display yet");
            Ok(V::default())
        }),
        m!("findViewById", "(I)Landroid/view/View;", |vm, a| Ok(V::NULL)),
        m!("getIntent", "()Landroid/content/Intent;", |vm, a| Ok(V::NULL)),
        m!("runOnUiThread", "(Ljava/lang/Runnable;)V", |vm, a| {
            let r = a[1].r().ok_or_else(|| vm.exception(NPE, "runOnUiThread"))?;
            vm.call(&r, "run", "()V", &[])?;
            Ok(V::default())
        }),
    ],
    ..BASE
};

const SYSTEM_CLOCK: NClass = NClass {
    methods: &[
        s!("uptimeMillis", "()J", |vm, a| Ok(V::long((monotonic_nanos() / 1_000_000) as i64))),
        s!("elapsedRealtime", "()J", |vm, a| Ok(V::long((monotonic_nanos() / 1_000_000) as i64))),
        s!("elapsedRealtimeNanos", "()J", |vm, a| Ok(V::long(monotonic_nanos() as i64))),
        s!("sleep", "(J)V", |vm, a| {
            std::thread::sleep(std::time::Duration::from_millis(a[0].j().max(0) as u64));
            Ok(V::default())
        }),
    ],
    ..BASE
};

/// Look up a native class by descriptor.
pub fn lookup(name: &str) -> Option<NClass> {
    if let Some((_, sup)) = EXCEPTIONS.iter().find(|(n, _)| *n == name) {
        return Some(NClass { super_: Some(sup), ..BASE });
    }
    Some(match name {
        "Ljava/lang/Object;" => OBJECT,
        "Ljava/lang/Class;" => CLASS,
        "Ljava/lang/String;" => STRING,
        "Ljava/lang/StringBuilder;" => STRING_BUILDER,
        "Ljava/lang/StringBuffer;" => STRING_BUFFER,
        "Ljava/lang/Throwable;" => THROWABLE,
        "Ljava/lang/Number;" => BASE,
        "Ljava/lang/Integer$Base;" => INTEGER,
        "Ljava/lang/Integer;" => INTEGER_STATICS,
        "Ljava/lang/Long$Base;" => LONG,
        "Ljava/lang/Long;" => LONG_STATICS,
        "Ljava/lang/Double$Base;" => DOUBLE,
        "Ljava/lang/Double;" => DOUBLE_STATICS,
        "Ljava/lang/Float;" => FLOAT,
        "Ljava/lang/Short;" => SHORT,
        "Ljava/lang/Byte;" => BYTE,
        "Ljava/lang/Boolean;" => BOOLEAN,
        "Ljava/lang/Character;" => CHARACTER,
        "Ljava/lang/Math;" | "Ljava/lang/StrictMath;" => MATH,
        "Ljava/lang/System;" => SYSTEM,
        "Ljava/lang/Thread;" => THREAD,
        "Ljava/lang/Enum;" => ENUM,
        "Ljava/lang/reflect/Array;" => REFLECT_ARRAY,
        "Ljava/io/PrintStream;" => PRINT_STREAM,
        "Ljava/util/Objects;" => OBJECTS,
        "Ljava/lang/Runnable;"
        | "Ljava/lang/CharSequence;"
        | "Ljava/lang/Comparable;"
        | "Ljava/lang/Iterable;"
        | "Ljava/lang/AutoCloseable;"
        | "Ljava/lang/Appendable;"
        | "Ljava/io/Closeable;"
        | "Ljava/io/Serializable;"
        | "Ljava/lang/Cloneable;"
        | "Ljava/util/Iterator;"
        | "Ljava/util/Comparator;"
        | "Ljava/util/RandomAccess;"
        | "Ljava/util/Map;"
        | "Ljava/util/Map$Entry;"
        | "Ljava/util/function/Consumer;"
        | "Ljava/util/function/Function;"
        | "Ljava/util/function/Supplier;"
        | "Ljava/util/function/Predicate;"
        | "Ljava/util/function/BiFunction;"
        | "Ljava/util/concurrent/Callable;"
        | "Landroid/view/View$OnClickListener;" => IFACE,
        "Ljava/util/Collection;" => NClass { interfaces: &["Ljava/lang/Iterable;"], ..IFACE },
        "Ljava/util/List;" | "Ljava/util/Set;" | "Ljava/util/Queue;" => {
            NClass { interfaces: &["Ljava/util/Collection;"], ..IFACE }
        }
        "Ljava/util/AbstractCollection;" => ABSTRACT_COLLECTION,
        "Ljava/util/ArrayList;" => ARRAY_LIST,
        "Ljava/util/LinkedList;" => NClass { super_: Some("Ljava/util/ArrayList;"), ..BASE },
        "Ljava/util/HashSet;" => HASH_SET,
        "Ljava/util/LinkedHashSet;" => NClass { super_: Some("Ljava/util/HashSet;"), ..BASE },
        "Ljava/util/Iterator$Impl;" => ITERATOR,
        "Ljava/util/HashMap;" => HASH_MAP,
        "Ljava/util/LinkedHashMap;" => NClass { super_: Some("Ljava/util/HashMap;"), ..BASE },
        "Ljava/util/HashMap$Node;" => MAP_NODE,
        "Ljava/util/Arrays;" => ARRAYS,
        "Ljava/util/Locale;" => LOCALE,
        "Ljava/util/Collections;" => COLLECTIONS,
        "Ljava/lang/StackTraceElement;" => BASE,
        "Landroid/util/Log;" => LOG,
        "Landroid/os/Build;" => BUILD,
        "Landroid/os/Build$VERSION;" => BUILD_VERSION,
        "Landroid/os/Bundle;" => BUNDLE,
        "Landroid/os/SystemClock;" => SYSTEM_CLOCK,
        "Landroid/content/Context;" => CONTEXT,
        "Landroid/content/ContextWrapper;" => NClass { super_: Some("Landroid/content/Context;"), ..BASE },
        "Landroid/view/ContextThemeWrapper;" => NClass { super_: Some("Landroid/content/ContextWrapper;"), ..BASE },
        "Landroid/app/Activity;" => ACTIVITY,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_number_formatting() {
        assert_eq!(jdouble(3.5), "3.5");
        assert_eq!(jdouble(2.0), "2.0");
        assert_eq!(jdouble(1e10), "1.0E10");
        assert_eq!(jdouble(0.0001), "1.0E-4");
        assert_eq!(jfloat(0.1), "0.1");
        assert_eq!(string_hash("hello"), 99162322);
    }
}
