//! Which business orders carry HKTAN process 4: one pure decision function.
//!
//! The answer comes from the selected security function, profile overrides,
//! facts learned from the bank, and HIPINS, in that order (first match wins).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::return_code::Kind;
use crate::types::SegmentType;

/// A bank profile's fixed answer for one order, beating HIPINS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TanOverride {
    /// Always send HKTAN with this order.
    Always,
    /// Never send HKTAN with this order.
    Never,
}

/// Per-order overrides, keyed by the order's segment code (e.g. `HKSAL`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TanOverrides(BTreeMap<SegmentType, TanOverride>);

impl TanOverrides {
    pub fn get(&self, code: &SegmentType) -> Option<TanOverride> {
        self.0.get(code).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (SegmentType, TanOverride)> + '_ {
        self.0.iter().map(|(c, o)| (c.clone(), *o))
    }
}

impl FromIterator<(SegmentType, TanOverride)> for TanOverrides {
    fn from_iter<I: IntoIterator<Item = (SegmentType, TanOverride)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Why [`order_tan`] answered as it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TanReason {
    /// PIN-only method 999: HKTAN is invalid.
    PinOnlyMethod,
    Override,
    Learned,
    Hipins,
    /// Not in HIPINS: under PIN/TAN such an order is not covered by the
    /// bank's TAN table (PIN/TAN data dictionary, HIPINS), so no HKTAN is
    /// sent; a `9075` answer still triggers the one retry with HKTAN.
    Unlisted,
    /// A payment order (HKCCS, HKIPZ): always TAN-required under a two-step
    /// function, whatever HIPINS says (PIN/TAN B.4.2; payments design 3.1).
    Payment,
}

/// The payment orders: HKTAN process 4 always goes with them.
pub(crate) fn is_payment_order(code: &str) -> bool {
    matches!(code, "HKCCS" | "HKIPZ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TanDecision {
    pub hktan: bool,
    pub reason: TanReason,
}

/// Does the order `code` need HKTAN process 4? `two_step` is false when the
/// selected security function is 999. First matching rule wins.
pub fn order_tan(
    code: &SegmentType,
    two_step: bool,
    hipins: Option<bool>,
    overrides: &TanOverrides,
    learned: &BTreeSet<SegmentType>,
) -> TanDecision {
    let (hktan, reason) = if !two_step {
        (false, TanReason::PinOnlyMethod)
    } else if let Some(o) = overrides.get(code) {
        (o == TanOverride::Always, TanReason::Override)
    } else if is_payment_order(code.as_str()) {
        (true, TanReason::Payment)
    } else if learned.contains(code) {
        (true, TanReason::Learned)
    } else if let Some(listed) = hipins {
        (listed, TanReason::Hipins)
    } else {
        (false, TanReason::Unlisted)
    };
    TanDecision { hktan, reason }
}

/// What to do after a bank answered an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Nothing to change; the caller sees the answer.
    No,
    /// 9075 without HKTAN: send the order once more with HKTAN, same dialog.
    SameDialog,
    /// 9075 with 9800: the bank ended the dialog. Learn, do not resend.
    DialogGone,
}

/// Classify an order answer by its return codes (segment and global).
/// 9075 ("strong authentication required") without HKTAN is the bank
/// asking for one; 9050 alone is the usual summary and means nothing here.
pub fn on_sca_required(decision: &TanDecision, codes: impl IntoIterator<Item = Kind>) -> Retry {
    if decision.hktan
        || matches!(
            decision.reason,
            TanReason::Override | TanReason::PinOnlyMethod
        )
    {
        return Retry::No;
    }
    let (mut sca, mut gone) = (false, false);
    for c in codes {
        sca |= c == Kind::ScaRequired;
        gone |= c == Kind::DialogAborted;
    }
    match (sca, gone) {
        (false, _) => Retry::No,
        (true, false) => Retry::SameDialog,
        (true, true) => Retry::DialogGone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(s: &str) -> SegmentType {
        SegmentType::new(s)
    }

    fn decide(
        code: &str,
        two_step: bool,
        hipins: &[(&str, bool)],
        overrides: &[(&str, TanOverride)],
        learned: &[&str],
    ) -> TanDecision {
        order_tan(
            &st(code),
            two_step,
            hipins.iter().find(|(c, _)| *c == code).map(|(_, b)| *b),
            &overrides.iter().map(|(c, o)| (st(c), *o)).collect(),
            &learned.iter().map(|c| st(c)).collect(),
        )
    }

    fn is(d: TanDecision, hktan: bool, reason: TanReason) {
        assert_eq!(d, TanDecision { hktan, reason });
    }

    #[test]
    fn row1_pin_only_method_sends_no_hktan() {
        is(
            decide(
                "HKSAL",
                false,
                &[("HKSAL", true)],
                &[("HKSAL", TanOverride::Always)],
                &["HKSAL"],
            ),
            false,
            TanReason::PinOnlyMethod,
        );
    }

    #[test]
    fn row2_override_never() {
        is(
            decide(
                "HKWPD",
                true,
                &[("HKWPD", true)],
                &[("HKWPD", TanOverride::Never)],
                &[],
            ),
            false,
            TanReason::Override,
        );
    }

    #[test]
    fn row3_override_always() {
        is(
            decide(
                "HKSAL",
                true,
                &[("HKSAL", false)],
                &[("HKSAL", TanOverride::Always)],
                &[],
            ),
            true,
            TanReason::Override,
        );
    }

    #[test]
    fn row4_learned() {
        is(
            decide("HKSAL", true, &[], &[], &["HKSAL"]),
            true,
            TanReason::Learned,
        );
    }

    #[test]
    fn row5_hipins_decides() {
        is(
            decide("HKKAZ", true, &[("HKKAZ", true)], &[], &[]),
            true,
            TanReason::Hipins,
        );
        is(
            decide("HKKAZ", true, &[("HKKAZ", false)], &[], &[]),
            false,
            TanReason::Hipins,
        );
    }

    #[test]
    fn row6_unlisted_sends_no_hktan() {
        is(
            decide("HKWDU", true, &[], &[], &[]),
            false,
            TanReason::Unlisted,
        );
        is(
            decide("HKSAL", true, &[], &[], &[]),
            false,
            TanReason::Unlisted,
        );
    }

    #[test]
    fn payment_orders_always_carry_hktan_unless_overridden() {
        for code in ["HKCCS", "HKIPZ"] {
            is(
                decide(code, true, &[(code, false)], &[], &[]),
                true,
                TanReason::Payment,
            );
            is(decide(code, true, &[], &[], &[]), true, TanReason::Payment);
            is(
                decide(code, false, &[], &[], &[]),
                false,
                TanReason::PinOnlyMethod,
            );
            is(
                decide(code, true, &[], &[(code, TanOverride::Never)], &[]),
                false,
                TanReason::Override,
            );
        }
    }

    #[test]
    fn precedence() {
        is(
            decide(
                "HKWPD",
                true,
                &[],
                &[("HKWPD", TanOverride::Never)],
                &["HKWPD"],
            ),
            false,
            TanReason::Override,
        );
        is(
            decide(
                "HKSAL",
                true,
                &[("HKSAL", false)],
                &[("HKSAL", TanOverride::Always)],
                &[],
            ),
            true,
            TanReason::Override,
        );
        is(
            decide("HKSAL", true, &[("HKSAL", false)], &[], &["HKSAL"]),
            true,
            TanReason::Learned,
        );
    }

    #[test]
    fn overrides_round_trip_as_json() {
        let o: TanOverrides = [(st("HKSAL"), TanOverride::Always)].into_iter().collect();
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(json, r#"{"HKSAL":"always"}"#);
        assert_eq!(serde_json::from_str::<TanOverrides>(&json).unwrap(), o);
    }

    #[test]
    fn retry_classification() {
        use Kind::*;
        let none = TanDecision {
            hktan: false,
            reason: TanReason::Hipins,
        };
        let sent = TanDecision {
            hktan: true,
            reason: TanReason::Hipins,
        };
        let never = TanDecision {
            hktan: false,
            reason: TanReason::Override,
        };
        assert_eq!(on_sca_required(&none, [ScaRequired]), Retry::SameDialog);
        assert_eq!(
            on_sca_required(&none, [PartialErrors, ScaRequired]),
            Retry::SameDialog
        );
        assert_eq!(
            on_sca_required(&none, [ScaRequired, DialogAborted]),
            Retry::DialogGone
        );
        assert_eq!(on_sca_required(&sent, [ScaRequired]), Retry::No);
        assert_eq!(on_sca_required(&never, [ScaRequired]), Retry::No);
        assert_eq!(on_sca_required(&none, [PartialErrors]), Retry::No);
        assert_eq!(on_sca_required(&none, [DialogAborted]), Retry::No);
    }
}
