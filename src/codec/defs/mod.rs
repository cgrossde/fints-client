//! Segment and group definitions, written from the FinTS 3.0 specification
//! (docs/spec-sources.md). One struct per segment version, one file per
//! segment code.

pub mod common;
pub mod feedback;
pub mod hibpa;
pub mod hicaz;
pub mod hicazs;
pub mod hiccss;
pub mod hiips;
pub mod hiipss;
pub mod hiipz;
pub mod hiipzs;
pub mod hikaz;
pub mod hikazs;
pub mod hipins;
pub mod hirmg;
pub mod hirms;
pub mod hisal;
pub mod hisals;
pub mod hispa;
pub mod hispas;
pub mod hisyn;
pub mod hitan;
pub mod hitans;
pub mod hiupa;
pub mod hiupd;
pub mod hivpas;
pub mod hivpp;
pub mod hivpps;
pub mod hiwdu;
pub mod hiwpd;
pub mod hiwpds;
pub mod hkcaz;
pub mod hkccs;
pub mod hkend;
pub mod hkidn;
pub mod hkips;
pub mod hkipz;
pub mod hkkaz;
pub mod hksal;
pub mod hkspa;
pub mod hksyn;
pub mod hktan;
pub mod hkvpa;
pub mod hkvpp;
pub mod hkvvb;
pub mod hkwdu;
pub mod hkwpd;
pub mod hnhbk;
pub mod hnhbs;
pub mod hnsha;
pub mod hnshk;
pub mod hnvsd;
pub mod hnvsk;
pub mod security;

pub use common::{Kik1, Kti1, Ktv3, Ktz1, ReferenceMessage, Sdo2, SdoBtg, Tsp1};
pub use feedback::Feedback1;
pub use hibpa::{HiBpa3, SupportedHbciVersions2, SupportedLanguages2};
pub use hicaz::{BookedCamt1, HiCaz1};
pub use hicazs::{CamtTurnoverParameters1, HiCazs1};
pub use hiccss::HiCcss1;
pub use hiips::HiIps1;
pub use hiipss::{HiIpss1, InstantStatusParameters1};
pub use hiipz::{HiIpz1, HiIpz2};
pub use hiipzs::{HiIpzs1, HiIpzs2, InstantPaymentParameters1, InstantPaymentParameters2};
pub use hikaz::{HiKaz5, HiKaz6, HiKaz7};
pub use hikazs::{HiKazs5, HiKazs6, HiKazs7, TurnoverParameters2};
pub use hipins::{HiPins1, PinTanOrder1, PinTanParameters1};
pub use hirmg::HiRmg2;
pub use hirms::HiRms2;
pub use hisal::{HiSal5, HiSal6, HiSal7, HiSal8};
pub use hisals::{BalanceParameters1, HiSals5, HiSals6, HiSals7, HiSals8};
pub use hispa::HiSpa1;
pub use hispas::{
    HiSpas1, HiSpas2, HiSpas3, SepaAccountParameters1, SepaAccountParameters2,
    SepaAccountParameters3,
};
pub use hisyn::HiSyn4;
pub use hitan::{ChallengeValidity1, HiTan1, HiTan2, HiTan3, HiTan4, HiTan5, HiTan6, HiTan7};
pub use hitans::{
    HiTans1, HiTans2, HiTans3, HiTans4, HiTans5, HiTans6, HiTans7, TanMethod1, TanMethod2,
    TanMethod3, TanMethod4, TanMethod5, TanMethod6, TanMethod7, TanParameters1, TanParameters2,
    TanParameters3, TanParameters4, TanParameters5, TanParameters6, TanParameters7,
};
pub use hiupa::HiUpa4;
pub use hiupd::{AccountLimit2, AllowedOrder2, HiUpd4, HiUpd6};
pub use hivpas::HiVpas1;
pub use hivpp::{HiVpp1, PayeeCheckResult1};
pub use hivpps::{HiVpps1, PayeeCheckParameters1};
pub use hiwdu::HiWdu5;
pub use hiwpd::{HiWpd5, HiWpd6};
pub use hiwpds::{DepotListParameters2, HiWpds5, HiWpds6};
pub use hkcaz::{HkCaz1, SupportedCamtMessages1};
pub use hkccs::HkCcs1;
pub use hkend::HkEnd1;
pub use hkidn::HkIdn2;
pub use hkips::{HkIps1, SupportedPainMessages1};
pub use hkipz::{HkIpz1, HkIpz2};
pub use hkkaz::{HkKaz5, HkKaz6, HkKaz7};
pub use hksal::{HkSal5, HkSal6, HkSal7, HkSal8};
pub use hkspa::HkSpa1;
pub use hksyn::HkSyn3;
pub use hktan::{ChallengeClassParameters1, HhdUcResponse1, HkTan5, HkTan6, HkTan7};
pub use hkvpa::HkVpa1;
pub use hkvpp::{HkVpp1, SupportedPaymentStatusReports1};
pub use hkvvb::HkVvb3;
pub use hkwdu::{HkWdu5, SecurityReference2};
pub use hkwpd::{HkWpd5, HkWpd6};
pub use hnhbk::HnHbk3;
pub use hnhbs::HnHbs1;
pub use hnsha::HnSha2;
pub use hnshk::HnShk4;
pub use hnvsd::HnVsd1;
pub use hnvsk::HnVsk3;
pub use security::{
    Certificate2, EncryptionAlgorithm2, HashAlgorithm2, KeyName3, SecurityIdentification2,
    SecurityProfile1, SecurityTimestamp2, SignatureAlgorithm2, UserDefinedSignature1,
};

