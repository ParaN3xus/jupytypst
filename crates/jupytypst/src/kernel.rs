use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use jupyter_protocol::{
    CodeMirrorMode, CommInfoReply, CompleteReply, DisplayData, ErrorOutput, ExecuteInput,
    ExecuteReply, ExecutionCount, HistoryReply, InspectReply, IoPubWelcome, IsCompleteReply,
    IsCompleteReplyStatus, JupyterMessage, JupyterMessageContent, KernelInfoReply, LanguageInfo,
    Media, MediaType, ReplyError, ReplyStatus, ShutdownReply, Status, StreamContent,
};
use runtimelib::{
    RouterRecvConnection, RouterSendConnection, SubscriptionEvent,
    create_kernel_control_connection, create_kernel_heartbeat_connection,
    create_kernel_iopub_xpub_connection, create_kernel_shell_connection,
    create_kernel_stdin_connection,
};
use uuid::Uuid;

use crate::cell::parse_cell;
use crate::output::{
    execution_output_to_html, format_diagnostics, format_diagnostics_rich_with_sources,
};
use crate::session::create_session;
use crate::{CODE_DISPLAY_NAME, MARKUP_DISPLAY_NAME};
use typsess::{
    ExecutionOutput, InputStatus, RenderMode, SourceMode, TypstReplSession, WorldOptions,
    classify_input,
};

pub const JUPYTER_PROTOCOL_VERSION: &str = "5.3";
const KERNEL_IMPLEMENTATION: &str = env!("CARGO_PKG_NAME");
const KERNEL_IMPLEMENTATION_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn run(
    connection_file: PathBuf,
    page_setup: String,
    default_mode: RenderMode,
    source_mode: SourceMode,
    world_options: WorldOptions,
) -> Result<()> {
    let bytes = std::fs::read(&connection_file)
        .with_context(|| format!("failed to read {}", connection_file.display()))?;
    let connection_info =
        serde_json::from_slice(&bytes).context("failed to parse connection file")?;
    KernelServer::run(
        connection_info,
        page_setup,
        default_mode,
        source_mode,
        world_options,
    )
    .await
}

struct KernelServer {
    execution_count: ExecutionCount,
    iopub: runtimelib::KernelIoPubXPubConnection,
    shell: RouterSendConnection,
    typst: TypstReplSession,
    default_mode: RenderMode,
    source_mode: SourceMode,
}

impl KernelServer {
    async fn run(
        connection_info: jupyter_protocol::ConnectionInfo,
        page_setup: String,
        default_mode: RenderMode,
        source_mode: SourceMode,
        world_options: WorldOptions,
    ) -> Result<()> {
        let session_id = Uuid::new_v4().to_string();
        let mut heartbeat = create_kernel_heartbeat_connection(&connection_info).await?;
        let shell_connection =
            create_kernel_shell_connection(&connection_info, &session_id).await?;
        let (shell_writer, shell_reader) = shell_connection.split();
        let mut control = create_kernel_control_connection(&connection_info, &session_id).await?;
        let _stdin = create_kernel_stdin_connection(&connection_info, &session_id).await?;
        let iopub = create_kernel_iopub_xpub_connection(&connection_info, &session_id).await?;

        let heartbeat_handle =
            tokio::spawn(async move { while heartbeat.single_heartbeat().await.is_ok() {} });

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let control_handle = tokio::spawn(async move {
            while let Ok(message) = control.read().await {
                match &message.content {
                    JupyterMessageContent::KernelInfoRequest(_) => {
                        let _ = control
                            .send(kernel_info(source_mode).as_child_of(&message))
                            .await;
                    }
                    JupyterMessageContent::ShutdownRequest(req) => {
                        let reply = ShutdownReply {
                            restart: req.restart,
                            status: ReplyStatus::Ok,
                            error: None,
                        }
                        .as_child_of(&message);
                        let _ = control.send(reply).await;
                        let _ = shutdown_tx.send(());
                        return;
                    }
                    _ => {}
                }
            }
        });

        let mut kernel = Self {
            execution_count: ExecutionCount::new(0),
            iopub,
            shell: shell_writer,
            typst: create_session(default_mode, source_mode, page_setup, world_options)
                .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?,
            default_mode,
            source_mode,
        };
        let shell_handle =
            tokio::spawn(async move { kernel.shell_loop(shell_reader, shutdown_rx).await });

        tokio::select! {
            _ = heartbeat_handle => {}
            _ = control_handle => {}
            _ = shell_handle => {}
        }

        Ok(())
    }

