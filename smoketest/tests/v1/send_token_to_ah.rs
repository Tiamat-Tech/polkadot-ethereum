use alloy::primitives::{utils::parse_units, Address};
use futures::StreamExt;
use snowbridge_smoketest::{
	constants::*,
	contracts::weth9,
	helper::{initial_clients, print_event_log_for_unit_tests},
	parachains::assethub::api::{
		foreign_assets::events::Issued,
		runtime_types::staging_xcm::v4::{
			junction::{
				Junction::{AccountKey20, GlobalConsensus},
				NetworkId,
			},
			junctions::Junctions::X2,
			location::Location,
		},
	},
};
use subxt::{ext::codec::Encode, utils::AccountId32};

#[cfg(feature = "legacy-v1")]
use snowbridge_smoketest::contracts::i_gateway::IGateway;
#[cfg(not(feature = "legacy-v1"))]
use snowbridge_smoketest::contracts::i_gateway_v1::IGatewayV1 as IGateway;

#[tokio::test]
async fn send_token_to_ah() {
	let test_clients = initial_clients().await.expect("initialize clients");
	let ethereum_client = test_clients.ethereum_client;
	let assethub = *(test_clients.asset_hub_client.clone());

	let gateway_addr: Address = (*GATEWAY_PROXY_CONTRACT).into();
	let gateway = IGateway::new(gateway_addr, ethereum_client.clone());

	let weth_addr: Address = (*WETH_CONTRACT).into();
	let weth = weth9::WETH9::new(weth_addr, ethereum_client.clone());

	// Mint WETH tokens
	let value = parse_units("0.01", "ether").unwrap().get_absolute();
	let mut receipt = weth
		.deposit()
		.value(value)
		.gas_price(GAS_PRICE)
		.send()
		.await
		.unwrap()
		.get_receipt()
		.await
		.unwrap();
	assert_eq!(receipt.status(), true);

	// Approve token spend
	receipt = weth
		.approve(gateway_addr, value.into())
		.gas_price(GAS_PRICE)
		.send()
		.await
		.unwrap()
		.get_receipt()
		.await
		.unwrap();

	assert_eq!(receipt.status(), true);

	let destination_fee = 0;
	let fee = gateway
		.quoteSendTokenFee(*weth.address(), ASSET_HUB_PARA_ID, destination_fee)
		.call()
		.await
		.unwrap();

	// Lock tokens into vault
	let amount: u128 = value.to::<u128>();
	let receipt = gateway
		.sendToken(
			*weth.address(),
			ASSET_HUB_PARA_ID,
			IGateway::MultiAddress { kind: 1, data: (*SUBSTRATE_RECEIVER).into() },
			destination_fee,
			amount,
		)
		.value(fee)
		.gas_price(GAS_PRICE)
		.send()
		.await
		.unwrap()
		.get_receipt()
		.await
		.expect("get receipt");

	println!(
		"receipt transaction hash: {:#?}, transaction block: {:#?}",
		hex::encode(receipt.transaction_hash),
		receipt.block_number
	);

	// Log for OutboundMessageAccepted
	let outbound_message_accepted_log = receipt.logs().first().unwrap().as_ref();

	// print log for unit tests
	print_event_log_for_unit_tests(outbound_message_accepted_log);

	assert_eq!(receipt.status(), true);

	let wait_for_blocks = (*WAIT_PERIOD) as usize;
	let mut blocks = assethub
		.blocks()
		.subscribe_finalized()
		.await
		.expect("block subscription")
		.take(wait_for_blocks);

	let expected_asset_id: Location = Location {
		parents: 2,
		interior: X2([
			GlobalConsensus(NetworkId::Ethereum { chain_id: ETHEREUM_CHAIN_ID }),
			AccountKey20 { network: None, key: (*WETH_CONTRACT).into() },
		]),
	};
	let expected_owner: AccountId32 = (*SUBSTRATE_RECEIVER).into();

	let mut issued_event_found = false;
	while let Some(Ok(block)) = blocks.next().await {
		println!("Polling assethub block {} for issued event.", block.number());

		let events = block.events().await.unwrap();
		for issued in events.find::<Issued>() {
			println!("Created event found in assethub block {}.", block.number());
			let issued = issued.unwrap();
			assert_eq!(issued.asset_id.encode(), expected_asset_id.encode());
			assert_eq!(issued.owner, expected_owner);
			assert_eq!(issued.amount, amount);
			issued_event_found = true;
		}
		if issued_event_found {
			break
		}
	}
	assert!(issued_event_found)
}
