/**
 * ============================================================================
 * ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
 * ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
 * ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
 * ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
 * https://rho.studio/
 * ============================================================================
 * File:        src/domain/consent.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-11-04
 * ============================================================================
 * Description:
 *      Pure domain logic for the LFPDPPP consent registry. A state machine 
 *      that the transport layer (`routes/consents.rs`) consults before 
 *      inserting an event. It has no dependency on Axum, SQLx, Redis, 
 *      or any provider SDK. The rule that governs whether a
 *      consent transition is allowed lives here and nowhere else.
 *
 *      Consent is an append-only stream of events per (user, notice). 
 *      The "current state" is not a column — it is the
 *      latest event by (recorded_at DESC, id DESC). This module defines
 *      what counts as a valid transition between two states and what
 *      counts as a no-op.
 *
 *      Public surface:
 *          ConsentAction         The two valid actions: 'Granted', 'Withdrawn'
 *          ConsentType           The legal labels for consent: 'Express', 'Tacit'
 *          ConsentError          Domain errors returned by the transition
 *                                policy
 *          should_record_event() The transition predicate: given the current
 *                                state, the requested action, whether the
 *                                purpose is high-risk, the consent type,
 *                                and whether the notice is published,
 *                                decide whether to append an event, no-op, or reject.
 *
 *      Transition table (exhaustive over the three inputs):
 *
*          current    requested    high_risk    consent_type    notice published    outcome                                 arm
 *          -------    ---------    ---------    ------------    ----------------    -------                                 ---
 *          Granted    Granted      Any          Any             Any                 no-op (idempotent)                      arm 1
 *          None       Granted      true         Tacit           Any                 reject: ExpressConsentRequired          arm 2
 *          Withdrawn  Granted      true         Tacit           Any                 reject: ExpressConsentRequired          arm 2
 *          None       Granted      false        Any             true                record                                  arm 3
 *          None       Granted      true         Express         true                record                                  arm 3
 *          Withdrawn  Granted      false        Any             true                record                                  arm 3
 *          Withdrawn  Granted      true         Express         true                record                                  arm 3
 *          None       Granted      false        Any             false               reject: NoticeNotPublished              arm 4
 *          None       Granted      true         Express         false               reject: NoticeNotPublished              arm 4
 *          Withdrawn  Granted      false        Any             false               reject: NoticeNotPublished              arm 4
 *          Withdrawn  Granted      true         Express         false               reject: NoticeNotPublished              arm 4
 *          Granted    Withdrawn    Any          Any             Any                 record                                  arm 5
 *          Withdrawn  Withdrawn    Any          Any             Any                 no-op (idempotent)                      arm 6
 *          None       Withdrawn    Any          Any             Any                 reject: NoActiveConsent                 arm 7
 *
 *      Why the arm ordering matters:
 *          1. Arm 1 (Idempotency) fires before anything else. If the user is already granted, 
 *             the system returns `Ok(false)` immediately, ignoring whether the notice was 
 *             later retired or if the request is missing an `Express` flag.
 *          2. Arm 2 (LFPDPPP Art. 7) fires before the publication check. This guarantees that 
 *             a request for KYC/Financial data with `Tacit` consent is rejected (400 Bad Request) 
 *             even if the notice is currently published.
 *          3. Arm 3 is the only "happy path" that returns `Ok(true)` for a new grant. It requires 
 *             the notice to be published AND passes the LFPDPPP check (either low-risk, or high-risk with Express).
 *          4. Arm 4 is the catch-all for `Granted`. If we reach this arm, the notice is unpublished.
 *          5. Arms 5, 6, and 7 handle `Withdrawn`. Notice that they do not check `high_risk`, 
 *             `consent_type`, or `notice published`. A user must always be able to revoke consent, 
 *             even if the notice has been retired.
 * 
 *      Any change to arm order must be accompanied by a
 *      change to the test suite in this file.
 *
 *      Rationale for the rules:
 *          - A grant requires a published notice unless the user is
 *            already granted. Granting consent against a draft or
 *            retired notice is meaningless: the content the user
 *            agreed to has not been shown to them, or is no longer
 *            current. Once granted, the grant stands on its own: it
 *            was recorded against a specific notice version at a
 *            specific time, and a later publication-state change does
 *            not invalidate it.
 *          - A grant for high-risk data (e.g., financial or patrimonial) strictly 
 *            requires Express consent. If the client submits Tacit consent for 
 *            these purposes, the domain rejects it with `ExpressConsentRequired` 
 *            to comply with LFPDPPP Article 7.
 *          - A repeated grant on the same notice is a no-op, not an
 *            error. Mobile clients retry on network failure, and a
 *            duplicate submission must not create a second event.
 *            Idempotency is a user-facing property of the endpoint.
 *          - A withdrawal requires a prior grant. Withdrawing consent
 *            that was never given is a client error, not a silent
 *            no-op: the client is confused about the current state
 *            and should be told. This returns `NoActiveConsent`.
 *          - A repeated withdrawal is a no-op, for the same idempotency
 *            reason as a repeated grant.
 *          - A withdrawal is permitted regardless of whether the
 *            notice is still published. A user whose notice has been
 *            retired must still be able to revoke prior consent.
 *            Blocking withdrawal on publication state would trap the
 *            user in a consent they can no longer revoke.
 *
 *      Event type mapping:
 *          The database stores `event_type` as `VARCHAR(16)` with a
 *          CHECK constraint (`'granted' | 'withdrawn'`). `ConsentAction`
 *          maps to and from those string values via `as_event_type` and
 *          `from_event_type`. `from_event_type` returns
 *          `ConsentError::UnknownEventType` for any value the enum does
 *          not recognize. That variant maps to `Internal` in the route
 *          layer because an unrecognized value in the database can only
 *          mean the CHECK constraint has been bypassed or the enum has
 *          drifted from the schema — neither is a client error.
 *
 *      Design notes:
 *          - The domain does not read from the database. The caller
 *            (the route handler) fetches the current state and the
 *            notice status, then passes them in. This makes the state
 *            machine trivially unit-testable and keeps the domain free
 *            of infrastructure concerns.
 *          - Both `ConsentAction` and `ConsentType` derives `Copy`. 
 *            The enum has two variants with no payload, so `Copy` is free.
 *          - `ConsentError` derives `PartialEq` and `Eq`. Unit tests
 *            compare results directly with `assert_eq!`.
 *          - The transition predicate is a free function, not a method
 *            on a struct. There is no aggregate with state to own —
 *            the state lives in the event log, and
 *            `should_record_event` is a pure calculation over its
 *            arguments.
 *          - Adding a new action requires touching the enum, the event
 *            type mapping, the transition table, and the database
 *            CHECK constraint in a single coordinated change. The
 *            compiler flags any `match` that does not cover the new
 *            variant; the database constraint rejects an event with an
 *            unrecognized type at write time. Both layers enforce the
 *            invariant.
 *          - No I/O, no async, no error type from any external crate.
 *            `ConsentError` is defined here because it describes a
 *            domain-level rejection, not a transport-level failure.
 *            The route layer translates it into `AppError`.
*          - `ConsentError` derives `PartialEq` and `Eq`. Unit tests
 *            compare results directly with `assert_eq!`.
 *
 *      Testing:
 *          Four unit tests in this file:
 *
 *              grants_require_publication_and_repeated_grants_are_idempotent
 *                  Covers: (None, Granted, true), (Granted, Granted, true),
 *                          (None, Granted, false)
 *              withdrawal_requires_a_grant_and_repeated_withdrawals_are_idempotent
 *                  Covers: (Granted, Withdrawn, false), (Withdrawn, Withdrawn, false),
 *                          (None, Withdrawn, false)
 *              database_event_types_round_trip
 *                  Covers: as_event_type and from_event_type for both variants
 *                          plus the unknown-value error path
 *              lfpdp_art_7_requires_express_consent_for_high_risk_data
 *                  Covers: rejecting Tacit consent for high-risk data, allowing 
 *                          Express consent, and allowing Tacit for low-risk data.
 *
 *          The tests are pure: no database, no fixtures, no async
 *          runtime. Container-backed tests in
 *          `route_tests/database.rs` verify that the route layer
 *          consults this predicate correctly against a live Postgres.
 *
 *          Two rows of the transition table are not covered by the
 *          current tests: (Granted, Granted, false) and
 *          (Withdrawn, Granted, false). Both exercise the arm-ordering
 *          property described above. Adding them is recommended before
 *          the file is considered frozen for Phase 1.
 *
 *      Known gaps (deferred to Phase 3, Feature F3 in DevPlan.md):
 *          - No time-bounded consent. A grant is valid until
 *            explicitly withdrawn; there is no notion of "consent
 *            valid for 12 months" that auto-revokes.
 *          - No time-bounded consent. A grant is valid until explicitly 
 *            withdrawn; there is no notion of "consent valid for 12 months" 
 *            that auto-revokes.
 *          - No delegated consent (guardian on behalf of a minor).
 *          - No re-consent prompt when a notice version is retired
 *            and replaced. This is a workflow concern that lives
 *            above the state machine, but the state machine is where
 *            the rule would be expressed if it were added.
 *
 *      Explicitly out of Phase 1 scope:
 *          - LFPDPPP ARCO request handling (access, rectification,
 *            cancellation, opposition). Schema exists in migration 5;
 *            no domain logic has been written.
 *          - Consent-driven data export and anonymization flows.
 *
 *      Not a serialized type. `ConsentAction` implements `Serialize`
 *      and `Deserialize` because it appears in the request body of
 *      `POST /api/v1/consents`; no other type in this module crosses
 *      the API boundary.
 * ============================================================================
 */
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentAction {
    Granted,
    Withdrawn,
}

