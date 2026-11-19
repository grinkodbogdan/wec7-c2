//! The component lifecycle state machine (spec §6).
//!
//! ```text
//! Discovered -Resolve-> Resolved -Authorize-> Authorized -Start-> Starting -Started-> Running
//! Running -Quiesce-> Quiescing -Quiesced-> Quiesced -Resume-> Running
//! Running|Starting|Quiescing|Quiesced -Stop-> Stopping -Stopped-> Stopped
//! (any live state) -Fail-> Failed
//! Failed|Stopped -Restart-> Authorized          (capabilities re-granted)
//! Discovered|Resolved|Failed|Stopped -Retire-> Retired
//! ```

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Discovered,
    Resolved,
    Authorized,
    Starting,
    Running,
    Quiescing,
    Quiesced,
    Stopping,
    Stopped,
    Failed,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Resolve,
    Authorize,
    Start,
    Started,
    Quiesce,
    Quiesced,
    Resume,
    Stop,
    Stopped,
    Fail,
    Restart,
    Retire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionError {
    pub from: State,
    pub event: Event,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "event {:?} is not valid in state {:?}", self.event, self.from)
    }
}

pub fn next(from: State, event: Event) -> Result<State, TransitionError> {
    use Event as E;
    use State as S;
    let to = match (from, event) {
        (S::Discovered, E::Resolve) => S::Resolved,
        (S::Resolved, E::Authorize) => S::Authorized,
        (S::Authorized, E::Start) => S::Starting,
        (S::Starting, E::Started) => S::Running,
        (S::Running, E::Quiesce) => S::Quiescing,
        (S::Quiescing, E::Quiesced) => S::Quiesced,
        (S::Quiesced, E::Resume) => S::Running,
        (S::Running | S::Starting | S::Quiescing | S::Quiesced, E::Stop) => S::Stopping,
        (S::Stopping, E::Stopped) => S::Stopped,
        (S::Retired | S::Failed, E::Fail) => return Err(TransitionError { from, event }),
        (_, E::Fail) => S::Failed,
        (S::Failed | S::Stopped, E::Restart) => S::Authorized,
        (S::Discovered | S::Resolved | S::Failed | S::Stopped, E::Retire) => S::Retired,
        _ => return Err(TransitionError { from, event }),
    };
    Ok(to)
}

impl State {
    /// Does a component in this state hold its capabilities?
    pub fn holds_capabilities(self) -> bool {
        matches!(
            self,
            State::Authorized | State::Starting | State::Running | State::Quiescing | State::Quiesced | State::Stopping
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[Event]) -> Result<State, TransitionError> {
        events.iter().try_fold(State::Discovered, |s, &e| next(s, e))
    }

    #[test]
    fn happy_path_and_restart() {
        use Event::*;
        assert_eq!(run(&[Resolve, Authorize, Start, Started]), Ok(State::Running));
        assert_eq!(run(&[Resolve, Authorize, Start, Started, Quiesce, Quiesced, Resume]), Ok(State::Running));
        assert_eq!(run(&[Resolve, Authorize, Start, Fail, Restart, Start, Started]), Ok(State::Running));
        assert_eq!(run(&[Resolve, Authorize, Start, Started, Stop, Stopped, Retire]), Ok(State::Retired));
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        use Event::*;
        assert!(run(&[Start]).is_err());
        assert!(run(&[Resolve, Authorize, Start, Started, Retire]).is_err());
        assert!(run(&[Resolve, Retire, Fail]).is_err());
        assert!(!State::Failed.holds_capabilities());
        assert!(State::Quiesced.holds_capabilities());
    }
}
