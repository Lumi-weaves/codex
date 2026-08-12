use std::collections::HashMap;

use serde::Serialize;
use serde_json::json;

use crate::codex_thread::BackgroundTerminalInfo;

use super::ContextualUserFragment;
use super::bound_fragment_text;

const RESOURCE_AUDIT_MAX_ENTRIES: usize = 16;
const RESOURCE_AUDIT_COMMAND_MAX_BYTES: usize = 128;
const RESOURCE_AUDIT_CWD_MAX_BYTES: usize = 256;
const RESOURCE_AUDIT_ITEM_ID_MAX_BYTES: usize = 64;
const RENDERED_FRAGMENT_MAX_BYTES: usize = 24 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResourceAuditEntry {
    kind: &'static str,
    process_id: i32,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

/// Output-free consolidated view of resources owned by one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnifiedExecResourceAuditEvent {
    sequence: u64,
    interval_seconds: u64,
    active_resource_count: usize,
    omitted_resource_count: usize,
    resources: Vec<ResourceAuditEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ResourceAuditConfiguration {
    pub(crate) interval_seconds: u64,
    pub(crate) armed: bool,
    pub(crate) active_resource_count: usize,
}

impl UnifiedExecResourceAuditEvent {
    pub(crate) fn new(
        sequence: u64,
        interval_seconds: u64,
        mut awaited_ids: Vec<i32>,
        live_terminals: Vec<BackgroundTerminalInfo>,
    ) -> Self {
        awaited_ids.sort_unstable();
        awaited_ids.dedup();
        let active_resource_count = awaited_ids.len();
        let omitted_resource_count =
            active_resource_count.saturating_sub(RESOURCE_AUDIT_MAX_ENTRIES);
        awaited_ids.truncate(RESOURCE_AUDIT_MAX_ENTRIES);

        let live_by_id = live_terminals
            .into_iter()
            .filter_map(|terminal| {
                terminal
                    .process_id
                    .parse::<i32>()
                    .ok()
                    .map(|id| (id, terminal))
            })
            .collect::<HashMap<_, _>>();
        let resources = awaited_ids
            .into_iter()
            .map(|process_id| {
                let terminal = live_by_id.get(&process_id);
                ResourceAuditEntry {
                    kind: "unified_exec_terminal",
                    process_id,
                    state: if terminal.is_some() {
                        "running"
                    } else {
                        "awaiting_completion_ingress"
                    },
                    item_id: terminal.map(|terminal| {
                        bound_fragment_text(&terminal.item_id, RESOURCE_AUDIT_ITEM_ID_MAX_BYTES)
                    }),
                    command: terminal.map(|terminal| {
                        bound_fragment_text(&terminal.command, RESOURCE_AUDIT_COMMAND_MAX_BYTES)
                    }),
                    cwd: terminal.map(|terminal| {
                        bound_fragment_text(
                            &terminal.cwd.inferred_native_path_string(),
                            RESOURCE_AUDIT_CWD_MAX_BYTES,
                        )
                    }),
                }
            })
            .collect();

        Self {
            sequence,
            interval_seconds,
            active_resource_count,
            omitted_resource_count,
            resources,
        }
    }
}

impl ContextualUserFragment for UnifiedExecResourceAuditEvent {
    fn role(&self) -> &'static str {
        "user"
    }

    fn requires_separate_message(&self) -> bool {
        true
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<resource_audit>", "</resource_audit>")
    }

    fn body(&self) -> String {
        let payload = json!({
            "status": "Periodic owner-local audit of resources that still keep this task active.",
            "sequence": self.sequence,
            "interval_seconds": self.interval_seconds,
            "active_resource_count": self.active_resource_count,
            "omitted_resource_count": self.omitted_resource_count,
            "resources": self.resources,
            "scope": "owning_session",
            "output_policy": "No terminal output is included. Use write_stdin or read_terminal_result only when inspection is needed.",
        });
        let escaped = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let escaped = escaped.replace('<', "\\u003c").replace('>', "\\u003e");
        let body = format!("\n{escaped}\n");
        debug_assert!(
            body.len() <= RENDERED_FRAGMENT_MAX_BYTES,
            "rendered resource-audit fragment exceeded its documented bound"
        );
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_path_uri::PathUri;

    #[test]
    fn audit_uses_awaited_ids_as_authority_and_bounds_untrusted_fields() {
        let event = UnifiedExecResourceAuditEvent::new(
            7,
            300,
            vec![42, 41],
            vec![BackgroundTerminalInfo {
                item_id: "i".repeat(1000),
                process_id: "41".to_string(),
                command: "c".repeat(1000),
                cwd: PathUri::from_host_native_path(std::path::PathBuf::from(format!(
                    "/{}",
                    "d".repeat(1000)
                )))
                .expect("absolute test path"),
            }],
        );

        assert_eq!(event.active_resource_count, 2);
        assert_eq!(event.resources[0].process_id, 41);
        assert_eq!(event.resources[0].state, "running");
        assert_eq!(event.resources[1].process_id, 42);
        assert_eq!(event.resources[1].state, "awaiting_completion_ingress");
        assert!(
            event.resources[0].item_id.as_ref().unwrap().len() <= RESOURCE_AUDIT_ITEM_ID_MAX_BYTES
        );
        assert!(
            event.resources[0].command.as_ref().unwrap().len() <= RESOURCE_AUDIT_COMMAND_MAX_BYTES
        );
        assert!(event.resources[0].cwd.as_ref().unwrap().len() <= RESOURCE_AUDIT_CWD_MAX_BYTES);
        assert!(event.render().len() <= RENDERED_FRAGMENT_MAX_BYTES);
    }

    #[test]
    fn audit_caps_entry_count_without_losing_truthful_total() {
        let event = UnifiedExecResourceAuditEvent::new(1, 300, (0..100).collect(), Vec::new());
        assert_eq!(event.active_resource_count, 100);
        assert_eq!(event.resources.len(), RESOURCE_AUDIT_MAX_ENTRIES);
        assert_eq!(event.omitted_resource_count, 84);
    }
}
