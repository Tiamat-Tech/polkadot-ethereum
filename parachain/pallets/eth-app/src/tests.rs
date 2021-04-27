use crate::mock::{new_tester, AccountId, Origin, Event, System, Assets, ETHApp};
use frame_support::{assert_ok, assert_noop, dispatch::DispatchError};
use sp_keyring::AccountKeyring as Keyring;
use sp_core::H160;
use crate::RawEvent;

use artemis_core::ChannelId;

use artemis_tokens::multi::{Mutate};

fn last_event() -> Event {
	System::events().pop().expect("Event expected").event
}

const ASSET: artemis_core::AssetId = artemis_core::AssetId::Ether;

#[test]
fn mints_after_handling_ethereum_event() {
	new_tester().execute_with(|| {
		let peer_contract = H160::repeat_byte(1);
		let sender = H160::repeat_byte(7);
		let recipient: AccountId = Keyring::Bob.into();
		let amount = 10;
		assert_ok!(
			ETHApp::mint(
				artemis_dispatch::Origin(peer_contract).into(),
				sender,
				recipient.clone(),
				amount.into()
			)
		);
		assert_eq!(Assets::balance(ASSET, &recipient), amount.into());

		assert_eq!(
			Event::eth_app(RawEvent::Minted(sender, recipient, amount.into())),
			last_event()
		);
	});
}

#[test]
fn burn_should_emit_bridge_event() {
	new_tester().execute_with(|| {
		let recipient = H160::repeat_byte(2);
		let bob: AccountId = Keyring::Bob.into();

		Assets::mint(ASSET, &bob, 500.into()).unwrap();

		assert_ok!(ETHApp::burn(
			Origin::signed(bob.clone()),
			ChannelId::Incentivized,
			recipient.clone(),
			20.into()));

		assert_eq!(
			Event::eth_app(RawEvent::Burned(bob, recipient, 20.into())),
			last_event()
		);
	});
}

#[test]
fn should_not_burn_on_commitment_failure() {
	new_tester().execute_with(|| {
		let sender: AccountId = Keyring::Bob.into();
		let recipient = H160::repeat_byte(9);

		Assets::mint(ASSET, &sender, 500.into()).unwrap();

		assert_noop!(
			ETHApp::burn(
				Origin::signed(sender.clone()),
				ChannelId::Basic,
				recipient.clone(),
				20.into()
			),
			DispatchError::Other("some error!")
		);
	});
}
