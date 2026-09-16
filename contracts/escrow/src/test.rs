#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _, testutils::Events as _, testutils::Ledger as _, token, Env, String,
    TryFromVal,
};

fn create_token_contract<'a>(env: &Env, admin: &Address) -> token::StellarAssetClient<'a> {
    let contract_address = env.register_stellar_asset_contract(admin.clone());
    token::StellarAssetClient::new(env, &contract_address)
}

#[test]
fn happy_path_single_milestone_early_confirm() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));

    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Active);

    client.confirm_milestone(&renter, &escrow_id, &0);

    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Completed);
    assert!(escrow.milestones.get(0).unwrap().released);

    // The indexer relies on these events to reconstruct escrow state off-chain,
    // so a release that doesn't publish one would silently break it.
    let released_count = env
        .events()
        .all()
        .iter()
        .filter(|(_, topics, _)| {
            soroban_sdk::Symbol::try_from_val(&env, &topics.get_unchecked(1))
                == Ok(symbol_short!("released"))
        })
        .count();
    assert_eq!(released_count, 1);
}

#[test]
fn dispute_flow_majority_vote() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // fund three jurors so they can stake
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    let dispute_id = client.raise_dispute(
        &host,
        &escrow_id,
        &0,
        &String::from_str(&env, "ipfs://evidence"),
    );
    assert_eq!(dispute_id, escrow_id);

    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Disputed);
}

#[test]
fn cancel_escrow_before_funding() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    // host (not just the renter who created it) can also call it off before funding
    client.cancel_escrow(&host, &escrow_id);

    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidState
fn cancel_escrow_after_funding_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.cancel_escrow(&renter, &escrow_id);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn cancel_escrow_by_stranger_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let stranger = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.cancel_escrow(&stranger, &escrow_id);
}

#[test]
fn mutual_cancel_refunds_unreleased_milestones() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 40_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    // release the first milestone early so only the second is refundable
    client.confirm_milestone(&renter, &escrow_id, &0);
    assert_eq!(token_client.balance(&host), 60_0000000i128);

    let renter_balance_before = token_client.balance(&renter);
    client.mutual_cancel(&renter, &host, &escrow_id);

    assert_eq!(token_client.balance(&renter), renter_balance_before + 40_0000000i128);
    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Cancelled);
    assert!(escrow.milestones.get(1).unwrap().released);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidState
fn mutual_cancel_on_disputed_escrow_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.mutual_cancel(&renter, &host, &escrow_id);
}

#[test]
fn protocol_fee_is_deducted_from_release_and_sent_to_treasury() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let treasury = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_fee_config(&admin, &500, &treasury); // 5%

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.confirm_milestone(&renter, &escrow_id, &0);

    assert_eq!(token_client.balance(&treasury), 5_0000000i128);
    assert_eq!(token_client.balance(&host), 95_0000000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")] // FeeTooHigh
fn fee_above_cap_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_config(&admin, &2001, &treasury);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn fee_config_requires_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let treasury = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_config(&stranger, &500, &treasury);
}

#[test]
fn juror_params_are_configurable_and_enforced() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // shrink the jury to 1 and raise the minimum stake
    client.set_juror_params(&admin, &500_0000000, &1, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &0);
    let params = client.get_juror_params();
    assert_eq!(params.jury_size, 1);
    assert_eq!(params.min_stake, 500_0000000);

    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &1_000_0000000);
    // the old default stake (100) is now below the configured minimum
    let err = client.try_register_juror(&juror, &asset_address, &100_0000000);
    assert!(err.is_err());
    client.register_juror(&juror, &asset_address, &500_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert_eq!(dispute.jurors.len(), 1);
}

#[test]
#[should_panic(expected = "Error(Contract, #19)")] // InvalidJurySize
fn even_jury_size_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &4, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &0);
}

#[test]
fn parties_cannot_be_drawn_as_jurors_on_their_own_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);
    token_admin_client.mint(&host, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // both parties register as jurors alongside one genuinely neutral juror
    let neutral = Address::generate(&env);
    token_admin_client.mint(&neutral, &200_0000000);
    client.register_juror(&renter, &asset_address, &100_0000000);
    client.register_juror(&host, &asset_address, &100_0000000);
    client.register_juror(&neutral, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert!(!dispute.jurors.contains(&renter));
    assert!(!dispute.jurors.contains(&host));
    assert!(dispute.jurors.contains(&neutral));
}

#[test]
fn juror_can_withdraw_stake_when_idle() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let juror = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&juror, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.register_juror(&juror, &asset_address, &150_0000000);
    assert_eq!(token_client.balance(&juror), 50_0000000);

    client.withdraw_juror_stake(&juror);
    assert_eq!(token_client.balance(&juror), 200_0000000);
    assert!(client.get_juror_stake(&juror).is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #21)")] // JurorHasActiveDispute
fn juror_cannot_withdraw_while_assigned_to_open_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let juror = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);
    token_admin_client.mint(&juror, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &1, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &0);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.withdraw_juror_stake(&juror);
}

