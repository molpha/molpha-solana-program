//! `register_gateway`.

use molpha::error::MolphaError;
use molpha::events::GatewayRegistered;
use molpha::instructions::RegisterGatewayArgs;
use molpha::state::{Gateway, GatewayStatus};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use molpha_tests::token;
use solana_signer::Signer;

fn args(bond_amount: u64) -> RegisterGatewayArgs {
    RegisterGatewayArgs {
        ip: [198, 51, 100, 4],
        port: 8_443,
        bond_amount,
    }
}

fn register_gateway(
    p: &mut Protocol,
    gateway_authority: &solana_keypair::Keypair,
    bond: u64,
) -> litesvm::types::TransactionMetadata {
    p.fund_usdc(&p.authority.pubkey(), bond);
    let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &args(bond));
    let protocol_authority = p.authority.insecure_clone();
    p.env.send_ok(&[ix], &[&protocol_authority])
}

#[test]
fn register_gateway_moves_the_bond_into_a_gateway_owned_escrow() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let bond = p.args.gateway_bond_min;
    let protocol_usdc = p.fund_usdc(&p.authority.pubkey(), bond * 3);
    let before = p.env.token_balance(&protocol_usdc);

    let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &args(bond));
    let protocol_authority = p.authority.insecure_clone();
    let meta = p.env.send_ok(&[ix], &[&protocol_authority]);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    let gateway: Gateway = p.env.get(&gateway_pda);
    assert_eq!(gateway.authority, gateway_authority.pubkey());
    assert_eq!(gateway.ip, [198, 51, 100, 4]);
    assert_eq!(gateway.port, 8_443);
    assert_eq!(gateway.status, GatewayStatus::Active);
    assert_eq!(gateway.registered_at, p.env.unix_timestamp());
    assert_eq!(gateway.withdrawable_slot, 0);

    // The bond lives in an ATA owned by the gateway PDA, not by the authority and not by the
    // treasury — dispute slashing depends on that separation.
    let bond_ata = pda::ata(&gateway_pda, &p.usdc_mint);
    let state = token::unpack_token_account(&p.env.account_data(&bond_ata));
    assert_eq!(state.owner, gateway_pda);
    assert_eq!(state.amount, bond);
    assert_eq!(p.env.token_balance(&protocol_usdc), before - bond);
    assert_eq!(p.treasury_balance(), 0);

    let event: GatewayRegistered = only_event(&meta);
    assert_eq!(event.authority, gateway_authority.pubkey());
    assert_eq!(event.bond, bond);
}

#[test]
fn register_gateway_accepts_a_bond_above_the_minimum() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let bond = p.args.gateway_bond_min * 2;

    register_gateway(&mut p, &gateway_authority, bond);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    assert_eq!(
        p.env.token_balance(&pda::ata(&gateway_pda, &p.usdc_mint)),
        bond
    );
}

#[test]
fn register_gateway_rejects_a_bond_below_the_minimum() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let bond = p.args.gateway_bond_min;
    p.fund_usdc(&p.authority.pubkey(), bond * 2);

    let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &args(bond - 1));
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert_molpha_error(&failed, MolphaError::InsufficientBond);
    assert!(!p.env.account_exists(&pda::gateway(&gateway_authority.pubkey())));
}

#[test]
fn register_gateway_rejects_an_unroutable_endpoint() {
    let mut p = Protocol::start();
    let bond = p.args.gateway_bond_min;

    let zero_ip_gateway = p.env.keypair();
    p.fund_usdc(&p.authority.pubkey(), bond * 2);
    let ix = p.register_gateway_ix(
        &zero_ip_gateway.pubkey(),
        &RegisterGatewayArgs {
            ip: [0, 0, 0, 0],
            ..args(bond)
        },
    );
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert_molpha_error(&failed, MolphaError::InvalidGatewayEndpoint);

    let zero_port_gateway = p.env.keypair();
    p.fund_usdc(&p.authority.pubkey(), bond * 2);
    let ix = p.register_gateway_ix(
        &zero_port_gateway.pubkey(),
        &RegisterGatewayArgs {
            port: 0,
            ..args(bond)
        },
    );
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert_molpha_error(&failed, MolphaError::InvalidGatewayEndpoint);
}

