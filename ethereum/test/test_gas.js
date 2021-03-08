const ScaleCodec = artifacts.require('ScaleCodec');
const ETHApp = artifacts.require('ETHApp');
const ERC20App = artifacts.require('ERC20App');
const BasicOutboundChannel = artifacts.require('BasicOutboundChannel');
const IncentivizedOutboundChannel = artifacts.require('IncentivizedOutboundChannel');
const TestToken = artifacts.require('TestToken');

const BigNumber = web3.BigNumber;

const { lockupETH } = require('./test_eth_app');
const { lockupERC20 } = require('./test_erc20_app');
const { deployAppContractWithChannels, ChannelId } = require("./helpers");

require('chai')
  .use(require('chai-as-promised'))
  .use(require('chai-bignumber')(BigNumber))
  .should();

describe('Gas expenditures', function () {
  // Accounts
  let accounts;
  let owner;
  let userOne;

  // Constants
  const POLKADOT_ADDRESS = "38j4dG5GzsL1bw2U2AVgeyAk6QTxq43V7zPbdXAmbVLjvDCK"

  before(async function() {
    const codec = await ScaleCodec.new();
    ETHApp.link(codec);
    ERC20App.link(codec);
    accounts = await web3.eth.getAccounts();
    owner = accounts[0];
    userOne = accounts[1];
  });

  describe('Gas costs', function () {

    beforeEach(async function () {
      [, this.ethApp] = await deployAppContractWithChannels(ETHApp);
      [, this.erc20App] = await deployAppContractWithChannels(ERC20App);
    });

    it('lock eth gas usage', async function () {
      // Prepare transaction parameters
      const weiAmount = web3.utils.toWei("0.25", "ether");

      // Deposit Ethereum to the contract

      const result = await lockupETH(this.ethApp, userOne, POLKADOT_ADDRESS, weiAmount,
        ChannelId.Basic).should.be.fulfilled;

      console.log('\lock eth gas: ' + result.receipt.gasUsed);
    });

    // Set up an ERC20 token for testing token deposits
    before(async function () {
      this.symbol = "TEST";
      this.token = await TestToken.new(100000, "Test Token", this.symbol);

      // Load user account with 'TEST' ERC20 tokens
      await this.token.transfer(userOne, 1000, {
        from: owner
      }).should.be.fulfilled;
    });

    it('lock erc20 gas usage', async function () {
      // Prepare transaction parameters
      const amount = 100;

      // Approve tokens to contract
      await this.token.approve(this.erc20App.address, amount, {
        from: userOne
      }).should.be.fulfilled;

      const result = await lockupERC20(this.erc20App, this.token, userOne,
        POLKADOT_ADDRESS, amount, ChannelId.Basic).should.be.fulfilled;

      console.log('\lock erc20 gas: ' + result.receipt.gasUsed);
    });
  });
});
