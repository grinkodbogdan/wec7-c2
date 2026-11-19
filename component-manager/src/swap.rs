//! Hot-swap planning (spec §6, "Replace or recover").
//!
//! Given a running system and a replacement for one component, produce the
//! ordered steps the component manager must perform, or explain why the
//! replacement is not allowed. Only the replaced component's direct clients
//! are disturbed; nothing else is restarted.

use crate::manifest::{Kind, Manifest, SwapPolicy};
use crate::resolve::Resolution;
use crate::version::InterfaceRef;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Hold new requests from these clients at the IPC boundary.
    QuiesceClients(Vec<String>),
    QuiesceComponent(String),
    Checkpoint(String),
    Stop(String),
    /// Revoke every capability held by (and derived from) the component.
    RevokeCapabilities(String),
    /// Move hardware authority (MMIO, IRQ, DMA) to the replacement.
    TransferCapabilities {
        from: String,
        to: String,
    },
    ResetHardware(String),
    Start(String),
    Restore {
        from: String,
        to: String,
    },
    ReconnectClients {
        clients: Vec<String>,
        to: String,
    },
    ResumeClients(Vec<String>),
    Retire(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Disruption {
    /// Clients see a pause, never an error.
    None,
    /// Clients see the service go away and reconnect.
    Interrupted,
    /// The device is reset; in-flight hardware state is lost.
    HardwareReset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapPlan {
    pub disruption: Disruption,
    pub steps: Vec<Step>,
    /// Components untouched by the swap.
    pub unaffected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwapError {
    UnknownComponent(String),
    /// A current client needs an interface the replacement does not provide.
    BreaksClient {
        client: String,
        interface: InterfaceRef,
    },
    /// The replacement needs something the rest of the system cannot provide.
    UnsatisfiedRequirement(InterfaceRef),
    /// A replacement driver must bind to the same hardware.
    BindingMismatch {
        old: Vec<String>,
        new: Vec<String>,
    },
    KindMismatch,
}

impl fmt::Display for SwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SwapError::UnknownComponent(n) => write!(f, "no component named '{}'", n),
            SwapError::BreaksClient { client, interface } => {
                write!(f, "'{}' uses {} which the replacement does not provide", client, interface)
            }
            SwapError::UnsatisfiedRequirement(i) => {
                write!(f, "the replacement requires {} which nothing else provides", i)
            }
            SwapError::BindingMismatch { old, new } => {
                write!(f, "replacement driver binds {:?} but the current driver binds {:?}", new, old)
            }
            SwapError::KindMismatch => write!(f, "a component can only be replaced by one of the same kind"),
        }
    }
}

