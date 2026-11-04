use hearthdeck_ai::categorization::ScanReport;
use hearthdeck_protocol::CategorizationSnapshot;
use tracing::{info, warn};

use super::{CategorizationService, ModelStatus, ScanRequest};
use crate::{
    collections::{self, CollectionStore},
    settings::SettingsRepository,
};

pub(crate) struct Control<'a> {
    pub service: Option<&'a CategorizationService>,
    pub settings: &'a SettingsRepository,
    pub collections: &'a CollectionStore,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ControlError {
    #[error("categorization unavailable")]
    Unavailable,
    #[error("categorization is not enabled")]
    Disabled,
    #[error("another categorization job is running")]
    Busy,
    #[error("the model is not downloaded yet")]
    ModelAbsent,
    #[error("categorization storage failed: {0:#}")]
    Storage(#[from] anyhow::Error),
}

impl Control<'_> {
    pub async fn snapshot(&self) -> Result<CategorizationSnapshot<ScanReport>, ControlError> {
        let service = self.service.ok_or(ControlError::Unavailable)?;
        let enabled = self.settings.get().await?.categorization_enabled;
        Ok(CategorizationSnapshot {
            enabled,
            status: service.status().await,
            report: service.latest_report().await?,
        })
    }

    pub async fn enable(&self) -> Result<(), ControlError> {
        let service = self.service.ok_or(ControlError::Unavailable)?;
        self.settings.set_categorization_enabled(true).await?;
        match service.request_download().await {
            ScanRequest::Started => info!("categorization enabled; model is ready or downloading"),
            ScanRequest::AlreadyRunning => {
                info!("categorization enabled; a job is already running")
            }
        }
        Ok(())
    }

    pub async fn disable(&self) -> Result<(), ControlError> {
        self.settings.set_categorization_enabled(false).await?;
        self.remove_collections().await;
        if let Some(service) = self.service {
            match service.clear_traits().await {
                Ok(0) => {}
                Ok(count) => info!(count, "categorization verdicts removed"),
                Err(error) => warn!(%error, "could not remove the categorization verdicts"),
            }
        }
        info!("categorization disabled");
        Ok(())
    }

    pub async fn remove_collections(&self) {
        if let Some(service) = self.service {
            service.discard_running().await;
        }
        match self.collections.remove_owned(collections::OWNER_LAYA).await {
            Ok(0) => {}
            Ok(count) => info!(count, "categorization rails removed"),
            Err(error) => warn!(%error, "could not remove the categorization rails"),
        }
    }

    pub async fn detect_form_factors(&self) -> Result<(), ControlError> {
        let service = self.enabled_service().await?;
        let status = service.status().await;
        if status.running {
            return Err(ControlError::Busy);
        }
        if status.model == ModelStatus::Absent {
            return Err(ControlError::ModelAbsent);
        }
        let service = service.clone();
        tokio::spawn(async move {
            match service.classify_pending().await {
                Ok(run) => info!(?run, "form-factor detection finished"),
                Err(error) => warn!(%error, "form-factor detection failed"),
            }
        });
        Ok(())
    }

    pub async fn publish_collections(&self) -> Result<(), ControlError> {
        match self.enabled_service().await?.request_scan().await {
            ScanRequest::Started => info!("categorization scan started"),
            ScanRequest::AlreadyRunning => info!("categorization scan coalesced"),
        }
        Ok(())
    }

    async fn enabled_service(&self) -> Result<&CategorizationService, ControlError> {
        let service = self.service.ok_or(ControlError::Unavailable)?;
        if !self.settings.get().await?.categorization_enabled {
            return Err(ControlError::Disabled);
        }
        Ok(service)
    }
}