#[test]
fn juror_can_withdraw_after_dispute_resolves() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let juror = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);
    token_admin_client.mint(&juror, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &1, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &0);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
    client.vote_dispute(&juror, &escrow_id, &0, &true);
    client.resolve_dispute(&escrow_id, &0);

    // now idle again - withdrawal should succeed
    client.withdraw_juror_stake(&juror);
    assert!(client.get_juror_stake(&juror).is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #20)")] // AssetMismatch
fn re_registering_with_a_different_asset_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let juror = Address::generate(&env);

    let token_a = create_token_contract(&env, &admin);
    let token_b = create_token_contract(&env, &admin);
    token_a.mint(&juror, &200_0000000);
    token_b.mint(&juror, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.register_juror(&juror, &token_a.address, &100_0000000);
    client.register_juror(&juror, &token_b.address, &100_0000000);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")] // TooEarly
fn force_resolve_before_deadline_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.force_resolve_stale_dispute(&escrow_id, &0);
}

#[test]
fn force_resolve_splits_funds_after_deadline_with_no_votes() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    // jurors go silent - jump past the voting window
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 3 * 24 * 60 * 60 + 1);

    let renter_before = token_client.balance(&renter);
    let host_before = token_client.balance(&host);
    client.force_resolve_stale_dispute(&escrow_id, &0);

    assert_eq!(token_client.balance(&renter), renter_before + 25_0000000i128);
    assert_eq!(token_client.balance(&host), host_before + 25_0000000i128);

    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert!(dispute.resolved);
    assert_eq!(dispute.outcome, DisputeOutcome::Split);

    // jurors are no longer tied up, so their stake can now be withdrawn
    for j in dispute.jurors.iter() {
        client.withdraw_juror_stake(&j);
    }
}

#[test]
fn add_and_remove_milestone_before_funding() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "move-out"), &40_0000000i128, &999_999u64);
    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.milestones.len(), 2);
    assert_eq!(escrow.total_amount, 100_0000000i128);

    client.remove_milestone(&renter, &escrow_id, &0);
    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.milestones.len(), 1);
    assert_eq!(escrow.total_amount, 40_0000000i128);
    assert_eq!(escrow.milestones.get(0).unwrap().description, String::from_str(&env, "move-out"));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")] // NoMilestones
fn cannot_remove_last_remaining_milestone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "only one"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.remove_milestone(&renter, &escrow_id, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidState
fn cannot_edit_milestones_after_funding() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "extra"), &10_0000000i128, &0u64);
}

#[test]
fn minority_juror_is_slashed_and_majority_rewarded() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let maj1 = Address::generate(&env);
    let maj2 = Address::generate(&env);
    let minority = Address::generate(&env);
    for j in [&maj1, &maj2, &minority] {
        token_admin_client.mint(j, &200_0000000);
        client.register_juror(j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert_eq!(dispute.jurors.len(), 3);

    // host wins 2-1: maj1 and maj2 vote for host, minority votes for renter
    client.vote_dispute(&maj1, &escrow_id, &0, &false);
    client.vote_dispute(&maj2, &escrow_id, &0, &false);
    client.vote_dispute(&minority, &escrow_id, &0, &true);
    client.resolve_dispute(&escrow_id, &0);

    // 10% of 100 = 10, split evenly between the two majority jurors = 5 each
    assert_eq!(client.get_juror_stake(&minority).unwrap().amount, 90_0000000i128);
    assert_eq!(client.get_juror_stake(&maj1).unwrap().amount, 105_0000000i128);
    assert_eq!(client.get_juror_stake(&maj2).unwrap().amount, 105_0000000i128);
}

#[test]
fn slash_with_no_matching_asset_majority_juror_is_not_misdirected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    let other_token_admin_client = create_token_contract(&env, &admin);
    let other_asset_address = other_token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let maj1 = Address::generate(&env);
    let maj2 = Address::generate(&env);
    let minority = Address::generate(&env);
    token_admin_client.mint(&maj1, &200_0000000);
    token_admin_client.mint(&maj2, &200_0000000);
    other_token_admin_client.mint(&minority, &200_0000000);
    client.register_juror(&maj1, &asset_address, &100_0000000);
    client.register_juror(&maj2, &asset_address, &100_0000000);
    client.register_juror(&minority, &other_asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.vote_dispute(&maj1, &escrow_id, &0, &false);
    client.vote_dispute(&maj2, &escrow_id, &0, &false);
    client.vote_dispute(&minority, &escrow_id, &0, &true);
    client.resolve_dispute(&escrow_id, &0);

    // minority still gets slashed even though nobody on the majority side
    // shares their staking asset...
    assert_eq!(client.get_juror_stake(&minority).unwrap().amount, 90_0000000i128);
    // ...but the majority jurors' stake is untouched, not credited from an
    // asset they never staked.
    assert_eq!(client.get_juror_stake(&maj1).unwrap().amount, 100_0000000i128);
    assert_eq!(client.get_juror_stake(&maj2).unwrap().amount, 100_0000000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #22)")] // SameParty
fn cannot_create_escrow_with_same_renter_and_host() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    client.create_escrow(&renter, &renter, &asset_address, &milestones, &false);
}

#[test]
#[should_panic(expected = "Error(Contract, #23)")] // MissingEvidence
fn raise_dispute_requires_non_empty_evidence() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, ""));
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")] // NonChronologicalMilestones
fn create_escrow_rejects_out_of_order_offsets() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-out"), 40_0000000i128, 999_999u64));
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")] // NonChronologicalMilestones
fn add_milestone_rejects_offset_earlier_than_previous() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "too-early"), &10_0000000i128, &0u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #25)")] // ContractPaused
fn paused_contract_rejects_new_escrows() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_paused(&admin, &true);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
}

