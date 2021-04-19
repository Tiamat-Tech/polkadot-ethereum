// SPDX-License-Identifier: MIT
pragma solidity >=0.7.6;
pragma experimental ABIEncoderV2;

import "@openzeppelin/contracts/math/SafeMath.sol";
import "./InboundChannel.sol";

contract BasicInboundChannel is InboundChannel {
    uint256 constant public MAX_GAS_PER_MESSAGE = 100000;

    constructor() {
        nonce = 0;
    }

    // TODO: Submit should take in all inputs required for verification,
    // including eg: _parachainBlockNumber, _parachainMerkleProof, parachainHeadsMMRProof
    function submit(uint32 _blockNumber, Message[] calldata _messages, bytes32 _commitment)
        public
        override
    {
        verifyMessages(_blockNumber, _messages, _commitment);
        processMessages(_blockNumber, _messages);
    }

    //TODO: verifyMessages should accept all needed proofs
    function verifyMessages(uint32 _blockNumber, Message[] calldata _messages, bytes32 _commitment)
        internal
        view
        returns (bool success)
    {
        // Prove we can get the MMRLeaf that is claimed to contain our Parachain Block Header
        // BEEFYLightClient.verifyMMRLeaf(parachainHeadsMMRProof)
        // BeefyLightClient{
        //   verifyMMRLeaf(parachainHeadsMMRProof) {
        //   MMRVerification.verifyInclusionProof(latestMMRRoot, parachainHeadsMMRProof)
        // }
        //}
        //}
        // returns mmrLeaf;

        // Prove we can get the claimed parachain block header from the MMRLeaf
        // allParachainHeadsMerkleTreeRoot = mmrLeaf.parachain_heads;
        // MerkeTree.verify(allParachainHeadsMerkleTreeRoot, ourParachainMerkleProof)
        // returns parachainBlockHeader

        // Prove that the commitment is in fact in the parachain block header
        // require(parachainBlockHeader.commitment == commitment)

        // Validate that the commitment matches the commitment contents
        require(
            validateMessagesMatchCommitment(_blockNumber, _messages, _commitment),
            "invalid commitment"
        );

        // Require there is enough gas to play all messages
        require(
            gasleft() >= _messages.length * MAX_GAS_PER_MESSAGE,
            "insufficient gas for delivery of all messages"
        );

        return true;
    }

    function processMessages(uint32 _blockNumber, Message[] calldata _messages) internal {
        blockNumber = _blockNumber;
        for (uint256 i = 0; i < _messages.length; i++) {
            // Check message nonce is correct and increment nonce for replay protection
            require(_messages[i].nonce == nonce + 1, "invalid nonce");

            nonce = nonce + 1;

            // Deliver the message to the target
            (bool success, ) =
                _messages[i].target.call{value: 0, gas: MAX_GAS_PER_MESSAGE}(_messages[i].payload);

            emit MessageDispatched(_messages[i].nonce, success);
        }
    }

    function validateMessagesMatchCommitment(
        uint32 _blockNumber,
        Message[] calldata _messages,
        bytes32 _commitment
    ) internal pure returns (bool) {
        return keccak256(abi.encode(_blockNumber, _messages)) == _commitment;
    }
}
