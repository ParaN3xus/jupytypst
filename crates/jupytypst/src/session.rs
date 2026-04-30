use std::sync::Arc;

use typsess::{
    RenderMode, SessionOptions, SessionPersistence, SessionState, SourceMode, TypstReplSession,
    WorldOptions,
};

use crate::persist::{collect_introspection_updates, filter_persistent_styles};

pub fn create_session(
    render_mode: RenderMode,
    source_mode: SourceMode,
    page_setup: String,
    world_options: WorldOptions,
) -> typst::diag::SourceResult<TypstReplSession> {
    let state = initial_state(&page_setup, world_options.clone())?;
    TypstReplSession::new(SessionOptions {
        render_mode,
        source_mode,
        world_options,
        state,
        persistence: jupytypst_persistence(),
    })
}

fn initial_state(
    page_setup: &str,
    world_options: WorldOptions,
) -> typst::diag::SourceResult<SessionState> {
    let mut session = TypstReplSession::new(SessionOptions {
        world_options,
        ..SessionOptions::default()
    })?;
    if !page_setup.is_empty() {
        session.apply_source(page_setup, SourceMode::Code)?;
    }
    Ok(session.into_state())
}

fn jupytypst_persistence() -> SessionPersistence {
    SessionPersistence {
        filter_styles: Arc::new(filter_persistent_styles),
        collect_introspection_updates: Arc::new(collect_introspection_updates),
    }
}