#[test]
fn pausing_does_not_freeze_already_active_escrows() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    // pause hits *after* this escrow is already active
    client.set_paused(&admin, &true);
    assert!(client.is_paused());

    // existing funded escrows must still be serviceable
    client.confirm_milestone(&renter, &escrow_id, &0);
    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Completed);
}

#[test]
fn custom_slash_rate_is_applied_instead_of_default() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    // double the default slash rate: 20% instead of 10%
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &2000, &i32::MIN, &0);
    assert_eq!(client.get_juror_params().slash_bps, 2000);

    let maj1 = Address::generate(&env);
    let maj2 = Address::generate(&env);
    let minority = Address::generate(&env);
    for j in [&maj1, &maj2, &minority] {
        token_admin_client.mint(j, &200_0000000);
        client.register_juror(j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.vote_dispute(&maj1, &escrow_id, &0, &false);
    client.vote_dispute(&maj2, &escrow_id, &0, &false);
    client.vote_dispute(&minority, &escrow_id, &0, &true);
    client.resolve_dispute(&escrow_id, &0);

    // 20% of 100 = 20, split evenly = 10 each
    assert_eq!(client.get_juror_stake(&minority).unwrap().amount, 80_0000000i128);
    assert_eq!(client.get_juror_stake(&maj1).unwrap().amount, 110_0000000i128);
    assert_eq!(client.get_juror_stake(&maj2).unwrap().amount, 110_0000000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #26)")] // SlashTooHigh
fn slash_rate_above_cap_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &5001, &i32::MIN, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #27)")] // ReputationTooLow
fn low_reputation_address_cannot_register_as_juror_once_gated() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let bad_host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);
    token_admin_client.mint(&bad_host, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // give bad_host negative reputation by losing a dispute as host
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }
    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &bad_host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&bad_host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    for j in dispute.jurors.iter() {
        client.vote_dispute(&j, &escrow_id, &0, &true); // renter wins, bad_host loses
    }
    client.resolve_dispute(&escrow_id, &0);
    assert!(client.get_reputation(&bad_host) < 0);

    // now gate juror registration on non-negative reputation
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &DEFAULT_JUROR_SLASH_BPS, &0, &0);
    client.register_juror(&bad_host, &asset_address, &100_0000000);
}

#[test]
fn neutral_reputation_address_can_still_register_once_gated() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let juror = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&juror, &200_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &DEFAULT_JUROR_SLASH_BPS, &0, &0);

    // default reputation (0) meets a min_reputation of 0
    client.register_juror(&juror, &asset_address, &100_0000000);
    assert!(client.get_juror_stake(&juror).is_some());
}

#[test]
fn confirm_all_milestones_releases_every_remaining_one() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "midterm"), 30_0000000i128, 500u64));
    milestones.push_back((String::from_str(&env, "move-out"), 40_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.confirm_all_milestones(&renter, &escrow_id);

    assert_eq!(token_client.balance(&host), 100_0000000i128);
    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Completed);
    for i in 0..3 {
        assert!(escrow.milestones.get(i).unwrap().released);
    }
}

#[test]
fn confirm_all_milestones_skips_already_released_ones() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 40_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.confirm_milestone(&renter, &escrow_id, &0);
    assert_eq!(token_client.balance(&host), 30_0000000i128);

    client.confirm_all_milestones(&renter, &escrow_id);
    assert_eq!(token_client.balance(&host), 70_0000000i128);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Completed);
}