/// Helpers shared by the tests of the definitions.
#[cfg(test)]
pub(crate) mod testing {
    use crate::codec::{Segment, from_wire, to_wire};

    /// Parses `wire` with `T`, writes it back and asserts the identical
    /// string; returns the value for further assertions.
    pub fn round_trip<T: Segment>(wire: &str) -> T {
        let (header, value) = from_wire::<T>(wire.as_bytes()).unwrap();
        let back = to_wire(&value, header.number, header.reference).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&back),
            wire,
            "{} v{} did not write back the string it read",
            T::CODE,
            T::VERSION
        );
        value
    }

    /// One synthetic wire string per definition, collected from the
    /// definition files; used by the registry tests and the property tests.
    pub fn all_samples() -> Vec<&'static str> {
        [
            super::hnhbk::SAMPLES,
            super::hnhbs::SAMPLES,
            super::hnshk::SAMPLES,
            super::hnsha::SAMPLES,
            super::hnvsk::SAMPLES,
            super::hnvsd::SAMPLES,
            super::hkidn::SAMPLES,
            super::hkvvb::SAMPLES,
            super::hksyn::SAMPLES,
            super::hisyn::SAMPLES,
            super::hkend::SAMPLES,
            super::hktan::SAMPLES,
            super::hitan::SAMPLES,
            super::hirmg::SAMPLES,
            super::hirms::SAMPLES,
            super::hibpa::SAMPLES,
            super::hipins::SAMPLES,
            super::hiupa::SAMPLES,
            super::hiupd::SAMPLES,
            super::hisal::SAMPLES,
            super::hisals::SAMPLES,
            super::hkspa::SAMPLES,
            super::hispa::SAMPLES,
            super::hispas::SAMPLES,
            super::hksal::SAMPLES,
            super::hitans::SAMPLES,
            super::hkkaz::SAMPLES,
            super::hikaz::SAMPLES,
            super::hikazs::SAMPLES,
            super::hkcaz::SAMPLES,
            super::hicaz::SAMPLES,
            super::hicazs::SAMPLES,
            super::hkwpd::SAMPLES,
            super::hiwpd::SAMPLES,
            super::hiwpds::SAMPLES,
            super::hkwdu::SAMPLES,
            super::hiwdu::SAMPLES,
            super::hiccss::SAMPLES,
            super::hiips::SAMPLES,
            super::hiipss::SAMPLES,
            super::hiipz::SAMPLES,
            super::hiipzs::SAMPLES,
            super::hivpas::SAMPLES,
            super::hivpp::SAMPLES,
            super::hivpps::SAMPLES,
            super::hkccs::SAMPLES,
            super::hkips::SAMPLES,
            super::hkipz::SAMPLES,
            super::hkvpa::SAMPLES,
            super::hkvpp::SAMPLES,
        ]
        .concat()
    }
}
