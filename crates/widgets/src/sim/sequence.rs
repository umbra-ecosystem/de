//! Opening and closing a workspace as a visible sequence of steps.
//!
//! Selecting a workspace does not switch at once: it runs the steps of `docs/design/workspace-lifecycle.md` (read
//! the workspace, git status per repo, start services, load the data; or the reverse for closing), each taking a
//! believable time, any of which can fail. The window shows them in a modal that cannot be left while they run.
//! Only when they are done (or the person chooses to carry on past failures) does the workspace actually change.

use super::Sim;
use super::model::*;
use super::worlds::INIT_COMMAND;
use crate::store::Outcome;
use crate::vm::{
    Branch, Btn, Command, Intent, PhaseVm, ProgressState, SequenceState, SequenceVm, StepVm,
    ToastKind, WorkspaceName,
};

/// How long the modal shows "Ready" before it closes by itself.
const LINGER_MS: i64 = 700;

const DOCKER_DOWN: &str = "Docker is not running. Start Docker Desktop, then retry.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepKind {
    Read,
    Git,
    DockerUp,
    DockerDown,
    Load,
    Check,
    Save,
    /// Move one project onto a branch (or onto its baseline when it does not have it).
    Switch,
}

#[derive(Clone, Debug)]
struct Step {
    label: String,
    kind: StepKind,
    ms: i64,
    state: ProgressState,
    detail: String,
    /// What it reports when it goes well.
    ok: String,
    /// Why it must not run, when it must not; empty when it always can.
    fail: String,
}

#[derive(Clone, Debug)]
struct Phase {
    title: String,
    steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Nothing is open: open the saved workspace at this index.
    Open(usize),
    /// Close the open one.
    Close,
    /// Close the open one, then open the one at this index.
    Switch(usize),
    /// Move every project of the open one onto this branch.
    SwitchBranch(Branch),
}

#[derive(Clone, Debug)]
pub(crate) struct Seq {
    kind: Kind,
    phases: Vec<Phase>,
    /// The `(phase, step)` being run.
    at: (usize, usize),
    /// Milliseconds spent on it.
    elapsed: i64,
    state: SequenceState,
    linger: i64,
    /// Something before the rest made them pointless (an active ticket holds the repos): they were skipped.
    blocked: bool,
}