#[test]
fn renter_can_extend_a_milestone_deadline() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-out"), 50_0000000i128, 1000u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    let before = client.get_escrow(&escrow_id).milestones.get(0).unwrap().auto_release_at;
    client.extend_milestone_deadline(&renter, &escrow_id, &0, &500);
    let after = client.get_escrow(&escrow_id).milestones.get(0).unwrap().auto_release_at;
    assert_eq!(after, before + 500);

    // check_auto_release should still refuse before the *new* deadline
    let err = client.try_check_auto_release(&escrow_id, &0);
    assert!(err.is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")] // InvalidAmount
fn extend_milestone_deadline_rejects_excessive_extension() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-out"), 50_0000000i128, 1000u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.extend_milestone_deadline(&renter, &escrow_id, &0, &(366 * 24 * 60 * 60));
}

#[test]
fn party_escrow_index_tracks_both_renter_and_host() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let other_host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));

    let escrow_1 = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    let escrow_2 = client.create_escrow(&renter, &other_host, &asset_address, &milestones, &false);

    let renter_escrows = client.get_escrows_for_party(&renter);
    assert_eq!(renter_escrows.len(), 2);
    assert!(renter_escrows.contains(&escrow_1));
    assert!(renter_escrows.contains(&escrow_2));

    let host_escrows = client.get_escrows_for_party(&host);
    assert_eq!(host_escrows.len(), 1);
    assert_eq!(host_escrows.get(0).unwrap(), escrow_1);

    let stranger_escrows = client.get_escrows_for_party(&Address::generate(&env));
    assert!(stranger_escrows.is_empty());
}

#[test]
#[should_panic(expected = "Error(Contract, #28)")] // HostAcceptancePending
fn deposit_blocked_until_host_accepts_when_gated() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.deposit(&renter, &escrow_id);
}

#[test]
fn deposit_succeeds_after_host_accepts_gated_escrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.accept_escrow(&host, &escrow_id);
    client.deposit(&renter, &escrow_id);

    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Active);
}

#[test]
fn host_can_reject_a_gated_escrow_before_funding() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.reject_escrow(&host, &escrow_id);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn only_host_can_accept_escrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.accept_escrow(&renter, &escrow_id);
}

#[test]
fn resolving_one_milestones_dispute_does_not_erase_another_milestones_record() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    // dispute + resolve milestone 0
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence-0"));
    let dispute_0 = client.get_dispute(&escrow_id, &0).unwrap();
    for j in dispute_0.jurors.iter() {
        client.vote_dispute(&j, &escrow_id, &0, &true);
    }
    client.resolve_dispute(&escrow_id, &0);

    // now dispute + resolve milestone 1 - a *different* dispute, same escrow
    client.raise_dispute(&host, &escrow_id, &1, &String::from_str(&env, "ipfs://evidence-1"));
    let dispute_1 = client.get_dispute(&escrow_id, &1).unwrap();
    for j in dispute_1.jurors.iter() {
        client.vote_dispute(&j, &escrow_id, &1, &false);
    }
    client.resolve_dispute(&escrow_id, &1);

    // milestone 0's resolved record must still be there, untouched by
    // milestone 1's dispute reusing the same escrow.
    let dispute_0_after = client.get_dispute(&escrow_id, &0).unwrap();
    assert!(dispute_0_after.resolved);
    assert_eq!(dispute_0_after.outcome, DisputeOutcome::RenterWins);
    assert_eq!(dispute_0_after.evidence_uri, String::from_str(&env, "ipfs://evidence-0"));

    let dispute_1_after = client.get_dispute(&escrow_id, &1).unwrap();
    assert!(dispute_1_after.resolved);
    assert_eq!(dispute_1_after.outcome, DisputeOutcome::HostWins);
}

#[test]
fn admin_can_be_rotated_and_new_admin_takes_effect() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let treasury = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert_eq!(client.get_admin(), admin);

    client.transfer_admin(&admin, &new_admin);
    // proposing alone must not switch admin over yet
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_pending_admin(), Some(new_admin.clone()));

    client.accept_admin_transfer(&new_admin);
    assert_eq!(client.get_admin(), new_admin);
    assert!(client.get_pending_admin().is_none());

    // the old admin key no longer has any power...
    let err = client.try_set_fee_config(&admin, &500, &treasury);
    assert!(err.is_err());
    // ...only the new one does
    client.set_fee_config(&new_admin, &500, &treasury);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn only_the_proposed_address_can_accept_admin_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let stranger = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.transfer_admin(&admin, &new_admin);
    client.accept_admin_transfer(&stranger);
}

#[test]
fn old_admin_retains_power_until_transfer_is_accepted() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let treasury = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // a typo'd/unresponsive new_admin doesn't brick the contract - the old
    // admin key still works until the handoff is actually accepted
    client.transfer_admin(&admin, &new_admin);
    client.set_fee_config(&admin, &500, &treasury);
    assert_eq!(client.get_fee_config().unwrap().bps, 500);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn only_current_admin_can_transfer_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.transfer_admin(&stranger, &new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #29)")] // StringTooLong
fn create_escrow_rejects_oversized_milestone_description() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let huge = "x".repeat(513);
    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, &huge), 60_0000000i128, 0u64));
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
}

