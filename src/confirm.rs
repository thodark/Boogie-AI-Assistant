use std::time::Instant;

const TIMEOUT_SECS: u64 = 90;

#[derive(Clone)]
pub struct PendingAction {
    pub key: String,
    pub title: String,
    pub detail: String,
    pub created: Instant,
}

pub struct ConfirmGate {
    pending: Option<PendingAction>,
}

impl ConfirmGate {
    pub fn new() -> Self {
        Self { pending: None }
    }

    pub fn request(&mut self, key: &str, title: &str, detail: &str) -> String {
        if let Some(p) = &self.pending {
            if p.created.elapsed().as_secs() > TIMEOUT_SECS {
                self.pending = None;
            } else {
                return format!(
                    "There is already a confirmation pending: '{}'. The user must respond to that first.",
                    p.title
                );
            }
        }
        self.pending = Some(PendingAction {
            key: key.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            created: Instant::now(),
        });
        format!(
            "CONFIRMATION REQUIRED: {} — {}. A confirmation dialog is now showing on the user's screen. \
             Do NOT proceed or claim it is done. Wait for the user to confirm or deny.",
            title, detail
        )
    }

    pub fn pending(&self) -> Option<&PendingAction> {
        if let Some(p) = &self.pending {
            if p.created.elapsed().as_secs() > TIMEOUT_SECS {
                return None;
            }
            return Some(p);
        }
        None
    }

    pub fn confirm(&mut self) -> Option<String> {
        self.pending.take().map(|p| p.key)
    }

    pub fn deny(&mut self) -> Option<String> {
        self.pending.take().map(|p| p.key)
    }

    pub fn clear_expired(&mut self) {
        if let Some(p) = &self.pending {
            if p.created.elapsed().as_secs() > TIMEOUT_SECS {
                self.pending = None;
            }
        }
    }
}

impl Default for ConfirmGate {
    fn default() -> Self {
        Self::new()
    }
}
