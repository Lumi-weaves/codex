use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::shell_spec::create_configure_resource_audit_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ConfigureResourceAuditArgs {
    interval_seconds: Option<u64>,
}

pub struct ConfigureResourceAuditHandler;

impl ToolExecutor<ToolInvocation> for ConfigureResourceAuditHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("configure_resource_audit")
    }

    fn spec(&self) -> ToolSpec {
        create_configure_resource_audit_tool()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(async move {
            let ToolInvocation {
                session, payload, ..
            } = invocation;
            let ToolPayload::Function { arguments } = payload else {
                return Err(FunctionCallError::RespondToModel(
                    "configure_resource_audit handler received unsupported payload".to_string(),
                ));
            };
            let args: ConfigureResourceAuditArgs = parse_arguments(&arguments)?;
            let result = session
                .configure_resource_audit(args.interval_seconds)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            let content = serde_json::to_string(&result).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize configure_resource_audit response: {err}"
                ))
            })?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                content,
                Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for ConfigureResourceAuditHandler {}