#[test]
#[should_panic(expected = "Error(Contract, #29)")] // StringTooLong
fn raise_dispute_rejects_oversized_evidence() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    let huge = "x".repeat(513);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, &huge));
}

fn count_complete_events(env: &Env) -> usize {
    env.events()
        .all()
        .iter()
        .filter(|(_, topics, _)| {
            soroban_sdk::Symbol::try_from_val(env, &topics.get_unchecked(1))
                == Ok(symbol_short!("complete"))
        })
        .count()
}

#[test]
fn completed_event_fires_only_on_the_final_milestone() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    client.confirm_milestone(&renter, &escrow_id, &0);
    assert_eq!(count_complete_events(&env), 0);

    client.confirm_milestone(&renter, &escrow_id, &1);
    assert_eq!(count_complete_events(&env), 1);
}

#[test]
fn completed_event_fires_via_dispute_resolution_too() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    for j in dispute.jurors.iter() {
        client.vote_dispute(&j, &escrow_id, &0, &true);
    }
    assert_eq!(count_complete_events(&env), 0);

    client.resolve_dispute(&escrow_id, &0);
    assert_eq!(count_complete_events(&env), 1);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Completed);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")] // TooManyMilestones
fn create_escrow_rejects_too_many_milestones() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    for i in 0..51u64 {
        milestones.push_back((String::from_str(&env, "m"), 1_0000000i128, i));
    }
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
}

#[test]
#[should_panic(expected = "Error(Contract, #30)")] // TooManyMilestones
fn add_milestone_rejects_past_the_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    for i in 0..50u64 {
        milestones.push_back((String::from_str(&env, "m"), 1_0000000i128, i));
    }
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "one too many"), &1_0000000i128, &50u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #31)")] // NotYetExpired
fn expire_unfunded_escrow_too_early_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.expire_unfunded_escrow(&escrow_id);
}

#[test]
fn expire_unfunded_escrow_after_window_succeeds() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 30 * 24 * 60 * 60 + 1);

    client.expire_unfunded_escrow(&escrow_id);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Cancelled);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidState
fn cannot_expire_an_already_funded_escrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 30 * 24 * 60 * 60 + 1);

    client.expire_unfunded_escrow(&escrow_id);
}

#[test]
fn escrow_count_tracks_total_created() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert_eq!(client.get_escrow_count(), 0);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 60_0000000i128, 0u64));
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    assert_eq!(client.get_escrow_count(), 2);
}

#[test]
fn active_dispute_count_reflects_open_assignments() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &1, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &0);

    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);
    assert_eq!(client.get_active_dispute_count(&juror), 0);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
    assert_eq!(client.get_active_dispute_count(&juror), 1);

    client.vote_dispute(&juror, &escrow_id, &0, &true);
    client.resolve_dispute(&escrow_id, &0);
    assert_eq!(client.get_active_dispute_count(&juror), 0);
}

#[test]
fn dispute_voting_window_is_configurable() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert_eq!(client.get_dispute_voting_window(), 3 * 24 * 60 * 60);

    let one_day = 24 * 60 * 60;
    client.set_dispute_voting_window(&admin, &one_day);
    assert_eq!(client.get_dispute_voting_window(), one_day);

    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    // shorter window means force-resolve is available sooner than the old
    // 3-day default would have allowed
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + one_day + 1);
    client.force_resolve_stale_dispute(&escrow_id, &0);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Completed);
}

#[test]
#[should_panic(expected = "Error(Contract, #32)")] // InvalidVotingWindow
fn dispute_voting_window_rejects_out_of_bounds_values() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_dispute_voting_window(&admin, &30); // well under the 1-hour floor
}

#[test]
fn jurors_earn_arbitration_fee_on_unanimous_resolved_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    // 5% arbitration fee, 3 jurors
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &500);

    let jurors = [Address::generate(&env), Address::generate(&env), Address::generate(&env)];
    for j in &jurors {
        token_admin_client.mint(j, &200_0000000);
        client.register_juror(j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 300_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let juror_balances_before: [i128; 3] = jurors.each_ref().map(|j| token_client.balance(j));
    let renter_balance_before = token_client.balance(&renter);

    // everyone agrees - no minority, so no slashing, but jurors should
    // still earn the arbitration fee for doing the work
    for j in &jurors {
        client.vote_dispute(j, &escrow_id, &0, &true);
    }
    client.resolve_dispute(&escrow_id, &0);

    // 5% of 300 = 15, split 3 ways = 5 each
    for (j, before) in jurors.iter().zip(juror_balances_before.iter()) {
        assert_eq!(token_client.balance(j) - before, 5_0000000i128);
    }
    // renter wins 300, minus the 15 arbitration fee = 285
    assert_eq!(token_client.balance(&renter) - renter_balance_before, 285_0000000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")] // ArbitrationFeeTooHigh
fn arbitration_fee_above_cap_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &1001);
}

#[test]
fn stale_dispute_fallback_does_not_pay_arbitration_fee() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &3, &DEFAULT_JUROR_SLASH_BPS, &i32::MIN, &500);

    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 100_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let juror_balance_before = token_client.balance(&juror);
    let renter_balance_before = token_client.balance(&renter);

    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 3 * 24 * 60 * 60 + 1);
    client.force_resolve_stale_dispute(&escrow_id, &0);

    // jurors never voted, so they get nothing here - only resolve_dispute
    // pays the arbitration fee
    assert_eq!(token_client.balance(&juror), juror_balance_before);
    assert_eq!(token_client.balance(&renter) - renter_balance_before, 50_0000000i128);
    assert_eq!(token_client.balance(&host), 50_0000000i128);
}

