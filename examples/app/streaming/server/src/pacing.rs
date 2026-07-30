// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Readiness-driven scheduling for feed checkpoints and the weather update.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::jitter::Jitter;
use crate::test_controls::TestSession;
use crate::weather;

pub(crate) const FEED_BATCH_COUNT: usize = 3;

/// Commands consumed by the one blocking worker that owns the response session.
pub(crate) enum RenderCommand {
    Weather(Value),
    Feed(usize),
    Finish,
}

/// Race backend readiness and preserve the order in which work becomes ready.
pub(crate) async fn drive(
    commands: mpsc::Sender<RenderCommand>,
    test_session: Option<Arc<TestSession>>,
    feed_delay_min_ms: u64,
    feed_delay_max_ms: u64,
) {
    let weather = weather::load_forecast(test_session.clone());
    tokio::pin!(weather);
    let mut weather_pending = true;
    let mut jitter = Jitter::from_clock();

    for batch in 0..FEED_BATCH_COUNT {
        let delay = jitter.delay_ms(feed_delay_min_ms, feed_delay_max_ms);
        let feed = wait_for_feed(test_session.as_deref(), batch, delay);
        tokio::pin!(feed);
        if weather_pending {
            tokio::select! {
                forecast = weather.as_mut() => {
                    if !send(&commands, RenderCommand::Weather(forecast)).await {
                        return;
                    }
                    weather_pending = false;
                    feed.await;
                }
                () = feed.as_mut() => {}
            }
        } else {
            feed.await;
        }

        if !send(&commands, RenderCommand::Feed(batch)).await {
            return;
        }
    }

    if weather_pending {
        let forecast = weather.as_mut().await;
        if !send(&commands, RenderCommand::Weather(forecast)).await {
            return;
        }
    }
    let _ = send(&commands, RenderCommand::Finish).await;
}

async fn wait_for_feed(
    test_session: Option<&TestSession>,
    batch: usize,
    delay: std::time::Duration,
) {
    if let Some(session) = test_session {
        session.wait_for_feed_gap(batch).await;
    } else {
        sleep(delay).await;
    }
}

async fn send(commands: &mpsc::Sender<RenderCommand>, command: RenderCommand) -> bool {
    commands.send(command).await.is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::{drive, RenderCommand, FEED_BATCH_COUNT};
    use crate::test_controls::TestControls;

    #[tokio::test]
    async fn ready_weather_can_arrive_between_feed_batches() {
        let controls = TestControls::default();
        let session = controls
            .session("interleaved")
            .unwrap_or_else(|| panic!("valid session"));
        let (sender, mut receiver) = mpsc::channel(1);
        let driver = tokio::spawn(drive(sender, Some(Arc::clone(&session)), 0, 0));

        session.release_next_feed_gap();
        assert!(matches!(
            receiver.recv().await,
            Some(RenderCommand::Feed(0))
        ));

        session.release_weather();
        assert!(matches!(
            receiver.recv().await,
            Some(RenderCommand::Weather(_))
        ));

        session.release_all();
        for expected in 1..FEED_BATCH_COUNT {
            assert!(matches!(
                receiver.recv().await,
                Some(RenderCommand::Feed(actual)) if actual == expected
            ));
        }
        assert!(matches!(receiver.recv().await, Some(RenderCommand::Finish)));
        driver
            .await
            .unwrap_or_else(|error| panic!("driver failed: {error}"));
    }
}
