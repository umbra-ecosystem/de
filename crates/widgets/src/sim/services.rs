//! Every service of the open workspace and how each one stands.
//!
//! Nothing here touches the outside world: a service the workspace started is running, one it
//! did not is stopped, and when docker itself is down there is nothing to list — only the
//! reason, with what to do about it.

use super::Sim;
use crate::vm::{
    ServiceGroupVm, ServiceIndicatorVm, ServiceRowVm, ServicesModalVm, Tone, service_indicator,
};

impl Sim {
    /// The footer's dot from what the workspace's services are doing.
    pub(crate) fn service_indicator(&self) -> Option<ServiceIndicatorVm> {
        let declared: u32 = self.repos.iter().map(|r| r.services.len() as u32).sum();
        let running = if self.sims.docker_down {
            0
        } else if self.ws.up {
            declared
        } else {
            0
        };
        service_indicator(running, declared, 0, !self.sims.docker_down)
    }

    /// The modal: every service grouped by project, or why docker could not be asked.
    pub(crate) fn services_modal(&self) -> ServicesModalVm {
        if self.sims.docker_down {
            return ServicesModalVm {
                title: "Services".to_string(),
                groups: Vec::new(),
                note: Some(
                    "Docker is not running. Start Docker Desktop, then try again.".to_string(),
                ),
            };
        }
        let running = self.ws.up;
        let groups = self
            .repos
            .iter()
            .filter(|r| !r.services.is_empty())
            .map(|r| ServiceGroupVm {
                project: r.name.clone(),
                rows: r
                    .services
                    .iter()
                    .map(|s| {
                        if running {
                            ServiceRowVm {
                                service: s.to_string(),
                                state: "running".to_string(),
                                tone: None,
                            }
                        } else {
                            ServiceRowVm {
                                service: s.to_string(),
                                state: "stopped".to_string(),
                                tone: Some(Tone::Warn),
                            }
                        }
                    })
                    .collect(),
            })
            .collect();
        ServicesModalVm {
            title: "Services".to_string(),
            groups,
            note: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The modal lists every service under its project; a workspace that never started shows
    /// what is stopped rather than an empty box.
    #[test]
    fn the_modal_lists_every_service_with_how_it_stands() {
        let sim = Sim::new();
        let modal = sim.services_modal();
        assert_eq!(modal.note, None);
        let names: Vec<String> = modal
            .groups
            .iter()
            .flat_map(|g| g.rows.iter().map(|r| format!("{}/{}", g.project, r.service)))
            .collect();
        assert_eq!(
            names,
            vec![
                "api-client/api",
                "api-client/db",
                "api-client/redis",
                "web/web",
                "web/db",
                "worker/worker",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        );
        // The demo workspace is running: every row says so, in plain text.
        assert!(
            modal.groups.iter().flat_map(|g| &g.rows).all(|r| {
                r.state == "running" && r.tone.is_none()
            })
        );
    }

    /// A workspace that never started shows what is stopped rather than an empty box.
    #[test]
    fn a_stopped_workspace_lists_what_is_stopped() {
        let mut sim = Sim::new();
        sim.ws.up = false;
        let modal = sim.services_modal();
        assert_eq!(modal.note, None);
        assert!(
            modal.groups.iter().flat_map(|g| &g.rows).all(|r| {
                r.state == "stopped" && r.tone == Some(Tone::Warn)
            })
        );
        // And the dot says so too.
        let dot = sim.service_indicator().expect("services to show");
        assert_eq!(dot.label, "services down");
    }

    /// A workspace with no services has no dot at the bottom: nothing to run is nothing to
    /// show, and the modal is unreachable from it.
    #[test]
    fn a_workspace_with_no_services_has_no_dot() {
        let mut sim = Sim::new();
        sim.repos.clear();
        assert_eq!(sim.service_indicator(), None);
        let modal = sim.services_modal();
        assert!(modal.groups.is_empty() && modal.note.is_none());
    }
}
