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
 * Date:        2026-10-13
 * ============================================================================
 * Description:
 *      Pure domain logic for the LFPDPPP consent registry. This module is
 *      the state machine that the transport layer (`routes/consents.rs`)
 *      consults before inserting an event. It has no dependency on Axum,
 *      SQLx, Redis, or any provider SDK. The rule that governs whether a
 *      consent transition is allowed lives here and nowhere else.
 *
 *      Consent is modeled as an append-only stream of events per
 *      (user, notice). The "current state" is not a column — it is the
 *      latest event by (recorded_at DESC, id DESC). This module defines
 *      what counts as a valid transition between two states and what
 *      counts as a no-op.
 *
 *      Public surface:
 *          ConsentAction         The two valid actions: Granted, Withdrawn
 *          ConsentError          Domain errors returned by the transition
 *                                policy
 *          should_record_event() The transition predicate: given the
 *                                current state, the requested action,
 *                                and whether the notice is published,
 *                                decide whether to append an event,
 *                                no-op, or reject.
 *
 *      Transition table (exhaustive over the three inputs):
 *
 *          current    requested    notice published    outcome                     arm
 *          -------    ---------    ----------------    -------                     ---
 *          None       Granted      true                record                      arm 2
 *          None       Granted      false               reject: NoticeNotPublished  arm 3
 *          Granted    Granted      true                no-op                       arm 1
 *          Granted    Granted      false               no-op                       arm 1
 *          Withdrawn  Granted      true                record                      arm 2
 *          Withdrawn  Granted      false               reject: NoticeNotPublished  arm 3
 *          None       Withdrawn    (either)            reject: NoActiveConsent     arm 6
 *          Granted    Withdrawn    (either)            record                      arm 4
 *          Withdrawn  Withdrawn    (either)            no-op                       arm 5
 *
 *      Arm ordering is load-bearing. The match arms are evaluated top
 *      to bottom, and the guard on arm 1 (`current == Granted`)
 *      fires before the publication guard on arm 3. The practical
 *      consequence: a repeated grant is a no-op even when the notice
 *      has since been retired. The prior grant is not retroactively
 *      invalidated by a change in publication state. The publication
 *      check only applies when the current state is not already
 *      Granted — i.e. on the first grant or on a re-grant after a
 *      withdrawal. Any change to arm order must be accompanied by a
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
 *          - `ConsentAction` derives `Copy`. The enum has two variants
 *            with no payload, so `Copy` is free.
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
 *
 *      Testing:
 *          Three unit tests in this file:
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
 *          - No conditional consent at the domain level. Purpose
 *            scoping is expressed by (purpose_code, locale) on the
 *            notice.
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConsentError {
    #[error("consent notice is not currently published")]
    NoticeNotPublished,
    #[error("no active consent exists for this notice")]
    NoActiveConsent,
    #[error("unknown consent event type: {0}")]
    UnknownEventType(String),
}

pub fn should_record_event(
    current: Option<ConsentAction>,
    requested: ConsentAction,
    notice_is_published: bool,
) -> Result<bool, ConsentError> {
    match requested {
        ConsentAction::Granted if current == Some(ConsentAction::Granted) => Ok(false),
        ConsentAction::Granted if notice_is_published => Ok(true),
        ConsentAction::Granted => Err(ConsentError::NoticeNotPublished),
        ConsentAction::Withdrawn if current == Some(ConsentAction::Granted) => Ok(true),
        ConsentAction::Withdrawn if current == Some(ConsentAction::Withdrawn) => Ok(false),
        ConsentAction::Withdrawn => Err(ConsentError::NoActiveConsent),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConsentAction, ConsentError, should_record_event};

    #[test]
    fn grants_require_publication_and_repeated_grants_are_idempotent() {
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, true),
            Ok(true)
        );
        assert_eq!(
            should_record_event(Some(ConsentAction::Granted), ConsentAction::Granted, true),
            Ok(false)
        );
        assert_eq!(
            should_record_event(None, ConsentAction::Granted, false),
            Err(ConsentError::NoticeNotPublished)
        );
    }

    #[test]
    fn withdrawal_requires_a_grant_and_repeated_withdrawals_are_idempotent() {
        assert_eq!(
            should_record_event(
                Some(ConsentAction::Granted),
                ConsentAction::Withdrawn,
                false
            ),
            Ok(true)
        );
        assert_eq!(
            should_record_event(
                Some(ConsentAction::Withdrawn),
                ConsentAction::Withdrawn,
                false
            ),
            Ok(false)
        );
        assert_eq!(
            should_record_event(None, ConsentAction::Withdrawn, false),
            Err(ConsentError::NoActiveConsent)
        );
    }

    #[test]
    fn database_event_types_round_trip() {
        assert_eq!(
            ConsentAction::from_event_type("granted"),
            Ok(ConsentAction::Granted)
        );
        assert_eq!(
            ConsentAction::from_event_type("withdrawn"),
            Ok(ConsentAction::Withdrawn)
        );
        assert_eq!(
            ConsentAction::from_event_type("invalid"),
            Err(ConsentError::UnknownEventType("invalid".to_string()))
        );
        assert_eq!(ConsentAction::Granted.as_event_type(), "granted");
        assert_eq!(ConsentAction::Withdrawn.as_event_type(), "withdrawn");
    }
}