#[test]
fn either_party_can_add_supplementary_dispute_evidence() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://host-evidence"));

    // renter (who didn't open it) responds with their own evidence
    client.add_dispute_evidence(&renter, &escrow_id, &0, &String::from_str(&env, "ipfs://renter-rebuttal"));
    // the opener can also add more
    client.add_dispute_evidence(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://host-followup"));

    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert_eq!(dispute.evidence_uri, String::from_str(&env, "ipfs://host-evidence"));
    assert_eq!(dispute.additional_evidence.len(), 2);
    assert_eq!(dispute.additional_evidence.get(0).unwrap(), String::from_str(&env, "ipfs://renter-rebuttal"));
    assert_eq!(dispute.additional_evidence.get(1).unwrap(), String::from_str(&env, "ipfs://host-followup"));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn stranger_cannot_add_dispute_evidence() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let stranger = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.add_dispute_evidence(&stranger, &escrow_id, &0, &String::from_str(&env, "ipfs://noise"));
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")] // TooMuchEvidence
fn cannot_exceed_the_additional_evidence_cap() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    let juror = Address::generate(&env);
    token_admin_client.mint(&juror, &200_0000000);
    client.register_juror(&juror, &asset_address, &100_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    for _ in 0..10 {
        client.add_dispute_evidence(&renter, &escrow_id, &0, &String::from_str(&env, "ipfs://more"));
    }
    // the 11th push should fail
    client.add_dispute_evidence(&renter, &escrow_id, &0, &String::from_str(&env, "ipfs://one-too-many"));
}

#[test]
fn fee_exempt_host_receives_full_milestone_amount() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let exempt_host = Address::generate(&env);
    let regular_host = Address::generate(&env);
    let treasury = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_fee_config(&admin, &1000, &treasury); // 10%
    client.set_fee_exempt(&admin, &exempt_host, &true);
    assert!(client.is_fee_exempt(&exempt_host));

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "check-in deposit"), 100_0000000i128, 0u64));

    // exempt host keeps the full amount, no fee taken
    let escrow_1 = client.create_escrow(&renter, &exempt_host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_1);
    client.confirm_milestone(&renter, &escrow_1, &0);
    assert_eq!(token_client.balance(&exempt_host), 100_0000000i128);

    // regular host still pays the normal fee
    let escrow_2 = client.create_escrow(&renter, &regular_host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_2);
    client.confirm_milestone(&renter, &escrow_2, &0);
    assert_eq!(token_client.balance(&regular_host), 90_0000000i128);
    assert_eq!(token_client.balance(&treasury), 10_0000000i128);
}

#[test]
fn fee_exemption_can_be_revoked() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let party = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_exempt(&admin, &party, &true);
    assert!(client.is_fee_exempt(&party));

    client.set_fee_exempt(&admin, &party, &false);
    assert!(!client.is_fee_exempt(&party));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn only_admin_can_set_fee_exemption() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let party = Address::generate(&env);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    client.set_fee_exempt(&stranger, &party, &true);
}

#[test]
fn stake_weighted_voting_can_flip_a_headcount_majority() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    client.set_stake_weighted_voting(&admin, &true);
    assert!(client.get_stake_weighted_voting());

    // two minimum-stake jurors outvote one much-larger-staked juror 2-to-1
    // by headcount, but the big staker's weight should win under
    // stake-weighted tallying
    let small1 = Address::generate(&env);
    let small2 = Address::generate(&env);
    let whale = Address::generate(&env);
    token_admin_client.mint(&small1, &200_0000000);
    token_admin_client.mint(&small2, &200_0000000);
    token_admin_client.mint(&whale, &2_000_0000000);
    client.register_juror(&small1, &asset_address, &100_0000000);
    client.register_juror(&small2, &asset_address, &100_0000000);
    client.register_juror(&whale, &asset_address, &1_000_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.vote_dispute(&small1, &escrow_id, &0, &true); // renter
    client.vote_dispute(&small2, &escrow_id, &0, &true); // renter
    client.vote_dispute(&whale, &escrow_id, &0, &false); // host
    client.resolve_dispute(&escrow_id, &0);

    // host wins on stake weight (1000 > 100+100) despite losing 1-2 on
    // headcount
    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert_eq!(dispute.outcome, DisputeOutcome::HostWins);
}

