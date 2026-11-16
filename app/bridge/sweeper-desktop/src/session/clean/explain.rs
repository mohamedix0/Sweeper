use sweeper_contract::ContractError;
use sweeper_core::analyzer;

use super::super::mapping::{analyzer_session_valid_at, risk_to_dto, scan_session_valid_at};
use super::super::{AnalyzerSession, ScanSession, SessionStore};
use crate::clock::Clock;

impl SessionStore {
    /// Explain an item by id (scan item id, analyzer node id, or target/knowledge id).
    pub fn explain_item<C: Clock>(
        &mut self,
        clock: &C,
        item_id: &str,
        current_settings_generation: u64,
    ) -> Result<sweeper_contract::ExplainItemResponse, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        self.explain_from_scan(now, item_id, current_settings_generation)
            .or_else(|| self.explain_from_analyzer(now, item_id, current_settings_generation))
            .or_else(|| Self::explain_from_knowledge(item_id))
            .ok_or_else(|| ContractError::NotFound {
                reason: "item not found".to_string(),
            })
    }

    fn explain_from_scan(
        &self,
        now: u64,
        item_id: &str,
        settings_gen: u64,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        for s in self.sessions.values() {
            if !scan_session_valid_at(s, now, settings_gen) {
                continue;
            }
            if let Some(resp) = Self::explain_scan_hit(s, item_id) {
                return Some(resp);
            }
        }
        None
    }

    fn explain_scan_hit(
        session: &ScanSession,
        item_id: &str,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        if let Some(item) = session.items.get(item_id) {
            return Some(sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: item.target_name.clone(),
                description: analyzer::target_description(&item.target_id)
                    .unwrap_or_else(|| "Scanned item.".to_string()),
                risk: risk_to_dto(item.risk),
            });
        }
        session
            .results
            .targets
            .iter()
            .find(|t| t.target_id == item_id)
            .map(|t| sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: t.target_name.clone(),
                description: analyzer::target_description(item_id)
                    .unwrap_or_else(|| "Target.".to_string()),
                risk: t.risk,
            })
    }

    fn explain_from_analyzer(
        &self,
        now: u64,
        item_id: &str,
        settings_gen: u64,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        self.analyzer_sessions
            .values()
            .filter(|s| analyzer_session_valid_at(s, now, settings_gen))
            .find_map(|s| Self::explain_analyzer_hit(s, item_id))
    }

    fn explain_analyzer_hit(
        session: &AnalyzerSession,
        item_id: &str,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        let node = analyzer::find_analyzer_node(&session.result.root, item_id)?;
        let title = if node.name.is_empty() {
            node.path.clone()
        } else {
            node.name.clone()
        };
        Some(sweeper_contract::ExplainItemResponse {
            item_id: item_id.to_string(),
            title,
            description: node.description.clone(),
            risk: risk_to_dto(node.risk),
        })
    }

    fn explain_from_knowledge(item_id: &str) -> Option<sweeper_contract::ExplainItemResponse> {
        analyzer::knowledge_explanation(item_id).map(|(title, description, risk)| {
            sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title,
                description,
                risk: risk_to_dto(risk),
            }
        })
    }
}