#[test]
fn register_gateway_rejects_a_non_protocol_authority() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let impostor = p.env.keypair();
    let bond = p.args.gateway_bond_min;
    p.fund_usdc(&impostor.pubkey(), bond);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    let ix = anchor_lang::solana_program::instruction::Instruction {
        program_id: molpha::ID,
        accounts: anchor_lang::ToAccountMetas::to_account_metas(
            &molpha::accounts::RegisterGateway {
                authority: impostor.pubkey(),
                gateway_authority: gateway_authority.pubkey(),
                protocol_config: pda::protocol_config(),
                gateway: gateway_pda,
                usdc_mint: p.usdc_mint,
                authority_usdc: pda::ata(&impostor.pubkey(), &p.usdc_mint),
                gateway_bond: pda::ata(&gateway_pda, &p.usdc_mint),
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
            },
            None,
        ),
        data: anchor_lang::InstructionData::data(&molpha::instruction::RegisterGateway {
            args: args(bond),
        }),
    };
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
    assert!(!p.env.account_exists(&gateway_pda));
}

#[test]
fn a_gateway_authority_can_only_register_once() {
    let mut p = Protocol::start();
    let gateway_authority = p.register_gateway(0);

    let bond = p.args.gateway_bond_min;
    p.fund_usdc(&p.authority.pubkey(), bond * 2);
    let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &args(bond));
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("already in use")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
}

#[test]
fn register_gateway_rejects_a_mint_that_is_not_the_settlement_mint() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let bond = p.args.gateway_bond_min;
    let (other_mint, other_authority) = p.env.create_mint(6);
    p.env
        .mint_to(&other_mint, &other_authority, &p.authority.pubkey(), bond * 2);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    let ix = anchor_lang::solana_program::instruction::Instruction {
        program_id: molpha::ID,
        accounts: anchor_lang::ToAccountMetas::to_account_metas(
            &molpha::accounts::RegisterGateway {
                authority: p.authority.pubkey(),
                gateway_authority: gateway_authority.pubkey(),
                protocol_config: pda::protocol_config(),
                gateway: gateway_pda,
                usdc_mint: other_mint,
                authority_usdc: pda::ata(&p.authority.pubkey(), &other_mint),
                gateway_bond: pda::ata(&gateway_pda, &other_mint),
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
            },
            None,
        ),
        data: anchor_lang::InstructionData::data(&molpha::instruction::RegisterGateway {
            args: args(bond),
        }),
    };
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert_molpha_error(&failed, MolphaError::InvalidOwnerUsdc);
}

#[test]
fn register_gateway_rejects_a_protocol_authority_who_cannot_fund_the_bond() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let bond = p.args.gateway_bond_min;
    p.fund_usdc(&p.authority.pubkey(), bond - 1);

    let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &args(bond));
    let protocol_authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&protocol_authority]);
    assert!(
        failed
            .meta
            .logs
            .iter()
            .any(|l| l.contains("insufficient funds")),
        "logs:\n{}",
        failed.meta.logs.join("\n")
    );
    assert!(!p.env.account_exists(&pda::gateway(&gateway_authority.pubkey())));
}

#[test]
fn gateways_are_independent_of_one_another() {
    let mut p = Protocol::start();
    let first = p.register_gateway(0);
    let second = p.register_gateway(0);

    assert_ne!(
        pda::gateway(&first.pubkey()),
        pda::gateway(&second.pubkey())
    );
    let a: Gateway = p.env.get(&pda::gateway(&first.pubkey()));
    let b: Gateway = p.env.get(&pda::gateway(&second.pubkey()));
    assert_eq!(a.authority, first.pubkey());
    assert_eq!(b.authority, second.pubkey());
    assert_eq!(
        p.env
            .token_balance(&pda::ata(&pda::gateway(&first.pubkey()), &p.usdc_mint)),
        p.env
            .token_balance(&pda::ata(&pda::gateway(&second.pubkey()), &p.usdc_mint))
    );
}