#[test]
fn flat_voting_is_unaffected_when_weighting_disabled() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert!(!client.get_stake_weighted_voting());

    let small1 = Address::generate(&env);
    let small2 = Address::generate(&env);
    let whale = Address::generate(&env);
    token_admin_client.mint(&small1, &200_0000000);
    token_admin_client.mint(&small2, &200_0000000);
    token_admin_client.mint(&whale, &2_000_0000000);
    client.register_juror(&small1, &asset_address, &100_0000000);
    client.register_juror(&small2, &asset_address, &100_0000000);
    client.register_juror(&whale, &asset_address, &1_000_0000000);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.vote_dispute(&small1, &escrow_id, &0, &true);
    client.vote_dispute(&small2, &escrow_id, &0, &true);
    client.vote_dispute(&whale, &escrow_id, &0, &false);
    client.resolve_dispute(&escrow_id, &0);

    // headcount wins by default: 2-1 renter
    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    assert_eq!(dispute.outcome, DisputeOutcome::RenterWins);
}

#[test]
fn mutual_extension_pushes_back_stale_resolution() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let original_deadline = client.get_dispute(&escrow_id, &0).unwrap().voting_deadline;
    let extra = 5 * 24 * 60 * 60;
    client.extend_dispute_deadline(&renter, &host, &escrow_id, &0, &extra);
    assert_eq!(client.get_dispute(&escrow_id, &0).unwrap().voting_deadline, original_deadline + extra);

    // jump past the *original* 3-day window - force-resolve should still
    // refuse since the deadline was pushed back
    let now = env.ledger().timestamp();
    env.ledger().set_timestamp(now + 3 * 24 * 60 * 60 + 1);
    let err = client.try_force_resolve_stale_dispute(&escrow_id, &0);
    assert!(err.is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")] // NotAuthorized
fn extend_dispute_deadline_requires_both_actual_parties() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);
    let stranger = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    client.extend_dispute_deadline(&renter, &stranger, &escrow_id, &0, &(24 * 60 * 60));
}

#[test]
fn host_earns_reputation_for_clean_completion() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    assert_eq!(client.get_reputation(&host), 0);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);

    // partial release shouldn't award anything yet - only full completion
    client.confirm_milestone(&renter, &escrow_id, &0);
    assert_eq!(client.get_reputation(&host), 0);

    client.confirm_milestone(&renter, &escrow_id, &1);
    assert_eq!(client.get_reputation(&host), 1);
}

#[test]
fn disputed_escrow_completion_does_not_double_award_reputation() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);
    for j in [Address::generate(&env), Address::generate(&env), Address::generate(&env)] {
        token_admin_client.mint(&j, &200_0000000);
        client.register_juror(&j, &asset_address, &100_0000000);
    }

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
    let dispute = client.get_dispute(&escrow_id, &0).unwrap();
    for j in dispute.jurors.iter() {
        client.vote_dispute(&j, &escrow_id, &0, &false); // host wins
    }
    client.resolve_dispute(&escrow_id, &0);

    // host wins the dispute (+2 from resolve_dispute's own outcome
    // adjustment) but the escrow was disputed, so no extra +1 clean-
    // completion bonus on top of that
    assert_eq!(client.get_reputation(&host), 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")] // InvalidState
fn cannot_raise_dispute_on_an_unfunded_escrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // an escrow that was created but never deposited into has no real
    // funds behind it - raising (and resolving) a dispute on it would
    // otherwise let resolve_dispute's token.transfer draw on the
    // contract's pooled balance from *other* escrows/stakes in this asset
    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "damage deposit"), 50_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Created);

    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));
}

