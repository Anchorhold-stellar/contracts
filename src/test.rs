#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, token, Env, String};

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
