//! The "Rückmeldung" group shared by HIRMG and HIRMS. Formals 2017, data
//! dictionary "Rückmeldung".

use crate::codec::{An, DataElementGroup, Dig};

/// Rückmeldung, version 1: one return code with its text.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct Feedback1 {
    /// 1 Rückmeldungscode, `dig 4`, M. First digit: 0 success, 3 warning,
    /// 9 error (list: Rückmeldungscodes document).
    #[de(len = 4)]
    pub code: Dig,
    /// 2 Bezugsdatenelement, `an ..7`, C: O in HIRMS, N in HIRMG.
    #[de(max = 7)]
    pub reference_element: Option<An>,
    /// 3 Rückmeldungstext, `an ..80`, M. Read without the length limit:
    /// DKB sent a 93-byte text on a failed login (live run, 2026-11-27),
    /// and a rejected HIRMG or HIRMS would hide the bank's message and
    /// every return code with it. The text is only shown, never sent.
    pub text: An,
    /// 4 Rückmeldungsparameter, `an ..35`, O, up to 10. Only the
    /// parameters listed for the code may be set.
    #[de(max = 35, max_count = 10)]
    pub parameters: Vec<An>,
}
