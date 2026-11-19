//! Interface resolution: which component serves each requirement, the order
//! components must start in, and who depends on whom.

use crate::manifest::Manifest;
use crate::version::InterfaceRef;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// `client` uses `interface` served by `provider` (indices into the manifest slice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub client: usize,
    pub provider: usize,
    pub interface: InterfaceRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Providers before their clients.
    pub start_order: Vec<usize>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    DuplicateName(String),
    Missing {
        component: String,
        interface: InterfaceRef,
    },
    /// Several components could serve the requirement. Flare never picks
    /// silently; the composition must disambiguate.
    Ambiguous {
        component: String,
        interface: InterfaceRef,
        providers: Vec<String>,
    },
    Cycle {
        components: Vec<String>,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::DuplicateName(n) => write!(f, "component name '{}' is used twice", n),
            ResolveError::Missing { component, interface } => {
                write!(f, "'{}' requires {} but no component provides it", component, interface)
            }
            ResolveError::Ambiguous { component, interface, providers } => {
                write!(
                    f,
                    "'{}' requires {} which is provided by several components: {:?}",
                    component, interface, providers
                )
            }
            ResolveError::Cycle { components } => write!(f, "dependency cycle between {:?}", components),
        }
    }
}

pub fn resolve(ms: &[Manifest]) -> Result<Resolution, Vec<ResolveError>> {
    let mut errs = Vec::new();
    let mut names = BTreeSet::new();
    for m in ms {
        if !names.insert(m.name.as_str()) {
            errs.push(ResolveError::DuplicateName(m.name.clone()));
        }
    }
    let mut edges = Vec::new();
    for (ci, client) in ms.iter().enumerate() {
        for req in &client.requires {
            let providers: Vec<usize> = ms
                .iter()
                .enumerate()
                .filter(|(pi, p)| *pi != ci && p.provides.iter().any(|i| i.satisfies(req)))
                .map(|(pi, _)| pi)
                .collect();
            match providers.as_slice() {
                [] => errs.push(ResolveError::Missing { component: client.name.clone(), interface: req.clone() }),
                [p] => edges.push(Edge { client: ci, provider: *p, interface: req.clone() }),
                many => errs.push(ResolveError::Ambiguous {
                    component: client.name.clone(),
                    interface: req.clone(),
                    providers: many.iter().map(|&p| ms[p].name.clone()).collect(),
                }),
            }
        }
    }

    // Kahn's algorithm, stable with respect to manifest order.
    let mut indegree = alloc::vec![0usize; ms.len()];
    for e in &edges {
        indegree[e.client] += 1;
    }
    let mut order = Vec::with_capacity(ms.len());
    let mut done = alloc::vec![false; ms.len()];
    while let Some(next) = (0..ms.len()).find(|&i| !done[i] && indegree[i] == 0) {
        done[next] = true;
        order.push(next);
        for e in edges.iter().filter(|e| e.provider == next) {
            indegree[e.client] -= 1;
        }
    }
    if order.len() != ms.len() {
        let components = (0..ms.len()).filter(|&i| !done[i]).map(|i| ms[i].name.clone()).collect();
        errs.push(ResolveError::Cycle { components });
    }
    if errs.is_empty() {
        Ok(Resolution { start_order: order, edges })
    } else {
        Err(errs)
    }
}

impl Resolution {
    /// Components that directly use an interface served by `provider`.
    pub fn clients_of(&self, provider: usize) -> Vec<usize> {
        let mut v: Vec<usize> = self.edges.iter().filter(|e| e.provider == provider).map(|e| e.client).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Everything that transitively depends on `provider`.
    pub fn dependents_of(&self, provider: usize) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut stack = alloc::vec![provider];
        while let Some(p) = stack.pop() {
            for c in self.clients_of(p) {
                if seen.insert(c) {
                    stack.push(c);
                }
            }
        }
        seen.remove(&provider);
        seen.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Kind, SwapPolicy};
    use alloc::vec;

    fn sys() -> Vec<Manifest> {
        vec![
            Manifest::new("ui", Kind::App, SwapPolicy::Restart, &[], &["flare.display@1.0", "flare.input@1.0"]),
            Manifest::new("compositor", Kind::Service, SwapPolicy::Restart, &["flare.display@1.1"], &["flare.gpu@2.0"]),
            Manifest::new("gpu-drv", Kind::Service, SwapPolicy::Live, &["flare.gpu@2.3"], &[]),
            Manifest::new("touch-drv", Kind::Service, SwapPolicy::Restart, &["flare.input@1.0"], &[]),
        ]
    }

    #[test]
    fn start_order_puts_providers_first() {
        let ms = sys();
        let r = resolve(&ms).unwrap();
        let pos = |n: &str| r.start_order.iter().position(|&i| ms[i].name == n).unwrap();
        assert!(pos("gpu-drv") < pos("compositor"));
        assert!(pos("compositor") < pos("ui"));
        assert!(pos("touch-drv") < pos("ui"));
        assert_eq!(r.dependents_of(2), vec![0, 1]);
        assert_eq!(r.clients_of(3), vec![0]);
    }

    #[test]
    fn reports_missing_ambiguous_and_cycles() {
        let mut ms = sys();
        ms.push(Manifest::new("gpu-alt", Kind::Service, SwapPolicy::Restart, &["flare.gpu@2.0"], &[]));
        ms.push(Manifest::new("a", Kind::Service, SwapPolicy::Restart, &["x.a@1.0"], &["x.b@1.0"]));
        ms.push(Manifest::new("b", Kind::Service, SwapPolicy::Restart, &["x.b@1.0"], &["x.a@1.0", "x.nope@1.0"]));
        let errs = resolve(&ms).unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ResolveError::Ambiguous { providers, .. } if providers.len() == 2)));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ResolveError::Missing { interface, .. } if interface.name == "x.nope")));
        assert!(errs
            .iter()
            .any(|e| matches!(e, ResolveError::Cycle { components } if components.contains(&"a".into()))));
    }

    #[test]
    fn incompatible_versions_do_not_resolve() {
        let ms = vec![
            Manifest::new("old-drv", Kind::Service, SwapPolicy::Restart, &["flare.audio@1.4"], &[]),
            Manifest::new("app", Kind::App, SwapPolicy::Restart, &[], &["flare.audio@2.0"]),
        ];
        assert!(matches!(resolve(&ms).unwrap_err()[0], ResolveError::Missing { .. }));
    }
}