fn step(label: impl Into<String>, kind: StepKind, ms: i64, ok: impl Into<String>) -> Step {
    Step {
        label: label.into(),
        kind,
        ms,
        state: ProgressState::Waiting,
        detail: String::new(),
        ok: ok.into(),
        fail: String::new(),
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

impl Sim {
    /// The steps of opening a workspace whose (parked) world is `world`.
    fn open_phase(&self, idx: usize) -> Phase {
        let saved = &self.saved[idx];
        let empty = super::worlds::World::default();
        let w = saved.world.as_ref().unwrap_or(&empty);
        let mut steps = vec![step(
            "Read the workspace",
            StepKind::Read,
            400,
            plural(w.repos.len(), "project"),
        )];
        for r in &w.repos {
            let branch = w.ws.branches.get(&r.name).cloned().unwrap_or_default();
            let state = if w.ws.dirty.contains(&r.name) {
                "uncommitted changes"
            } else {
                "clean"
            };
            steps.push(step(
                format!("Git status \u{b7} {}", r.name),
                StepKind::Git,
                450,
                format!("{branch}, {state}"),
            ));
        }
        for r in w.repos.iter().filter(|r| r.services > 0) {
            steps.push(step(
                format!("Start services \u{b7} {}", r.name),
                StepKind::DockerUp,
                1700,
                plural(r.services as usize, "service"),
            ));
        }
        steps.push(step(
            "Load the workspace",
            StepKind::Load,
            700,
            plural(w.tickets.len(), "ticket"),
        ));
        Phase {
            title: format!("Opening {}", saved.name),
            steps,
        }
    }

    /// The steps of closing the open workspace (the live world), in the reverse order of opening.
    fn close_phase(&self) -> Phase {
        let name = self.current_workspace().cloned().unwrap_or_default();
        let mut steps = vec![step(
            "Check nothing is in progress",
            StepKind::Check,
            500,
            "nothing is",
        )];
        for r in self.repos.iter().rev().filter(|r| r.services > 0) {
            steps.push(step(
                format!("Stop services \u{b7} {}", r.name),
                StepKind::DockerDown,
                1200,
                "stopped",
            ));
        }
        steps.push(step(
            "Save the workspace",
            StepKind::Save,
            400,
            "kept as you left it",
        ));
        Phase {
            title: format!("Closing {name}"),
            steps,
        }
    }

    /// The steps of moving every project onto `target`: the check that gates anything else, then
    /// one step per project saying where it ends up (or why it does not move).
    fn switch_phase(&self, target: &Branch) -> Phase {
        let mut steps = vec![step(
            "Check nothing is in progress",
            StepKind::Check,
            500,
            "nothing is",
        )];
        for r in &self.repos {
            let (detail, _, fail) = self.switch_here(r, target);
            let mut s = step(
                format!("Switch \u{b7} {}", r.name),
                StepKind::Switch,
                550,
                detail,
            );
            s.fail = fail.unwrap_or_default();
            steps.push(s);
        }
        Phase {
            title: format!("Switch to {target}"),
            steps,
        }
    }

    fn build(&self, kind: Kind) -> Seq {
        let phases = match &kind {
            Kind::Open(i) => vec![self.open_phase(*i)],
            Kind::Close => vec![self.close_phase()],
            Kind::Switch(i) => vec![self.close_phase(), self.open_phase(*i)],
            Kind::SwitchBranch(branch) => vec![self.switch_phase(branch)],
        };
        Seq {
            kind,
            phases,
            at: (0, 0),
            elapsed: 0,
            state: SequenceState::Running,
            linger: 0,
            blocked: false,
        }
    }

    /// Select `name`: start the sequence that opens it (after closing the open one, if any).
    pub(crate) fn start_select(&mut self, name: &WorkspaceName) -> Outcome {
        if self.wseq.is_some() {
            return Outcome::fail("A workspace is being opened or closed. Wait for it to finish.");
        }
        let Some(target) = self.saved.iter().position(|s| s.name == *name) else {
            return Outcome::fail(format!(
                "There is no workspace called {name}. Create one with {INIT_COMMAND}."
            ));
        };
        if self.open_ws == Some(target) {
            return Outcome::ok();
        }
        let kind = if self.open_ws.is_some() {
            Kind::Switch(target)
        } else {
            Kind::Open(target)
        };
        self.wseq = Some(self.build(kind));
        Outcome::ok()
    }

    /// Close the open workspace: start the sequence that does it.
    pub(crate) fn start_close(&mut self) -> Outcome {
        if self.wseq.is_some() {
            return Outcome::fail("A workspace is being opened or closed. Wait for it to finish.");
        }
        if self.open_ws.is_none() {
            return Outcome::ok();
        }
        self.wseq = Some(self.build(Kind::Close));
        Outcome::ok()
    }

    /// Move every project of the open workspace onto `branch`, showing every step of it.
    pub(crate) fn start_switch(&mut self, branch: Branch) -> Outcome {
        if self.wseq.is_some() {
            return Outcome::fail("A workspace is being opened or closed. Wait for it to finish.");
        }
        if self.open_ws.is_none() {
            return Outcome::fail(
                "No workspace is open, so there are no projects to switch. Open one first.",
            );
        }
        self.wseq = Some(self.build(Kind::SwitchBranch(branch)));
        Outcome::ok()
    }

    /// Run the steps for `ms` milliseconds of real time.
    pub fn progress_sequence(&mut self, ms: i64) {
        let Some(mut seq) = self.wseq.take() else {
            return;
        };
        let mut left = ms;
        while left > 0 {
            match seq.state {
                SequenceState::NeedsDecision => break,
                SequenceState::Ready => {
                    seq.linger -= left;
                    left = 0;
                    if seq.linger <= 0 {
                        self.wseq = Some(seq);
                        self.finish_sequence();
                        return;
                    }
                }
                SequenceState::Running => {
                    let (p, s) = seq.at;
                    seq.phases[p].steps[s].state = ProgressState::Running;
                    let need = seq.phases[p].steps[s].ms - seq.elapsed;
                    if left < need {
                        seq.elapsed += left;
                        left = 0;
                    } else {
                        left -= need;
                        seq.elapsed = 0;
                        self.complete_step(&mut seq);
                    }
                }
            }
        }
        self.wseq = Some(seq);
    }

    /// The running step ends: it went fine or it did not, and the sequence moves on (or ends).
    fn complete_step(&self, seq: &mut Seq) {
        let (p, s) = seq.at;
        let kind = seq.phases[p].steps[s].kind;
        let ok = seq.phases[p].steps[s].ok.clone();
        let fail = seq.phases[p].steps[s].fail.clone();
        let (state, detail) = match kind {
            StepKind::DockerUp | StepKind::DockerDown if self.sims.docker_down => {
                (ProgressState::Failed, DOCKER_DOWN.to_string())
            }
            StepKind::Check => match self.in_progress() {
                Some(why) => (ProgressState::Failed, why),
                None => (ProgressState::Done, ok),
            },
            // A project that must not be switched (its work would be left behind, or it has
            // nowhere to go) says why, and the rest carry on: only the check stops everything.
            StepKind::Switch if !fail.is_empty() => (ProgressState::Failed, fail),
            _ => (ProgressState::Done, ok),
        };
        let blocked_now = kind == StepKind::Check && state == ProgressState::Failed;
        let step = &mut seq.phases[p].steps[s];
        step.state = state;
        step.detail = detail;

        if blocked_now {
            // What is in progress holds the repos: nothing after this can be done.
            seq.blocked = true;
            for (pi, phase) in seq.phases.iter_mut().enumerate() {
                for (si, st) in phase.steps.iter_mut().enumerate() {
                    if (pi, si) > (p, s) {
                        st.state = ProgressState::Skipped;
                        st.detail = "not run".to_string();
                    }
                }
            }
            seq.state = SequenceState::NeedsDecision;
            return;
        }

        // The next step, or the end.
        let next = if s + 1 < seq.phases[p].steps.len() {
            Some((p, s + 1))
        } else if p + 1 < seq.phases.len() {
            Some((p + 1, 0))
        } else {
            None
        };
        match next {
            Some(at) => seq.at = at,
            None => {
                let failed = seq
                    .phases
                    .iter()
                    .flat_map(|ph| &ph.steps)
                    .any(|st| st.state == ProgressState::Failed);
                if failed {
                    seq.state = SequenceState::NeedsDecision;
                } else {
                    seq.state = SequenceState::Ready;
                    seq.linger = LINGER_MS;
                }
            }
        }
    }

    /// Why the open workspace cannot be closed right now, if it cannot.
    fn in_progress(&self) -> Option<String> {
        if let Some(key) = self.active_key() {
            return Some(format!("{key} is active. Park it first."));
        }
        self.locks
            .iter()
            .next()
            .map(|(repo, l)| format!("{repo} is busy: {}", Self::lock_text(l)))
    }

    /// The sequence is over (everything fine, or the person chose to go on): make the change it was for.
    fn finish_sequence(&mut self) {
        let Some(seq) = self.wseq.take() else {
            return;
        };
        let out = match seq.kind {
            Kind::Open(i) => self.apply_select(i),
            Kind::Close => self.apply_close(),
            Kind::Switch(i) => {
                self.apply_close();
                self.apply_select(i)
            }
            Kind::SwitchBranch(branch) => self.finish_switch(&branch),
        };
        if let Outcome::Done {
            toast: Some(t), ..
        } = out
        {
            self.toast(t.text, t.kind);
        }
    }

    /// What moving every project to `target` left behind, in one line.
    ///
    /// The projects that already had the branch and the ones that fell back are both "done" for
    /// the toast's purposes; only a project that could not be moved at all is worth a warning,
    /// because its work is still sitting where it was.
    fn finish_switch(&mut self, target: &Branch) -> Outcome {
        let (on_target, refused, total) = self.apply_switch(target);
        self.ok("workspace.branch", None, None, target);
        let text = if refused == 0 && on_target == total {
            format!("Switched to {target}")
        } else {
            format!("Switched {on_target} of {total} projects to {target}")
        };
        let kind = if refused == 0 {
            ToastKind::Ok
        } else {
            ToastKind::Warn
        };
        Outcome::ok().with_toast(text, kind, None)
    }

    /// After failures: carry on anyway.
    pub(crate) fn continue_sequence(&mut self) -> Outcome {
        match &self.wseq {
            Some(s) if s.state == SequenceState::NeedsDecision && !s.blocked => {
                self.finish_sequence();
                Outcome::ok()
            }
            Some(_) => Outcome::fail("It cannot go on yet."),
            None => Outcome::ok(),
        }
    }

    /// After failures: run it again from the top.
    pub(crate) fn retry_sequence(&mut self) -> Outcome {
        match self.wseq.as_ref().map(|s| (s.state, s.kind.clone())) {
            Some((SequenceState::NeedsDecision, kind)) => {
                self.wseq = Some(self.build(kind));
                Outcome::ok()
            }
            _ => Outcome::ok(),
        }
    }

    /// After failures: give up; the workspace stays as it was.
    pub(crate) fn abort_sequence(&mut self) -> Outcome {
        match self.wseq.as_ref().map(|s| s.state) {
            Some(SequenceState::NeedsDecision) => {
                self.wseq = None;
                Outcome::ok().with_toast(
                    "Stopped. Nothing was changed in the window; services that were started keep running.",
                    ToastKind::Info,
                    None,
                )
            }
            _ => Outcome::ok(),
        }
    }

    /// The modal's view: phases and steps, how far along, and what the footer offers.
    pub fn sequence_vm(&self) -> Option<SequenceVm> {
        let seq = self.wseq.as_ref()?;
        let title = match &seq.kind {
            Kind::Open(i) => format!("Opening {}", self.saved[*i].name),
            Kind::Close => format!(
                "Closing {}",
                self.current_workspace().map(|n| n.as_str()).unwrap_or("the workspace")
            ),
            Kind::Switch(i) => format!("Switching to {}", self.saved[*i].name),
            Kind::SwitchBranch(branch) => format!("Switch to {branch}"),
        };
        let all: Vec<&Step> = seq.phases.iter().flat_map(|p| &p.steps).collect();
        let done = all
            .iter()
            .filter(|s| matches!(s.state, ProgressState::Done | ProgressState::Failed | ProgressState::Skipped))
            .count();
        // What carrying on past a failure means here: closing the workspace, opening the other
        // one, or finishing the switch that is part-way through.
        let carry = match &seq.kind {
            Kind::Close => "Close anyway",
            Kind::SwitchBranch(_) => "Switch anyway",
            _ => "Open anyway",
        };
        let mut actions = Vec::new();
        if seq.state == SequenceState::NeedsDecision {
            actions.push(Btn::new("Back", Intent::Do(Command::AbortSequence)));
            actions.push(Btn::new("Try again", Intent::Do(Command::RetrySequence)));
            if !seq.blocked {
                actions.push(
                    Btn::new(carry, Intent::Do(Command::ContinueSequence)).primary(),
                );
            }
        }
        Some(SequenceVm {
            title,
            phases: seq
                .phases
                .iter()
                .map(|p| PhaseVm {
                    title: p.title.clone(),
                    steps: p
                        .steps
                        .iter()
                        .map(|s| StepVm {
                            label: s.label.clone(),
                            detail: s.detail.clone(),
                            state: s.state,
                        })
                        .collect(),
                })
                .collect(),
            state: seq.state,
            progress: format!("{done} of {}", all.len()),
            actions,
        })
    }
}
