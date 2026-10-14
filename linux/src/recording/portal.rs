//! Wayland screen capture through the ScreenCast desktop portal. The portal hands out PipeWire streams,
//! one per shared monitor. The first time, it asks the user which screens to share; the restore token
//! saved in the settings lets later recordings skip that dialog.

use std::os::fd::OwnedFd;

use anyhow::{anyhow, Context, Result};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream};
use ashpd::desktop::{PersistMode, Session};

use super::Rect;

/// An open screencast of one monitor. Closing it (or dropping the recording) ends the share.
pub struct PortalCapture {
    proxy: Screencast,
    session: Session<Screencast>,
    pub fd: OwnedFd,
    pub node_id: u32,
    /// The stream's size in logical pixels, to scale the region into the stream's real pixels.
    pub logical_size: (i32, i32),
}

pub enum OpenError {
    /// The user shared screens, but not the one the region is on.
    WrongScreen,
    Cancelled,
    Failed(anyhow::Error),
}

/// Opens a screencast and picks the stream of the monitor at `monitor`. Returns the capture and the
/// new restore token to save.
pub async fn open(
    monitor: Rect,
    monitor_count: usize,
    show_cursor: bool,
    restore_token: Option<&str>,
) -> std::result::Result<(PortalCapture, Option<String>), OpenError> {
    let (proxy, session, streams, token) = start(show_cursor, restore_token).await.map_err(|e| {
        // Closing the dialog surfaces as a "cancelled" response.
        if matches!(e.downcast_ref::<ashpd::Error>(), Some(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled))) {
            OpenError::Cancelled
        } else {
            OpenError::Failed(e)
        }
    })?;

    let Some(stream) = pick_stream(&streams, monitor, monitor_count) else {
        let _ = session.close().await;
        return Err(OpenError::WrongScreen);
    };
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .context("Couldn't connect to PipeWire")
        .map_err(OpenError::Failed)?;
    let logical_size = stream.size().unwrap_or((monitor.width, monitor.height));
    crate::log::write(format!(
        "Screencast stream {} at {:?}, size {:?}",
        stream.pipe_wire_node_id(),
        stream.position(),
        stream.size()
    ));
    let capture = PortalCapture { node_id: stream.pipe_wire_node_id(), proxy, session, fd, logical_size };
    Ok((capture, token))
}

impl PortalCapture {
    pub async fn close(self) {
        if let Err(e) = self.session.close().await {
            crate::log::error("Closing the screencast session", e);
        }
        drop(self.proxy);
    }
}

async fn start(
    show_cursor: bool,
    restore_token: Option<&str>,
) -> Result<(Screencast, Session<Screencast>, Vec<Stream>, Option<String>)> {
    let proxy = Screencast::new().await.context("The screen sharing portal isn't available")?;
    let session = proxy.create_session(Default::default()).await?;
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(if show_cursor { CursorMode::Embedded } else { CursorMode::Hidden })
                .set_sources(SourceType::Monitor)
                // Sharing several screens at once means one saved choice covers them all.
                .set_multiple(true)
                .set_restore_token(restore_token)
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await?;
    let response = proxy.start(&session, None, Default::default()).await?.response()?;
    let streams = response.streams().to_vec();
    let token = response.restore_token().map(str::to_owned);
    if streams.is_empty() {
        return Err(anyhow!("No screen was shared."));
    }
    Ok((proxy, session, streams, token))
}

/// The stream showing `monitor`: matched by position (and size when there's no position); a single
/// stream on a single-monitor setup always matches.
fn pick_stream(streams: &[Stream], monitor: Rect, monitor_count: usize) -> Option<Stream> {
    let by_position = streams.iter().find(|s| s.position() == Some((monitor.x, monitor.y)));
    let by_size = || {
        let same_size: Vec<_> = streams
            .iter()
            .filter(|s| s.position().is_none() && s.size() == Some((monitor.width, monitor.height)))
            .collect();
        (same_size.len() == 1).then(|| same_size[0])
    };
    by_position
        .or_else(by_size)
        .or_else(|| (streams.len() == 1 && monitor_count == 1).then(|| &streams[0]))
        .cloned()
}
