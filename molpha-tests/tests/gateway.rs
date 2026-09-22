//! Gateway registration and deactivation.

use anchor_lang::{InstructionData, ToAccountMetas};
use molpha::error::MolphaError;
use molpha::events::{GatewayDeactivated, GatewayRegistered};
use molpha::instructions::RegisterGatewayArgs;
use molpha::state::{Gateway, GatewayStatus};
use molpha_tests::env::{assert_anchor_error, assert_molpha_error, only_event};
use molpha_tests::pda;
use molpha_tests::scenario::Protocol;
use solana_signer::Signer;

fn args(authority: anchor_lang::prelude::Pubkey) -> RegisterGatewayArgs {
    RegisterGatewayArgs {
        authority,
        ip: [198, 51, 100, 4],
        port: 8_443,
    }
}

#[test]
fn protocol_authority_registers_a_gateway_without_a_bond() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let ix = p.register_gateway_ix(
        &gateway_authority.pubkey(),
        &args(gateway_authority.pubkey()),
    );
    let authority = p.authority.insecure_clone();
    let meta = p.env.send_ok(&[ix], &[&authority]);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    let gateway: Gateway = p.env.get(&gateway_pda);
    assert_eq!(gateway.authority, gateway_authority.pubkey());
    assert_eq!(gateway.ip, [198, 51, 100, 4]);
    assert_eq!(gateway.port, 8_443);
    assert_eq!(gateway.status, GatewayStatus::Active);
    assert_eq!(gateway.registered_at, p.env.unix_timestamp());

    let event: GatewayRegistered = only_event(&meta);
    assert_eq!(event.authority, gateway_authority.pubkey());
    assert_eq!(event.ip, gateway.ip);
    assert_eq!(event.port, gateway.port);
}

#[test]
fn registration_rejects_unroutable_endpoints() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let authority = p.authority.insecure_clone();

    for invalid in [
        RegisterGatewayArgs {
            ip: [0; 4],
            ..args(gateway_authority.pubkey())
        },
        RegisterGatewayArgs {
            port: 0,
            ..args(gateway_authority.pubkey())
        },
    ] {
        let ix = p.register_gateway_ix(&gateway_authority.pubkey(), &invalid);
        let failed = p.env.send_err(&[ix], &[&authority]);
        assert_molpha_error(&failed, MolphaError::InvalidGatewayEndpoint);
    }
    assert!(!p
        .env
        .account_exists(&pda::gateway(&gateway_authority.pubkey())));
}

#[test]
fn registration_requires_the_protocol_authority() {
    let mut p = Protocol::start();
    let gateway_authority = p.env.keypair();
    let impostor = p.env.keypair();
    let gateway = pda::gateway(&gateway_authority.pubkey());
    let ix = anchor_lang::solana_program::instruction::Instruction {
        program_id: molpha::ID,
        accounts: molpha::accounts::RegisterGateway {
            authority: impostor.pubkey(),
            protocol_config: pda::protocol_config(),
            gateway,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None),
        data: molpha::instruction::RegisterGateway {
            args: args(gateway_authority.pubkey()),
        }
        .data(),
    };
    let failed = p.env.send_err(&[ix], &[&impostor]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
    assert!(!p.env.account_exists(&gateway));
}

#[test]
fn a_gateway_authority_can_only_be_registered_once() {
    let mut p = Protocol::start();
    let gateway_authority = p.register_gateway(0);
    let ix = p.register_gateway_ix(
        &gateway_authority.pubkey(),
        &args(gateway_authority.pubkey()),
    );
    let authority = p.authority.insecure_clone();
    let failed = p.env.send_err(&[ix], &[&authority]);
    assert!(failed
        .meta
        .logs
        .iter()
        .any(|line| line.contains("already in use")));
}

#[test]
fn protocol_authority_deactivates_a_gateway() {
    let mut p = Protocol::start();
    let gateway_authority = p.register_gateway(0);
    let authority = p.authority.insecure_clone();
    let ix = p.deactivate_gateway_ix(&authority.pubkey(), &gateway_authority.pubkey());
    let meta = p.env.send_ok(&[ix], &[&authority]);

    let gateway_pda = pda::gateway(&gateway_authority.pubkey());
    assert_eq!(
        p.env.get::<Gateway>(&gateway_pda).status,
        GatewayStatus::Deactivated
    );
    let event: GatewayDeactivated = only_event(&meta);
    assert_eq!(event.gateway, gateway_pda);
    assert_eq!(event.authority, gateway_authority.pubkey());
}

#[test]
fn gateway_authority_cannot_deactivate_itself() {
    let mut p = Protocol::start();
    let gateway_authority = p.register_gateway(0);
    let ix = p.deactivate_gateway_ix(&gateway_authority.pubkey(), &gateway_authority.pubkey());
    let failed = p.env.send_err(&[ix], &[&gateway_authority]);
    assert_anchor_error(&failed, anchor_lang::error::ErrorCode::ConstraintHasOne);
}
