const BigNumber = require('bignumber.js');
const rlp = require("rlp");
const { ethers } = require("ethers");

const assert = require('chai').assert;

const channelContracts = {
  basic: {
    inbound: artifacts.require("BasicInboundChannel"),
    outbound: artifacts.require("BasicOutboundChannel"),
  },
  incentivized: {
    inbound: artifacts.require("IncentivizedInboundChannel"),
    outbound: artifacts.require("IncentivizedOutboundChannel"),
  },
};

const confirmBasicChannelSend = (channelEvent, channelAddress, sendingAppAddress, expectedNonce = 0, expectedPayload) => {
  var abi = ["event Message(address source, uint64 nonce, bytes payload)"];
  var iface = new ethers.utils.Interface(abi);
  let decodedEvent = iface.decodeEventLog('Message(address,uint64,bytes)', channelEvent.data, channelEvent.topics);

  channelEvent.address.should.be.equal(channelAddress);
  decodedEvent.source.should.be.equal(sendingAppAddress);

  assert(decodedEvent.nonce.eq(ethers.BigNumber.from(expectedNonce)));
  if (expectedPayload) {
    decodedEvent.payload.should.be.equal(expectedPayload);
  }
};

const confirmIncentivizedChannelSend = (channelEvent, channelAddress, sendingAppAddress, expectedNonce = 0, expectedPayload) => {
  var abi = ["event Message(address source, uint64 nonce, bytes payload)"];
  var iface = new ethers.utils.Interface(abi);
  let decodedEvent = iface.decodeEventLog('Message(address,uint64,bytes)', channelEvent.data, channelEvent.topics);

  channelEvent.address.should.be.equal(channelAddress);
  decodedEvent.source.should.be.equal(sendingAppAddress);

  assert(decodedEvent.nonce.eq(ethers.BigNumber.from(expectedNonce)));
  if (expectedPayload) {
    decodedEvent.payload.should.be.equal(expectedPayload);
  }
};


const confirmUnlock = (rawEvent, ethAppAddress, expectedRecipient, expectedAmount) => {
  unlockLogFields = [
    {
      type: 'bytes32',
      name: 'sender'
    },
    {
      type: 'address',
      name: 'recipient'
    },
    {
      type: 'uint256',
      name: 'amount'
    }
  ];

  const decodedEvent = web3.eth.abi.decodeLog(unlockLogFields, rawEvent.data, rawEvent.topics);

  rawEvent.address.should.be.equal(ethAppAddress);
  decodedEvent.recipient.should.be.equal(expectedRecipient);
  decodedEvent.amount.should.be.bignumber.equal(expectedAmount);
};

const confirmUnlockTokens = (rawEvent, erc20AppAddress, expectedRecipient, expectedAmount) => {
  unlockLogFields = [
    {
      type: 'address',
      name: 'token'
    },
    {
      type: 'bytes32',
      name: 'sender'
    },
    {
      type: 'address',
      name: 'recipient'
    },
    {
      type: 'uint256',
      name: 'amount'
    }
  ];

  const decodedEvent = web3.eth.abi.decodeLog(unlockLogFields, rawEvent.data, rawEvent.topics);

  rawEvent.address.should.be.equal(erc20AppAddress);
  decodedEvent.recipient.should.be.equal(expectedRecipient);
  decodedEvent.amount.should.be.bignumber.equal(expectedAmount);
};

const confirmMessageDispatched = (rawEvent, expectedNonce, expectedResult) => {
  messageDispatchedLogFields = [{
    type: 'uint64',
    name: 'nonce'
  }, {
    type: 'bool',
    name: 'result'
  }];

  const decodedEvent = web3.eth.abi.decodeLog(messageDispatchedLogFields, rawEvent.data, rawEvent.topics);

  parseFloat(decodedEvent.nonce).should.be.equal(expectedNonce);
  decodedEvent.result.should.be.equal(expectedResult);
};

const buildCommitment = (messages) => {
  const byteSize = 32;
  const msgsLength = messages.length * byteSize;
  let headEncoding = ethers.utils.defaultAbiCoder.encode(
      [ 'uint256', 'uint256', 'uint256'],
      [ byteSize, messages.length, msgsLength ], // offset of first param, message count, messages length
  )
  let encodedMessages = headEncoding + buildMessagesHead(messages, msgsLength);

  for(message of messages) {
    const encodedMessage = encodeMessage(message);
    encodedMessages += encodedMessage.slice(2, encodedMessage.length);
  }

  return ethers.utils.solidityKeccak256(
      [ 'bytes'],
      [ encodedMessages ]
  );
}

const buildMessagesHead = (messages, msgsLength) => {
  let messagesHead = "";
  for(var i = 1; i < messages.length; i++) {
    // Encode padded byte count integer
    let encodedNum = ethers.utils.defaultAbiCoder.encode(
      [ 'uint256'],
      [ msgsLength ],
    )
    let formattedNum = encodedNum.slice(2, encodedNum.length);

    // Count zeros to determine insert index
    let z = 0;
    while(formattedNum.charAt(z) == "0") {
      z++;
    }
    let insertAt = (formattedNum.length - (64-z));

    // Splice message index number just before padded byte count
    let finalizedStr = formattedNum.substr(0, insertAt-1) + i + formattedNum.substr(insertAt, formattedNum.length);
    messagesHead += finalizedStr;
  }

  return messagesHead;
}

const encodeMessage = (message) => {
  return ethers.utils.defaultAbiCoder.encode(
    [ 'address', 'uint64', 'bytes' ],
    [ message.target, message.nonce, message.payload ]
  );
}

const deployAppContractWithChannels = async (appContract, ...appContractArgs) => {


  const channels = {
    basic: {
      inbound: await channelContracts.basic.inbound.new(),
      outbound: await channelContracts.basic.outbound.new(),
    },
    incentivized: {
      inbound: await channelContracts.incentivized.inbound.new(),
      outbound: await channelContracts.incentivized.outbound.new(),
    },
  };

  const app = await appContract.new(
    ...appContractArgs,
    {
      inbound: channels.basic.inbound.address,
      outbound: channels.basic.outbound.address,
    },
    {
      inbound: channels.incentivized.inbound.address,
      outbound: channels.incentivized.outbound.address,
    },
  );

  return [channels, app]
}

const addressBytes = (address) => Buffer.from(address.replace(/^0x/, ""), "hex");

const BASIC_CHANNEL = 0;
const INCENTIVIZED_CHANNEL = 1;

const ChannelId = {
  Basic: 0,
  Incentivized: 1,
}

const encodeLog = (log) => {
  return rlp.encode([log.address, log.topics, log.data]).toString("hex")
}

module.exports = {
  confirmBasicChannelSend,
  confirmIncentivizedChannelSend,
  confirmUnlock,
  confirmUnlockTokens,
  confirmMessageDispatched,
  deployAppContractWithChannels,
  addressBytes,
  ChannelId,
  buildCommitment,
};
