//! The tick profiler's report: where Linux programs spent their time,
//! sampled on each CPU's timer tick (`echo profile on > /dev/flarectl`,
//! then `echo profile > /dev/flarectl; cat /dev/flarectl`).
//!
//! Samples are grouped by mapping: a library is named by its file size
//! (unique enough to look up), JIT code is "anon rx". For the busiest
//! mappings the busiest offsets into the file follow (64-byte buckets),
//! for a symbolizer on the build machine. Time inside the kernel (system
//! calls, faults) shows up at the instruction after the call.

use super::mm::Backing;
use super::Kernel;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

impl Kernel {
    pub(crate) fn linux_profile_report(&self, samples: &[(u32, u64)], all: bool) -> String {
        // (program, mapping description, file offset of the mapping's start
        // minus its address) per sample; grouped by program and mapping.
        let mut groups: BTreeMap<(String, String), (u64, BTreeMap<u64, u64>)> = BTreeMap::new();
        let mut lost = 0u64;
        for &(c, pc) in samples {
            let c = c as usize;
            if !self.live_task(c) {
                lost += 1;
                continue;
            }
            let name = self.comps[c].name.clone();
            let p = self.proc(c);
            let (desc, off) = match p.mm.find(pc) {
                Some(v) => match &v.backing {
                    Backing::File(d, o) => (format!("file {:#x}", d.len()), o + (pc - v.start)),
                    Backing::Private(m, o) => (format!("file {:#x}", m.borrow().size), o + (pc - v.start)),
                    Backing::Anon if v.prot & 4 != 0 => ("anon rx (JIT)".into(), pc - v.start),
                    _ => ("other".into(), pc - v.start),
                },
                None => ("unmapped".into(), pc),
            };
            let e = groups.entry((name, desc)).or_insert((0, BTreeMap::new()));
            e.0 += 1;
            *e.1.entry(off & !63).or_insert(0) += 1;
        }
        let total = samples.len() as u64 + self.prof_idle + self.prof_other;
        let pct = |n: u64| n * 100 / total.max(1);
        let mut out = format!(
            "profile: {} ticks: programs {}%, idle {}%, Flare components {}%, exited {}%\n",
            total,
            pct(samples.len() as u64 - lost),
            pct(self.prof_idle),
            pct(self.prof_other),
            pct(lost)
        );
        // The display: how often the shell presented, and what copying its
        // frames to the panel cost (under the kernel lock).
        let secs = (crate::arch::timer::now_ns() - self.prof_t0) as f64 / 1e9;
        let d = &self.linux.as_ref().unwrap().drm;
        out.push_str(&format!(
            "profile: {:.1} s; display: {} presents ({:.1}/s), {} ms each to copy\n",
            secs,
            d.presents,
            d.presents as f64 / secs.max(0.001),
            d.present_ns / d.presents.max(1) / 1_000_000
        ));
        let mut list: Vec<_> = groups.into_iter().collect();
        list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        for (i, ((name, desc), (n, offs))) in list.iter().take(10).enumerate() {
            out.push_str(&format!("profile: {:>2}% {} {}", pct(*n), name, desc));
            out.push_str(&format!(" ({} ticks)", n));
            if i < 3 && !desc.starts_with("anon") {
                let mut o: Vec<_> = offs.iter().collect();
                o.sort_by(|a, b| b.1.cmp(a.1));
                for (off, k) in o.iter().take(6) {
                    out.push_str(&format!(" {:x}:{}", off, k));
                }
            }
            out.push('\n');
        }
        // (everything, for a symbolizer: one line per 64-byte bucket)
        if all {
            for ((name, desc), (_, offs)) in &list {
                for (off, k) in offs {
                    out.push_str(&format!("profile-all: {} {} {:x} {}\n", name, desc.replace(' ', "_"), off, k));
                }
            }
        }
        out
    }
}