pub fn plan_replacement(
    ms: &[Manifest],
    res: &Resolution,
    old_name: &str,
    new: &Manifest,
) -> Result<SwapPlan, Vec<SwapError>> {
    let Some(old_idx) = ms.iter().position(|m| m.name == old_name) else {
        return Err(alloc::vec![SwapError::UnknownComponent(old_name.into())]);
    };
    let old = &ms[old_idx];
    let mut errs = Vec::new();

    if old.kind != new.kind {
        errs.push(SwapError::KindMismatch);
    }
    if old.kind == Kind::Driver && !old.binds.iter().any(|b| new.binds.contains(b)) {
        errs.push(SwapError::BindingMismatch { old: old.binds.clone(), new: new.binds.clone() });
    }
    for e in res.edges.iter().filter(|e| e.provider == old_idx) {
        if !new.provides.iter().any(|p| p.satisfies(&e.interface)) {
            errs.push(SwapError::BreaksClient { client: ms[e.client].name.clone(), interface: e.interface.clone() });
        }
    }
    for req in &new.requires {
        let ok = ms.iter().enumerate().any(|(i, m)| i != old_idx && m.provides.iter().any(|p| p.satisfies(req)));
        if !ok {
            errs.push(SwapError::UnsatisfiedRequirement(req.clone()));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let clients: Vec<String> = res.clients_of(old_idx).into_iter().map(|i| ms[i].name.clone()).collect();
    let o = old.name.clone();
    let n = new.name.clone();
    // The swap is as disruptive as the less capable of the two components.
    let policy = old.swap.max(new.swap);
    let mut steps = Vec::new();
    if !clients.is_empty() {
        steps.push(Step::QuiesceClients(clients.clone()));
    }
    let disruption = match policy {
        SwapPolicy::Live => {
            steps.push(Step::QuiesceComponent(o.clone()));
            let checkpoint = old.hooks.checkpoint && new.hooks.checkpoint;
            if checkpoint {
                steps.push(Step::Checkpoint(o.clone()));
            }
            steps.push(Step::TransferCapabilities { from: o.clone(), to: n.clone() });
            steps.push(Step::Start(n.clone()));
            if checkpoint {
                steps.push(Step::Restore { from: o.clone(), to: n.clone() });
            }
            Disruption::None
        }
        SwapPolicy::Restart | SwapPolicy::Reset => {
            steps.push(Step::Stop(o.clone()));
            steps.push(Step::RevokeCapabilities(o.clone()));
            if policy == SwapPolicy::Reset {
                steps.push(Step::ResetHardware(o.clone()));
            }
            steps.push(Step::Start(n.clone()));
            if policy == SwapPolicy::Reset {
                Disruption::HardwareReset
            } else {
                Disruption::Interrupted
            }
        }
    };
    if !clients.is_empty() {
        steps.push(Step::ReconnectClients { clients: clients.clone(), to: n.clone() });
        steps.push(Step::ResumeClients(clients.clone()));
    }
    if policy == SwapPolicy::Live {
        steps.push(Step::RevokeCapabilities(o.clone()));
    }
    steps.push(Step::Retire(o.clone()));

    let unaffected = ms.iter().filter(|m| m.name != o && !clients.contains(&m.name)).map(|m| m.name.clone()).collect();
    Ok(SwapPlan { disruption, steps, unaffected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::resolve;
    use alloc::string::ToString;
    use alloc::vec;

    fn drv(name: &str, swap: SwapPolicy, provides: &[&str]) -> Manifest {
        let mut m = Manifest::new(name, Kind::Driver, swap, provides, &[]);
        m.binds = vec!["vendor,audio-codec".to_string()];
        m
    }

    fn system() -> Vec<Manifest> {
        vec![
            drv("codec-v1", SwapPolicy::Restart, &["flare.pcm@1.0"]),
            Manifest::new("audio", Kind::Service, SwapPolicy::Restart, &["flare.audio@1.0"], &["flare.pcm@1.0"]),
            Manifest::new("music", Kind::App, SwapPolicy::Restart, &[], &["flare.audio@1.0"]),
            Manifest::new("clock", Kind::App, SwapPolicy::Restart, &[], &[]),
        ]
    }

    #[test]
    fn restart_swap_only_touches_direct_clients() {
        let ms = system();
        let res = resolve(&ms).unwrap();
        let plan =
            plan_replacement(&ms, &res, "codec-v1", &drv("codec-v2", SwapPolicy::Restart, &["flare.pcm@1.1"])).unwrap();
        assert_eq!(plan.disruption, Disruption::Interrupted);
        assert_eq!(
            plan.steps,
            vec![
                Step::QuiesceClients(vec!["audio".into()]),
                Step::Stop("codec-v1".into()),
                Step::RevokeCapabilities("codec-v1".into()),
                Step::Start("codec-v2".into()),
                Step::ReconnectClients { clients: vec!["audio".into()], to: "codec-v2".into() },
                Step::ResumeClients(vec!["audio".into()]),
                Step::Retire("codec-v1".into()),
            ]
        );
        // `music` depends on the codec only transitively: it is not restarted.
        assert_eq!(plan.unaffected, vec!["music".to_string(), "clock".to_string()]);
    }

    #[test]
    fn live_swap_transfers_authority_and_state() {
        let ms = {
            let mut ms = system();
            ms[0] = drv("codec-v1", SwapPolicy::Live, &["flare.pcm@1.0"]);
            ms[0].hooks.checkpoint = true;
            ms
        };
        let res = resolve(&ms).unwrap();
        let mut new = drv("codec-v2", SwapPolicy::Live, &["flare.pcm@1.0"]);
        new.hooks.checkpoint = true;
        let plan = plan_replacement(&ms, &res, "codec-v1", &new).unwrap();
        assert_eq!(plan.disruption, Disruption::None);
        assert!(plan.steps.contains(&Step::Checkpoint("codec-v1".into())));
        assert!(plan.steps.contains(&Step::TransferCapabilities { from: "codec-v1".into(), to: "codec-v2".into() }));
        assert!(plan.steps.contains(&Step::Restore { from: "codec-v1".into(), to: "codec-v2".into() }));
        // A live-capable driver replaced by a restart-only one degrades the swap.
        let plan =
            plan_replacement(&ms, &res, "codec-v1", &drv("codec-v3", SwapPolicy::Reset, &["flare.pcm@1.0"])).unwrap();
        assert_eq!(plan.disruption, Disruption::HardwareReset);
        assert!(plan.steps.contains(&Step::ResetHardware("codec-v1".into())));
    }

    #[test]
    fn incompatible_replacements_are_refused() {
        let ms = system();
        let res = resolve(&ms).unwrap();
        let mut wrong_hw = drv("other", SwapPolicy::Restart, &["flare.pcm@2.0"]);
        wrong_hw.binds = vec!["vendor,other".into()];
        wrong_hw.requires = vec![InterfaceRef::parse("flare.dsp@1.0").unwrap()];
        let errs = plan_replacement(&ms, &res, "codec-v1", &wrong_hw).unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, SwapError::BreaksClient { client, .. } if client == "audio")));
        assert!(errs.iter().any(|e| matches!(e, SwapError::BindingMismatch { .. })));
        assert!(errs.iter().any(|e| matches!(e, SwapError::UnsatisfiedRequirement(_))));
        assert!(plan_replacement(&ms, &res, "nope", &wrong_hw).is_err());
    }
}
