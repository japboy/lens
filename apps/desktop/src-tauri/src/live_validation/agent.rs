//! In-process ACP transport; production session and output admission remain unchanged.
use super::ports::Journal;
use crate::{agent, agent_runtime::ResolvedAgentRuntime, model::AgentKind};
use agent_client_protocol::{
    schema::{v1::*, ProtocolVersion},
    Agent, Client, ConnectionTo, DynConnectTo, Responder,
};
use std::sync::Arc;

pub(crate) fn services<R: tauri::Runtime>(journal: Arc<Journal>) -> agent::AgentServices<R> {
    agent::AgentServices(Arc::new(ScriptedHost(journal)))
}
struct ScriptedHost(Arc<Journal>);
fn options() -> Vec<SessionConfigOption> {
    vec![SessionConfigOption::select(
        "mode",
        "Mode",
        "safe",
        vec![SessionConfigSelectOption::new("safe", "Safe")],
    )
    .category(SessionConfigOptionCategory::Mode)]
}
impl<R: tauri::Runtime> agent::AgentHost<R> for ScriptedHost {
    fn output_server(&self) -> Result<std::path::PathBuf, String> {
        crate::output_mcp::bundled_executable()
    }

    fn resolve<'a>(
        &'a self,
        _: &'a tauri::AppHandle<R>,
        kind: AgentKind,
    ) -> agent::HostFuture<'a, ResolvedAgentRuntime> {
        Box::pin(async move {
            Ok(ResolvedAgentRuntime {
                kind,
                adapter_name: "live-validation-acp",
                adapter_version: "1",
                safe_mode_id: "safe",
                command: "/live-validation/never-executed".into(),
                args: vec![],
            })
        })
    }
    fn resolve_installed<'a>(
        &'a self,
        _: &'a tauri::AppHandle<R>,
        _: AgentKind,
    ) -> agent::HostFuture<'a, Option<ResolvedAgentRuntime>> {
        Box::pin(async { Err("Live validation does not restore Agent installations".into()) })
    }
    fn connect(&self, _: &agent::AgentDescriptor) -> DynConnectTo<Client> {
        let journal = self.0.clone();
        DynConnectTo::new(Agent.builder()
            .on_receive_request(async move |_: InitializeRequest, responder: Responder<InitializeResponse>, _| {
                responder.respond(InitializeResponse::new(ProtocolVersion::V1).agent_capabilities(
                    AgentCapabilities::new().prompt_capabilities(PromptCapabilities::new().image(true))))
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(async move |_: NewSessionRequest, responder: Responder<NewSessionResponse>, _| {
                responder.respond(NewSessionResponse::new("live-validation-session").config_options(options()))
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(async move |_: SetSessionConfigOptionRequest, responder: Responder<SetSessionConfigOptionResponse>, _| {
                responder.respond(SetSessionConfigOptionResponse::new(options()))
            }, agent_client_protocol::on_receive_request!())
            .on_receive_request(async move |request: PromptRequest, responder: Responder<PromptResponse>, connection: ConnectionTo<Client>| {
                journal.record_prompt();
                connection.send_notification(SessionNotification::new(request.session_id,
                    SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
                        "Synthetic local validation interpretation; no external Agent was contacted."))))))?;
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            }, agent_client_protocol::on_receive_request!()))
    }
}
