pub mod aggregate;
pub mod alias;
pub mod coefficients;
pub mod ingest;
pub mod model;
pub mod normalize;
pub mod required_aliases;
pub mod score;
pub mod scoreboard;
pub mod synthesize;

pub use alias::{AliasIndex, normalize_name, normalize_vendor_hint};
pub use coefficients::{Coefficients, MetricDef, MetricTransform, SynthesisConfig};
pub use ingest::{IngestStats, ingest_rows, warn_stale_overrides};
pub use model::{
    GroupKey, MetricKey, MissingInfo, ModelRecord, RawRow, RoleScores, SourceId,
    SynthesisProvenance, ThinkingEffort, Vendor,
};
pub use score::{compute_scores, compute_scores_with};
pub use scoreboard::{SCHEMA_VERSION, Scoreboard, SourceSummary};
pub use synthesize::{
    SynthesisStats, load_embedded_pairs, load_pairs_from_str, synthesize_rows,
    warn_stale_synthesis_pairs,
};