#[test]
fn unfunded_dispute_attempt_cannot_drain_other_escrows_pooled_funds() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    // a legitimate, fully funded escrow whose money sits in the
    // contract's pooled balance for this asset
    let victim_renter = Address::generate(&env);
    let victim_host = Address::generate(&env);
    // an attacker pair trying to dispute an escrow they never funded
    let attacker_renter = Address::generate(&env);
    let attacker_host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let token_client = token::Client::new(&env, &token_admin_client.address);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&victim_renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut victim_milestones = Vec::new(&env);
    victim_milestones.push_back((String::from_str(&env, "real deposit"), 500_0000000i128, 999_999u64));
    let victim_escrow = client.create_escrow(&victim_renter, &victim_host, &asset_address, &victim_milestones, &false);
    client.deposit(&victim_renter, &victim_escrow);
    assert_eq!(token_client.balance(&contract_id), 500_0000000i128);

    let mut attacker_milestones = Vec::new(&env);
    attacker_milestones.push_back((String::from_str(&env, "fake claim"), 500_0000000i128, 999_999u64));
    let attacker_escrow = client.create_escrow(
        &attacker_renter,
        &attacker_host,
        &asset_address,
        &attacker_milestones,
        &false,
    );

    let err = client.try_raise_dispute(
        &attacker_host,
        &attacker_escrow,
        &0,
        &String::from_str(&env, "ipfs://fake-evidence"),
    );
    assert!(err.is_err());

    // the pooled balance backing the victim's real escrow is untouched
    assert_eq!(token_client.balance(&contract_id), 500_0000000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #28)")] // HostAcceptancePending
fn adding_a_milestone_after_acceptance_revokes_it() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.accept_escrow(&host, &escrow_id);
    assert!(client.get_escrow(&escrow_id).host_accepted);

    // renter sneaks in a bigger milestone after the host signed off on the
    // original, smaller terms
    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "surprise fee"), &500_0000000i128, &999_999u64);
    assert!(!client.get_escrow(&escrow_id).host_accepted);

    // deposit must be blocked until the host reviews and re-accepts the
    // actual terms they're now bound to
    client.deposit(&renter, &escrow_id);
}

#[test]
fn removing_a_milestone_after_acceptance_also_revokes_it() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.accept_escrow(&host, &escrow_id);
    client.remove_milestone(&renter, &escrow_id, &1);
    assert!(!client.get_escrow(&escrow_id).host_accepted);
}

#[test]
fn editing_milestones_on_an_ungated_escrow_does_not_touch_host_accepted() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    // requires_host_acceptance = false: host_accepted starts true and was
    // never a real consent signal, so editing terms shouldn't block it
    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);
    client.add_milestone(&renter, &escrow_id, &String::from_str(&env, "move-out"), &30_0000000i128, &999_999u64);
    assert!(client.get_escrow(&escrow_id).host_accepted);

    client.deposit(&renter, &escrow_id);
}

#[test]
fn get_escrow_status_and_get_milestone_match_get_escrow() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();
    token_admin_client.mint(&renter, &1_000_0000000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    assert_eq!(client.get_escrow_status(&escrow_id), EscrowStatus::Created);
    assert_eq!(
        client.get_milestone(&escrow_id, &1).description,
        String::from_str(&env, "move-out")
    );

    client.deposit(&renter, &escrow_id);
    assert_eq!(client.get_escrow_status(&escrow_id), EscrowStatus::Active);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")] // InvalidMilestone
fn get_milestone_rejects_out_of_range_index() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.get_milestone(&escrow_id, &5);
}

#[test]
fn update_milestone_edits_in_place_and_adjusts_total() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "midterm"), 30_0000000i128, 500u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    client.update_milestone(&renter, &escrow_id, &1, &String::from_str(&env, "midterm (revised)"), &50_0000000i128, &600u64);

    let escrow = client.get_escrow(&escrow_id);
    assert_eq!(escrow.milestones.len(), 3); // position preserved, not removed+re-added at the end
    assert_eq!(escrow.total_amount, 110_0000000i128); // 30 + 50 + 30
    let updated = client.get_milestone(&escrow_id, &1);
    assert_eq!(updated.description, String::from_str(&env, "midterm (revised)"));
    assert_eq!(updated.amount, 50_0000000i128);
    assert_eq!(updated.auto_release_offset, 600);
}

#[test]
#[should_panic(expected = "Error(Contract, #24)")] // NonChronologicalMilestones
fn update_milestone_rejects_offset_conflicting_with_next_neighbor() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    milestones.push_back((String::from_str(&env, "midterm"), 30_0000000i128, 500u64));
    milestones.push_back((String::from_str(&env, "move-out"), 30_0000000i128, 999_999u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &false);

    // pushing milestone 0's offset past milestone 1's breaks ordering
    client.update_milestone(&renter, &escrow_id, &0, &String::from_str(&env, "move-in"), &30_0000000i128, &600u64);
}

#[test]
fn update_milestone_on_gated_escrow_revokes_acceptance() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let renter = Address::generate(&env);
    let host = Address::generate(&env);

    let token_admin_client = create_token_contract(&env, &admin);
    let asset_address = token_admin_client.address.clone();

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let mut milestones = Vec::new(&env);
    milestones.push_back((String::from_str(&env, "move-in"), 30_0000000i128, 0u64));
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones, &true);

    client.accept_escrow(&host, &escrow_id);
    client.update_milestone(&renter, &escrow_id, &0, &String::from_str(&env, "move-in (bigger)"), &500_0000000i128, &0u64);
    assert!(!client.get_escrow(&escrow_id).host_accepted);
}