impl ConsentAction {
    pub fn as_event_type(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Withdrawn => "withdrawn",
        }
    }

    pub fn from_event_type(event_type: &str) -> Result<Self, ConsentError> {
        match event_type {
            "granted" => Ok(Self::Granted),
            "withdrawn" => Ok(Self::Withdrawn),
            unknown => Err(ConsentError::UnknownEventType(unknown.to_string())),
        }
    }
}

/// The mechanism by which the user agreed (LFPDPPP Art. 7).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentType {
    Express,
    Tacit,
}

impl ConsentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Express => "express",
            Self::Tacit => "tacit",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConsentError {
    #[error("consent notice is not currently published")]
    NoticeNotPublished,
    #[error("no active consent exists for this notice")]
    NoActiveConsent,
    #[error("unknown consent event type: {0}")]
    UnknownEventType(String),
    #[error("Financial or sensitive data requires express consent (LFPDPPP Art. 7)")]
    ExpressConsentRequired,
}

pub fn should_record_event(
    current: Option<ConsentAction>,
    requested: ConsentAction,
    notice_is_published: bool,
    is_high_risk_purpose: bool,
    consent_type: ConsentType,
) -> Result<bool, ConsentError> {
    match requested {
        // 1. Idempotency: If already granted, it's a no-op regardless of other factors. 
        ConsentAction::Granted if current == Some(ConsentAction::Granted) => Ok(false),
        // 2. LFPDPPP Art. 7: High-risk purposes (Financial/KYC) strictly require Express consent.
        ConsentAction::Granted 
            if is_high_risk_purpose && consent_type != ConsentType::Express => 
        {
            Err(ConsentError::ExpressConsentRequired)
        }
        // 3. Standard Grant: Requires published notice.
        ConsentAction::Granted if notice_is_published => Ok(true),
        ConsentAction::Granted => Err(ConsentError::NoticeNotPublished),
        // 4. Withdrawal Logic (No restrictions on consent type or publication status for revocation).
        ConsentAction::Withdrawn if current == Some(ConsentAction::Granted) => Ok(true),
        ConsentAction::Withdrawn if current == Some(ConsentAction::Withdrawn) => Ok(false),
        ConsentAction::Withdrawn => Err(ConsentError::NoActiveConsent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_require_publication_and_repeated_grants_are_idempotent() {
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, true, false, ConsentType::Express),
            Ok(true)
        );
        assert_eq!(
            should_record_event(Some(ConsentAction::Granted), ConsentAction::Granted, true, false, ConsentType::Express),
            Ok(false)
        );
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, false, false, ConsentType::Express),
            Err(ConsentError::NoticeNotPublished)
        );
    }

    #[test]
    fn withdrawal_requires_a_grant_and_repeated_withdrawals_are_idempotent() {
        assert_eq!(
            should_record_event(
                Some(ConsentAction::Granted),
                ConsentAction::Withdrawn,
                false,
                false,
                ConsentType::Tacit
            ),
            Ok(true)
        );
        assert_eq!(
            should_record_event(
                Some(ConsentAction::Withdrawn),
                ConsentAction::Withdrawn,
                false,
                false,
                ConsentType::Tacit        
            ),
            Ok(false)
        );
        assert_eq!(
            should_record_event(None, ConsentAction::Withdrawn, false, false, ConsentType::Tacit),
            Err(ConsentError::NoActiveConsent)
        );
    }

    #[test]
    fn database_event_types_round_trip() {
        assert_eq!(ConsentAction::from_event_type("granted"), Ok(ConsentAction::Granted));
        assert_eq!(ConsentAction::from_event_type("withdrawn"), Ok(ConsentAction::Withdrawn));
        assert_eq!(ConsentAction::from_event_type("invalid"), Err(ConsentError::UnknownEventType("invalid".to_string())));
        assert_eq!(ConsentAction::Granted.as_event_type(), "granted");
        assert_eq!(ConsentAction::Withdrawn.as_event_type(), "withdrawn");
    }

     #[test]
    fn lfpdp_art_7_requires_express_consent_for_high_risk_data() {
        // Attempting to grant high-risk data with Tacit consent must fail
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, true, true, ConsentType::Tacit),
            Err(ConsentError::ExpressConsentRequired)
        );

        // Attempting to grant high-risk data with Express consent succeeds
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, true, true, ConsentType::Express),
            Ok(true)
        );

        // Low-risk data can still use Tacit consent
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, true, false, ConsentType::Tacit),
            Ok(true)
        );
    }
}
