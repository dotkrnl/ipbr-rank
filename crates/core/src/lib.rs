pub mod alias;
pub mod coefficients;
pub mod ingest;
pub mod model;
pub mod normalize;
pub mod required_aliases;
pub mod score;
pub mod scoreboard;

pub use alias::{
    AliasCollision, AliasIndex, normalize_name, normalize_vendor_hint, warn_alias_collisions,
};
pub use coefficients::{
    AggregationConfig, Coefficients, EffortException, EffortPolicy, EvidenceConfig, MetricDef,
    MetricEligibility, MetricTransform, NormalizationConfig,
};
pub use ingest::{
    IngestStats, audit_fuzzy_matches, ingest_rows, ingest_rows_with_policy, warn_stale_overrides,
};
pub use model::{
    EligibilityQualificationPath, EvidenceCoverage, EvidenceSummary, GroupKey, MetricKey,
    MissingInfo, ModelRecord, RawRow, RoleScores, SourceId, Vendor,
};
pub use score::{balanced_is_provisional, balanced_is_provisional_with, compute_scores_with};
pub use scoreboard::{SCHEMA_VERSION, Scoreboard, SourceSummary};
