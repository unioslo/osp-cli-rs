//! Stable parsed DSL data structures.
//!
//! These types are the boundary between the parser and later compilation or
//! execution. They intentionally preserve the original raw text alongside the
//! parser's stage classification so diagnostics and traces can still explain
//! what the user typed without reparsing.

/// High-level parser classification for a stage token.
///
/// The parser deliberately separates "known explicit verb", "unknown
/// verb-shaped token", and "quick-search text" so the evaluator does not have
/// to guess later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedStageKind {
    Explicit,
    UnknownExplicit,
    Quick,
}

/// One stage after the parser has decided how the evaluator should treat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStage {
    pub kind: ParsedStageKind,
    pub verb: String,
    pub spec: String,
    pub raw: String,
}

impl ParsedStage {
    /// Creates a parsed stage with explicit kind, verb, spec, and raw text.
    pub fn new(
        kind: ParsedStageKind,
        verb: impl Into<String>,
        spec: impl Into<String>,
        raw: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            verb: verb.into(),
            spec: spec.into(),
            raw: raw.into(),
        }
    }
}

/// Full parsed pipeline used by the evaluator.
///
/// `raw` is preserved for trace/debug output. `stages` carries the structured
/// stage classification that drives execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPipeline {
    pub raw: String,
    pub stages: Vec<ParsedStage>,
}

/// The sole execution substrate. Ungrouped data has one partition (even when
/// empty); G creates named partitions. Keys and aggregates are partition
/// metadata, never synthetic fields inserted into member rows.
pub(crate) struct RowSet {
    pub partitions: Vec<crate::core::output_model::Group>,
    pub grouped: bool,
}

impl RowSet {
    pub fn rows(rows: Vec<crate::core::row::Row>) -> Self {
        Self {
            partitions: vec![crate::core::output_model::Group {
                groups: Default::default(),
                aggregates: Default::default(),
                rows,
            }],
            grouped: false,
        }
    }

    pub fn map_rows(
        mut self,
        mut transform: impl FnMut(
            Vec<crate::core::row::Row>,
        ) -> anyhow::Result<Vec<crate::core::row::Row>>,
    ) -> anyhow::Result<Self> {
        for partition in &mut self.partitions {
            partition.rows = transform(std::mem::take(&mut partition.rows))?;
        }
        Ok(self)
    }
}

impl From<crate::core::output_model::OutputItems> for RowSet {
    fn from(items: crate::core::output_model::OutputItems) -> Self {
        use crate::core::output_model::OutputItems;
        match items {
            OutputItems::Rows(rows) => Self::rows(rows),
            OutputItems::Groups(partitions) => Self {
                partitions,
                grouped: true,
            },
        }
    }
}

impl From<RowSet> for crate::core::output_model::OutputItems {
    fn from(set: RowSet) -> Self {
        if set.grouped {
            Self::Groups(set.partitions)
        } else {
            Self::Rows(
                set.partitions
                    .into_iter()
                    .flat_map(|partition| partition.rows)
                    .collect(),
            )
        }
    }
}