    async fn shell_loop(
        &mut self,
        mut shell_reader: RouterRecvConnection,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                result = shell_reader.read() => {
                    match result {
                        Ok(message) => {
                            if let Err(error) = self.handle_shell_message(&message).await {
                                eprintln!("jupytypst kernel error: {error:#}");
                            }
                        }
                        Err(error) => {
                            eprintln!("jupytypst shell read error: {error}");
                            break;
                        }
                    }
                }
                result = self.iopub.recv_subscription() => {
                    match result {
                        Ok(SubscriptionEvent::Subscribe(topic)) => {
                            let welcome: JupyterMessage = IoPubWelcome::new(topic).into();
                            if let Err(error) = self.iopub.send(welcome).await {
                                eprintln!("jupytypst iopub welcome error: {error}");
                            }
                        }
                        Ok(SubscriptionEvent::Unsubscribe(_)) => {}
                        Err(error) => eprintln!("jupytypst iopub subscription error: {error}"),
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
    }

    async fn handle_shell_message(&mut self, parent: &JupyterMessage) -> Result<()> {
        self.iopub.send(Status::busy().as_child_of(parent)).await?;

        match &parent.content {
            JupyterMessageContent::KernelInfoRequest(_) => {
                self.shell
                    .send(kernel_info(self.source_mode).as_child_of(parent))
                    .await?;
            }
            JupyterMessageContent::ExecuteRequest(request) => {
                self.handle_execute_request(&request.code, parent).await?;
            }
            JupyterMessageContent::CompleteRequest(request) => {
                let reply = CompleteReply {
                    matches: vec![],
                    cursor_start: request.cursor_pos,
                    cursor_end: request.cursor_pos,
                    metadata: Default::default(),
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::InspectRequest(_) => {
                let reply = InspectReply {
                    found: false,
                    data: Media::default(),
                    metadata: Default::default(),
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::IsCompleteRequest(request) => {
                let status = match classify_input(&request.code, self.source_mode) {
                    InputStatus::Complete => IsCompleteReplyStatus::Complete,
                    InputStatus::Incomplete(_) => IsCompleteReplyStatus::Incomplete,
                    InputStatus::Invalid(_) => IsCompleteReplyStatus::Invalid,
                };
                let reply = IsCompleteReply {
                    status,
                    indent: String::new(),
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::HistoryRequest(_) => {
                let reply = HistoryReply {
                    history: vec![],
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::CommInfoRequest(_) => {
                let reply = CommInfoReply {
                    status: ReplyStatus::Ok,
                    comms: Default::default(),
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            JupyterMessageContent::ShutdownRequest(request) => {
                let reply = ShutdownReply {
                    restart: request.restart,
                    status: ReplyStatus::Ok,
                    error: None,
                };
                self.shell.send(reply.as_child_of(parent)).await?;
            }
            _ => {}
        }

        self.iopub.send(Status::idle().as_child_of(parent)).await?;
        Ok(())
    }

    async fn handle_execute_request(&mut self, code: &str, parent: &JupyterMessage) -> Result<()> {
        self.execution_count.0 += 1;
        let execution_count = self.execution_count;

        self.iopub
            .send(
                ExecuteInput {
                    code: code.to_string(),
                    execution_count,
                }
                .as_child_of(parent),
            )
            .await?;

        let result = match parse_cell(code, self.default_mode) {
            Ok(cell) => {
                self.typst
                    .execute_with_mode(&cell.body, cell.mode)
                    .map_err(|diagnostics| {
                        format_diagnostics_rich_with_sources(
                            diagnostics,
                            self.typst.diagnostic_sources(),
                        )
                    })
            }
            Err(error) => Err(error.to_string()),
        };

        let reply = match result {
            Ok(result) => {
                for warning in result.warnings {
                    self.iopub
                        .send(
                            StreamContent::stderr(&format!("{}\n", warning.message))
                                .as_child_of(parent),
                        )
                        .await?;
                }
                self.publish_output(result.output, parent).await?;
                ExecuteReply {
                    status: ReplyStatus::Ok,
                    execution_count,
                    payload: vec![],
                    user_expressions: None,
                    error: None,
                }
            }
            Err(error) => {
                let evalue = error.to_string();
                let traceback = traceback_lines(&evalue);
                let error_output = ErrorOutput {
                    ename: "TypstError".to_string(),
                    evalue: evalue.clone(),
                    traceback: traceback.clone(),
                };
                self.iopub.send(error_output.as_child_of(parent)).await?;
                ExecuteReply {
                    status: ReplyStatus::Error,
                    execution_count,
                    payload: vec![],
                    user_expressions: None,
                    error: Some(Box::new(ReplyError {
                        ename: "TypstError".to_string(),
                        evalue,
                        traceback,
                    })),
                }
            }
        };
        self.shell.send(reply.as_child_of(parent)).await?;
        Ok(())
    }

    async fn publish_output(
        &mut self,
        output: ExecutionOutput,
        parent: &JupyterMessage,
    ) -> Result<()> {
        let plain = match &output {
            ExecutionOutput::Paged(_) => "<svg>",
            ExecutionOutput::Html(_) => "<html>",
        };
        let html = execution_output_to_html(output)
            .map_err(|diagnostics| anyhow!(format_diagnostics(diagnostics)))?;
        let media = Media::new(vec![
            MediaType::Html(html),
            MediaType::Plain(plain.to_string()),
        ]);
        self.iopub
            .send(DisplayData::new(media).as_child_of(parent))
            .await?;
        Ok(())
    }
}

fn traceback_lines(message: &str) -> Vec<String> {
    let lines = message.lines().map(str::to_string).collect::<Vec<_>>();
    if lines.is_empty() {
        vec![message.to_string()]
    } else {
        lines
    }
}

fn kernel_info(source_mode: SourceMode) -> KernelInfoReply {
    let language = language_metadata(source_mode);
    KernelInfoReply {
        status: ReplyStatus::Ok,
        protocol_version: JUPYTER_PROTOCOL_VERSION.to_string(),
        implementation: KERNEL_IMPLEMENTATION.to_string(),
        implementation_version: KERNEL_IMPLEMENTATION_VERSION.to_string(),
        language_info: LanguageInfo {
            name: language.name.to_string(),
            version: typst::syntax::package::PackageVersion::compiler().to_string(),
            mimetype: Some(language.mimetype.to_string()),
            file_extension: Some(language.file_extension.to_string()),
            pygments_lexer: Some(language.name.to_string()),
            codemirror_mode: Some(CodeMirrorMode::Simple(language.name.to_string())),
            nbconvert_exporter: None,
        },
        banner: language.display_name.to_string(),
        help_links: vec![],
        debugger: false,
        error: None,
    }
}

struct LanguageMetadata {
    name: &'static str,
    display_name: &'static str,
    mimetype: &'static str,
    file_extension: &'static str,
}

fn language_metadata(source_mode: SourceMode) -> LanguageMetadata {
    match source_mode {
        SourceMode::Code => LanguageMetadata {
            name: "typst-code",
            display_name: CODE_DISPLAY_NAME,
            mimetype: "text/x-typst-code",
            file_extension: ".typc",
        },
        SourceMode::Markup => LanguageMetadata {
            name: "typst",
            display_name: MARKUP_DISPLAY_NAME,
            mimetype: "text/x-typst",
            file_extension: ".typ",
        },
    }
}
