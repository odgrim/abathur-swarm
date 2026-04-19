//! Integration / cross-cutting tests for the convergence engine.
//!
//! Tests in this module exercise engine-level helpers that don't belong
//! to a single sibling module (e.g., `attractor_type_name`, which lives
//! in `convergence_engine::mod`).

use crate::domain::models::convergence::*;

use super::test_engine;

// -----------------------------------------------------------------------
// Helper method tests
// -----------------------------------------------------------------------

#[test]
fn test_attractor_type_name() {
    let engine = test_engine();

    assert_eq!(
        engine.attractor_type_name(&AttractorType::FixedPoint {
            estimated_remaining_iterations: 3,
            estimated_remaining_tokens: 60_000,
        }),
        "fixed_point"
    );
    assert_eq!(
        engine.attractor_type_name(&AttractorType::LimitCycle {
            period: 2,
            cycle_signatures: vec![],
        }),
        "limit_cycle"
    );
    assert_eq!(
        engine.attractor_type_name(&AttractorType::Divergent {
            divergence_rate: -0.1,
            probable_cause: DivergenceCause::Unknown,
        }),
        "divergent"
    );
    assert_eq!(
        engine.attractor_type_name(&AttractorType::Plateau {
            stall_duration: 5,
            plateau_level: 0.5,
        }),
        "plateau"
    );
    assert_eq!(
        engine.attractor_type_name(&AttractorType::Indeterminate {
            tendency: ConvergenceTendency::Flat,
        }),
        "indeterminate"
    );
}
