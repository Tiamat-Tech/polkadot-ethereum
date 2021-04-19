// Copyright 2020 Snowfork
// SPDX-License-Identifier: LGPL-3.0-only

package parachain

import (
	"context"
	"fmt"

	"golang.org/x/sync/errgroup"

	"github.com/sirupsen/logrus"
	"github.com/snowfork/go-substrate-rpc-client/v2/types"
	"github.com/snowfork/polkadot-ethereum/relayer/chain"
	"github.com/snowfork/polkadot-ethereum/relayer/chain/ethereum"
	"github.com/snowfork/polkadot-ethereum/relayer/crypto/sr25519"
	"github.com/snowfork/polkadot-ethereum/relayer/store"
)

type Chain struct {
	config   *Config
	listener *Listener
	writer   *Writer
	conn     *Connection
	log      *logrus.Entry
}

const Name = "Parachain"

func NewChain(config *Config) (*Chain, error) {
	log := logrus.WithField("chain", Name)

	// Generate keypair from secret
	kp, err := sr25519.NewKeypairFromSeed(config.PrivateKey, "")
	if err != nil {
		return nil, err
	}

	return &Chain{
		config:   config,
		conn:     NewConnection(config.Endpoint, kp.AsKeyringPair(), log),
		listener: nil,
		writer:   nil,
		log:      log,
	}, nil
}

func (ch *Chain) SetReceiver(ethMessages <-chan []chain.Message, ethHeaders <-chan chain.Header, _ chan<- store.DatabaseCmd) error {
	writer, err := NewWriter(ch.conn, ethMessages, ethHeaders, ch.log)
	if err != nil {
		return err
	}
	ch.writer = writer
	return nil
}

func (ch *Chain) SetSender(subMessages chan<- []chain.Message, _ chan<- chain.Header, _ chan<- store.DatabaseCmd) error {
	listener := NewListener(
		ch.config,
		ch.conn,
		subMessages,
		ch.log,
	)
	ch.listener = listener
	return nil
}

func (ch *Chain) Start(ctx context.Context, eg *errgroup.Group, ethInit chan<- chain.Init, subInit <-chan chain.Init) error {
	if ch.listener == nil && ch.writer == nil {
		return fmt.Errorf("Sender and/or receiver need to be set before starting chain")
	}

	err := ch.conn.Connect(ctx)
	if err != nil {
		return err
	}

	// Send init params to Ethereum chain
	err = ch.sendInitParams(ethInit)
	if err != nil {
		return err
	}

	startingBlocks, err := ch.receiveInitParams(subInit)

	if ch.listener != nil {
		ch.listener.Start(ctx, eg, startingBlocks[0], startingBlocks[1])
	}

	if ch.writer != nil {
		err = ch.writer.Start(ctx, eg)
		if err != nil {
			return err
		}
	}

	return nil
}

// Send init params to Substrate chain
func (ch *Chain) sendInitParams(ethInit chan<- chain.Init) error {
	headerID, err := ch.queryVerifierFinalizedBlock()
	if err != nil {
		return err
	}
	ch.log.WithFields(logrus.Fields{
		"blockNumber": headerID.Number,
		"blockHash":   headerID.Hash.Hex(),
	}).Info("Retrieved init params for Ethereum from Substrate")

	ethInit <- headerID
	close(ethInit)

	return nil
}

// Receive init params from Ethereum chain
func (ch *Chain) receiveInitParams(subInit <-chan chain.Init) (*[2]uint32, error) {
	holder, ok := <-subInit
	if !ok {
		return nil, fmt.Errorf("Channel is closed")
	}
	startingBlocks, ok := holder.([2]uint32)
	if !ok {
		return nil, fmt.Errorf("invalid starting params")
	}

	return &startingBlocks, nil
}

func (ch *Chain) Stop() {
	if ch.conn != nil {
		ch.conn.Close()
	}
}

func (ch *Chain) Name() string {
	return Name
}

func (ch *Chain) queryVerifierFinalizedBlock() (*ethereum.HeaderID, error) {
	storageKey, err := types.CreateStorageKey(&ch.conn.metadata, "VerifierLightclient", "FinalizedBlock", nil, nil)
	if err != nil {
		return nil, err
	}

	var headerID ethereum.HeaderID
	_, err = ch.conn.api.RPC.State.GetStorageLatest(storageKey, &headerID)
	if err != nil {
		return nil, err
	}

	nextHeaderID := ethereum.HeaderID{Number: types.NewU64(uint64(headerID.Number) + 1)}
	return &nextHeaderID, nil
}

func (ch *Chain) QueryCurrentEpoch() error {
	ch.log.Info("Creating storage key...")

	storageKey, err := types.CreateStorageKey(&ch.conn.metadata, "Babe", "Epoch", nil, nil)
	if err != nil {
		return err
	}

	ch.log.Info("Attempting to query current epoch...")

	// var headerID ethereum.HeaderID
	var epochData interface{}
	_, err = ch.conn.api.RPC.State.GetStorageLatest(storageKey, &epochData)
	if err != nil {
		return err
	}

	ch.log.Info("Retrieved current epoch data:", epochData)

	// nextHeaderID := ethereum.HeaderID{Number: types.NewU64(uint64(headerID.Number) + 1)}
	// return &nextHeaderID, nil

	return nil
}
