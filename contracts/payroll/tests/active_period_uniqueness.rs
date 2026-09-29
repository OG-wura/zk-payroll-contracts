//! Active payroll period uniqueness tests (#578).
//!
//! Coverage:
//! - Open/close lifecycle, with the active period readable while open.
//! - Uniqueness: a second period cannot be opened while one is active, and
//!   re-opening the already-active period is rejected separately.
//! - The payroll flow honours the active period: once a period is open, a
//!   new run draft must target it.
//! - Regression guard: with no period active (the pre-#578 default) draft
//!   creation is unrestricted, so existing flows are unaffected.
//! - Per-period state survives a close/re-open cycle.
//! - Both open and close require the admin, and each emits its event.
//! - Failure states expose no payroll values, only period labels.

#![cfg(test)]

use ::token::Token;
use payroll::{Payroll, PayrollClient};
use proof_verifier::{ProofVerifier, ProofVerifierClient, VerificationKey};
use salary_commitment::{SalaryCommitmentContract, SalaryCommitmentContractClient};
use soroban_sdk::testutils::{Address as _, Events};
use soroban_sdk::{Address, BytesN, Env, Symbol, TryIntoVal, Vec};

fn mock_vk(env: &Env) -> VerificationKey {
    VerificationKey {
        alpha: BytesN::from_array(env, &[0u8; 64]),
        beta: BytesN::from_array(env, &[0u8; 128]),
        gamma: BytesN::from_array(env, &[0u8; 128]),
        delta: BytesN::from_array(env, &[0u8; 128]),
        ic: Vec::from_array(
            env,
            [
                BytesN::from_array(env, &[0u8; 64]),
                BytesN::from_array(env, &[0u8; 64]),
                BytesN::from_array(env, &[0u8; 64]),
                BytesN::from_array(env, &[0u8; 64]),
            ],
        ),
    }
}

fn setup_payroll(env: &Env) -> (PayrollClient<'_>, Address) {
    env.mock_all_auths();
    let verifier_id = env.register_contract(None, ProofVerifier);
    let verifier_client = ProofVerifierClient::new(env, &verifier_id);
    verifier_client.init_verifier_admin(&Address::generate(env));
    verifier_client.initialize_verifier(&mock_vk(env));
    let commitment_id = env.register_contract(None, SalaryCommitmentContract);
    let commitment_client = SalaryCommitmentContractClient::new(env, &commitment_id);
    commitment_client.init_commitment_admin(&Address::generate(env));
    let token_id = env.register_contract(None, Token);
    let payroll_id = env.register_contract(None, Payroll);
    let payroll_client = PayrollClient::new(env, &payroll_id);
    let admin = Address::generate(env);
    payroll_client.initialize(
        &admin,
        &token_id,
        &verifier_id,
        &commitment_id,
        &Address::generate(env),
        &Address::generate(env),
    );
    (payroll_client, admin)
}

// ─── Successful path ────────────────────────────────────────────────────────

#[test]
fn open_close_roundtrip() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");

    // Absent by default: no period is active before one is opened.
    assert_eq!(payroll.get_active_period(), None);

    payroll.open_payroll_period(&admin, &period);
    assert_eq!(payroll.get_active_period(), Some(period.clone()));
    assert!(!payroll.is_period_frozen(&period));

    payroll.close_payroll_period(&admin);
    assert_eq!(payroll.get_active_period(), None);
}

#[test]
fn closing_then_opening_a_different_period_succeeds() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let august = Symbol::new(&env, "aug_2026");
    let september = Symbol::new(&env, "sep_2026");

    payroll.open_payroll_period(&admin, &august);
    payroll.close_payroll_period(&admin);
    payroll.open_payroll_period(&admin, &september);

    assert_eq!(payroll.get_active_period(), Some(september));
}

#[test]
fn open_and_close_emit_their_events() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");

    let before = env.events().all().len();
    payroll.open_payroll_period(&admin, &period);
    let events = env.events().all();
    let opened = events.get(before).unwrap();
    let opened_topic: Symbol = opened.1.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(opened_topic, Symbol::new(&env, "payroll_period_opened"));
    let (emitted_period, opened_by): (Symbol, Address) = opened.2.try_into_val(&env).unwrap();
    assert_eq!(emitted_period, period);
    assert_eq!(opened_by, admin);

    let before = env.events().all().len();
    payroll.close_payroll_period(&admin);
    let events = env.events().all();
    let closed = events.get(before).unwrap();
    let closed_topic: Symbol = closed.1.get(1).unwrap().try_into_val(&env).unwrap();
    assert_eq!(closed_topic, Symbol::new(&env, "payroll_period_closed"));
    let (closed_period, closed_by): (Symbol, Address) = closed.2.try_into_val(&env).unwrap();
    assert_eq!(closed_period, period);
    assert_eq!(closed_by, admin);
}

// ─── Uniqueness validation ──────────────────────────────────────────────────

