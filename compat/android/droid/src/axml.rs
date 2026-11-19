//! Android's binary XML (compiled AndroidManifest.xml).

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub depth: usize,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn string_pool(b: &[u8], at: usize) -> Vec<String> {
    let count = u32_at(b, at + 8) as usize;
    let flags = u32_at(b, at + 16);
    let strings_start = at + u32_at(b, at + 20) as usize;
    let utf8 = flags & (1 << 8) != 0;
    (0..count)
        .map(|i| {
            let mut p = strings_start + u32_at(b, at + 28 + i * 4) as usize;
            if utf8 {
                // utf16 length, then utf8 length, each 1 or 2 bytes
                p += if b[p] & 0x80 != 0 { 2 } else { 1 };
                let mut len = b[p] as usize;
                if len & 0x80 != 0 {
                    len = ((len & 0x7f) << 8) | b[p + 1] as usize;
                    p += 2;
                } else {
                    p += 1;
                }
                String::from_utf8_lossy(&b[p..p + len]).into_owned()
            } else {
                let mut len = u16_at(b, p) as usize;
                p += 2;
                if len & 0x8000 != 0 {
                    len = ((len & 0x7fff) << 16) | u16_at(b, p) as usize;
                    p += 2;
                }
                let units: Vec<u16> = (0..len).map(|k| u16_at(b, p + k * 2)).collect();
                String::from_utf16_lossy(&units)
            }
        })
        .collect()
}

/// All start elements, in document order, with their depth.
pub fn parse(b: &[u8]) -> Result<Vec<Element>, String> {
    if b.len() < 8 || u16_at(b, 0) != 0x0003 {
        return Err("not a binary XML document".into());
    }
    let mut strings = Vec::new();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut p = u16_at(b, 2) as usize;
    while p + 8 <= b.len() {
        let ty = u16_at(b, p);
        let header = u16_at(b, p + 2) as usize;
        let size = u32_at(b, p + 4) as usize;
        if size == 0 {
            break;
        }
        match ty {
            0x0001 => strings = string_pool(b, p),
            0x0102 => {
                // start element: ext at p + header
                let e = p + header;
                let name = strings.get(u32_at(b, e + 4) as usize).cloned().unwrap_or_default();
                let attr_start = u16_at(b, e + 8) as usize;
                let attr_size = u16_at(b, e + 10) as usize;
                let attr_count = u16_at(b, e + 12) as usize;
                let mut attrs = Vec::new();
                for i in 0..attr_count {
                    let a = e + attr_start + i * attr_size;
                    let an = strings.get(u32_at(b, a + 4) as usize).cloned().unwrap_or_default();
                    let raw = u32_at(b, a + 8);
                    let dtype = b[a + 15];
                    let data = u32_at(b, a + 16);
                    let value = if raw != u32::MAX {
                        strings.get(raw as usize).cloned().unwrap_or_default()
                    } else if dtype == 0x03 {
                        strings.get(data as usize).cloned().unwrap_or_default()
                    } else if dtype == 0x12 {
                        (data != 0).to_string()
                    } else if dtype == 0x01 {
                        format!("@0x{:08x}", data)
                    } else {
                        (data as i32).to_string()
                    };
                    attrs.push((an, value));
                }
                out.push(Element { name, attrs, depth });
                depth += 1;
            }
            0x0103 => depth = depth.saturating_sub(1),
            _ => {}
        }
        p += size;
    }
    Ok(out)
}

/// The app's package and its launcher activity (fully qualified).
pub fn launcher(elements: &[Element]) -> Option<(String, String)> {
    let package = elements.iter().find(|e| e.name == "manifest")?.attr("package")?.to_string();
    let mut current: Option<(String, usize)> = None;
    let (mut main, mut launcher) = (false, false);
    for e in elements {
        if let Some((_, d)) = &current {
            if e.depth <= *d {
                current = None;
            }
        }
        match e.name.as_str() {
            "activity" | "activity-alias" => {
                current = e.attr("name").or_else(|| e.attr("targetActivity")).map(|n| (n.to_string(), e.depth));
                main = false;
                launcher = false;
            }
            "action" if e.attr("name") == Some("android.intent.action.MAIN") => main = true,
            "category" if e.attr("name") == Some("android.intent.category.LAUNCHER") => launcher = true,
            _ => {}
        }
        if main && launcher {
            if let Some((name, _)) = &current {
                let full = if name.starts_with('.') {
                    format!("{}{}", package, name)
                } else if !name.contains('.') {
                    format!("{}.{}", package, name)
                } else {
                    name.clone()
                };
                return Some((package, full));
            }
        }
    }
    None
}
