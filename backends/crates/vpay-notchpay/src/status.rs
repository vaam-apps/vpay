//! Notch Pay's payment statuses, in Notch Pay's own words.
//!
//! This module names the statuses. It does **not** say what any of them
//! means for a vpay charge — that is a decision about money, and it belongs
//! to the adapter's mapping table, which RFC-0012 § 4 lays out and which a
//! sandbox session fills in. A client that decided "`canceled` means the
//! payer declined" would be the adapter's mapping hiding in a library.

/// Every payment status Notch Pay documents.
///
/// The Payments API page documents six (`pending`, `processing`,
/// `complete`, `failed`, `canceled`, `expired`); the Collect page documents
/// eleven, including those six. All eleven are here, because a status the
/// caller does not know must reach it as `None` from [`PaymentStatus::parse`]
/// and never as a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaymentStatus {
    /// "initialized but not yet processed".
    Pending,
    /// "being processed by the payment provider".
    Processing,
    /// "Customer pay less than payment amount" — Collect's own words.
    Incomplete,
    /// "successfully completed".
    Complete,
    /// "successfully refunded to customer".
    Refunded,
    /// Partly refunded. Notch Pay spells it `partialy-refunded`.
    PartiallyRefunded,
    /// "Transaction has failed".
    Failed,
    /// "canceled by the merchant or customer".
    Canceled,
    /// "expired after 3 hours".
    Expired,
    /// "Customer abandoned the payment on our hosted page".
    Abandoned,
    /// "rejected by our system or by operator".
    Rejected,
}

impl PaymentStatus {
    /// All eleven, in the order the Collect page lists them.
    pub const ALL: [Self; 11] = [
        Self::Pending,
        Self::Processing,
        Self::Incomplete,
        Self::Canceled,
        Self::Failed,
        Self::Rejected,
        Self::Abandoned,
        Self::Expired,
        Self::Complete,
        Self::Refunded,
        Self::PartiallyRefunded,
    ];

    /// Notch Pay's spelling of this status, byte for byte.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Incomplete => "incomplete",
            Self::Complete => "complete",
            Self::Refunded => "refunded",
            Self::PartiallyRefunded => "partialy-refunded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
            Self::Abandoned => "abandoned",
            Self::Rejected => "rejected",
        }
    }

    /// The status Notch Pay sent, or `None` for a string it does not
    /// document.
    ///
    /// Exact and case-sensitive. `partially-refunded`, with the second `l`
    /// that Notch Pay does not write, is `None`: correcting a rail's spelling
    /// is guessing what it meant.
    ///
    /// ```
    /// use vpay_notchpay::status::PaymentStatus;
    ///
    /// assert_eq!(PaymentStatus::parse("complete"), Some(PaymentStatus::Complete));
    /// assert_eq!(PaymentStatus::parse("partialy-refunded"), Some(PaymentStatus::PartiallyRefunded));
    /// assert_eq!(PaymentStatus::parse("Complete"), None);
    /// assert_eq!(PaymentStatus::parse("succeeded"), None);
    /// ```
    #[must_use]
    pub fn parse(status: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.wire() == status)
    }
}

#[cfg(test)]
mod tests {
    use super::PaymentStatus;

    #[test]
    fn every_documented_status_round_trips_through_its_own_spelling() {
        for status in PaymentStatus::ALL {
            assert_eq!(PaymentStatus::parse(status.wire()), Some(status));
        }
    }

    #[test]
    fn the_eleven_spellings_are_distinct() {
        let mut spellings: Vec<&str> = PaymentStatus::ALL.iter().map(|s| s.wire()).collect();
        spellings.sort_unstable();
        spellings.dedup();
        assert_eq!(spellings.len(), PaymentStatus::ALL.len());
    }

    #[test]
    fn a_spelling_notch_pay_does_not_write_is_not_recognised() {
        for unknown in ["", "partially-refunded", "COMPLETE", "success", "cancelled"] {
            assert_eq!(PaymentStatus::parse(unknown), None, "{unknown:?}");
        }
    }
}
