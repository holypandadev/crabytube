//! Applies user preferences at the playback boundary so the engine and resolver ignore them.
use crate::{
    playback::{AudioQuality, StreamError, StreamResolver, TrackId, ValidatedPlaybackResolution},
    preferences::Quality,
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Default)]
pub struct QualitySetting(Arc<AtomicU8>);
impl QualitySetting {
    pub fn set(&self, value: Quality) {
        self.0.store(
            match value {
                Quality::Auto => 0,
                Quality::Normal => 1,
                Quality::High => 2,
            },
            Ordering::Relaxed,
        );
    }
    fn get(&self) -> AudioQuality {
        match self.0.load(Ordering::Relaxed) {
            1 => AudioQuality::Normal,
            2 => AudioQuality::High,
            _ => AudioQuality::Auto,
        }
    }
}
pub struct ConfiguredResolver {
    pub inner: Arc<dyn StreamResolver>,
    pub quality: QualitySetting,
}
#[async_trait::async_trait]
impl StreamResolver for ConfiguredResolver {
    async fn resolve(
        &self,
        track: &TrackId,
        _quality: AudioQuality,
        cancellation: CancellationToken,
    ) -> Result<ValidatedPlaybackResolution, StreamError> {
        self.inner
            .resolve(track, self.quality.get(), cancellation)
            .await
    }

    async fn prewarm(&self) {
        self.inner.prewarm().await;
    }
}
