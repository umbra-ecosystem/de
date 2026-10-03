//! The footer's services dot and the modal behind it: every service of the open workspace and
//! how each one stands.
//!
//! Pure view-model work. Both stores build it the same way from what their checks found, so the
//! dot and the modal say the same thing whichever is running.

use super::common::Tone;
use super::ids::RepoName;
use super::intent::Intent;

/// The footer's services dot: a word for how the workspace's services stand. There is no value
/// at all when there is no open workspace with services to say anything about.
#[derive(Clone, Debug, PartialEq)]
pub struct ServiceIndicatorVm {
    pub label: String,
    pub tone: Tone,
    pub open: Intent,
}

/// One service as the modal lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ServiceRowVm {
    pub service: String,
    /// How it stands: `running` when that is all there is to say, otherwise the exception
    /// (`stopped`, `unhealthy`, `never started`) in plain words.
    pub state: String,
    /// `None` for a running service: the normal state is plain text, not a row of green.
    pub tone: Option<Tone>,
}

/// One project's services, under its name.
#[derive(Clone, Debug, PartialEq)]
pub struct ServiceGroupVm {
    pub project: RepoName,
    pub rows: Vec<ServiceRowVm>,
}

/// The modal: every service grouped by project, or why there is nothing to list.
#[derive(Clone, Debug, PartialEq)]
pub struct ServicesModalVm {
    pub title: String,
    pub groups: Vec<ServiceGroupVm>,
    /// Instead of groups: why there is nothing to list (docker could not be asked, or the check
    /// has not run yet). Always the friendly reason with what to do.
    pub note: Option<String>,
}

/// The footer's dot from the summary counts.
///
/// `None` when the workspace declares no services: nothing to run is nothing to show. Otherwise
/// the dot is always there — quiet when everything runs, loud when something does not.
pub fn service_indicator(
    running: u32,
    declared: u32,
    unhealthy: u32,
    docker_ok: bool,
) -> Option<ServiceIndicatorVm> {
    if declared == 0 {
        return None;
    }
    let (label, tone) = if !docker_ok {
        ("Docker unavailable".to_string(), Tone::Bad)
    } else if unhealthy > 0 {
        (format!("{unhealthy} unhealthy"), Tone::Bad)
    } else if running == 0 {
        ("services down".to_string(), Tone::Warn)
    } else if running < declared {
        (format!("{running} of {declared} services"), Tone::Warn)
    } else {
        ("Services".to_string(), Tone::Ok)
    };
    Some(ServiceIndicatorVm {
        label,
        tone,
        open: Intent::OpenServices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dot says what is off, and only what is off: docker first (nothing could be asked),
    /// then unhealthy services, then stopped ones. All running is one quiet word.
    #[test]
    fn the_dot_names_the_exception_and_is_quiet_when_everything_runs() {
        // Nothing to run is nothing to show.
        assert_eq!(service_indicator(0, 0, 0, true), None);

        let dot = service_indicator(6, 6, 0, true).expect("a workspace with services");
        assert_eq!(dot.label, "Services");
        assert_eq!(dot.tone, Tone::Ok);
        assert_eq!(dot.open, Intent::OpenServices);

        let dot = service_indicator(0, 6, 0, true).expect("stopped");
        assert_eq!(dot.label, "services down");
        assert_eq!(dot.tone, Tone::Warn);

        let dot = service_indicator(4, 6, 0, true).expect("partial");
        assert_eq!(dot.label, "4 of 6 services");
        assert_eq!(dot.tone, Tone::Warn);

        // An unhealthy service outranks stopped ones: it is the thing to fix.
        let dot = service_indicator(4, 6, 2, true).expect("unhealthy");
        assert_eq!(dot.label, "2 unhealthy");
        assert_eq!(dot.tone, Tone::Bad);

        // Docker down outranks everything: no state could be read at all.
        let dot = service_indicator(0, 6, 0, false).expect("docker down");
        assert_eq!(dot.label, "Docker unavailable");
        assert_eq!(dot.tone, Tone::Bad);
    }
}
