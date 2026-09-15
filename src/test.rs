#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _, testutils::Events as _, token, Env, String, TryFromVal,
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

    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);

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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);

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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
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
    client.set_juror_params(&admin, &500_0000000, &1);
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
    let escrow_id = client.create_escrow(&renter, &host, &asset_address, &milestones);
    client.deposit(&renter, &escrow_id);
    client.raise_dispute(&host, &escrow_id, &0, &String::from_str(&env, "ipfs://evidence"));

    let dispute = client.get_dispute(&escrow_id).unwrap();
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

    client.set_juror_params(&admin, &DEFAULT_MIN_JUROR_STAKE, &4);
}