#[test]
#[should_panic(
    expected = "An active payroll period already exists: close it before opening a new one"
)]
fn opening_a_second_period_is_rejected() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    payroll.open_payroll_period(&admin, &Symbol::new(&env, "aug_2026"));
    payroll.open_payroll_period(&admin, &Symbol::new(&env, "sep_2026"));
}

#[test]
#[should_panic(expected = "Payroll period is already the active period")]
fn reopening_the_active_period_is_rejected() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");
    payroll.open_payroll_period(&admin, &period);
    payroll.open_payroll_period(&admin, &period);
}

#[test]
#[should_panic(expected = "No active payroll period to close")]
fn closing_without_an_active_period_is_rejected() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    payroll.close_payroll_period(&admin);
}

#[test]
#[should_panic(expected = "No active payroll period to close")]
fn closing_twice_is_rejected() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    payroll.open_payroll_period(&admin, &Symbol::new(&env, "aug_2026"));
    payroll.close_payroll_period(&admin);
    payroll.close_payroll_period(&admin);
}

// ─── Authorization ───────────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Unauthorized")]
fn open_requires_admin() {
    let env = Env::default();
    let (payroll, _admin) = setup_payroll(&env);
    let outsider = Address::generate(&env);
    payroll.open_payroll_period(&outsider, &Symbol::new(&env, "aug_2026"));
}

#[test]
#[should_panic(expected = "Unauthorized")]
fn close_requires_admin() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    payroll.open_payroll_period(&admin, &Symbol::new(&env, "aug_2026"));
    let outsider = Address::generate(&env);
    payroll.close_payroll_period(&outsider);
}

// ─── The payroll flow honours the active period ─────────────────────────────

#[test]
fn draft_against_the_active_period_succeeds() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");

    payroll.open_payroll_period(&admin, &period);
    let draft_id = payroll.create_run_draft(&admin, &5_000i128, &1u32, &period);

    assert_eq!(payroll.get_run_draft(&draft_id).period_label, period);
}

#[test]
#[should_panic(expected = "A different payroll period is active")]
fn draft_for_a_different_period_is_rejected() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    payroll.open_payroll_period(&admin, &Symbol::new(&env, "aug_2026"));
    payroll.create_run_draft(&admin, &5_000i128, &1u32, &Symbol::new(&env, "sep_2026"));
}

#[test]
fn draft_becomes_allowed_again_after_the_period_is_closed() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let august = Symbol::new(&env, "aug_2026");
    let september = Symbol::new(&env, "sep_2026");

    payroll.open_payroll_period(&admin, &august);
    payroll.close_payroll_period(&admin);

    // No period is active, so the label is no longer constrained.
    let draft_id = payroll.create_run_draft(&admin, &5_000i128, &1u32, &september);
    assert_eq!(payroll.get_run_draft(&draft_id).period_label, september);
}

#[test]
fn drafts_are_unrestricted_when_no_period_is_active() {
    // Regression guard: #578 must not constrain the pre-#578 default, where
    // no period is active and any label is accepted.
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);

    let draft_id =
        payroll.create_run_draft(&admin, &5_000i128, &1u32, &Symbol::new(&env, "jul_2026"));

    assert_eq!(
        payroll.get_run_draft(&draft_id).period_label,
        Symbol::new(&env, "jul_2026")
    );
}

// ─── Edge cases ─────────────────────────────────────────────────────────────

#[test]
fn closing_a_period_preserves_its_existing_draft() {
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");

    payroll.open_payroll_period(&admin, &period);
    let draft_id = payroll.create_run_draft(&admin, &5_000i128, &1u32, &period);

    // Closing clears the active slot only; per-period state is keyed by label
    // and must survive, so re-opening resumes against what is already there.
    payroll.close_payroll_period(&admin);
    assert_eq!(payroll.get_active_period(), None);
    assert_eq!(payroll.get_run_draft(&draft_id).period_label, period);

    payroll.open_payroll_period(&admin, &period);
    assert_eq!(payroll.get_active_period(), Some(period.clone()));

    // The per-period draft slot from #398 is still held, so a second draft
    // for the same period is still rejected after the close/re-open cycle.
    let duplicate = payroll.try_create_run_draft(&admin, &5_000i128, &1u32, &period);
    assert!(duplicate.is_err());
}

#[test]
fn a_frozen_period_can_still_be_opened_and_closed() {
    // Freezing and the active period are independent: freezing blocks edits
    // to a period, it does not own the active slot.
    let env = Env::default();
    let (payroll, admin) = setup_payroll(&env);
    let period = Symbol::new(&env, "aug_2026");

    payroll.open_payroll_period(&admin, &period);
    payroll.freeze_payroll_period(&admin, &period, &Symbol::new(&env, "finalized"));

    assert!(payroll.is_period_frozen(&period));
    assert_eq!(payroll.get_active_period(), Some(period.clone()));

    payroll.close_payroll_period(&admin);
    assert_eq!(payroll.get_active_period(), None);
    assert!(payroll.is_period_frozen(&period));
}
