use serde::Serialize;

use crate::commands::impact::ImpactReport;

#[derive(Debug, Serialize)]
pub struct MemberInfo {
    pub name: String,
    pub kind: String,
    pub line: Option<i64>,
    pub is_exported: bool,
}

#[derive(Debug, Serialize)]
pub struct ExplainReport {
    pub symbol: String,
    pub id: String,
    pub kind: String,
    pub path: Option<String>,
    pub language: Option<String>,
    pub start_line: Option<i64>,
    pub end_line: Option<i64>,
    pub is_exported: bool,
    /// Subsystem (community) the symbol belongs to, when one was detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub members: Vec<MemberInfo>,
    /// One-hop callers/callees with risk flags, from the same engine as `impact`.
    pub relations: ImpactReport,
}
